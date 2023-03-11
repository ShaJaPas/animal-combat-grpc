use sqlx::{Pool, Postgres};
use tonic::{Request, Response, Status};

use super::auth::Claims;

pub type ClanServer<T> = clan_server::ClanServer<T>;

const CLAN_CREATION_PRICE: i32 = 1000;
const MAX_MEMBERS: i32 = 50;

tonic::include_proto!("clans");

#[derive(Default)]
pub struct ClanService;

#[tonic::async_trait]
impl clan_server::Clan for ClanService {
    async fn create_clan(&self, request: Request<ClanInfo>) -> Result<Response<()>, Status> {
        let (_, extensions, request) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        let (coins,): (i32,) = sqlx::query_as(
            "SELECT coins
            FROM players
            WHERE email = $1",
        )
        .bind(&credetials.email)
        .fetch_one(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        if coins - CLAN_CREATION_PRICE < 0 {
            return Err(Status::permission_denied(format!(
                "Player don't have enough coins (less than {CLAN_CREATION_PRICE}) for creating a clan"
            )));
        }

        if sqlx::query(
            "SELECT NULL
            FROM clans
            WHERE clan_name = $1",
        )
        .bind(&request.name)
        .fetch_optional(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        .is_some()
        {
            return Err(Status::already_exists(format!(
                "Clan with name '{}' already exists",
                &request.name
            )));
        }

        if sqlx::query(
            "SELECT NULL
            FROM players
            WHERE email = $1
              AND clan_id IS NOT NULL",
        )
        .bind(&credetials.email)
        .fetch_optional(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        .is_some()
        {
            return Err(Status::permission_denied("Player is already in clan"));
        }

        let (id,): (i32,) = sqlx::query_as(
            format!(
                "INSERT INTO clans (clan_name, description, min_glory, max_members, type)
                VALUES ($1, $2, $3, $4, '{}')
                RETURNING id",
                ClanType::from_i32(request.clan_type).unwrap().as_str_name()
            )
            .as_str(),
        )
        .bind(request.name)
        .bind(request.description)
        .bind(request.min_glory)
        .bind(MAX_MEMBERS)
        .fetch_one(pool)
        .await
        .map_err(|_| Status::permission_denied("Clan description was too long"))?;

        sqlx::query("UPDATE players SET clan_id = $1 WHERE email = $2")
            .bind(id)
            .bind(&credetials.email)
            .execute(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        Ok(Response::new(()))
    }

    async fn recommended_clans(
        &self,
        request: Request<RecommenedClansRequest>,
    ) -> Result<Response<ShortClanInfoList>, Status> {
        let (_, extensions, request) = request.into_parts();

        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        let (trophies,): (i32,) = sqlx::query_as(
            "SELECT trophies
            FROM players
            WHERE email = $1",
        )
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
          ORDER BY clan_name
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
          ORDER BY clan_name
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
