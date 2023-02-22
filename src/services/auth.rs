use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

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

//TODO: Вынесети генерацию токенов в макрос или метод
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

            let mut claims = Claims {
                email,
                exp: Utc::now().timestamp() + REFRESH_TOKEN_EXP_TIME,
            };
            let header = Header::default();
            let key = EncodingKey::from_base64_secret(dotenv!("JWT_SECRET")).unwrap();
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

        let mut claims = Claims {
            email: credentials.email,
            exp: Utc::now().timestamp() + REFRESH_TOKEN_EXP_TIME,
        };
        let header = Header::default();
        let key = EncodingKey::from_base64_secret(dotenv!("JWT_SECRET")).unwrap();
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

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hashed_password = argon2
            .hash_password(credentials.password.as_bytes(), &salt)
            .map_err(|e| Status::internal(format!("Password hashing failed: {e}")))?;

        sqlx::query(
            "INSERT INTO players (email, hashed_password, refresh_token) VALUES ($1, $2, $3)",
        )
        .bind(claims.email)
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
            &DecodingKey::from_base64_secret(dotenv!("JWT_SECRET")).unwrap(),
            &Validation::default(),
        )
        .map_err(|_| Status::unauthenticated("Token invalid or expired"))?
        .claims;

        let (id, refresh): (i32, String) =
            sqlx::query_as("SELECT id, refresh_token FROM players WHERE email = $1;")
                .bind(&claims.email)
                .fetch_one(&pool)
                .await
                .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        if refresh != token.token {
            return Err(Status::unauthenticated("Token is invalid"));
        }

        let mut claims = Claims {
            email: claims.email,
            exp: Utc::now().timestamp() + REFRESH_TOKEN_EXP_TIME,
        };
        let header = Header::default();
        let key = EncodingKey::from_base64_secret(dotenv!("JWT_SECRET")).unwrap();
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

        sqlx::query("UPDATE players SET refresh_token = $1 WHERE id = $2")
            .bind(&refresh_token)
            .bind(id)
            .execute(&pool)
            .await
            .map_err(|e| Status::data_loss(format!("Database error: {e}")))?;

        Ok(Response::new(JwtPair {
            access_token,
            refresh_token,
            access_token_expiry,
        }))
    }
}
