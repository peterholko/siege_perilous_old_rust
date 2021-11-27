// Configure clippy for Bevy usage
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::enum_glob_use)]

use bevy::{
    app::{ScheduleRunnerPlugin, ScheduleRunnerSettings, Events},
    core::CorePlugin,
    prelude::*,
    tasks::IoTaskPool,
    utils::Duration,
};
use crossbeam_channel::{unbounded, Receiver as CBReceiver, Sender as CBSender};
use tokio::sync::mpsc::Sender;

use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::{Message, Result};
use tokio_tungstenite::{accept_async, tungstenite::Error};

use async_compat::Compat;

use serde::{Deserialize, Serialize};

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

mod map;

const TIMESTEP_5_PER_SECOND: f64 = 30.0 / 60.0;

type Clients = Arc<Mutex<HashMap<i32, Client>>>;
type Accounts = Arc<Mutex<HashMap<i32, Account>>>;

#[derive(Debug, Clone)]
pub struct Client {
    pub id: i32,
    pub sender: Sender<String>,
}

#[derive(Clone, Debug)]
struct Account {
    player_id: i32,
    username: String,
    password: String,
    class: HeroClass,
}

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
enum HeroClass {
    Warrior,
    Ranger,
    Mage,
    None
}

#[derive(Debug)]
struct MapObj {
    x: u32,
    y: u32,
    player_id: i32,
    name: String,
    template: String
}

#[derive(Clone, Debug)]
enum PlayerEvent {
    NewPlayer {player_id: i32},
    Move {player_id: i32, x: u32, y: u32}
}

#[derive(Debug)]
struct MoveEvent {
    id: i32, 
    player_id: i32,
    x: u32, 
    y: u32,
    run_at: i32
}

#[derive(Debug, Default)]
struct GameTick(i32);

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "cmd")]
enum NetworkPacket {
    #[serde(rename = "login")]
    Login {username: String, password: String},
    #[serde(rename = "select_class")]
    SelectedClass {classname: String},
    #[serde(rename = "move_unit")]
    Move {x: u32, y: u32}
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "packet")]
enum ResponsePacket {
    #[serde(rename = "select_class")]
    SelectClass {player: u32},
    #[serde(rename = "info_select_class")]
    InfoSelectClass {result: String},    
    PlayerMoved{player_id: i32, x: u32, y: u32},
    Ok,
    Error{errmsg: String}
}

fn main() {
    App::build()
        .insert_resource(ScheduleRunnerSettings::run_loop(Duration::from_secs_f64(
            TIMESTEP_5_PER_SECOND,
        )))
        .add_plugin(CorePlugin::default())
        .add_plugin(ScheduleRunnerPlugin::default())
        .init_resource::<GameTick>()
        .init_resource::<map::Map>()
        .init_resource::<Events<MoveEvent>>()
        .add_startup_system(setup.system())
        .add_system_to_stage(CoreStage::PreUpdate, update_game_tick.system())
        .add_system(message_system.system())
        .add_system(move_system.system())
        .run();
}

fn setup(mut commands: Commands, task_pool: Res<IoTaskPool>) {
    println!("Bevy Setup System");

    //Initialize Arc Mutex Hashmap to store the client to game channel per connected client
    let clients = Clients::new(Mutex::new(HashMap::new()));
    let accounts = Accounts::new(Mutex::new(HashMap::new()));
    
    //Add accounts
    let account = Account {
        player_id: 1,
        username: "peter".to_string(),
        password: "123123".to_string(),
        class: HeroClass::None
    };

    let account2 = Account {
        player_id: 2,
        username: "joe".to_string(),
        password: "123123".to_string(),
        class: HeroClass::None
    };    

    accounts.lock().unwrap().insert(1, account);
    accounts.lock().unwrap().insert(2, account2);

    //Create the client to game channel, note the sender will be cloned by each connected client
    let (client_to_game_sender, client_to_game_receiver) = unbounded::<PlayerEvent>();

    //Spawn the tokio runtime setup using a Compat with the clients and client to game channel
    task_pool
        .spawn(Compat::new(tokio_setup(
            client_to_game_sender,
            clients.clone(),
            accounts
        )))
        .detach();

    //Event loop tick 
    let game_tick = 0;

    //Insert the clients and client to game channel into the Bevy resources
    commands.insert_resource(clients);
    commands.insert_resource(client_to_game_receiver);
    commands.insert_resource(game_tick);
}

async fn tokio_setup(client_to_game_sender: CBSender<PlayerEvent>, clients: Clients, accounts: Accounts) {
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
            accounts.clone()
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
    if let Err(e) = handle_connection(peer, stream, client_to_game_sender, clients, accounts).await {
        match e {
            Error::ConnectionClosed | Error::Protocol(_) | Error::Utf8 => (),
            err => println!("Error processing connection: {}", err),
        }
    }
}

async fn handle_connection(
    peer: SocketAddr,
    stream: TcpStream,
    client_to_game_sender: CBSender<PlayerEvent>,
    clients: Clients,
    accounts: Accounts
) -> Result<()> {
    println!("New WebSocket connection: {}", peer);
    let ws_stream = accept_async(stream).await.expect("Failed to accept");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    //Create a tokio sync channel to for messages from the game to each client
    let (game_to_client_sender, mut game_to_client_receiver) = tokio::sync::mpsc::channel(100);

    //Get the number of clients for a client id
    let num_clients = clients.lock().unwrap().keys().len() as i32;

    //Store the incremented client id and the game to client sender in the clients hashmap
    clients.lock().unwrap().insert(
        num_clients + 1,
        Client {
            id: num_clients + 1,
            sender: game_to_client_sender,
        },
    );

    let mut player_id = -1;

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
                                println!("authenticated");
                                let res_packet: ResponsePacket = match serde_json::from_str(msg.to_text().unwrap()) {
                                    Ok(packet) => {
                                        match packet {
                                            NetworkPacket::SelectedClass{classname} => {
                                                handle_selected_class(player_id, classname, accounts.clone(), client_to_game_sender.clone())
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
            (player_id, ResponsePacket::SelectClass{player: player_id as u32})
        } else {
            //TODO replace with initial game state login packet
            (player_id, ResponsePacket::Ok)
        }
    } else if found_account && !password_match {
        (player_id, ResponsePacket::Error{errmsg: "Incorrect password".to_owned()})
    } else {
        (player_id, ResponsePacket::Ok)
    };

    ret
}

fn handle_selected_class(player_id: i32, classname: String, accounts: Accounts, client_to_game_sender: CBSender<PlayerEvent>) -> ResponsePacket {
    println!("handle_selected_class: {:?}", player_id);
    let mut accounts = accounts.lock().unwrap();
    println!("{:?}", accounts);
    let mut account = accounts.get_mut(&player_id).unwrap();

    if account.class == HeroClass::None {
        let selected_class = match classname.as_str() {
            "warrior" => HeroClass::Warrior,
            "ranger" => HeroClass::Ranger,
            "mage" => HeroClass::Mage,
            _ => HeroClass::None
        };

        account.class = selected_class;

        //Send new player event to game
        client_to_game_sender.send(PlayerEvent::NewPlayer {player_id: player_id}).expect("Could not send message");

        ResponsePacket::InfoSelectClass{result: "success".to_owned()}
    } else {
        ResponsePacket::Error{errmsg: "Hero class already selected.".to_owned()}
    }
    
}

fn handle_move(player_id: i32, x: u32, y: u32, client_to_game_sender: CBSender<PlayerEvent>) -> ResponsePacket {
    client_to_game_sender.send(PlayerEvent::Move {player_id: player_id, x: x, y: y}).expect("Could not send message");

    ResponsePacket::Ok
}

fn message_system(mut commands: Commands, clients: Res<Clients>, client_to_game_receiver: Res<CBReceiver<PlayerEvent>>, 
    mut events: ResMut<Events<MoveEvent>>, mut query: Query<(Entity, &mut MapObj)>) {
    //Broadcast a message to each connected client on each Bevy System iteration.
    for (_id, client) in clients.lock().unwrap().iter() {
        //println!("{:?}", client);
        client
            .sender
            .try_send("Broadcast message from Bevy System".to_string())
            .expect("Could not send message");
    }

    //Attempts to receive a message from the channel without blocking.
    if let Ok(evt) = client_to_game_receiver.try_recv() {
        println!("{:?}", evt);
        let res  = match evt { 
            PlayerEvent::NewPlayer{player_id} => {
                let map_obj = MapObj {
                    x: 0,
                    y: 0,
                    player_id: player_id,
                    name: "Hero".to_string(),
                    template: "Hero_Mage".to_string()
                };
            
                commands.spawn().insert(map_obj);

                {}
            }
            PlayerEvent::Move{player_id, x, y} => {
                println!("looking for obj");
                for (entity, mut map_obj) in query.iter_mut() { 
                    println!("{:?}", map_obj.player_id);
                    if map_obj.player_id == player_id {
                        println!("found player: {:?}", player_id);
                        events.send(MoveEvent{id: 1, player_id: player_id, x: x, y: y, run_at: 50});
                    }
                }
            }        
        }; 
    }
}

fn move_system(game_tick: Res<GameTick>, mut events: ResMut<Events<MoveEvent>>, mut query: Query<(Entity, &mut MapObj)>, clients: Res<Clients>) {
    println!("Game Tick {:?}", game_tick.0);

    let mut reader = events.get_reader();

    for event in reader.iter(&events) {
        println!("{:?}", event);
        if game_tick.0 == event.run_at {
            println!("Running event");

            for(entity, mut map_obj) in query.iter_mut() {
                if map_obj.player_id == event.player_id {
                    map_obj.x = event.x;
                    map_obj.y = event.y;

                    for (id, client) in clients.lock().unwrap().iter() {
                        if *id == 1 {
                            client
                                .sender
                                .try_send("Move completed, new perception".to_string())
                                .expect("Could not send message");
                        }
                    }
                }
            }
            
        }
    }
  
}

fn update_game_tick(mut game_tick: ResMut<GameTick>, query: Query<(Entity, &MapObj)>) {    
    game_tick.0 = game_tick.0 + 1;

    for(entity, map_obj) in query.iter() {
        println!("entity: {:?} map_obj: {:?}", entity, map_obj);
    }
}