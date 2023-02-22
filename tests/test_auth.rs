mod common;

use animal_combat_grpc::services::auth::{auth_client::AuthClient, JwtPair, LoginRequest, Token};
use sqlx::PgPool;
use std::time::Duration;
use tonic::Request;

use crate::common::get_test_channel;

#[sqlx::test]
async fn test_token_rotation(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let mut client = AuthClient::new(channel);

    let user_credentials = LoginRequest {
        email: "test@gmail.com".to_string(),
        password: "TestPass".to_string(),
    };
    let request = Request::new(user_credentials.clone());

    //User logs in
    let user_response: JwtPair = client.sign_up(request).await?.into_inner();

    //Imagine refresh was stolen and hacker used it
    let request = Request::new(Token {
        token: user_response.refresh_token.clone(),
    });

    let hacker_response: JwtPair = client.obtain_jwt_pair(request).await?.into_inner();
    std::thread::sleep(Duration::from_secs_f32(1.1));

    //User cannot login in
    let request = Request::new(Token {
        token: user_response.refresh_token.clone(),
    });
    assert!(client.obtain_jwt_pair(request).await.is_err());

    //Then user login using his credentials
    let request = Request::new(user_credentials);
    assert!(client.sign_in(request).await.is_ok());
    std::thread::sleep(Duration::from_secs_f32(1.1));

    //After that, hacker's refresh is useless
    let request = Request::new(Token {
        token: hacker_response.refresh_token.clone(),
    });
    assert!(client.obtain_jwt_pair(request).await.is_err());

    Ok(())
}
