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
    email: String,
    exp: i64,
}

const ACCESS_TOKEN_EXP_TIME: i64 = 1800; //30min
const REFRESH_TOKEN_EXP_TIME: i64 = 86400; //1 day

#[derive(Default)]
pub struct Auth;

#[inline]
fn generate_jwt_pair(email: String) -> Result<(String, String, i64), Status> {
    let mut claims = Claims {
        email,
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
impl auth_server::Auth for Auth {
    async fn sign_in(&self, request: Request<LoginRequest>) -> Result<Response<JwtPair>, Status> {
        let pool = request
            .extensions()
            .get::<Pool<Postgres>>()
            .unwrap()
            .clone();

        let credentials = request.into_inner();
        let row: Option<(i32, String, String)> =
            sqlx::query_as("SELECT id, hashed_password, email FROM players WHERE email = $1;")
                .bind(&credentials.email)
                .fetch_optional(&pool)
                .await
                .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;
        if let Some((id, hashed_password, email)) = row {
            let hash = PasswordHash::new(&hashed_password).unwrap();
            let argon2 = Argon2::default();
            argon2
                .verify_password(credentials.password.as_bytes(), &hash)
                .map_err(|_| Status::unauthenticated("Password validation failed"))?;

            let (access_token, refresh_token, access_token_expiry) = generate_jwt_pair(email)?;
            sqlx::query("UPDATE players SET refresh_token = $1 WHERE id = $2")
                .bind(&refresh_token)
                .bind(id)
                .execute(&pool)
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
        let pool = request
            .extensions()
            .get::<Pool<Postgres>>()
            .unwrap()
            .clone();

        let credentials = request.into_inner();

        let row = sqlx::query("SELECT null FROM players WHERE email = $1;")
            .bind(&credentials.email)
            .fetch_optional(&pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        if row.is_some() {
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

        let (access_token, refresh_token, access_token_expiry) =
            generate_jwt_pair(credentials.email.clone())?;

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hashed_password = argon2
            .hash_password(credentials.password.as_bytes(), &salt)
            .map_err(|e| Status::internal(format!("Password hashing failed: {e}")))?;

        sqlx::query(
            "INSERT INTO players (email, hashed_password, refresh_token) VALUES ($1, $2, $3)",
        )
        .bind(credentials.email)
        .bind(hashed_password.to_string())
        .bind(&refresh_token)
        .execute(&pool)
        .await
        .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        Ok(Response::new(JwtPair {
            access_token,
            refresh_token,
            access_token_expiry,
        }))
    }

    async fn obtain_jwt_pair(&self, request: Request<Token>) -> Result<Response<JwtPair>, Status> {
        let pool = request
            .extensions()
            .get::<Pool<Postgres>>()
            .unwrap()
            .clone();

        let token = request.into_inner();
        let claims = jsonwebtoken::decode::<Claims>(
            &token.token,
            &DecodingKey::from_base64_secret(&std::env::var("JWT_SECRET").unwrap()).unwrap(),
            &Validation::default(),
        )
        .map_err(|_| Status::unauthenticated("Token invalid or expired"))?
        .claims;

        let (access_token, refresh_token, access_token_expiry) =
            generate_jwt_pair(claims.email.clone())?;

        if sqlx::query(
            "UPDATE players SET refresh_token = $1 WHERE email = $2 AND refresh_token = $3",
        )
        .bind(&refresh_token)
        .bind(&claims.email)
        .bind(&token.token)
        .execute(&pool)
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
