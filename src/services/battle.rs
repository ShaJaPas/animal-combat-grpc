use std::pin::Pin;

use futures::Stream;
use skillratings::sticko::StickoRating;
use sqlx::{Pool, Postgres};
use tokio::sync::{
    broadcast::Receiver,
    mpsc::{self, Sender},
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::MatchmakerMessage;

use super::auth::Claims;

pub type BattleServer<T> = battle_server::BattleServer<T>;

tonic::include_proto!("battle");

pub struct BattleService {
    pub sender: Sender<MatchmakerMessage>,
    pub receiver: Receiver<MatchmakerMessage>,
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

        let (tx, rx) = mpsc::channel(128);

        let mut rcv = self.receiver.resubscribe();
        let player_id = credetials.id;
        tokio::spawn(async move {
            while let Ok(msg) = rcv.recv().await {
                match msg {
                    MatchmakerMessage::MatchFound(m)
                        if m.player1 == player_id || m.player2 == player_id =>
                    {
                        if tx
                            .send(Result::<_, Status>::Ok(MatchFound {
                                opponent_id: m.player1,
                            }))
                            .await
                            .is_err()
                        {
                            break;
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
}
