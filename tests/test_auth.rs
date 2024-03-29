mod common;

use animal_combat_grpc::services::auth::{auth_client::AuthClient, JwtPair, LoginRequest, Token};
use sqlx::PgPool;
use tonic::{Code, Request};

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

    //User cannot login in
    let request = Request::new(Token {
        token: user_response.refresh_token.clone(),
    });
    assert!(client.obtain_jwt_pair(request).await.err().unwrap().code() == Code::Unauthenticated);

    //Then user login using his credentials
    let request = Request::new(user_credentials);
    client.sign_in(request).await?;

    //After that, hacker's refresh is useless
    let request = Request::new(Token {
        token: hacker_response.refresh_token.clone(),
    });
    assert!(client.obtain_jwt_pair(request).await.err().unwrap().code() == Code::Unauthenticated);

    Ok(())
}

#[sqlx::test]
async fn test_upper_case_email(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let mut client = AuthClient::new(channel);
    let user_credentials = LoginRequest {
        email: "test@gmail.com".to_string(),
        password: "TestPass".to_string(),
    };
    let request = Request::new(user_credentials);

    //User logs in
    client.sign_up(request).await?;

    let user_credentials = LoginRequest {
        email: "TeSt@gmAil.com".to_string(),
        password: "TestPass".to_string(),
    };
    let request = Request::new(user_credentials);

    //He can sign in using upper case
    client.sign_in(request).await?;

    Ok(())
}

#[sqlx::test]
async fn test_two_times_sign_up(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let mut client = AuthClient::new(channel);
    let user_credentials = LoginRequest {
        email: "test@gmail.com".to_string(),
        password: "TestPass".to_string(),
    };
    let request = Request::new(user_credentials.clone());

    //User logs in
    client.sign_up(request).await?;

    let request = Request::new(user_credentials);

    //He can't sign up twice
    assert!(client.sign_up(request).await.err().unwrap().code() == Code::AlreadyExists);

    Ok(())
}

#[sqlx::test]
async fn test_wrong_email_and_password(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let mut client = AuthClient::new(channel);
    let user_credentials = LoginRequest {
        email: "test@gmail.com".to_string(),
        password: "TestPass".to_string(),
    };
    let request = Request::new(user_credentials);

    //User logs in
    client.sign_up(request).await?;

    let user_credentials = LoginRequest {
        email: "test1@gmail.com".to_string(),
        password: "TestPass".to_string(),
    };
    let request = Request::new(user_credentials);

    //He uses wrong email
    assert!(client.sign_in(request).await.err().unwrap().code() == Code::NotFound);

    let user_credentials = LoginRequest {
        email: "test@gmail.com".to_string(),
        password: "TestPass1".to_string(),
    };
    let request = Request::new(user_credentials);
    //He uses wrong password
    assert!(client.sign_in(request).await.err().unwrap().code() == Code::PermissionDenied);

    Ok(())
}
