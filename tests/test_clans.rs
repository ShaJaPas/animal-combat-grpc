mod common;

use animal_combat_grpc::services::{
    auth::{auth_client::AuthClient, JwtPair, LoginRequest},
    clans::{
        clan_client::ClanClient, ClanId, ClanInfo, ClanType, Pagination, SearchClansRequest,
        TextMessage,
    },
};
use sqlx::PgPool;
use tonic::{Code, Request};

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
async fn test_clan_creation(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;

    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });

    //Creation of clan with 0 coins gives failure
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    assert!(client.create_clan(request).await.err().unwrap().code() == Code::PermissionDenied);

    //Creation of clan with more than 1000 coins gives success
    sqlx::query("UPDATE players SET coins = 1000")
        .execute(&pool)
        .await?;
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    let (coins,): (i32,) = sqlx::query_as("SELECT coins from players")
        .fetch_one(&pool)
        .await?;
    assert!(coins == 0);
    Ok(())
}

#[sqlx::test]
async fn test_clan_exists_in_creation(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;

    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });

    sqlx::query("UPDATE players SET coins = 1000")
        .execute(&pool)
        .await?;

    //Create a clan
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    sqlx::query("UPDATE players SET coins = 1000")
        .execute(&pool)
        .await?;

    //Creation of clan with a same name gives an error
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    assert!(client.create_clan(request).await.err().unwrap().code() == Code::AlreadyExists);

    Ok(())
}

#[sqlx::test]
async fn test_clan_create_constraints(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;

    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });

    sqlx::query("UPDATE players SET coins = 1000")
        .execute(&pool)
        .await?;

    //min_glory constraint
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 150,
        clan_type: ClanType::Open.into(),
    });
    assert!(client.create_clan(request).await.err().unwrap().code() == Code::PermissionDenied);

    //description constraint
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: Some("1".repeat(256)),
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    assert!(client.create_clan(request).await.err().unwrap().code() == Code::PermissionDenied);

    //name constraint
    let request = Request::new(ClanInfo {
        name: "".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    assert!(client.create_clan(request).await.err().unwrap().code() == Code::PermissionDenied);

    Ok(())
}

#[sqlx::test]
async fn test_create_clan_already_in_clan(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;

    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });

    sqlx::query("UPDATE players SET coins = 1000")
        .execute(&pool)
        .await?;

    //Create a clan
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    //Cannot create another clan
    let request = Request::new(ClanInfo {
        name: "Test2".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    assert!(client.create_clan(request).await.err().unwrap().code() == Code::PermissionDenied);

    Ok(())
}

#[sqlx::test]
async fn test_clan_join(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });
    sqlx::query("UPDATE players SET coins = 1000")
        .execute(&pool)
        .await?;

    //Create a clan
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test2@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });

    let request = Request::new(ClanId { id: 1 });
    client.join_clan(request).await?;

    //Already in clan
    let request = Request::new(ClanId { id: 1 });
    assert!(client.join_clan(request).await.err().unwrap().code() == Code::PermissionDenied);

    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test3@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });

    //Clan does not exists
    let request = Request::new(ClanId { id: 2 });
    assert!(client.join_clan(request).await.err().unwrap().code() == Code::NotFound);

    sqlx::query("UPDATE clans SET type = 'Closed'")
        .execute(&pool)
        .await?;

    //Clan is closed
    let request = Request::new(ClanId { id: 1 });
    assert!(client.join_clan(request).await.err().unwrap().code() == Code::PermissionDenied);

    sqlx::query("UPDATE clans SET type = 'Open', min_glory = 300")
        .execute(&pool)
        .await?;

    //PLayer has lower glory
    let request = Request::new(ClanId { id: 1 });
    assert!(client.join_clan(request).await.err().unwrap().code() == Code::PermissionDenied);
    Ok(())
}

#[sqlx::test]
async fn test_clan_leave(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });
    sqlx::query("UPDATE players SET coins = 1000")
        .execute(&pool)
        .await?;

    //Create a clan
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test2@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });

    //Not in clan
    let request = Request::new(());
    assert!(client.leave_clan(request).await.err().unwrap().code() == Code::PermissionDenied);

    let request = Request::new(ClanId { id: 1 });
    client.join_clan(request).await?;

    //Leave clan
    let request = Request::new(());
    client.leave_clan(request).await?;

    //Test if he can join again
    let request = Request::new(ClanId { id: 1 });
    client.join_clan(request).await?;

    Ok(())
}

#[sqlx::test]
async fn test_search_clans(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });
    sqlx::query("UPDATE players SET coins = 1000")
        .execute(&pool)
        .await?;
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test2@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });
    sqlx::query("UPDATE players SET coins = 1000")
        .execute(&pool)
        .await?;
    let request = Request::new(ClanInfo {
        name: "Est".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    let request = Request::new(SearchClansRequest {
        offset: 0,
        limit: 10,
        pattern: "est".to_owned(),
    });
    assert!(client.search_clans(request).await?.into_inner().infos.len() == 2);

    let request = Request::new(SearchClansRequest {
        offset: 1,
        limit: 10,
        pattern: "est".to_owned(),
    });
    assert!(client.search_clans(request).await?.into_inner().infos.len() == 1);

    let request = Request::new(SearchClansRequest {
        offset: 0,
        limit: 0,
        pattern: "est".to_owned(),
    });
    assert!(client
        .search_clans(request)
        .await?
        .into_inner()
        .infos
        .is_empty());

    Ok(())
}

#[sqlx::test]
async fn test_recommended_clans(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });
    sqlx::query("UPDATE players SET coins = 1000, glory = 49")
        .execute(&pool)
        .await?;
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test2@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });
    sqlx::query("UPDATE players SET coins = 1000, glory = 100 WHERE refresh_token = $1")
        .bind(&user_response.refresh_token)
        .execute(&pool)
        .await?;
    let request = Request::new(ClanInfo {
        name: "Est".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    let request = Request::new(Pagination {
        offset: None,
        limit: 10,
    });
    assert!(
        client
            .recommended_clans(request)
            .await?
            .into_inner()
            .infos
            .len()
            == 1
    );
    sqlx::query("UPDATE players SET coins = 1000, glory = 70 WHERE refresh_token = $1")
        .bind(&user_response.refresh_token)
        .execute(&pool)
        .await?;

    let request = Request::new(Pagination {
        offset: None,
        limit: 10,
    });
    assert!(
        client
            .recommended_clans(request)
            .await?
            .into_inner()
            .infos
            .len()
            == 2
    );

    let request = Request::new(Pagination {
        offset: Some(1),
        limit: 10,
    });
    assert!(
        client
            .recommended_clans(request)
            .await?
            .into_inner()
            .infos
            .len()
            == 1
    );

    let request = Request::new(Pagination {
        offset: Some(1),
        limit: 0,
    });
    assert!(client
        .recommended_clans(request)
        .await?
        .into_inner()
        .infos
        .is_empty());
    Ok(())
}

#[sqlx::test]
async fn test_clan_info(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });
    sqlx::query("UPDATE players SET coins = 1000, glory = 49")
        .execute(&pool)
        .await?;
    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test2@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });

    client.join_clan(Request::new(ClanId { id: 1 })).await?;

    assert!(
        client
            .get_clan_info(Request::new(ClanId { id: 2 }))
            .await
            .err()
            .unwrap()
            .code()
            == Code::NotFound
    );
    assert!(
        client
            .get_clan_info(Request::new(ClanId { id: 1 }))
            .await?
            .into_inner()
            .members
            .len()
            == 2
    );

    assert!(
        client
            .get_clan_info(Request::new(ClanId { id: 1 }))
            .await?
            .into_inner()
            .members[0]
            .glory
            == 49
    );
    Ok(())
}

#[sqlx::test]
async fn test_send_message(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });
    sqlx::query("UPDATE players SET coins = 1000, glory = 49")
        .execute(&pool)
        .await?;

    assert!(
        client
            .send_message(Request::new(TextMessage {
                text: "  ".to_string()
            }))
            .await
            .err()
            .unwrap()
            .code()
            == Code::PermissionDenied
    );

    assert!(
        client
            .send_message(Request::new(TextMessage {
                text: "Some message".to_string(),
            }))
            .await
            .err()
            .unwrap()
            .code()
            == Code::PermissionDenied
    );

    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    client
        .send_message(Request::new(TextMessage {
            text: "Some message".to_string(),
        }))
        .await?;
    Ok(())
}

#[sqlx::test]
async fn test_get_messages(pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let channel = get_test_channel(pool.clone()).await?;
    let user_response = create_user(&pool, "test@gmail.com".to_owned()).await?;

    let mut client = ClanClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", user_response.access_token.parse().unwrap());
        Ok(req)
    });
    sqlx::query("UPDATE players SET coins = 1000, glory = 49")
        .execute(&pool)
        .await?;

    let request = Request::new(ClanInfo {
        name: "Test".to_owned(),
        description: None,
        min_glory: 0,
        clan_type: ClanType::Open.into(),
    });
    client.create_clan(request).await?;

    assert!(client
        .get_messages(Request::new(Pagination {
            offset: None,
            limit: 10
        }))
        .await?
        .into_inner()
        .messages
        .is_empty());

    client
        .send_message(Request::new(TextMessage {
            text: "Some message".to_string(),
        }))
        .await?;
    client
        .send_message(Request::new(TextMessage {
            text: "Another message".to_string(),
        }))
        .await?;

    assert!(
        client
            .get_messages(Request::new(Pagination {
                offset: None,
                limit: 10
            }))
            .await?
            .into_inner()
            .messages
            .len()
            == 2
    );

    assert!(
        client
            .get_messages(Request::new(Pagination {
                offset: Some(0),
                limit: 1
            }))
            .await?
            .into_inner()
            .messages[0]
            .message
            .as_ref()
            .unwrap()
            .text
            == "Some message"
    );

    assert!(
        client
            .get_messages(Request::new(Pagination {
                offset: None,
                limit: 1
            }))
            .await?
            .into_inner()
            .messages[0]
            .message
            .as_ref()
            .unwrap()
            .text
            == "Another message"
    );

    Ok(())
}
