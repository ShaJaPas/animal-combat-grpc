use animal_combat_grpc::{
    jwt_interceptor, run_matchmaking_loop,
    services::{
        auth::{AuthServer, AuthService},
        battle::{BattleServer, BattleService},
        clans::{ClanServer, ClanService},
        players::{PlayerServer, PlayerService},
    },
};
use sqlx::PgPool;
use tokio::sync::{broadcast, mpsc};

use std::time::Duration;
use tonic::{
    transport::{Channel, Endpoint, Server, Uri},
    Request,
};
use tower::service_fn;

//NEVER FORGET TO UPDATE THIS (ADD NEW SERVICES)
pub async fn get_test_channel(pool: PgPool) -> Result<Channel, Box<dyn std::error::Error>> {
    let (client, server) = tokio::io::duplex(1024);

    //Create services
    let auth = AuthService::default();
    let clans = ClanService::default();
    let players = PlayerService::default();
    let (tx, rx) = mpsc::channel(128);
    let (tx2, rx2) = broadcast::channel(128);
    tokio::spawn(run_matchmaking_loop(rx, tx2));
    let battle = BattleService {
        sender: tx,
        receiver: rx2,
    };

    let layer = tower::ServiceBuilder::new()
        .timeout(Duration::from_secs(30))
        .layer(tonic::service::interceptor(move |mut req: Request<()>| {
            req.extensions_mut().insert(pool.clone());
            Ok(req)
        }))
        .into_inner();

    tokio::spawn(async move {
        Server::builder()
            .layer(layer)
            .add_service(AuthServer::new(auth))
            .add_service(ClanServer::with_interceptor(clans, jwt_interceptor))
            .add_service(PlayerServer::with_interceptor(players, jwt_interceptor))
            .add_service(BattleServer::with_interceptor(battle, jwt_interceptor))
            .serve_with_incoming(futures::stream::iter(vec![Ok::<_, std::io::Error>(server)]))
            .await
    });
    let mut client = Some(client);
    Ok(Endpoint::try_from("http://[::]:50051")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let client = client.take();

            async move {
                if let Some(client) = client {
                    Ok(client)
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Client already taken",
                    ))
                }
            }
        }))
        .await?)
}
