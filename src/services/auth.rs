use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use uuid::Uuid;

use chrono::Utc;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use rand_core::OsRng;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};
use tonic::{Request, Response, Status};

pub type AuthServer<T> = auth_server::AuthServer<T>;

tonic::include_proto!("auth");

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub id: i32,
    exp: i64,
}

const ACCESS_TOKEN_EXP_TIME: i64 = 30 * 60; //30min
const REFRESH_TOKEN_EXP_TIME: i64 = 60 * 60 * 24 * 30; //30 days

#[derive(Default)]
pub struct AuthService;

#[inline]
fn generate_jwt_pair(id: i32) -> Result<(String, String, i64), Status> {
    let mut claims = Claims {
        id,
        exp: Utc::now().timestamp() + REFRESH_TOKEN_EXP_TIME,
    };
    let header = Header {
        kid: Some(Uuid::new_v4().to_string()),
        ..Default::default()
    };
    let key = EncodingKey::from_base64_secret(&std::env::var("JWT_SECRET").unwrap()).unwrap();
    let refresh_token = match jsonwebtoken::encode(&header, &claims, &key) {
        Ok(token) => token,
        Err(e) => {
            return Err(Status::internal(format!(
                "Refresh token generation failed: {e}"
            )));
        }
    };

    let access_token_expiry = Utc::now().timestamp() + ACCESS_TOKEN_EXP_TIME;
    claims.exp = access_token_expiry;
    let access_token = match jsonwebtoken::encode(&header, &claims, &key) {
        Ok(token) => token,
        Err(e) => {
            return Err(Status::internal(format!(
                "Refresh token generation failed: {e}"
            )));
        }
    };
    Ok((access_token, refresh_token, access_token_expiry))
}

#[tonic::async_trait]
impl auth_server::Auth for AuthService {
    async fn sign_in(&self, request: Request<LoginRequest>) -> Result<Response<JwtPair>, Status> {
        let (_, extensions, mut credentials) = request.into_parts();
        let pool = extensions.get::<Pool<Postgres>>().unwrap();

        //Make email lowercase to store in Database
        credentials.email = credentials.email.to_lowercase();

        let row: Option<(i32, String)> = sqlx::query_as(
            "SELECT id,
                   hashed_password
            FROM players
            WHERE email = $1",
        )
        .bind(&credentials.email)
        .fetch_optional(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;
        if let Some((id, hashed_password)) = row {
            let hash = PasswordHash::new(&hashed_password).unwrap();
            let argon2 = Argon2::default();
            argon2
                .verify_password(credentials.password.as_bytes(), &hash)
                .map_err(|_| Status::permission_denied("Password validation failed"))?;

            let (access_token, refresh_token, access_token_expiry) = generate_jwt_pair(id)?;
            sqlx::query(
                "UPDATE players
                SET refresh_token = $1
                WHERE id = $2",
            )
            .bind(&refresh_token)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

            return Ok(Response::new(JwtPair {
                access_token,
                refresh_token,
                access_token_expiry,
            }));
        } else {
            return Err(Status::not_found(format!(
                "Player with email '{}' not found",
                credentials.email
            )));
        }
    }

    async fn sign_up(&self, request: Request<LoginRequest>) -> Result<Response<JwtPair>, Status> {
        let (_, extensions, mut credentials) = request.into_parts();

        let pool = extensions.get::<Pool<Postgres>>().unwrap();

        //Make email lowercase to store in Database
        credentials.email = credentials.email.to_lowercase();

        if sqlx::query(
            "SELECT NULL
            FROM players
            WHERE email = $1",
        )
        .bind(&credentials.email)
        .fetch_optional(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        .is_some()
        {
            return Err(Status::already_exists(format!(
                "Player with email '{}' already exists",
                credentials.email
            )));
        }

        let email_regex =
            Regex::new(r"^[a-zA-Z0-9.!#$%&’*+/=?^_`{|}~-]+@[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)*$")
                .unwrap();

        if !email_regex.is_match(&credentials.email) {
            return Err(Status::unauthenticated("Invalid email is provided"));
        }

        let password_regex = Regex::new(r"^[0-9a-zA-Z]{8,}$").unwrap();

        if !password_regex.is_match(&credentials.password) {
            return Err(Status::unauthenticated(
                "Password must be at least 8 characters and contain only latin letters and numbers",
            ));
        }

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hashed_password = argon2
            .hash_password(credentials.password.as_bytes(), &salt)
            .map_err(|e| Status::internal(format!("Password hashing failed: {e}")))?;

        let (id,): (i32,) = sqlx::query_as(
            "INSERT INTO players (email, hashed_password, refresh_token)
            VALUES ($1, $2, '') RETURNING id",
        )
        .bind(credentials.email)
        .bind(hashed_password.to_string())
        .fetch_one(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        let (access_token, refresh_token, access_token_expiry) = generate_jwt_pair(id)?;

        sqlx::query("UPDATE players SET refresh_token = $2 WHERE id = $1")
            .bind(id)
            .bind(&refresh_token)
            .execute(pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        sqlx::query(
            "INSERT INTO players_emotes (player_id, emote_id)
            SELECT $1,
                   id
            FROM emotes
            WHERE file_name = 'sad'
            UNION ALL
            SELECT $1,
                   id
            FROM emotes
            WHERE file_name = 'happy';",
        )
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        Ok(Response::new(JwtPair {
            access_token,
            refresh_token,
            access_token_expiry,
        }))
    }

    async fn obtain_jwt_pair(&self, request: Request<Token>) -> Result<Response<JwtPair>, Status> {
        let (_, extensions, token) = request.into_parts();

        let pool = extensions.get::<Pool<Postgres>>().unwrap();

        let claims = jsonwebtoken::decode::<Claims>(
            &token.token,
            &DecodingKey::from_base64_secret(&std::env::var("JWT_SECRET").unwrap()).unwrap(),
            &Validation::default(),
        )
        .map_err(|_| Status::unauthenticated("Token invalid or expired"))?
        .claims;

        let (access_token, refresh_token, access_token_expiry) = generate_jwt_pair(claims.id)?;

        if sqlx::query(
            "UPDATE players
            SET refresh_token = $1
            WHERE id = $2
              AND refresh_token = $3",
        )
        .bind(&refresh_token)
        .bind(claims.id)
        .bind(&token.token)
        .execute(pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?
        .rows_affected()
            == 0
        {
            return Err(Status::unauthenticated("Token is invalid"));
        }

        Ok(Response::new(JwtPair {
            access_token,
            refresh_token,
            access_token_expiry,
        }))
    }
}
