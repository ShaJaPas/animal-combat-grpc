pub mod services;

use chrono::{DateTime, Utc};
use jsonwebtoken::{DecodingKey, Validation};
use rand::Rng;
use skillratings::sticko::StickoRating;
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

#[derive(Clone, Copy)]
pub struct Match {
    player1: i32,
    player2: i32,
}

pub struct Player {
    rating: StickoRating,
    join_time: DateTime<Utc>,
}

pub struct Matchmaker {
    players: HashMap<i32, Player>,
}

impl Matchmaker {
    pub fn new() -> Self {
        Self {
            players: HashMap::new(),
        }
    }

    pub fn add_player(&mut self, id: i32, rating: StickoRating) {
        self.players.insert(
            id,
            Player {
                rating,
                join_time: Utc::now(),
            },
        );
    }

    pub fn remove_player(&mut self, id: i32) {
        self.players.remove(&id);
    }

    pub fn get_all_ids(&self) -> Vec<i32> {
        let mut vec = self
            .players
            .iter()
            .map(|f| (*f.0, f.1.join_time.timestamp()))
            .collect::<Vec<(i32, i64)>>();
        vec.sort_by(|a, b| a.1.cmp(&b.1));
        vec.into_iter().map(|f| f.0).collect()
    }

    pub fn find_match(&mut self, player_id: i32) -> Option<Match> {
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
                });
            }
        }

        // If there are no matches, return None
        if matches.is_empty() {
            return None;
        }

        // Randomly select a match from the list of possible matches
        let match_index = rng.gen_range(0..matches.len());
        let matched_players = matches[match_index];
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

pub async fn run_matchmaking_loop(
    mut rx: Receiver<MatchmakerMessage>,
    tx: Sender<MatchmakerMessage>,
) {
    let mut matchmaker = Matchmaker::new();

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
                    if let Some(m) = matchmaker.find_match(id) {
                        matchmaker.remove_player(m.player1);
                        matchmaker.remove_player(m.player2);
                        tx.send(MatchmakerMessage::MatchFound(m)).ok();
                    }
                }
            }
        }
    }
}
