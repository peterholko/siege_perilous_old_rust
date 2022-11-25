use crossbeam_channel::Sender as CBSender;

use std::{
    collections::HashMap,
    collections::HashSet,
    sync::{Arc, Mutex},
};

use lazy_static::lazy_static;

use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::{Message, Result};
use tokio_tungstenite::{accept_async, tungstenite::Error};

use serde::{Deserialize, Serialize};

use crate::game::{Account, Accounts, Client, Clients, HeroClass, PlayerEvent};
use crate::map::MapTile;

use std::path::Path;

use glob::glob;

pub type Tileset = HashMap<String, String>;

//pub struct Network; // Is this needed?

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "cmd")]
enum NetworkPacket {
    #[serde(rename = "login")]
    Login { username: String, password: String },
    #[serde(rename = "select_class")]
    SelectedClass { classname: String },
    #[serde(rename = "get_stats")]
    GetStats { id: i32 },
    #[serde(rename = "image_def")]
    ImageDef { name: String },
    #[serde(rename = "move_unit")]
    Move { x: i32, y: i32 },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "packet")]
pub enum ResponsePacket {
    #[serde(rename = "select_class")]
    SelectClass {
        player: u32,
    },
    #[serde(rename = "info_select_class")]
    InfoSelectClass {
        result: String,
    },
    #[serde(rename = "map")]
    Map {
        data: Vec<MapTile>,
    },
    #[serde(rename = "stats")]
    Stats {
        data: StatsData
    },
    #[serde(rename = "image_def")]
    ImageDef {
        name: String,
        data: serde_json::Value,
    },
    PlayerMoved {
        player_id: i32,
        x: i32,
        y: i32,
    },
    #[serde(rename = "perception")]
    Perception {
        data: PerceptionData,
    },
    #[serde(rename = "changes")]
    Changes {
        events: Vec<ChangeEvents>,
    },
    Ok,
    Error {
        errmsg: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PerceptionData {
    pub map: Vec<MapTile>,
    pub objs: Vec<MapObj>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ChangeEvents {
    ObjCreate{event: String, obj_id: i32, state: String},
    ObjUpdate{event: String, obj_id: i32, attr: String, value: String},
    ObjMove{event: String, obj_id: i32, src_x: i32, src_y: i32}
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StatsData {
    id: i32,
    hp: i32,
    base_hp: i32,
    stamina: i32,
    base_stamina: i32,
    effects: Vec<i32>,
}


#[derive(Debug, Clone, Deserialize, Serialize, Eq, Hash, PartialEq)]
pub struct MapObj {
    pub id: i32,
    pub player: i32,
    pub name: String,
    pub class: String,
    pub subclass: String,
    pub template: String,
    pub image: String,
    pub x: i32,
    pub y: i32,
    pub state: String,
    pub vision: u32,
    pub hsl: Vec<i32>,
    pub groups: Vec<i32>,
}

lazy_static! {
    static ref TILESET: HashMap<String, serde_json::Value> = {
        println!("Loading tilesets");
        let mut tileset = HashMap::new();

        // Load tilesets
        for entry in glob("./tileset/*.json").expect("Failed to read glob pattern") {
          match entry {
              Ok(path) => {
                let path = Path::new(&path);
                let file_stem = path.file_stem();
                let data = std::fs::read_to_string(&path).expect("Unable to read file");
                let json: serde_json::Value = serde_json::from_str(&data).expect("JSON does not have correct format.");
                //tileset.insert(file_stem.unwrap().to_str().unwrap().to_string(), serde_json::to_string(&json).unwrap());
                tileset.insert(file_stem.unwrap().to_str().unwrap().to_string(), json);
              },
              Err(e) => println!("{:?}", e),
          }
        }

        tileset
    };
}

// Initialize tileset hashmap

/* #[derive(Debug)]
struct Perception {
    data: Vec<MapTile>
} */

pub async fn tokio_setup(
    client_to_game_sender: CBSender<PlayerEvent>,
    clients: Clients,
    accounts: Accounts,
) {
    env_logger::init();

    let addr = "127.0.0.1:9002";
    let listener = TcpListener::bind(&addr).await.expect("Can't listen");
    println!("Listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        let peer = stream
            .peer_addr()
            .expect("connected streams should have a peer address");
        println!("Peer address: {}", peer);

        //Spawn a connection handler per client
        tokio::spawn(accept_connection(
            peer,
            stream,
            client_to_game_sender.clone(),
            clients.clone(),
            accounts.clone(),
        ));
    }

    println!("Finished");
}

async fn accept_connection(
    peer: SocketAddr,
    stream: TcpStream,
    client_to_game_sender: CBSender<PlayerEvent>,
    clients: Clients,
    accounts: Accounts,
) {
    if let Err(e) = handle_connection(peer, stream, client_to_game_sender, clients, accounts).await
    {
        match e {
            Error::ConnectionClosed | Error::Protocol(_) | Error::Utf8 => {
                println!("Connection closed")
            }
            err => println!("Error processing connection: {}", err),
        }
    }
}

async fn handle_connection(
    peer: SocketAddr,
    stream: TcpStream,
    client_to_game_sender: CBSender<PlayerEvent>,
    clients: Clients,
    accounts: Accounts,
) -> Result<()> {
    println!("New WebSocket connection: {}", peer);
    let ws_stream = accept_async(stream).await.expect("Failed to accept");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    //Create a tokio sync channel to for messages from the game to each client
    let (game_to_client_sender, mut game_to_client_receiver) = tokio::sync::mpsc::channel(100);

    //Get the number of clients for a client id
    let num_clients = clients.lock().unwrap().keys().len() as i32;

    //Client ID
    let client_id = num_clients + 1;
    
    let mut player_id = -1;

    //Store the incremented client id and the game to client sender in the clients hashmap
    clients.lock().unwrap().insert(
        client_id,
        Client {
            id: client_id,
            player_id: player_id,
            sender: game_to_client_sender,
        },
    );


    //This loop uses the tokio select! macro to receive messages from either the websocket receiver
    //or the game to client receiver
    loop {
        tokio::select! {
            //Receive messages from the websocket
            msg = ws_receiver.next() => {
                match msg {
                    Some(msg) => {
                        let msg = msg?;
                        if msg.is_text() || msg.is_binary() {

                            println!("player_id: {:?}", player_id);

                            //Check if the player is authenticated
                            if player_id == -1 {
                                //Attempt to login
                                let res_packet: ResponsePacket = match serde_json::from_str(msg.to_text().unwrap()) {
                                    Ok(packet) => {
                                        match packet {
                                            NetworkPacket::Login{username, password} => {
                                                println!("{:?}", username);
                                                //Retrieve player id, note will be set if authenticated
                                                let (pid, res) = handle_login(username, password, accounts.clone());

                                                //Set player_id
                                                player_id = pid;

                                                if let Some(client) = clients.lock().unwrap().get_mut(&client_id) {
                                                    (*client).player_id = player_id;
                                                }

                                                //Return packet
                                                res
                                            }
                                            _ => ResponsePacket::Error{errmsg: "Unknown packet".to_owned()}
                                        }
                                    },

                                    Err(_) => ResponsePacket::Error{errmsg: "Unknown packet".to_owned()}
                                };
                                println!("{:?}", res_packet);
                                //TODO send event to game
                                //client_to_game_sender.send(Message::text(res)).expect("Could not send message");

                                //Send response to client
                                let res = serde_json::to_string(&res_packet).unwrap();
                                ws_sender.send(Message::Text(res)).await?;
                            } else {
                                println!("Authenticated packet: {:?}", msg.to_text().unwrap());


                                let res_packet: ResponsePacket = match serde_json::from_str(msg.to_text().unwrap()) {
                                    Ok(packet) => {
                                        match packet {
                                            NetworkPacket::SelectedClass{classname} => {
                                                handle_selected_class(player_id, classname, accounts.clone(), client_to_game_sender.clone())
                                            }
                                            NetworkPacket::GetStats{id} => {
                                                println!("GetStats: {:?}", id);

                                                ResponsePacket::Stats{
                                                    data: StatsData {
                                                        id: 1,
                                                        hp: 100,
                                                        base_hp: 100,
                                                        stamina: 10000,
                                                        base_stamina: 10000,
                                                        effects: Vec::new()                                                    
                                                    }
                                                }
                                            }
                                            NetworkPacket::ImageDef{name} => {

                                                let mut name_stripped = name.clone();
                                                let raw_name = name;

                                                if name_stripped.chars().last().unwrap().is_numeric() {
                                                    name_stripped.pop();
                                                }

                                                ResponsePacket::ImageDef{
                                                    name: raw_name,
                                                    data: TILESET.get(&name_stripped).unwrap().clone()
                                                }
                                            }
                                            NetworkPacket::Move{x, y} => {
                                                handle_move(player_id, x, y, client_to_game_sender.clone())
                                            }
                                            _ => ResponsePacket::Ok
                                        }
                                    },
                                    Err(_) => ResponsePacket::Error{errmsg: "Unknown packet".to_owned()}
                                };

                                let res = serde_json::to_string(&res_packet).unwrap();
                                ws_sender.send(Message::Text(res)).await?;
                            }
                        } else if msg.is_close() {
                            println!("Message is closed for player: {:?}", player_id);
                            handle_disconnect(client_id, clients.clone());
                            break;
                        }
                    }
                    None => break,
                }
            }
            //Receive messages from the game
            game_msg = game_to_client_receiver.recv() => {
                let game_msg = game_msg.unwrap();
                ws_sender.send(Message::Text(game_msg)).await?;
            }
        }
    }
    Ok(())
}

fn handle_login(username: String, password: String, accounts: Accounts) -> (i32, ResponsePacket) {
    println!("handle_login");

    let mut found_account = false;
    let mut password_match = false;
    let mut player_id: i32 = -1;
    let mut account_class = HeroClass::None;

    let accounts = accounts.lock().unwrap();

    for (_, account) in accounts.iter() {
        if account.username == username {
            found_account = true;

            if account.password == password {
                password_match = true;
                player_id = account.player_id;
                account_class = account.class;
            }
        }
    }

    println!("found_account: {:?}", found_account);

    let ret = if found_account && password_match {
        if account_class == HeroClass::None {
            (
                player_id,
                ResponsePacket::SelectClass {
                    player: player_id as u32,
                },
            )
        } else {
            //TODO replace with initial game state login packet
            (player_id, ResponsePacket::Ok)
        }
    } else if found_account && !password_match {
        (
            player_id,
            ResponsePacket::Error {
                errmsg: "Incorrect password".to_owned(),
            },
        )
    } else {
        (player_id, ResponsePacket::Ok)
    };

    ret
}

fn handle_disconnect(client_id: i32, clients: Clients) {
    let mut clients = clients.lock().unwrap();

    clients.remove(&client_id);
}

fn handle_selected_class(
    player_id: i32,
    classname: String,
    accounts: Accounts,
    client_to_game_sender: CBSender<PlayerEvent>,
) -> ResponsePacket {
    println!("handle_selected_class: {:?}", player_id);
    let mut accounts = accounts.lock().unwrap();
    println!("{:?}", accounts);
    let mut account = accounts.get_mut(&player_id).unwrap();

    if account.class == HeroClass::None {
        let selected_class = match classname.as_str() {
            "warrior" => HeroClass::Warrior,
            "ranger" => HeroClass::Ranger,
            "mage" => HeroClass::Mage,
            _ => HeroClass::None,
        };

        account.class = selected_class;

        //Send new player event to game
        client_to_game_sender
            .send(PlayerEvent::NewPlayer {
                player_id: player_id,
            })
            .expect("Could not send message");

        ResponsePacket::InfoSelectClass {
            result: "success".to_owned(),
        }
    } else {
        ResponsePacket::Error {
            errmsg: "Hero class already selected.".to_owned(),
        }
    }
}

fn handle_move(
    player_id: i32,
    x: i32,
    y: i32,
    client_to_game_sender: CBSender<PlayerEvent>,
) -> ResponsePacket {
    client_to_game_sender
        .send(PlayerEvent::Move {
            player_id: player_id,
            x: x,
            y: y,
        })
        .expect("Could not send message");

    ResponsePacket::Ok
}
