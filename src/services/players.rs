use sqlx::{Pool, Postgres};
use tonic::{Request, Response, Status};

use super::auth::Claims;

pub type PlayerServer<T> = player_server::PlayerServer<T>;

tonic::include_proto!("players");

const MAX_LVL: i32 = 30;

#[derive(Default)]
pub struct PlayerService;

#[tonic::async_trait]
impl player_server::Player for PlayerService {
    async fn get_profile(&self, request: Request<()>) -> Result<Response<PlayerProfile>, Status> {
        let (_, extensions, _) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        let (nickname, coins, crystals, glory, xp, clan_id, level): (
            Option<String>,
            i32,
            i32,
            i32,
            i32,
            Option<i32>,
            i32,
        ) = sqlx::query_as(
            "SELECT nickname, coins, crystals, glory, xp, clan_id, level FROM players WHERE id = $1",
        )
        .bind(credetials.id)
        .fetch_one(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        let (clan_name,): (Option<String>,) = if clan_id.is_some() {
            sqlx::query_as("SELECT clan_name FROM clans WHERE id = $1")
                .bind(clan_id.unwrap())
                .fetch_one(pool)
                .await
                .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        } else {
            (None,)
        };

        Ok(Response::new(PlayerProfile {
            nickname,
            coins,
            crystals,
            glory,
            clan_name,
            xp,
            max_xp: (((1f32 + (0.4f32 * (level + 1) as f32) / MAX_LVL as f32).powi(level + 1)
                * (MAX_LVL + 1 - level) as f32)
                + 20f32) as i32,
            level,
            clan_id,
            id: credetials.id,
        }))
    }

    async fn get_emotes(&self, request: Request<()>) -> Result<Response<AllEmotesList>, Status> {
        let (_, extensions, _) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        let rows: Vec<(Option<i32>, String)> = sqlx::query_as(
            "WITH cl AS
            (SELECT emote_id
             FROM players_emotes
             WHERE player_id = $1)
          SELECT emote_id,
                 file_name
          FROM cl
          RIGHT JOIN emotes ON emote_id = id",
        )
        .bind(credetials.id)
        .fetch_all(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        Ok(Response::new(AllEmotesList {
            player_emotes: Some(EmotesList {
                list: rows
                    .iter()
                    .filter(|f| f.0.is_some())
                    .map(|f| f.1.clone())
                    .collect(),
            }),
            other_emotes: Some(EmotesList {
                list: rows
                    .iter()
                    .filter(|f| f.0.is_none())
                    .map(|f| f.1.clone())
                    .collect(),
            }),
        }))
    }
}
