use sqlx::{Pool, Postgres};
use tonic::{Request, Response, Status};

use super::auth::Claims;

pub type ClanServer<T> = clan_server::ClanServer<T>;

tonic::include_proto!("clans");

#[derive(Default)]
pub struct ClanService;

#[tonic::async_trait]
impl clan_server::Clan for ClanService {
    async fn recommended_clans(
        &self,
        request: Request<RecommenedClansRequest>,
    ) -> Result<Response<ShortClanInfoList>, Status> {
        let (_, extensions, request) = request.into_parts();

        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        let (trophies,): (i32,) = sqlx::query_as("SELECT trophies FROM players WHERE email = $1")
            .bind(&credetials.email)
            .fetch_one(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        let clans: Vec<ShortClanInfo> = sqlx::query_as(
            "WITH cl AS
            (SELECT COUNT(*) AS members,
                    AVG(trophies) AS avg_trophies,
                    clan_id
             FROM players
             WHERE clan_id IS NOT NULL
             GROUP BY clan_id)
          SELECT clan_name,
                 CAST(members AS INT),
                 CAST(avg_trophies AS INT),
                 max_members,
                 id
          FROM cl
          JOIN clans ON clans.id = cl.clan_id
          WHERE avg_trophies >= $1-50
            AND avg_trophies <= $1+50
            OFFSET $2
          LIMIT $3",
        )
        .bind(trophies)
        .bind(request.offset)
        .bind(request.limit)
        .fetch_all(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        .into_iter()
        .map(
            |(name, members, average_trophies, max_members, id): (String, i32, i32, i32, i32)| {
                ShortClanInfo {
                    name,
                    members,
                    max_members,
                    average_trophies,
                    id,
                }
            },
        )
        .collect();

        Ok(Response::new(ShortClanInfoList {
            offset: request.offset,
            infos: clans,
        }))
    }

    async fn search_clans(
        &self,
        request: Request<SearchClansRequest>,
    ) -> Result<Response<ShortClanInfoList>, Status> {
        let (_, extensions, request) = request.into_parts();

        let pool = extensions.get::<Pool<Postgres>>().unwrap();

        let clans: Vec<ShortClanInfo> = sqlx::query_as(
            "WITH cl AS
            (SELECT COUNT(*) AS members,
                    AVG(trophies) AS avg_trophies,
                    clan_id
             FROM players
             WHERE clan_id IS NOT NULL
             GROUP BY clan_id)
          SELECT clan_name,
                 CAST(members AS INT),
                 CAST(avg_trophies AS INT),
                 max_members,
                 id
          FROM cl
          JOIN clans ON clans.id = cl.clan_id
          WHERE LOWER(clan_name) LIKE $1
            OFFSET $2
          LIMIT $3",
        )
        .bind(format!("%{}%", request.pattern.to_lowercase()))
        .bind(request.offset)
        .bind(request.limit)
        .fetch_all(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        .into_iter()
        .map(
            |(name, members, average_trophies, max_members, id): (String, i32, i32, i32, i32)| {
                ShortClanInfo {
                    name,
                    members,
                    max_members,
                    average_trophies,
                    id,
                }
            },
        )
        .collect();

        Ok(Response::new(ShortClanInfoList {
            offset: request.offset,
            infos: clans,
        }))
    }
}
