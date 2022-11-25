use bevy::ecs::schedule::ShouldRun;
use bevy::{
    prelude::*,
    tasks::{IoTaskPool, Task},
};

use itertools::{Itertools, Update};
use serde_json::{Number, Value};

use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::{
    collections::HashMap,
    collections::HashSet,
    hash::Hash,
    sync::{Arc, Mutex},
};

use crossbeam_channel::{unbounded, Receiver as CBReceiver};
use tokio::sync::mpsc::Sender;

use async_compat::Compat;

use crate::map::{Map, MapPlugin, MapTile};
use crate::network;
use crate::network::ResponsePacket;

pub struct GamePlugin;

pub type Clients = Arc<Mutex<HashMap<i32, Client>>>;
pub type Accounts = Arc<Mutex<HashMap<i32, Account>>>;

#[derive(Debug, Default)]
struct GameTick(i32);

#[derive(Debug, Clone)]
pub struct Client {
    pub id: i32,
    pub player_id: i32,
    pub sender: Sender<String>,
}

#[derive(Clone, Debug)]
pub struct Account {
    pub player_id: i32,
    pub username: String,
    pub password: String,
    pub class: HeroClass,
}

#[derive(Debug, Component, Clone)]
struct Id(i32);

#[derive(Debug, Component)]
struct Position {
    x: i32,
    y: i32,
}

#[derive(Debug, Component)]
struct Hp(i32);

#[derive(Debug, Component)]
struct PlayerId(i32);

#[derive(Debug, Component)]
struct Name(String);

#[derive(Debug, Component)]
struct Template(String);

#[derive(Debug, Component)]
struct Class(String);

#[derive(Debug, Component)]
struct Subclass(String);

#[derive(Debug, Component)]
struct State(String);

#[derive(Debug, Component, Clone)]
struct Viewshed {
    entities: HashSet<i32>,
    range: u32,
}

#[derive(Debug, Component)]
struct Hero;

#[derive(Debug, Component)]
struct Misc {
    image: String,
    hsl: Vec<i32>,
    groups: Vec<i32>,
}

#[derive(Debug, Component)]
struct MoveEvent;

#[derive(Debug, Component)]
struct StateChangeEvent;

#[derive(Debug, Component)]
struct UpdateViewshed;

#[derive(Bundle)]
struct Obj {
    id: Id,
    player_id: PlayerId,
    position: Position,
    name: Name,
    template: Template,
    class: Class,
    subclass: Subclass,
    state: State,
    viewshed: Viewshed,
    misc: Misc,
}

#[derive(Debug, Default)]
struct VisibilityChanged(bool);

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum HeroClass {
    Warrior,
    Ranger,
    Mage,
    None,
}

// States
// const NONE: String = String::from("none");
// const MOVING: String = String::from("moving");

#[derive(Clone, Debug)]
pub enum PlayerEvent {
    NewPlayer { player_id: i32 },
    Move { player_id: i32, x: i32, y: i32 },
}

#[derive(Clone, Debug)]
struct MapEventId(i32);

#[derive(Clone, Debug)]
struct MapEvent {
    event_id: i32,
    entity_id: Entity,
    obj_id: i32,
    player_id: i32,
    pos_x: i32,
    pos_y: i32,
    run_tick: i32,
    map_event_type: MapEventType,
}

#[derive(Clone, Debug)]
enum MapEventType {
    NewObjEvent,
    MoveEvent { dst_x: i32, dst_y: i32 },
    StateChangeEvent { new_state: String },
}

#[derive(Clone, Debug)]
struct ExploredMap {
    player_id: i32,
    tiles: HashSet<i32>,
}

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(MapPlugin)
            .init_resource::<GameTick>()
            .add_startup_system(Game::setup)
            .add_system_to_stage(CoreStage::PreUpdate, update_game_tick)
            .add_system(message_system)
            .add_system(event_system)
            .add_system(processed_event_system)
            .add_system(perception_system);
        //.add_system(
        //    clear_visibility_system
        //        .before(visibility_system)
        //        .with_run_criteria(run_if_visibility_changed),
        //)
        //.add_system(visibility_system.with_run_criteria(run_if_visibility_changed));
        // .add_system(use_neighbour);
    }
}

#[derive(Debug, Clone)]
pub struct Game {
    pub num_players: u32,
}

#[derive(Component)]
struct NetworkHandler(Task<IoTaskPool>);

impl Game {
    // pub fn setup(mut commands: Commands, task_pool: Res<IoTaskPool>) {
    pub fn setup(mut commands: Commands) {
        println!("Bevy Setup System");

        // Initialize game tick
        let game_tick = 0;

        // Initialize game event id
        let map_event_id: MapEventId = MapEventId(0);

        // Initialize run visibility
        let visibility_changed = VisibilityChanged(false);

        // Initialize game events vector
        let map_events: HashMap<i32, MapEvent> = HashMap::new();
        let processed_map_events: Vec<MapEvent> = Vec::new();

        let perception_updates: HashSet<i32> = HashSet::new();

        //Initialize Arc Mutex Hashmap to store the client to game channel per connected client
        let clients = Clients::new(Mutex::new(HashMap::new()));
        let accounts = Accounts::new(Mutex::new(HashMap::new()));

        //Add accounts
        let account = Account {
            player_id: 1,
            username: "peter".to_string(),
            password: "123123".to_string(),
            class: HeroClass::None,
        };

        let account2 = Account {
            player_id: 2,
            username: "joe".to_string(),
            password: "123123".to_string(),
            class: HeroClass::None,
        };

        accounts.lock().unwrap().insert(1, account);
        accounts.lock().unwrap().insert(2, account2);

        //Create the client to game channel, note the sender will be cloned by each connected client
        let (client_to_game_sender, client_to_game_receiver) = unbounded::<PlayerEvent>();

        let thread_pool = IoTaskPool::get();

        //Spawn the tokio runtime setup using a Compat with the clients and client to game channel
        let task = thread_pool
            .spawn(Compat::new(network::tokio_setup(
                client_to_game_sender,
                clients.clone(),
                accounts,
            )))
            .detach();

        //commands.spawn().insert(NetworkHandler(task));

        //Insert the clients and client to game channel into the Bevy resources
        commands.insert_resource(clients);
        commands.insert_resource(client_to_game_receiver);
        commands.insert_resource(game_tick);
        commands.insert_resource(visibility_changed);
        commands.insert_resource(map_event_id);
        commands.insert_resource(map_events);
        commands.insert_resource(processed_map_events);
        commands.insert_resource(perception_updates);
    }
}

fn message_system(
    commands: Commands,
    game_tick: ResMut<GameTick>,
    clients: Res<Clients>,
    client_to_game_receiver: Res<CBReceiver<PlayerEvent>>,
    mut map_event_id: ResMut<MapEventId>,
    mut map_events: ResMut<HashMap<i32, MapEvent>>,
    map: Res<Map>,
    query: Query<(Entity, &Id, &Position, &PlayerId), With<Hero>>,
) {
    //Broadcast a message to each connected client on each Bevy System iteration.
    /* for (id, client) in clients.lock().unwrap().iter() {
        //println!("{:?}", client);
        match client.sender.try_send("Broadcast message from Bevy System".to_string()) {
            Ok(()) => println!("sent broadcast to {:?}", id),
            Err(e) => println!("error to send to {:?} - {:?}", id, e)
        }

            //.unwrap();
            //.expect("Could not send message");
    } */

    //Attempts to receive a message from the channel without blocking.
    if let Ok(evt) = client_to_game_receiver.try_recv() {
        println!("{:?}", evt);
        let res = match evt {
            PlayerEvent::NewPlayer { player_id } => {
                new_player(
                    player_id,
                    map,
                    commands,
                    map_event_id,
                    map_events,
                    game_tick,
                );
            }
            PlayerEvent::Move { player_id, x, y } => {
                println!("looking for obj");
                for (entity_id, obj_id, pos, obj_player_id) in query.iter() {
                    println!("Move for PlayerId: {:?}", player_id);
                    if player_id == obj_player_id.0 {
                        println!("found player: {:?}", player_id);

                        // Insert state change event
                        let state_change_event = MapEventType::StateChangeEvent {
                            new_state: "moving".to_string(),
                        };
                        let map_state_event = MapEvent {
                            event_id: map_event_id.0,
                            entity_id: entity_id,
                            obj_id: obj_id.0,
                            player_id: player_id,
                            pos_x: pos.x,
                            pos_y: pos.y,
                            run_tick: game_tick.0,
                            map_event_type: state_change_event,
                        };

                        map_events.insert(map_event_id.0.try_into().unwrap(), map_state_event);

                        map_event_id.0 = map_event_id.0 + 1;

                        // Insert move event
                        let move_event = MapEventType::MoveEvent { dst_x: x, dst_y: y };
                        let map_move_event = MapEvent {
                            event_id: map_event_id.0,
                            entity_id: entity_id,
                            obj_id: obj_id.0,
                            player_id: player_id,
                            pos_x: pos.x,
                            pos_y: pos.y,
                            run_tick: 40,
                            map_event_type: move_event,
                        };

                        map_events.insert(map_event_id.0.try_into().unwrap(), map_move_event);

                        map_event_id.0 = map_event_id.0 + 1;
                    }
                }
            }
        };
    }
}

fn event_system(
    mut commands: Commands,
    game_tick: Res<GameTick>,
    clients: Res<Clients>,
    mut map_events: ResMut<HashMap<i32, MapEvent>>,
    mut processed_map_events: ResMut<Vec<MapEvent>>,
    mut perception_updates: ResMut<HashSet<i32>>,
    mut set: ParamSet<(
        Query<(&Id, &PlayerId, &mut Position, &mut State, &Viewshed)>, // p0 mutable for the event processing
        Query<(
            &Id,
            &PlayerId,
            &Position,
            &Name,
            &Template,
            &Class,
            &Subclass,
            &State,
            &Viewshed,
            &Misc,
        )>, // p1 immutable for looking up other entities
    )>,
) {
    println!("Game Tick {:?}", game_tick.0);

    let mut events_to_remove = Vec::new();

    let mut perceptions: HashMap<i32, HashSet<network::MapObj>> = HashMap::new();

    for (map_event_id, map_event) in map_events.iter_mut() {
        println!("Processing {:?}", map_event);

        if map_event.run_tick < game_tick.0 {
            // Execute event
            match &map_event.map_event_type {
                MapEventType::NewObjEvent => {
                    println!("Processing NewObjEvent");

                    /*if let Ok(viewshed) = set.p0().get_component::<Viewshed>(map_event.entity_id) {
                        let new_obj_range = viewshed.range;

                        for (
                            id,
                            player_id,
                            pos,
                            name,
                            template,
                            class,
                            subclass,
                            state,
                            viewshed,
                            misc,
                        ) in set.p1().iter()
                        {
                            let distance =
                                Map::distance((map_event.pos_x, map_event.pos_y), (pos.x, pos.y));

                            if new_obj_range >= distance {
                                println!("Add to new object's perception");

                                let visible_obj = network_obj(
                                    id.0,
                                    player_id.0,
                                    pos.x,
                                    pos.y,
                                    name.0.to_owned(),
                                    template.0.to_owned(),
                                    class.0.to_owned(),
                                    subclass.0.to_owned(),
                                    state.0.to_owned(),
                                    viewshed.range,
                                    misc.image.to_owned(),
                                    misc.hsl.to_owned(),
                                    misc.groups.to_owned(),
                                );

                                perceptions
                                    .entry(player_id.0)
                                    .or_default()
                                    .insert(visible_obj);
                            }
                        }
                    }*/

                    perception_updates.insert(map_event.player_id);

                    processed_map_events.push(map_event.clone());
                    events_to_remove.push(*map_event_id);
                }
                MapEventType::MoveEvent { dst_x, dst_y } => {
                    println!("Processing MoveEvent");

                    // Check if destination is open
                    let mut is_dst_open = true;

                    for (
                        _id,
                        player_id,
                        pos,
                        _name,
                        _template,
                        _class,
                        _subclass,
                        _state,
                        _viewshed,
                        _misc,
                    ) in set.p1().iter()
                    {
                        if (map_event.player_id != player_id.0)
                            && (pos.x == *dst_x && pos.y == *dst_y)
                        {
                            is_dst_open = false;
                        }
                    }

                    if is_dst_open {
                        // Get entity and update state
                        if let Ok((id, player_id, mut pos, mut state, _viewshed)) =
                            set.p0().get_mut(map_event.entity_id)
                        {
                            pos.x = *dst_x;
                            pos.y = *dst_y;

                            println!("Adding processed map event");
                            processed_map_events.push(map_event.clone());
                        }
                    }

                    events_to_remove.push(*map_event_id);
                }
                MapEventType::StateChangeEvent { new_state } => {
                    println!("Processing StateChangeEvent");

                    // Get entity and update state
                    if let Ok((id, playerId, mut pos, mut state, _viewshed)) =
                        set.p0().get_mut(map_event.entity_id)
                    {
                        state.0 = new_state.to_string();

                        println!("Adding processed map event");
                        processed_map_events.push(map_event.clone());
                    }

                    events_to_remove.push(*map_event_id);
                }
            }
        }
    }

    for event_id in events_to_remove.iter() {
        map_events.remove(event_id);
    }

    /* for (player_id, perception) in perceptions.iter_mut() {
        let perception_data = network::PerceptionData {
            map: Vec::new(),
            objs: perception.clone().into_iter().collect(),
        };

        let perception_packet = ResponsePacket::Perception {
            data: perception_data,
        };

        for (_client_id, client) in clients.lock().unwrap().iter() {
            println!("Player: {:?} == client: {:?}", player_id, client);
            if client.player_id == *player_id {
                client
                    .sender
                    .try_send(serde_json::to_string(&perception_packet).unwrap())
                    .expect("Could not send message");
            }
        }
    } */
}

fn processed_event_system(
    clients: Res<Clients>,
    mut processed_map_events: ResMut<Vec<MapEvent>>,
    query: Query<(&Id, &PlayerId, &Position, &State, &Viewshed)>,
) {
    let mut all_change_events: HashMap<i32, HashSet<network::ChangeEvents>> = HashMap::new();

    for map_event in processed_map_events.iter() {
        println!("Checking if processed map event is visible...");

        for (id, player_id, pos, state, viewshed) in query.iter() {
            match &map_event.map_event_type {
                MapEventType::NewObjEvent => {
                    let distance =
                        Map::distance((map_event.pos_x, map_event.pos_y), (pos.x, pos.y));

                    if viewshed.range >= distance {
                        println!("Send obj create to client");

                        let change_event = network::ChangeEvents::ObjCreate {
                            event: "obj_create".to_string(),
                            obj_id: map_event.obj_id,
                            state: "none".to_string(),
                        };

                        all_change_events
                            .entry(player_id.0)
                            .or_default()
                            .insert(change_event);
                    }
                }
                MapEventType::MoveEvent { dst_x, dst_y } => {
                    let src_distance =
                        Map::distance((map_event.pos_x, map_event.pos_y), (pos.x, pos.y));

                    if viewshed.range >= src_distance {
                        let change_event = network::ChangeEvents::ObjMove {
                            event: "obj_move".to_string(),
                            obj_id: map_event.obj_id,
                            src_x: map_event.pos_x,
                            src_y: map_event.pos_y,
                        };

                        all_change_events
                            .entry(player_id.0)
                            .or_default()
                            .insert(change_event);
                    }

                    let dst_distance = Map::distance((*dst_x, *dst_y), (pos.x, pos.y));

                    if viewshed.range >= dst_distance {
                        let change_event = network::ChangeEvents::ObjMove {
                            event: "obj_move".to_string(),
                            obj_id: map_event.obj_id,
                            src_x: *dst_x,
                            src_y: *dst_y,
                        };

                        all_change_events
                            .entry(player_id.0)
                            .or_default()
                            .insert(change_event);
                    }
                }
                MapEventType::StateChangeEvent { new_state } => {
                    let distance =
                        Map::distance((map_event.pos_x, map_event.pos_y), (pos.x, pos.y));

                    if viewshed.range >= distance {
                        println!("Send obj update to client");

                        let change_event = network::ChangeEvents::ObjUpdate {
                            event: "obj_update".to_string(),
                            obj_id: map_event.obj_id,
                            attr: "state".to_string(),
                            value: new_state.clone(),
                        };

                        all_change_events
                            .entry(player_id.0)
                            .or_default()
                            .insert(change_event);
                    }
                }
            }
        }
    }

    for (player_id, change_events) in all_change_events.iter_mut() {
        let changes_packet = ResponsePacket::Changes {
            events: change_events.clone().into_iter().collect(),
        };

        for (_client_id, client) in clients.lock().unwrap().iter() {
            println!("Player: {:?} == client: {:?}", player_id, client);
            if client.player_id == *player_id {
                client
                    .sender
                    .try_send(serde_json::to_string(&changes_packet).unwrap())
                    .expect("Could not send message");
            }
        }
    }

    processed_map_events.clear();
}

fn dedup<T: Eq + Hash + Copy>(v: &mut Vec<T>) {
    // note the Copy constraint
    let mut uniques = HashSet::new();
    v.retain(|e| uniques.insert(*e));
}

fn perception_system(
    map: Res<Map>,
    clients: Res<Clients>,
    mut perception_updates: ResMut<HashSet<i32>>,
    query: Query<(
        &Id,
        &PlayerId,
        &Position,
        &Name,
        &Template,
        &Class,
        &Subclass,
        &State,
        &Viewshed,
        &Misc,
    )>,
) {
    let mut perceptions_to_send: HashMap<i32, HashSet<network::MapObj>> = HashMap::new();
    // Could use HashSet here due to the trait `FromIterator<&std::collections::HashSet<(i32, i32)>>` is not implemented for `Vec<(i32, i32)>`
    let mut tiles_to_send: HashMap<i32, Vec<(i32, i32)>> = HashMap::new();

    println!("Perceptions to update: {:?}", perception_updates);

    for perception_player in perception_updates.iter() {
        for [obj1, obj2] in query.iter_combinations() {
            let (id1, player1, pos1, name1, template1, class1, subclass1, state1, viewshed1, misc1) =
                obj1;
            let (id2, player2, pos2, name2, template2, class2, subclass2, state2, viewshed2, misc2) =
                obj2;

            // Check if obj1 is owned by perception_player
            if *perception_player == player1.0 {
                let distance = Map::distance((pos1.x, pos1.y), (pos2.x, pos2.y));

                if viewshed1.range >= distance {
                    println!("Adding visible obj to percetion");

                    let visible_obj = network_obj(
                        id2.0,
                        player2.0,
                        pos2.x,
                        pos2.y,
                        name2.0.to_owned(),
                        template2.0.to_owned(),
                        class2.0.to_owned(),
                        subclass2.0.to_owned(),
                        state2.0.to_owned(),
                        viewshed2.range,
                        misc2.image.to_owned(),
                        misc2.hsl.to_owned(),
                        misc2.groups.to_owned(),
                    );

                    perceptions_to_send
                        .entry(*perception_player)
                        .or_default()
                        .insert(visible_obj);
                }

                // Get visible tiles by player owned obj
                let visible_tiles_pos = Map::range((pos1.x, pos1.y), viewshed1.range);

                tiles_to_send
                    .entry(*perception_player)
                    .or_default()
                    .extend(visible_tiles_pos);
            }

            // Check if obj2 is owned by perception_player
            if *perception_player == player2.0 {
                let distance = Map::distance((pos1.x, pos1.y), (pos2.x, pos2.y));

                if viewshed2.range >= distance {
                    println!("Adding visible obj to percetion");

                    let visible_obj = network_obj(
                        id1.0,
                        player1.0,
                        pos1.x,
                        pos1.y,
                        name1.0.to_owned(),
                        template1.0.to_owned(),
                        class1.0.to_owned(),
                        subclass1.0.to_owned(),
                        state1.0.to_owned(),
                        viewshed1.range,
                        misc1.image.to_owned(),
                        misc1.hsl.to_owned(),
                        misc1.groups.to_owned(),
                    );

                    perceptions_to_send
                        .entry(*perception_player)
                        .or_default()
                        .insert(visible_obj);
                }

                // Get visible tiles by player owned obj
                let visible_tiles_pos = Map::range((pos2.x, pos2.y), viewshed2.range);

                tiles_to_send
                    .entry(*perception_player)
                    .or_default()
                    .extend(visible_tiles_pos);
            }
        }

        for (player_id, perception) in perceptions_to_send.iter_mut() {
            println!("Perceptions to send player: {:?} perception: {:?}", player_id, perception);
            let mut visible_tiles: &mut Vec<(i32, i32)> = tiles_to_send.get_mut(player_id).unwrap();

            dedup(&mut visible_tiles);

            let tiles = Map::pos_to_tiles(&visible_tiles.clone(), &map); // Used for network obj

            let perception_data = network::PerceptionData {
                map: tiles,
                objs: perception.clone().into_iter().collect(),
            };

            let perception_packet = ResponsePacket::Perception {
                data: perception_data,
            };

            for (_client_id, client) in clients.lock().unwrap().iter() {
                println!("Player: {:?} == client: {:?}", player_id, client);
                if client.player_id == *player_id {
                    client
                        .sender
                        .try_send(serde_json::to_string(&perception_packet).unwrap())
                        .expect("Could not send message");
                }
            }
        }
    }

    perception_updates.clear();
}

/* fn event_system(
    game_tick: Res<GameTick>,
    mut visibility_changed: ResMut<VisibilityChanged>,
    mut game_events: ResMut<HashMap<i32, GameEvent>>,
    clients: Res<Clients>,
    mut query: Query<(&Id, &PlayerId, &mut Position, &mut State, &mut Viewshed)>,
) {
    println!("Game Tick {:?}", game_tick.0);

    // local system variable might be faster than setting the resource on every move event
    let mut processed_move = false;

    let mut events_to_remove = Vec::new();

    for (game_event_id, game_event) in game_events.iter_mut() {
        println!("Processing {:?}", game_event);

        if game_event.run_tick < game_tick.0 {
            // Execute event
            match &game_event.game_event_type {
                GameEventType::NewObj { new_obj_id } => {
                    // Loop through all entities viewsheds to check if they can see the
                    for (id, player_id, mut pos, mut obj_state, mut viewshed) in query.iter_mut() {
                        if viewshed.entities.contains(&id.0) {
                            println!("{:?} can see {:?} state change to {:?}", id.0, game_event.obj_id, new_state);

                            let change_event = network::ChangeEvents::ObjUpdate { event: "obj_update".to_string(), obj_id: game_event.obj_id, attr: "state".to_string(), value: new_state.clone()};

                            let mut change_events = Vec::new();
                            change_events.push(change_event);

                            let changes_packet = ResponsePacket::Changes {
                                events: change_events
                            };

                            for (_client_id, client) in clients.lock().unwrap().iter() {
                                println!("Player: {:?} == client: {:?}", player_id, client);
                                if client.player_id == player_id.0 {
                                    client
                                        .sender
                                        .try_send(serde_json::to_string(&changes_packet).unwrap())
                                        .expect("Could not send message");
                                }
                            }
                        }

                    }
                },
                GameEventType::MoveEvent { src_x, src_y, dst_x, dst_y} => {

                    for (id, player_id, mut pos, mut obj_state, mut viewshed) in query.iter_mut() {

                        // Update the entity's position
                        if game_event.obj_id == id.0 {
                            pos.x = *dst_x;
                            pos.y = *dst_y;
                            obj_state.0 = "none".to_string();

                            events_to_remove.push(*game_event_id);

                            // visibility_changed.0 = true;
                        }

                        // Check if the moving entity was visible by the observer and has moved out of range
                        if viewshed.entities.contains(&game_event.obj_id) {
                            let distance = Map::distance((pos.x, pos.y), (*dst_x, *dst_y));

                            if viewshed.range < distance {
                                // Remove entity as it is out of range
                                println!("{:?} is no longer observing {:?}", id.0, game_event.obj_id);
                                viewshed.entities.remove(&game_event.obj_id);
                            }

                        } else {
                            // Check if the moving entity has entered the range of any observer
                            let distance = Map::distance((pos.x, pos.y), (*dst_x, *dst_y));

                            if viewshed.range >= distance {
                                // Moved in range
                                println!("{:?} is now observing {:?}", id.0, game_event.obj_id);
                                viewshed.entities.insert(game_event.obj_id);


                                let change_event = network::ChangeEvents::ObjMove { event: "obj_move".to_string(), obj_id: game_event.obj_id, src_x: *src_x, src_y: *src_y };

                                let mut change_events = Vec::new();
                                change_events.push(change_event);

                                let changes_packet = ResponsePacket::Changes {
                                    events: change_events
                                };

                                for (_client_id, client) in clients.lock().unwrap().iter() {
                                    println!("Player: {:?} == client: {:?}", player_id, client);
                                    if client.player_id == player_id.0 {
                                        client
                                            .sender
                                            .try_send(serde_json::to_string(&changes_packet).unwrap())
                                            .expect("Could not send message");
                                    }
                                }
                            }
                        }

                    }

                },
                GameEventType::StateChangeEvent { new_state } => {
                    for (id, player_id, mut pos, mut obj_state, mut viewshed) in query.iter_mut() {

                        // Update the obj's state
                        if game_event.obj_id == id.0 {
                            obj_state.0 = new_state.clone();

                            events_to_remove.push(*game_event_id);

                            // visibility_changed.0 = true;
                        }

                        println!("StateChangeEvent viewshed of {:?}: {:?}", id, viewshed);
                        // Check if entity with the state change is visible by the observer
                        if viewshed.entities.contains(&id.0) {
                            println!("{:?} can see {:?} state change to {:?}", id.0, game_event.obj_id, new_state);

                            let change_event = network::ChangeEvents::ObjUpdate { event: "obj_update".to_string(), obj_id: game_event.obj_id, attr: "state".to_string(), value: new_state.clone()};

                            let mut change_events = Vec::new();
                            change_events.push(change_event);

                            let changes_packet = ResponsePacket::Changes {
                                events: change_events
                            };

                            for (_client_id, client) in clients.lock().unwrap().iter() {
                                println!("Player: {:?} == client: {:?}", player_id, client);
                                if client.player_id == player_id.0 {
                                    client
                                        .sender
                                        .try_send(serde_json::to_string(&changes_packet).unwrap())
                                        .expect("Could not send message");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    for event_id in events_to_remove.iter() {
        game_events.remove(event_id);
    }

    //let mut reader = events.get_reader();

    /*for event in reader.iter(&events) {
        println!("{:?}", event);
        if game_tick.0 == event.run_at {
            println!("Running event");

            for (entity, mut pos, mut viewshed, player_id, id) in query.iter_mut() {
                if id.0 == 1 {
                    pos.x = event.x;
                    pos.y = event.y;

                    let tiles = Map::range((pos.x, pos.y), viewshed.range);
                    viewshed.tiles = tiles;

                    println!("viewshed: {:?}", viewshed);

                    processed_move = true;

                    /* let tiles = Map::get_neighbour_tiles(event.x, event.y, 2, map.clone());

                    let res_packet = ResponsePacket::Perception{data: tiles};

                    println!("{:?}", res_packet);

                    for (id, client) in clients.lock().unwrap().iter() {
                      // Need to lookup player_id <=> client_id
                        if *id == 1 {
                            client
                                .sender
                                .try_send(serde_json::to_string(&res_packet).unwrap())
                                .expect("Could not send message");
                        }
                    } */
                }
            }
        }
    }*/
} */

/* fn visibility_system(
    mut visibility_changed: ResMut<VisibilityChanged>,
    map: Res<Map>,
    clients: Res<Clients>,
    mut set: ParamSet<(
        Query<&Id, &PlayerId, &Position,
        Query<&mut Health, With<Player>>,
        &World,
    )>
) {
    visibility_changed.0 = false;
} */

/*fn visibility_system(
    mut visibility_changed: ResMut<VisibilityChanged>,
    map: Res<Map>,
    clients: Res<Clients>,
    mut query: Query<(
        &Id,
        &PlayerId,
        &Position,
        &Name,
        &Template,
        &Class,
        &Subclass,
        &State,
        &mut Viewshed,
        &Misc,
    )>,
) {
    println!("visibility_system");

    let mut iter = query.iter_combinations_mut();

    let mut perception_update: HashMap<i32, Vec<network::MapObj>> = HashMap::new();

    while let Some(
        [(id1, player1, pos1, name1, template1, class1, subclass1, state1, mut viewshed1, misc1), (id2, player2, pos2, name2, template2, class2, subclass2, state2, mut viewshed2, misc2)],
    ) = iter.fetch_next()
    {
        println!(
            "{:?} - {:?} distance: {:?}",
            name1,
            name2,
            Map::distance((pos1.x, pos1.y), (pos2.x, pos2.y))
        );

        let distance = Map::distance((pos1.x, pos1.y), (pos2.x, pos2.y));

        if viewshed1.range >= distance {
            let visible_obj = network_obj(
                id2.0,
                player2.0,
                pos2.x,
                pos2.y,
                name2.0.to_owned(),
                template2.0.to_owned(),
                class2.0.to_owned(),
                subclass2.0.to_owned(),
                state2.0.to_owned(),
                viewshed2.range,
                misc2.image.to_owned(),
                misc2.hsl.to_owned(),
                misc2.groups.to_owned(),
            );

            viewshed1.entities.insert(id1.0);

            perception_update
                .entry(player1.0)
                .or_default()
                .push(visible_obj);
        }

        if viewshed2.range >= distance {
            let visible_obj = network_obj(
                id2.0,
                player2.0,
                pos2.x,
                pos2.y,
                name2.0.to_owned(),
                template2.0.to_owned(),
                class2.0.to_owned(),
                subclass2.0.to_owned(),
                state2.0.to_owned(),
                viewshed2.range,
                misc2.image.to_owned(),
                misc2.hsl.to_owned(),
                misc2.groups.to_owned(),
            );

            viewshed2.entities.insert(id2.0);

            perception_update
                .entry(player2.0)
                .or_default()
                .push(visible_obj);
        }

        /*let mut visible_objs: Vec<network::MapObj> = Vec::new();

        println!("id: {:?} player1: {:?} name: {:?} viewshed1: {:?}", id1, player1, name1);

        for tile in viewshed1.tiles.iter() {

            println!("viewshed1 tile: {:?}", tile);
            let (x, y) = tile;
            if (*x == pos2.x) && (*y == pos2.y) {


                let network_visible_obj = network_obj(
                    id2.0,
                    player2.0,
                    pos2.x,
                    pos2.y,
                    name2.0.to_owned(),
                    template2.0.to_owned(),
                    class2.0.to_owned(),
                    subclass2.0.to_owned(),
                    state2.0.to_owned(),
                    viewshed2.range,
                    misc2.image.to_owned(),
                    misc2.hsl.to_owned(),
                    misc2.groups.to_owned(),
                );

                println!("network_visible_obj: {:?}", network_visible_obj);

                visible_objs.push(network_visible_obj);
            }
        }

        // Send perception to client
        let tiles = Map::pos_to_tiles(&viewshed1.tiles, &map); // Used for network obj

        let perception_data = network::PerceptionData {
            map: tiles,
            objs: visible_objs,
        };

        let perception_packet = ResponsePacket::Perception {
            data: perception_data,
        };

        for (id, client) in clients.lock().unwrap().iter() {
              if *id == player1.0 {
                  client
                      .sender
                      .try_send(serde_json::to_string(&perception_packet).unwrap())
                      .expect("Could not send message");
              }
          }*/
    }

    for (id, player, pos, name, template, class, subclass, state, viewshed, misc) in query.iter() {
        println!("{:?}'s viewshed entites: {:?}", name, viewshed.entities);

        // Send perception to client
        /*let tiles = Map::pos_to_tiles(&viewshed.tiles, &map); // Used for network obj

        let perception_data = network::PerceptionData {
            map: tiles,
            objs: viewshed.entities.clone(), // TODO might not be efficient for large number of objects
        };

        let perception_packet = ResponsePacket::Perception {
            data: perception_data,
        };

        for (_client_id, client) in clients.lock().unwrap().iter() {
            println!("Player: {:?} == client: {:?}", player, client);
              if client.player_id == player.0 {
                  client
                      .sender
                      .try_send(serde_json::to_string(&perception_packet).unwrap())
                      .expect("Could not send message");
              }
        }*/
    }

    for (player, visible_objs) in perception_update.iter() {
        println!("player: {:?} visible_objs: {:?}", player, visible_objs);

        // let tiles = Map::pos_to_tiles(&viewshed.tiles, &map); // Used for network obj

        let perception_data = network::PerceptionData {
            map: Vec::new(),
            objs: visible_objs.iter().cloned().unique().collect()
        };

        let perception_packet = ResponsePacket::Perception {
            data: perception_data,
        };


        for (_client_id, client) in clients.lock().unwrap().iter() {
            println!("Player: {:?} == client: {:?}", player, client);
            if client.player_id == *player {
                client
                    .sender
                    .try_send(serde_json::to_string(&perception_packet).unwrap())
                    .expect("Could not send message");
            }
        }
    }

    visibility_changed.0 = false;
}*/

//fn update_game_tick(mut game_tick: ResMut<GameTick>, query: Query<(Entity, &MapObj)>) {
fn update_game_tick(
    mut game_tick: ResMut<GameTick>,
    query: Query<(Entity, &Id, &Name, &Position)>,
) {
    game_tick.0 = game_tick.0 + 1;

    /*for (entity, id, name, pos) in query.iter() {
        println!("id: {:?} name: {:?} name: {:?}", id, name, pos);
    }*/
}

fn new_player(
    player_id: i32,
    map: Res<Map>,
    mut commands: Commands,
    mut map_event_id: ResMut<MapEventId>,
    mut map_events: ResMut<HashMap<i32, MapEvent>>,
    game_tick: ResMut<GameTick>,
) {
    let start_x = 16;
    let start_y = 36;
    let range = 2;

    let hero = Obj {
        id: Id(1),
        player_id: PlayerId(player_id),
        position: Position {
            x: start_x,
            y: start_y,
        },
        name: Name("Peter".into()),
        template: Template("Novice Warrior".into()),
        class: Class("unit".into()),
        subclass: Subclass("hero".into()),
        state: State("none".into()),
        viewshed: Viewshed {
            entities: HashSet::new(),
            range: range,
        },
        misc: Misc {
            image: "novicewarrior".into(),
            hsl: Vec::new().into(),
            groups: Vec::new().into(),
        },
    };

    let heroEntityId = commands.spawn().insert_bundle(hero).insert(Hero).id();

    // Insert new obj event
    let new_obj_event = MapEventType::NewObjEvent;

    let map_state_event = MapEvent {
        event_id: map_event_id.0,
        entity_id: heroEntityId,
        obj_id: 1,
        player_id: player_id,
        pos_x: start_x,
        pos_y: start_y,
        run_tick: game_tick.0 + 1, // Add one game tick
        map_event_type: new_obj_event,
    };

    map_events.insert(map_event_id.0.try_into().unwrap(), map_state_event);

    map_event_id.0 = map_event_id.0 + 1;

    let villager = Obj {
        id: Id(2),
        player_id: PlayerId(player_id),
        position: Position { x: 16, y: 35 },
        name: Name("Villager 1".into()),
        template: Template("Human Villager".into()),
        class: Class("unit".into()),
        subclass: Subclass("villager".into()),
        state: State("none".into()),
        viewshed: Viewshed {
            entities: HashSet::new(),
            range: 2,
        },
        misc: Misc {
            image: "humanvillager1".into(),
            hsl: Vec::new().into(),
            groups: Vec::new().into(),
        },
    };

    let villagerEntityId = commands.spawn().insert_bundle(villager).id();

    // Insert state change event
    let new_obj_event = MapEventType::NewObjEvent;

    let map_state_event = MapEvent {
        event_id: map_event_id.0,
        entity_id: villagerEntityId,
        obj_id: 2,
        player_id: player_id,
        pos_x: 16,
        pos_y: 35,
        run_tick: game_tick.0 + 1, // Add one game tick
        map_event_type: new_obj_event,
    };

    map_events.insert(map_event_id.0.try_into().unwrap(), map_state_event);

    map_event_id.0 = map_event_id.0 + 1;

    /*
    let hero_tile_positions = Map::range((start_x, start_y), range);

    let hero_tiles = Map::pos_to_tiles(&hero_tile_positions, &map); // Used for network obj

    let network_hero = network_obj_from_bundle(&hero);

    let villager2 = Obj {
        id: Id(3),
        player_id: PlayerId(player_id),
        position: Position { x: 16, y: 34 },
        name: Name("Villager 2".into()),
        template: Template("Human Villager".into()),
        class: Class("unit".into()),
        subclass: Subclass("villager".into()),
        state: State("none".into()),
        viewshed: Viewshed {
            entities: HashSet::new(),
            range: 2,
        },
        misc: Misc {
            image: "humanvillager2".into(),
            hsl: Vec::new().into(),
            groups: Vec::new().into(),
        },
    };

    commands.spawn().insert_bundle(villager2);

    let villager3 = Obj {
        id: Id(4),
        player_id: PlayerId(player_id),
        position: Position { x: 15, y: 35 },
        name: Name("Villager 3".into()),
        template: Template("Human Villager".into()),
        class: Class("unit".into()),
        subclass: Subclass("villager".into()),
        state: State("none".into()),
        viewshed: Viewshed {
            entities: HashSet::new(),
            range: 2,
        },
        misc: Misc {
            image: "humanvillager3".into(),
            hsl: Vec::new().into(),
            groups: Vec::new().into(),
        },
    };

    // commands.spawn().insert_bundle(villager3);

    let villager4 = Obj {
        id: Id(5),
        player_id: PlayerId(player_id),
        position: Position { x: 15, y: 34 },
        name: Name("Villager 4".into()),
        template: Template("Human Villager".into()),
        class: Class("unit".into()),
        subclass: Subclass("villager".into()),
        state: State("none".into()),
        viewshed: Viewshed {
            entities: HashSet::new(),
            range: 2,
        },
        misc: Misc {
            image: "humanvillager1".into(),
            hsl: Vec::new().into(),
            groups: Vec::new().into(),
        },
    };

    // commands.spawn().insert_bundle(villager4);

    let tiles = hero_tiles;

    let mut objs: Vec<network::MapObj> = Vec::new();
    objs.push(network_hero);

    let perception_data = network::PerceptionData {
        map: tiles,
        objs: objs,
    };

    let explored_map = ExploredMap {
        player_id: player_id,
        tiles: HashSet::new(),
    };

    let tiles2 = Map::get_neighbour_tiles(16, 36, 2, map.clone());

    let perception = ResponsePacket::Perception {
        data: perception_data,
    };

    let res_map = ResponsePacket::Map { data: tiles2 };

    for (id, client) in clients.lock().unwrap().iter() {
        // Need to lookup player_id <=> client_id
        if *id == 1 {
            client
                .sender
                .try_send(serde_json::to_string(&perception).unwrap())
                //.try_send(tileset.get(&"warrior".to_string()).unwrap().to_string())
                .expect("Could not send message");

            client
                .sender
                .try_send(serde_json::to_string(&res_map).unwrap())
                //.try_send(tileset.get(&"warrior".to_string()).unwrap().to_string())
                .expect("Could not send message");
        }
    } */
}

fn network_obj_from_bundle(obj: &Obj) -> network::MapObj {
    let network_obj = network::MapObj {
        id: obj.id.0,
        player: obj.player_id.0,
        x: obj.position.x,
        y: obj.position.y,
        name: obj.name.0.clone(),
        template: obj.template.0.clone(),
        class: obj.class.0.clone(),
        subclass: obj.subclass.0.clone(),
        state: obj.state.0.clone(),
        vision: obj.viewshed.range,
        image: obj.misc.image.clone(),
        hsl: Vec::new(),
        groups: Vec::new(),
    };

    network_obj
}

fn network_obj(
    id: i32,
    player_id: i32,
    x: i32,
    y: i32,
    name: String,
    template: String,
    class: String,
    subclass: String,
    state: String,
    vision: u32,
    image: String,
    hsl: Vec<i32>,
    groups: Vec<i32>,
) -> network::MapObj {
    let network_obj = network::MapObj {
        id: id,
        player: player_id,
        x: x,
        y: y,
        name: name,
        template: template,
        class: class,
        subclass: subclass,
        state: state,
        vision: vision,
        image: image,
        hsl: hsl,
        groups: groups,
    };

    network_obj
}

fn run_if_visibility_changed(visibility_changed: Res<VisibilityChanged>) -> ShouldRun {
    if visibility_changed.0 {
        ShouldRun::Yes
    } else {
        ShouldRun::No
    }
}
