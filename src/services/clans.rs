use chrono::{DateTime, Utc};
use futures::Stream;
use prost_types::Timestamp;
use sqlx::{Pool, Postgres};
use std::pin::Pin;
use tokio::sync::{
    broadcast::{self, Receiver, Sender},
    mpsc,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use super::auth::Claims;

pub type ClanServer<T> = clan_server::ClanServer<T>;

const CLAN_CREATION_PRICE: i32 = 1000;
const MAX_MEMBERS: i32 = 50;

tonic::include_proto!("clans");

pub struct ClanService {
    sender: Sender<(i32, ClanMesage)>,
    receiver: Receiver<(i32, ClanMesage)>,
}

impl ClanService {
    pub fn new() -> Self {
        let (sender, receiver) = broadcast::channel(16);
        Self { sender, receiver }
    }
}

impl Default for ClanService {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(sqlx::Type, PartialEq)]
#[sqlx(type_name = "clan_type")]
enum SqlClanType {
    Open,
    Closed,
    InviteOnly,
}

#[derive(sqlx::Type, PartialEq)]
#[sqlx(type_name = "message_type")]
enum SqlMessageType {
    SystemPositive,
    SystemNegative,
    Player,
}

impl From<ClanType> for SqlClanType {
    fn from(value: ClanType) -> Self {
        match value {
            ClanType::Closed => Self::Closed,
            ClanType::Open => Self::Open,
            ClanType::InviteOnly => Self::InviteOnly,
        }
    }
}

impl From<SqlClanType> for ClanType {
    fn from(value: SqlClanType) -> Self {
        match value {
            SqlClanType::Closed => Self::Closed,
            SqlClanType::Open => Self::Open,
            SqlClanType::InviteOnly => Self::InviteOnly,
        }
    }
}

impl From<SqlMessageType> for MessageType {
    fn from(value: SqlMessageType) -> Self {
        match value {
            SqlMessageType::SystemPositive => Self::SystemPositive,
            SqlMessageType::SystemNegative => Self::SystemNegative,
            SqlMessageType::Player => Self::Player,
        }
    }
}

#[tonic::async_trait]
impl clan_server::Clan for ClanService {
    async fn create_clan(&self, request: Request<ClanInfo>) -> Result<Response<()>, Status> {
        let (_, extensions, request) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        if request.name.is_empty() {
            return Err(Status::permission_denied("Clan name cannot be empty"));
        }
        let (coins,): (i32,) = sqlx::query_as(
            "SELECT coins
            FROM players
            WHERE id = $1",
        )
        .bind(credetials.id)
        .fetch_one(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        if coins - CLAN_CREATION_PRICE < 0 {
            return Err(Status::permission_denied(format!(
                "Need {CLAN_CREATION_PRICE} coins to create a clan"
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
            WHERE id = $1
              AND clan_id IS NOT NULL",
        )
        .bind(credetials.id)
        .fetch_optional(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        .is_some()
        {
            return Err(Status::permission_denied("Player is already in clan"));
        }

        let (id,): (i32,) = sqlx::query_as(
            "WITH cl AS (INSERT INTO chat_rooms DEFAULT VALUES RETURNING id)
            INSERT INTO clans (clan_name, description, min_glory, max_members, type, creator_id, chat_room_id)
            SELECT $1, $2, $3, $4, $5, $6, id FROM cl
            RETURNING id",
        )
        .bind(request.name)
        .bind(request.description)
        .bind(request.min_glory)
        .bind(MAX_MEMBERS)
        .bind::<SqlClanType>(ClanType::from_i32(request.clan_type).unwrap().into())
        .bind(credetials.id)
        .fetch_one(pool)
        .await
        .map_err(|_| Status::permission_denied("Clan description was too long"))?;

        sqlx::query("UPDATE players SET clan_id = $1, coins = coins - $2 WHERE id = $3")
            .bind(id)
            .bind(CLAN_CREATION_PRICE)
            .bind(credetials.id)
            .execute(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        Ok(Response::new(()))
    }

    async fn recommended_clans(
        &self,
        request: Request<Pagination>,
    ) -> Result<Response<ShortClanInfoList>, Status> {
        let (_, extensions, request) = request.into_parts();

        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        let (glory,): (i32,) = sqlx::query_as(
            "SELECT glory
            FROM players
            WHERE id = $1",
        )
        .bind(credetials.id)
        .fetch_one(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        let clans: Vec<ShortClanInfo> = sqlx::query_as(
            "WITH cl AS
            (SELECT COUNT(*) AS members,
                    AVG(glory) AS avg_glory,
                    clan_id
             FROM players
             WHERE clan_id IS NOT NULL
             GROUP BY clan_id)
          SELECT clan_name,
                 CAST(members AS INT),
                 CAST(avg_glory AS INT),
                 max_members,
                 id
          FROM cl
          JOIN clans ON clans.id = cl.clan_id
          WHERE avg_glory >= $1-50
            AND avg_glory <= $1+50
          ORDER BY clan_name
          OFFSET $2
          LIMIT $3",
        )
        .bind(glory)
        .bind(request.offset.unwrap_or(0))
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
            offset: request.offset.unwrap_or(0),
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
                    AVG(glory) AS avg_glory,
                    clan_id
             FROM players
             WHERE clan_id IS NOT NULL
             GROUP BY clan_id)
          SELECT clan_name,
                 CAST(members AS INT),
                 CAST(avg_glory AS INT),
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

    async fn get_clan_info(
        &self,
        request: Request<ClanId>,
    ) -> Result<Response<ClanFullInfo>, Status> {
        let (_, extensions, request) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();

        let row: Option<(String, SqlClanType, i32, Option<String>, i32, i32)> = sqlx::query_as(
            "SELECT clan_name,
                    type,
                    max_members,
                    description,
                    min_glory,
                    creator_id
            FROM clans
            WHERE id = $1",
        )
        .bind(request.id)
        .fetch_optional(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        if let Some((name, clan_type, max_members, description, min_glory, creator_id)) = row {
            let members: Vec<ClanMember> = sqlx::query_as(
                "SELECT nickname,
                        glory,
                        id
                FROM players
                WHERE clan_id = $1
                ORDER BY glory DESC",
            )
            .bind(request.id)
            .fetch_all(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
            .into_iter()
            .map(
                |(nickname, glory, id): (Option<String>, i32, i32)| ClanMember {
                    creator: id == creator_id,
                    nickname,
                    glory,
                    player_id: id,
                },
            )
            .collect();

            return Ok(Response::new(ClanFullInfo {
                id: request.id,
                name,
                max_members,
                average_trophies: members.iter().map(|f| f.glory).sum::<i32>()
                    / members.len() as i32,
                description,
                min_glory,
                clan_type: Into::<ClanType>::into(clan_type).into(),
                members,
            }));
        } else {
            return Err(Status::not_found("Clan not found"));
        }
    }

    async fn join_clan(&self, request: Request<ClanId>) -> Result<Response<()>, Status> {
        let (_, extensions, request) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        if sqlx::query(
            "SELECT NULL
            FROM players
            WHERE id = $1
              AND clan_id IS NOT NULL",
        )
        .bind(credetials.id)
        .fetch_optional(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        .is_some()
        {
            return Err(Status::permission_denied("Player is already in clan"));
        }

        let row: Option<(i32, SqlClanType, i32, i32)> = sqlx::query_as(
            "SELECT min_glory, type, max_members, chat_room_id
            FROM clans
            WHERE id = $1",
        )
        .bind(request.id)
        .fetch_optional(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        if let Some((min_glory, clan_type, max_members, chat_room_id)) = row {
            if clan_type != SqlClanType::Open {
                return Err(Status::permission_denied("Clan is not open"));
            }

            let (glory, nickname): (i32, Option<String>) = sqlx::query_as(
                "SELECT glory, nickname
                FROM players
                WHERE id = $1",
            )
            .bind(credetials.id)
            .fetch_one(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

            if glory < min_glory {
                return Err(Status::permission_denied(
                    "Player's glory is less than clan's minimal glory",
                ));
            }

            let (members_count,): (i32,) =
                sqlx::query_as("SELECT CAST(COUNT(*) AS INT) FROM players WHERE clan_id = $1")
                    .bind(request.id)
                    .fetch_one(pool)
                    .await
                    .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;
            if max_members == members_count {
                return Err(Status::permission_denied("Clan is already full"));
            }

            sqlx::query("UPDATE players SET clan_id = $1 WHERE id = $2")
                .bind(request.id)
                .bind(credetials.id)
                .execute(pool)
                .await
                .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

            let message = format!(
                "{} joined the Clan",
                if nickname.is_some() {
                    nickname.unwrap()
                } else {
                    "Anonymous".to_string()
                }
            );
            let time = Utc::now();
            sqlx::query(
                "INSERT INTO messages (player_id, created_at, content, msg_type, chat_room_id)
                 VALUES($1, $2, $3, 'SystemPositive', $4)
                ",
            )
            .bind(request.id)
            .bind(time)
            .bind(&message)
            .bind(chat_room_id)
            .execute(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;
            self.sender
                .send((
                    request.id,
                    ClanMesage {
                        time: Some(Timestamp {
                            seconds: time.timestamp(),
                            nanos: 0,
                        }),
                        message: Some(TextMessage { text: message }),
                        message_type: MessageType::SystemPositive as i32,
                        sender: None,
                    },
                ))
                .unwrap();
        } else {
            return Err(Status::not_found("Clan not found"));
        }

        Ok(Response::new(()))
    }

    async fn leave_clan(&self, request: Request<()>) -> Result<Response<()>, Status> {
        let (_, extensions, _) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        let (clan_id, nickname): (Option<i32>, Option<String>) = sqlx::query_as(
            "SELECT clan_id, nickname
            FROM players
            WHERE id = $1",
        )
        .bind(credetials.id)
        .fetch_one(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        if clan_id.is_none() {
            return Err(Status::permission_denied("Player is not in clan"));
        }

        sqlx::query("UPDATE players SET clan_id = NULL WHERE id = $1")
            .bind(credetials.id)
            .execute(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        let message = format!(
            "{} has left the Clan",
            if nickname.is_some() {
                nickname.unwrap()
            } else {
                "Anonymous".to_string()
            }
        );
        let time = Utc::now();
        sqlx::query(
            "WITH cl AS
            (SELECT chat_room_id
             FROM clans
             WHERE id = $4)
            INSERT INTO messages (player_id, created_at, content, msg_type, chat_room_id)
             SELECT $1, $2, $3, 'SystemNegative', chat_room_id FROM cl
            ",
        )
        .bind(credetials.id)
        .bind(time)
        .bind(&message)
        .bind(clan_id)
        .execute(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        self.sender
            .send((
                clan_id.unwrap(),
                ClanMesage {
                    time: Some(Timestamp {
                        seconds: time.timestamp(),
                        nanos: 0,
                    }),
                    message: Some(TextMessage { text: message }),
                    message_type: MessageType::SystemNegative as i32,
                    sender: None,
                },
            ))
            .unwrap();

        Ok(Response::new(()))
    }

    async fn send_message(&self, request: Request<TextMessage>) -> Result<Response<()>, Status> {
        let (_, extensions, message) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        if message.text.trim().is_empty() {
            return Err(Status::permission_denied("Empty messages are forbidden"));
        }

        if let Some::<(Option<i32>, i32, Option<String>, bool)>((
            clan_id,
            glory,
            nickname,
            creator,
        )) = sqlx::query_as(
            "WITH cl AS
                (SELECT clan_id,
                    glory,
                    nickname
                FROM players
                WHERE id = $1)
                SELECT cl.clan_id,
                    cl.glory,
                    cl.nickname,
                    (clans.creator_id = $1) AS creator
                FROM cl
                JOIN clans ON clans.id = cl.clan_id",
        )
        .bind(credetials.id)
        .fetch_optional(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        {
            let time = Utc::now();
            sqlx::query(
                "WITH cl AS
            (SELECT chat_room_id
             FROM clans
             WHERE id = $5)
            INSERT INTO messages (player_id, created_at, content, msg_type, chat_room_id)
            SELECT $1, $2, $3, $4, chat_room_id
            FROM cl",
            )
            .bind(credetials.id)
            .bind(time)
            .bind(message.text.trim())
            .bind(SqlMessageType::Player)
            .bind(clan_id)
            .execute(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

            self.sender
                .send((
                    clan_id.unwrap(),
                    ClanMesage {
                        time: Some(Timestamp {
                            seconds: time.timestamp(),
                            nanos: 0,
                        }),
                        message: Some(TextMessage {
                            text: message.text.trim().to_string(),
                        }),
                        message_type: MessageType::Player as i32,
                        sender: Some(ClanMember {
                            creator,
                            glory,
                            nickname,
                            player_id: credetials.id,
                        }),
                    },
                ))
                .unwrap();
            Ok(Response::new(()))
        } else {
            return Err(Status::permission_denied("Player is not in clan"));
        }
    }

    async fn get_messages(
        &self,
        request: Request<Pagination>,
    ) -> Result<Response<ClanMessages>, Status> {
        let (_, extensions, pagination) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        if pagination.offset.is_none() {
            let messages: Vec<(ClanMesage, i64)> =
                sqlx::query_as(
                    "WITH cl AS
                    (SELECT chat_room_id,
                            creator_id
                     FROM clans
                     WHERE id IN
                         (SELECT clan_id
                          FROM players
                          WHERE id = $1)),
                       msg AS
                    (SELECT *
                     FROM
                       (SELECT player_id,
                               created_at,
                               content,
                               msg_type,
                               ROW_NUMBER() OVER (
                                                  ORDER BY created_at) AS row_num
                        FROM messages
                        WHERE chat_room_id IN
                            (SELECT chat_room_id
                             FROM cl)
                        ORDER BY created_at DESC
                        LIMIT $2) sub
                     ORDER BY created_at)
                  SELECT player_id,
                         created_at,
                         content,
                         msg_type,
                         (player_id IN
                            (SELECT creator_id
                             FROM cl)) AS creator,
                         nickname,
                         glory,
                         row_num
                  FROM msg
                  JOIN players ON id = player_id
                  ORDER BY row_num DESC",
                )
                .bind(credetials.id)
                .bind(pagination.limit)
                .fetch_all(pool)
                .await
                .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
                .into_iter()
                .map(
                    |(
                        player_id,
                        created_at,
                        content,
                        msg_type,
                        creator,
                        nickname,
                        glory,
                        row_num,
                    ): (
                        i32,
                        DateTime<Utc>,
                        String,
                        SqlMessageType,
                        bool,
                        Option<String>,
                        i32,
                        i64,
                    )| {
                        (
                            ClanMesage {
                                time: Some(Timestamp {
                                    seconds: created_at.timestamp(),
                                    nanos: 0,
                                }),
                                message: Some(TextMessage { text: content }),
                                message_type: Into::<MessageType>::into(msg_type).into(),
                                sender: Some(ClanMember {
                                    creator,
                                    glory,
                                    nickname,
                                    player_id,
                                }),
                            },
                            row_num,
                        )
                    },
                )
                .collect();
            Ok(Response::new(ClanMessages {
                offset: if messages.is_empty() {
                    0
                } else {
                    messages.last().unwrap().1 as i32 - 1
                },
                messages: messages.into_iter().map(|f| f.0).collect(),
            }))
        } else {
            let messages: Vec<ClanMesage> = sqlx::query_as(
                "WITH cl AS
                    (SELECT chat_room_id,
                            creator_id
                     FROM clans
                     WHERE id IN
                         (SELECT clan_id
                          FROM players
                          WHERE id = $1)),
                       msg AS
                    (SELECT *
                     FROM
                       (SELECT player_id,
                               created_at,
                               content,
                               msg_type
                        FROM messages
                        WHERE chat_room_id IN
                            (SELECT chat_room_id
                             FROM cl)
                        ORDER BY created_at DESC) sub
                     ORDER BY created_at
                     OFFSET $2
                     LIMIT $3)
                  SELECT player_id,
                         created_at,
                         content,
                         msg_type,
                         (player_id IN
                            (SELECT creator_id
                             FROM cl)) AS creator,
                         nickname,
                         glory
                  FROM msg
                  JOIN players ON id = player_id
                  ORDER BY created_at DESC",
            )
            .bind(credetials.id)
            .bind(pagination.offset)
            .bind(pagination.limit)
            .fetch_all(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
            .into_iter()
            .map(
                |(player_id, created_at, content, msg_type, creator, nickname, glory): (
                    i32,
                    DateTime<Utc>,
                    String,
                    SqlMessageType,
                    bool,
                    Option<String>,
                    i32,
                )| {
                    ClanMesage {
                        time: Some(Timestamp {
                            seconds: created_at.timestamp(),
                            nanos: 0,
                        }),
                        message: Some(TextMessage { text: content }),
                        message_type: Into::<MessageType>::into(msg_type).into(),
                        sender: Some(ClanMember {
                            creator,
                            glory,
                            nickname,
                            player_id,
                        }),
                    }
                },
            )
            .collect();
            Ok(Response::new(ClanMessages {
                offset: pagination.offset.unwrap(),
                messages,
            }))
        }
    }

    type ReceiveMessageStream = Pin<Box<dyn Stream<Item = Result<ClanMesage, Status>> + Send>>;

    async fn receive_message(
        &self,
        request: Request<()>,
    ) -> Result<Response<Self::ReceiveMessageStream>, Status> {
        let (_, extensions, _) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();
        let credetials = extensions.get::<Claims>().unwrap();

        let (clan_id,): (Option<i32>,) = sqlx::query_as(
            "SELECT clan_id
                FROM players
                WHERE id = $1",
        )
        .bind(credetials.id)
        .fetch_one(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        if clan_id.is_none() {
            return Err(Status::permission_denied("Player is not in clan"));
        }

        let (tx, rx) = mpsc::channel(128);

        let mut rcv = self.receiver.resubscribe();
        tokio::spawn(async move {
            while let Ok((id, msg)) = rcv.recv().await {
                if id == clan_id.unwrap() && tx.send(Result::<_, Status>::Ok(msg)).await.is_err() {
                    break;
                }
            }
        });

        let output_stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(output_stream) as Self::ReceiveMessageStream
        ))
    }
}
