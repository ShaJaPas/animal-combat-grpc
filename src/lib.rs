pub mod services;

use crate::services::battle::battle_command::Command;
use crate::services::battle::{
    AnimalPicked, AnimalPlaced, AnimalsPlaced, BattleState, GameMap, GameObject, GameObjectType,
    PickAnimal, PlaceAnimal, PlaceAnimals, SetBattleState, TurnToPick,
};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ExecutorKind;
use chrono::{DateTime, NaiveDateTime, Utc};
use jsonwebtoken::{DecodingKey, Validation};
use prost_types::Timestamp;
use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};
use services::battle;
use skillratings::sticko::StickoRating;
use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Arc;
use std::{collections::HashMap, time::Duration};
use tokio::{
    sync::{broadcast::Sender, mpsc::Receiver},
    time,
};
use tonic::{Request, Status};

//Put this in any service, except Auth
pub fn jwt_interceptor(mut req: Request<()>) -> Result<Request<()>, Status> {
    let token = match req.metadata().get("authorization") {
        Some(token) => token.to_str(),
        None => return Err(Status::unauthenticated("JWT token not found")),
    };

    if let Ok(token) = token {
        let claims = jsonwebtoken::decode::<services::auth::Claims>(
            token,
            &DecodingKey::from_base64_secret(&std::env::var("JWT_SECRET").unwrap()).unwrap(),
            &Validation::default(),
        )
        .map_err(|e| Status::unauthenticated(e.to_string()))?;

        req.extensions_mut().insert(claims.claims);
    } else {
        return Err(Status::unauthenticated(
            "Key \"authorization\" was invalid string",
        ));
    }
    Ok(req)
}

impl From<Map> for GameMap {
    fn from(value: Map) -> Self {
        Self {
            name: value.map_name,
            objects: value.objects.into_iter().map(|f| f.into()).collect(),
        }
    }
}

impl From<MapObject> for GameObject {
    fn from(value: MapObject) -> Self {
        Self {
            x: value.x,
            y: value.y,
            png_name: value.png_name,
            object_type: Into::<GameObjectType>::into(value.object_type).into(),
        }
    }
}

impl From<ObjectType> for GameObjectType {
    fn from(value: ObjectType) -> Self {
        match value {
            ObjectType::Water => GameObjectType::Water,
            ObjectType::Solid => GameObjectType::Solid,
            ObjectType::CanWalkThrough => GameObjectType::Walkable,
        }
    }
}

#[derive(Clone)]
pub struct Match {
    player1: i32,
    player1_ready: bool,
    player2: i32,
    player2_ready: bool,
    map: Map,
}

pub struct Player {
    rating: StickoRating,
    join_time: DateTime<Utc>,
}

pub struct Matchmaker {
    players: HashMap<i32, Player>,
}

impl Matchmaker {
    fn new() -> Self {
        Self {
            players: HashMap::new(),
        }
    }

    fn add_player(&mut self, id: i32, rating: StickoRating) {
        self.players.insert(
            id,
            Player {
                rating,
                join_time: Utc::now(),
            },
        );
    }

    fn remove_player(&mut self, id: i32) {
        self.players.remove(&id);
    }

    fn get_all_ids(&self) -> Vec<i32> {
        let mut vec = self
            .players
            .iter()
            .map(|f| (*f.0, f.1.join_time.timestamp()))
            .collect::<Vec<(i32, i64)>>();
        vec.sort_by(|a, b| a.1.cmp(&b.1));
        vec.into_iter().map(|f| f.0).collect()
    }

    fn find_match(&mut self, player_id: i32, maps: &Maps) -> Option<Match> {
        let player = match self.players.get(&player_id) {
            Some(p) => p,
            None => return None,
        };
        // Calculate the maximum rating difference based on the waiting time
        let elapsed_time = (Utc::now() - player.join_time)
            .to_std()
            .unwrap_or(Duration::default());
        let max_rating_diff = ((elapsed_time.as_secs_f32() / 6f32 + 1f32) * 100f32) as i32;
        let max_rating_diff = std::cmp::min(max_rating_diff, 500);

        // Find a player with a matching rating and within the allowed rating difference
        let mut rng = rand::thread_rng();
        let mut matches = Vec::new();
        for (&id, other_player) in &self.players {
            if id == player_id {
                continue;
            }
            let rating_diff = (player.rating.rating - other_player.rating.rating).abs() as i32;
            if rating_diff <= max_rating_diff {
                matches.push(Match {
                    player1: player_id,
                    player2: id,
                    player1_ready: false,
                    player2_ready: false,
                    map: maps.maps.choose(&mut rand::thread_rng()).unwrap().clone(),
                });
            }
        }

        // If there are no matches, return None
        if matches.is_empty() {
            return None;
        }

        // Randomly select a match from the list of possible matches
        let match_index = rng.gen_range(0..matches.len());
        let matched_players = matches[match_index].clone();
        Some(matched_players)
    }
}

impl Default for Matchmaker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub enum MatchmakerMessage {
    JoinMatchmaking { id: i32, rating: StickoRating },
    LeaveMatchmaking { id: i32 },
    MatchFound(Match),
}

#[derive(Clone)]
pub enum BattleMessage {
    //Can trust
    CreateBattle(Match),
    //Need check
    Pick {
        player_id: i32,
        cmd: PickAnimal,
    },
    Ready {
        player_id: i32,
    },
    PlacePlayerAnimals {
        player_id: i32,
        animals: PlaceAnimals,
    },
    Response {
        receivers: Vec<i32>,
        res: Result<Command, Status>,
    },
}

#[derive(Serialize, Deserialize)]
struct Maps {
    maps: Vec<Map>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Map {
    map_name: String,
    objects: Vec<MapObject>,
}

#[derive(Serialize, Deserialize, Clone)]
struct MapObject {
    x: i32,
    y: i32,
    png_name: Option<String>,
    object_type: ObjectType,
}

#[derive(Serialize, Deserialize, Clone)]
enum ObjectType {
    #[serde(rename = "solid")]
    Solid,
    #[serde(rename = "can_walk_through")]
    CanWalkThrough,
    #[serde(rename = "water")]
    Water,
}

#[derive(Serialize, Deserialize)]
struct Animals {
    animals: Vec<Animal>,
}

#[derive(Serialize, Deserialize)]
struct Animal {
    id: i32,
    name: String,
    hp: i32,
    damage: i32,
    resistance: f32,
    mobility: i32,
    png_name: String,
    description: String,
    action_points: i32,
    action_points_per_turn: f32,
    abilities: Vec<Ability>,
}

#[derive(Serialize, Deserialize)]
enum AbilityType {
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "passive")]
    Passive,
}

#[derive(Serialize, Deserialize)]
struct Ability {
    name: String,
    icon_name: String,
    description: String,
    #[serde(rename = "type")]
    ability_type: AbilityType,
    cooldown: Option<i32>,
    cost: Option<i32>,
}

pub async fn run_matchmaking_loop(
    mut rx: Receiver<MatchmakerMessage>,
    tx: Sender<MatchmakerMessage>,
    battle_tx: tokio::sync::mpsc::Sender<BattleMessage>,
) {
    let mut matchmaker = Matchmaker::new();
    let maps: Maps = serde_json::from_str(include_str!("../data/maps.json")).unwrap();
    let mut interval = time::interval(Duration::from_secs(1)); // Run the matchmaking algorithm every 1 second
    loop {
        tokio::select! {
            Some(msg) = rx.recv() => {
                match msg {
                    MatchmakerMessage::JoinMatchmaking { id, rating } => matchmaker.add_player(id, rating),
                    MatchmakerMessage::LeaveMatchmaking { id } => matchmaker.remove_player(id),
                    _ => continue
                }
            },
            _ = interval.tick() => {
                let ids = matchmaker.get_all_ids();
                for id in ids {
                    if let Some(m) = matchmaker.find_match(id, &maps) {
                        matchmaker.remove_player(m.player1);
                        matchmaker.remove_player(m.player2);
                        tx.send(MatchmakerMessage::MatchFound(m.clone())).ok();
                        battle_tx.send(BattleMessage::CreateBattle(m)).await.ok();
                    }
                }
            }
        }
    }
}

const PICK_TIME: u64 = 30;
const PLACE_TIME: u64 = 60;
const PICK_COUNT: usize = 6;

// Events need to be updated in every frame in order to clear our buffers.
// This update should happen before we use the events.
// Here, we use system sets to control the ordering.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlushEvents;

pub async fn run_battles_loop(mut rx: Receiver<BattleMessage>, tx: Sender<BattleMessage>) {
    let animals: Arc<Animals> =
        Arc::new(serde_json::from_str(include_str!("../data/animals.json")).unwrap());
    let mut index_map = HashMap::new();
    let mut worlds = Vec::new();
    let mut interval = time::interval(Duration::from_secs(1));
    let mut schedule = Schedule::default();
    schedule.set_executor_kind(ExecutorKind::Simple);

    schedule.add_system(Events::<Event>::update_system.in_set(FlushEvents));
    schedule.add_systems((
        pick_timeout.after(FlushEvents),
        ready.after(FlushEvents),
        pick.after(FlushEvents),
        place.after(FlushEvents),
        place_timeout.after(FlushEvents),
    ));
    loop {
        tokio::select! {
            Some(msg) = rx.recv() => {
                match msg {
                    BattleMessage::Ready { player_id } => {
                        if let Some(&index) = index_map.get(&player_id)
                        {
                            let world: &mut World = &mut worlds[index];
                            world.send_event(Event {
                                message: msg
                            });
                            schedule.run(world);
                        }
                    },
                    BattleMessage::CreateBattle(m) => {
                        let mut world = World::new();
                        index_map.insert(m.player1, worlds.len());
                        index_map.insert(m.player2, worlds.len());
                        world.insert_resource(Events::<Event>::default());
                        for object in &m.map.objects {
                            world.spawn(Position {
                                x: object.x,
                                y: object.y
                            });
                        }
                        world.insert_resource(GameState::new(m, tx.clone(), animals.clone()));
                        worlds.push(world);
                    }
                    BattleMessage::Pick { player_id, cmd: _ } => {
                        if let Some(&index) = index_map.get(&player_id)
                        {
                            let world: &mut World = &mut worlds[index];
                            world.send_event(Event {
                                message: msg
                            });
                            schedule.run(world);
                        } else {
                            tx.send(BattleMessage::Response{
                                receivers: vec![player_id],
                                res: Err(Status::not_found(
                                    "Player is not playing",
                                ))
                            })
                            .ok();
                        }
                    },
                    BattleMessage::PlacePlayerAnimals { player_id, animals: _ } => {
                        if let Some(&index) = index_map.get(&player_id)
                        {
                            let world: &mut World = &mut worlds[index];
                            world.send_event(Event {
                                message: msg
                            });
                            schedule.run(world);
                        } else {
                            tx.send(BattleMessage::Response{
                                receivers: vec![player_id],
                                res: Err(Status::not_found(
                                    "Player is not playing",
                                ))
                            })
                            .ok();
                        }
                    },
                    BattleMessage::Response { receivers: _, res: _ } => continue,
                }
            },
            _ = interval.tick() => {
                for world in &mut worlds {
                    schedule.run(world);
                }
            }
        }
    }
}

//Resources

#[derive(Resource)]
struct GameState {
    state: BattleState,
    current_turn: i32,
    m: Match,
    tx: Sender<BattleMessage>,
    deadline: DateTime<Utc>,
    animals: Arc<Animals>,
}

impl GameState {
    fn new(m: Match, tx: Sender<BattleMessage>, animals: Arc<Animals>) -> Self {
        Self {
            state: BattleState::PickStage,
            current_turn: if rand::thread_rng().gen_range(0..=1) == 0 {
                m.player1
            } else {
                m.player2
            },
            m,
            tx,
            deadline: Utc::now(),
            animals,
        }
    }

    fn next_turn(&mut self) {
        if self.state != BattleState::PlacementStage {
            self.current_turn = if self.current_turn == self.m.player1 {
                self.m.player2
            } else {
                self.m.player1
            };
        }
    }

    fn all_ready(&self) -> bool {
        self.m.player1_ready && self.m.player2_ready
    }

    fn set_ready(&mut self, player_id: i32) {
        if self.m.player1 == player_id {
            self.m.player1_ready = true;
        }
        if self.m.player2 == player_id {
            self.m.player2_ready = true;
        }
    }
}

//Components

#[derive(Component)]
struct AnimalId {
    player_id: i32,
    id: i32,
}

#[derive(Component, Debug, Clone)]
struct Position {
    x: i32,
    y: i32,
}

//Systems

fn pick_timeout(mut state: ResMut<GameState>, mut commands: Commands, query: Query<&AnimalId>) {
    if state.state != BattleState::PickStage {
        return;
    }
    let now = Utc::now();
    if state.all_ready()
        && (state.deadline - now).num_milliseconds() <= 0
        && query.iter().count() != PICK_COUNT
    {
        let available_animals: Vec<i32> = state
            .animals
            .animals
            .iter()
            .filter(|f| query.iter().all(|g| g.id != f.id))
            .map(|f| f.id)
            .collect();
        let animal_id = *available_animals.choose(&mut rand::thread_rng()).unwrap();

        let turn = state.current_turn;
        commands.spawn(AnimalId {
            id: animal_id,
            player_id: turn,
        });

        if query.iter().count() % 2 == 0 {
            state.next_turn();
        }

        state
            .tx
            .send(BattleMessage::Response {
                receivers: vec![state.m.player1, state.m.player2],
                res: Ok(Command::Picked(AnimalPicked {
                    animal_id,
                    player_id: turn,
                })),
            })
            .ok();

        if query.iter().count() == PICK_COUNT - 1 {
            state
                .tx
                .send(BattleMessage::Response {
                    receivers: vec![state.m.player1, state.m.player2],
                    res: Ok(Command::SetState(SetBattleState {
                        state: BattleState::PlacementStage.into(),
                    })),
                })
                .ok();
            state.state = BattleState::PlacementStage;
            state.deadline = DateTime::<Utc>::from_utc(
                NaiveDateTime::from_timestamp_opt(now.timestamp() + PLACE_TIME as i64, 0).unwrap(),
                Utc,
            );
            state
                .tx
                .send(BattleMessage::Response {
                    receivers: vec![state.m.player1, state.m.player2],
                    res: Ok(Command::TurnToPick(TurnToPick {
                        player_id: None,
                        deadline: Some(Timestamp {
                            seconds: state.deadline.timestamp(),
                            nanos: 0,
                        }),
                    })),
                })
                .ok();
        } else {
            state.deadline = DateTime::<Utc>::from_utc(
                NaiveDateTime::from_timestamp_opt(now.timestamp() + PICK_TIME as i64, 0).unwrap(),
                Utc,
            );
            state
                .tx
                .send(BattleMessage::Response {
                    receivers: vec![state.m.player1, state.m.player2],
                    res: Ok(Command::TurnToPick(TurnToPick {
                        player_id: Some(state.current_turn),
                        deadline: Some(Timestamp {
                            seconds: state.deadline.timestamp(),
                            nanos: 0,
                        }),
                    })),
                })
                .ok();
        }
    }
}

fn ready(mut state: ResMut<GameState>, mut event_reader: EventReader<Event>) {
    if state.state != BattleState::PickStage {
        return;
    }
    for my_event in event_reader.iter() {
        if let BattleMessage::Ready { player_id } = my_event.message {
            state.set_ready(player_id);

            if state.all_ready() {
                let now = Utc::now();
                state
                    .tx
                    .send(BattleMessage::Response {
                        receivers: vec![state.m.player1, state.m.player2],
                        res: Ok(Command::TurnToPick(TurnToPick {
                            player_id: Some(state.current_turn),
                            deadline: Some(Timestamp {
                                seconds: now.timestamp() + PICK_TIME as i64,
                                nanos: 0,
                            }),
                        })),
                    })
                    .ok();
                state.deadline = DateTime::<Utc>::from_utc(
                    NaiveDateTime::from_timestamp_opt(now.timestamp() + PICK_TIME as i64, 0)
                        .unwrap(),
                    Utc,
                );
            }
        }
    }
}

fn pick(
    mut state: ResMut<GameState>,
    mut event_reader: EventReader<Event>,
    mut commands: Commands,
    query: Query<&AnimalId>,
) {
    if state.state != BattleState::PickStage {
        return;
    }
    for my_event in event_reader.iter() {
        if let BattleMessage::Pick {
            player_id,
            cmd: PickAnimal { animal_id },
        } = my_event.message
        {
            //Check if animal exists and not taken
            if state.animals.animals.iter().any(|f| f.id == animal_id)
                && query.iter().all(|f| f.id != animal_id)
                && state.current_turn == player_id
                && query.iter().count() != PICK_COUNT
            {
                commands.spawn(AnimalId {
                    id: animal_id,
                    player_id,
                });

                state
                    .tx
                    .send(BattleMessage::Response {
                        receivers: vec![state.m.player1, state.m.player2],
                        res: Ok(Command::Picked(AnimalPicked {
                            animal_id,
                            player_id: state.current_turn,
                        })),
                    })
                    .ok();

                if query.iter().count() % 2 == 0 {
                    state.next_turn();
                }

                let now = Utc::now();
                if query.iter().count() == PICK_COUNT - 1 {
                    state
                        .tx
                        .send(BattleMessage::Response {
                            receivers: vec![state.m.player1, state.m.player2],
                            res: Ok(Command::SetState(SetBattleState {
                                state: BattleState::PlacementStage.into(),
                            })),
                        })
                        .ok();
                    state.state = BattleState::PlacementStage;
                    state.deadline = DateTime::<Utc>::from_utc(
                        NaiveDateTime::from_timestamp_opt(now.timestamp() + PLACE_TIME as i64, 0)
                            .unwrap(),
                        Utc,
                    );
                    state
                        .tx
                        .send(BattleMessage::Response {
                            receivers: vec![state.m.player1, state.m.player2],
                            res: Ok(Command::TurnToPick(TurnToPick {
                                player_id: None,
                                deadline: Some(Timestamp {
                                    seconds: state.deadline.timestamp(),
                                    nanos: 0,
                                }),
                            })),
                        })
                        .ok();
                } else {
                    state.deadline = DateTime::<Utc>::from_utc(
                        NaiveDateTime::from_timestamp_opt(now.timestamp() + PICK_TIME as i64, 0)
                            .unwrap(),
                        Utc,
                    );
                    state
                        .tx
                        .send(BattleMessage::Response {
                            receivers: vec![state.m.player1, state.m.player2],
                            res: Ok(Command::TurnToPick(TurnToPick {
                                player_id: Some(state.current_turn),
                                deadline: Some(Timestamp {
                                    seconds: state.deadline.timestamp(),
                                    nanos: 0,
                                }),
                            })),
                        })
                        .ok();
                }
            } else {
                state
                    .tx
                    .send(BattleMessage::Response {
                        receivers: vec![player_id],
                        res: Err(Status::not_found("Animal is not available to pick")),
                    })
                    .ok();
            }
        }
    }
}

fn place(
    mut state: ResMut<GameState>,
    mut commands: Commands,
    query: Query<(Entity, &AnimalId), Without<Position>>,
    already_set: Query<(&AnimalId, &Position)>,
    objects: Query<&Position, Without<AnimalId>>,
    mut event_reader: EventReader<Event>,
) {
    if state.state != BattleState::PlacementStage {
        return;
    }
    for my_event in event_reader.iter() {
        if let BattleMessage::PlacePlayerAnimals { player_id, animals } = &my_event.message {
            //Iterate over entities which player has
            let filtered_query = query
                .iter()
                .filter(|(_, animal_id)| animal_id.player_id == *player_id);
            let animals: Vec<PlaceAnimal> = animals
                .animals
                .iter()
                .map(|f| PlaceAnimal {
                    animal_id: f.animal_id,
                    position: f.position.as_ref().map(|g| battle::Position {
                        x: g.x,
                        y: if state.m.player2 == *player_id {
                            23 - g.y
                        } else {
                            g.y
                        },
                    }),
                })
                .collect();
            if (filtered_query.count() != PICK_COUNT / 2 || animals.len() != PICK_COUNT / 2)
                && animals.iter().all(|f| {
                    f.position.is_some()
                        && (0..7).contains(&f.position.as_ref().unwrap().x)
                        && (0..24).contains(&f.position.as_ref().unwrap().y)
                })
                && animals.iter().all(|f| {
                    query
                        .iter()
                        .filter(|(_, animal_id)| animal_id.player_id == *player_id)
                        .any(|(_, id)| id.id == f.animal_id)
                        && all_unique_elements(animals.iter().map(|f| f.animal_id))
                        && all_unique_elements(animals.iter().map(|f| {
                            (
                                f.position.as_ref().unwrap().x,
                                f.position.as_ref().unwrap().y,
                            )
                        }))
                        && objects.iter().all(|g| {
                            !(g.x == f.position.as_ref().unwrap().x
                                && g.y == f.position.as_ref().unwrap().y)
                        })
                })
            {
                state
                    .tx
                    .send(BattleMessage::Response {
                        receivers: vec![*player_id],
                        res: Err(Status::permission_denied("Not all animals position send")),
                    })
                    .ok();
            } else {
                let mut vec = Vec::with_capacity(PICK_COUNT / 2);
                for (entity, animal_id) in query
                    .iter()
                    .filter(|(_, animal_id)| animal_id.player_id == *player_id)
                {
                    let animal = animals
                        .iter()
                        .find(|f| f.animal_id == animal_id.id)
                        .unwrap();

                    commands.entity(entity).insert(Position {
                        x: animal.position.as_ref().unwrap().x,
                        y: animal.position.as_ref().unwrap().y,
                    });
                    for (id, position) in already_set.iter() {
                        vec.push(AnimalPlaced {
                            player_id: id.player_id,
                            position: Some(battle::Position {
                                x: position.x,
                                y: position.y,
                            }),
                            animal_id: id.id,
                        });
                    }
                    vec.push(AnimalPlaced {
                        player_id: *player_id,
                        position: Some(battle::Position {
                            x: animal.position.as_ref().unwrap().x,
                            y: animal.position.as_ref().unwrap().y,
                        }),
                        animal_id: animal_id.id,
                    });
                }

                if already_set.iter().count() == PICK_COUNT / 2 {
                    state
                        .tx
                        .send(BattleMessage::Response {
                            receivers: vec![state.m.player1, state.m.player2],
                            res: Ok(Command::Placed(AnimalsPlaced { animals: vec })),
                        })
                        .ok();
                    state
                        .tx
                        .send(BattleMessage::Response {
                            receivers: vec![state.m.player1, state.m.player2],
                            res: Ok(Command::SetState(SetBattleState {
                                state: BattleState::GameStage.into(),
                            })),
                        })
                        .ok();
                    state.state = BattleState::GameStage;
                }
            }
        }
    }
}

fn place_timeout(
    mut state: ResMut<GameState>,
    mut commands: Commands,
    query: Query<(Entity, &AnimalId), Without<Position>>,
    already_placed: Query<(&Position, &AnimalId)>,
    objects: Query<&Position, Without<AnimalId>>,
) {
    if state.state != BattleState::PlacementStage {
        return;
    }

    let rng = &mut rand::thread_rng();

    let mut vec = Vec::with_capacity(PICK_COUNT / 2);

    for player_id in [state.m.player1, state.m.player2] {
        let mut positions = Vec::new();
        for x in 0..7 {
            let bound = if player_id == state.m.player2 {
                12..24
            } else {
                0..12
            };
            for y in bound {
                if !objects.iter().any(|f| f.x == x && f.y == y) {
                    positions.push(Position { x, y });
                }
            }
        }

        let filtered_query: Vec<(Entity, &AnimalId)> = query
            .iter()
            .filter(|(_, animal_id)| animal_id.player_id == player_id)
            .collect();

        let now = Utc::now();
        let random_positions = positions.choose_multiple(rng, PICK_COUNT / 2);
        if !filtered_query.is_empty() && (state.deadline - now).num_milliseconds() <= 0 {
            for ((entity, animal_id), position) in filtered_query.iter().zip(random_positions) {
                vec.push(AnimalPlaced {
                    player_id,
                    position: Some(battle::Position {
                        x: position.x,
                        y: position.y,
                    }),
                    animal_id: animal_id.id,
                });
                commands.entity(*entity).insert((*position).clone());
            }
        }
    }

    if !vec.is_empty() {
        vec.append(
            &mut already_placed
                .iter()
                .map(|(position, animal_id)| AnimalPlaced {
                    player_id: animal_id.player_id,
                    position: Some(battle::Position {
                        x: position.x,
                        y: position.y,
                    }),
                    animal_id: animal_id.id,
                })
                .collect::<Vec<AnimalPlaced>>(),
        );
        state
            .tx
            .send(BattleMessage::Response {
                receivers: vec![state.m.player1, state.m.player2],
                res: Ok(Command::Placed(AnimalsPlaced { animals: vec })),
            })
            .ok();
        state
            .tx
            .send(BattleMessage::Response {
                receivers: vec![state.m.player1, state.m.player2],
                res: Ok(Command::SetState(SetBattleState {
                    state: BattleState::GameStage.into(),
                })),
            })
            .ok();
        state.state = BattleState::GameStage;
    }
}

//Events

struct Event {
    message: BattleMessage,
}

//Utils

fn all_unique_elements<T>(iter: T) -> bool
where
    T: IntoIterator,
    T::Item: Eq + Hash,
{
    let mut uniq = HashSet::new();
    iter.into_iter().all(move |x| uniq.insert(x))
}
