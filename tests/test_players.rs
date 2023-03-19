mod common;

use animal_combat_grpc::services::{
    auth::{auth_client::AuthClient, JwtPair, LoginRequest},
    players::player_client::PlayerClient,
};
use sqlx::PgPool;
use tonic::Request;

use crate::common::get_test_channel;

async fn create_user(pool: &PgPool, email: String) -> Result<JwtPair, Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let mut client = AuthClient::new(channel);

    //Create test user
    let user_credentials = LoginRequest {
        email,
        password: "TestPass".to_string(),
    };
    let request = Request::new(user_credentials.clone());

    Ok(client.sign_up(request).await?.into_inner())
}

#[sqlx::test]
async fn test_token_rotation(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = PlayerClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });

    client.get_profile(Request::new(())).await?;

    Ok(())
}
