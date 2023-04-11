use std::pin::Pin;

use futures::Stream;
use skillratings::sticko::StickoRating;
use sqlx::{Pool, Postgres};
use tokio::sync::{
    broadcast::Receiver,
    mpsc::{self, Sender},
};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use tracing::warn;

use crate::{BattleMessage, MatchmakerMessage};

use super::auth::Claims;

pub type BattleServer<T> = battle_server::BattleServer<T>;

tonic::include_proto!("battle");

pub struct BattleService {
    pub sender: Sender<MatchmakerMessage>,
    pub receiver: Receiver<MatchmakerMessage>,
    pub battle_tx: Sender<BattleMessage>,
    pub battle_rx: Receiver<BattleMessage>,
}

#[tonic::async_trait]
impl battle_server::Battle for BattleService {
    async fn join_matchmaking(&self, request: Request<()>) -> Result<Response<()>, Status> {
        let (_, extensions, _) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();
        let (glory, deviation): (i32, f64) =
            sqlx::query_as("SELECT glory, deviation FROM players WHERE id = $1")
                .bind(credetials.id)
                .fetch_one(pool)
                .await
                .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        self.sender
            .send(MatchmakerMessage::JoinMatchmaking {
                id: credetials.id,
                rating: StickoRating {
                    rating: glory as f64,
                    deviation,
                },
            })
            .await
            .map_err(|_| Status::aborted("Matchmaking is closed"))?;
        Ok(Response::new(()))
    }

    async fn leave_matchmaking(&self, request: Request<()>) -> Result<Response<()>, Status> {
        let (_, extensions, _) = request.into_parts();
        let credetials = extensions.get::<Claims>().unwrap();
        self.sender
            .send(MatchmakerMessage::LeaveMatchmaking { id: credetials.id })
            .await
            .map_err(|_| Status::aborted("Matchmaking is closed"))?;
        Ok(Response::new(()))
    }

    type FindMatchStream = Pin<Box<dyn Stream<Item = Result<MatchFound, Status>> + Send>>;

    async fn find_match(
        &self,
        request: Request<()>,
    ) -> Result<Response<Self::FindMatchStream>, Status> {
        let (_, extensions, _) = request.into_parts();
        let credetials = extensions.get::<Claims>().unwrap();
        let pool = extensions.get::<Pool<Postgres>>().unwrap().clone();

        let (tx, rx) = mpsc::channel(128);

        let mut rcv = self.receiver.resubscribe();
        let player_id = credetials.id;
        tokio::spawn(async move {
            while let Ok(msg) = rcv.recv().await {
                match msg {
                    MatchmakerMessage::MatchFound(m)
                        if m.player1 == player_id || m.player2 == player_id =>
                    {
                        let res = sqlx::query_as(
                            "SELECT glory,
                                    nickname,
                                    clan_name
                            FROM players
                            LEFT JOIN clans ON clans.id = players.clan_id
                            WHERE players.id = $1",
                        )
                        .bind(if m.player1 == player_id {
                            m.player2
                        } else {
                            m.player1
                        })
                        .fetch_one(&pool)
                        .await;
                        if let Ok((glory, nickname, clan_name)) = res {
                            if tx
                                .send(Result::<_, Status>::Ok(MatchFound {
                                    opponent_id: if m.player1 == player_id {
                                        m.player2
                                    } else {
                                        m.player1
                                    },
                                    nickname,
                                    clan_name,
                                    glory,
                                    map: Some(m.map.into()),
                                    invert: m.player2 == player_id,
                                }))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        } else {
                            tx.send(Result::<_, Status>::Err(Status::data_loss(format!(
                                "Database error: {}",
                                res.err().unwrap()
                            ))))
                            .await
                            .ok();
                        }
                    }
                    _ => continue,
                }
            }
        });

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(output_stream) as Self::FindMatchStream
        ))
    }

    type BattleMessagesStream = Pin<Box<dyn Stream<Item = Result<BattleCommand, Status>> + Send>>;

    async fn battle_messages(
        &self,
        request: Request<Streaming<ClientBattleMessage>>,
    ) -> Result<Response<Self::BattleMessagesStream>, Status> {
        let (_, extensions, mut in_stream) = request.into_parts();
        let credetials = extensions.get::<Claims>().unwrap();

        let (tx, rx) = mpsc::channel(128);

        let mut rcv = self.battle_rx.resubscribe();
        let sender = self.battle_tx.clone();
        let player_id = credetials.id;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(result) = in_stream.next() => {
                        match result {
                            Ok(v) => {
                                if let Some(msg) = v.message {
                                    match msg {
                                        client_battle_message::Message::Pick(v) => {
                                            sender
                                                .send(BattleMessage::Pick {
                                                    player_id,
                                                    cmd: v,
                                                })
                                                .await
                                                .ok();
                                        },
                                        client_battle_message::Message::Ready(_) => {
                                            sender
                                                .send(BattleMessage::Ready {
                                                    player_id
                                                })
                                                .await
                                                .ok();
                                        },
                                        client_battle_message::Message::Place(animals) => {
                                            sender
                                                .send(BattleMessage::PlacePlayerAnimals {
                                                    player_id,
                                                    animals
                                                })
                                                .await
                                                .ok();
                                        }
                                    }
                                }
                            }
                            Err(err) => {
                                warn!("Client error in streaming {}", err);
                                break;
                            }
                        }
                    },
                    Ok(value) = rcv.recv() => {
                        match value {
                            BattleMessage::Response{ receivers, res } => {
                                if receivers.contains(&player_id) {
                                    tx.send(res.map(|cmd| BattleCommand {
                                        command: Some(cmd)
                                    })).await.ok();
                                }
                            },
                            _ => continue
                        }
                    }
                }
            }
        });
        let out_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(out_stream) as Self::BattleMessagesStream
        ))
    }
}
