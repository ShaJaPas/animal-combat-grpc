use std::time::Duration;

use animal_combat_grpc::{
    jwt_interceptor, run_battles_loop, run_matchmaking_loop,
    services::{
        auth::{AuthServer, AuthService},
        battle::{BattleServer, BattleService},
        clans::{ClanServer, ClanService},
        players::{PlayerServer, PlayerService},
    },
};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    ConnectOptions,
};
use tokio::sync::{broadcast, mpsc};
use tonic::{
    transport::{Body, Server},
    Request,
};

use http::Method;
use tonic_web::GrpcWebLayer;
use tower_http::{
    classify::{GrpcCode, GrpcErrorsAsFailures, SharedClassifier},
    cors::{Any, CorsLayer},
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::{log::LevelFilter, Level};

#[tokio::main()]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv()?;
    tracing_subscriber::fmt()
        .with_max_level(Level::ERROR)
        .init();

    let mut connect_options = std::env::var("DATABASE_URL")
        .unwrap()
        .parse::<PgConnectOptions>()?;
    connect_options.log_statements(LevelFilter::Trace);

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(connect_options)
        .await?;

    let addr = "0.0.0.0:3009".parse().unwrap();
    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<AuthServer<AuthService>>()
        .await;

    // Create services
    let auth = AuthService::default();
    let clans = ClanService::default();
    let players = PlayerService::default();
    let (tx, rx) = mpsc::channel(128);
    let (tx2, rx2) = broadcast::channel(128);
    let (battle_tx2, battle_rx2) = broadcast::channel(128);
    let (battle_tx, battle_rx) = mpsc::channel(128);
    tokio::spawn(run_matchmaking_loop(rx, tx2, battle_tx.clone()));
    tokio::spawn(run_battles_loop(battle_rx, battle_tx2));
    let battle = BattleService {
        sender: tx,
        receiver: rx2,
        battle_rx: battle_rx2,
        battle_tx,
    };

    // Add cors support
    let cors_layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any)
        .expose_headers([
            "grpc-message".parse().unwrap(),
            "grpc-status".parse().unwrap(),
        ])
        .allow_origin(Any);

    // Response classifier that doesn't consider this codes as failures
    let classifier = GrpcErrorsAsFailures::new()
        .with_success(GrpcCode::InvalidArgument)
        .with_success(GrpcCode::AlreadyExists)
        .with_success(GrpcCode::PermissionDenied)
        .with_success(GrpcCode::Unauthenticated)
        .with_success(GrpcCode::NotFound);

    // Add tracing
    let tracing_layer = tower::ServiceBuilder::new()
        .layer(
            TraceLayer::new(SharedClassifier::new(classifier))
                .make_span_with(|request: &http::Request<Body>| {
                    tracing::error_span!(
                        "request",
                        uri = %request.uri(),
                        version = ?request.version()
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .into_inner();

    let layer = tower::ServiceBuilder::new()
        .timeout(Duration::from_secs(30))
        .layer(tonic::service::interceptor(move |mut req: Request<()>| {
            req.extensions_mut().insert(pool.clone());
            Ok(req)
        }))
        .into_inner();

    Server::builder()
        .accept_http1(true)
        .layer(tracing_layer)
        .layer(cors_layer)
        .layer(GrpcWebLayer::new())
        .layer(layer)
        .add_service(health_service)
        .add_service(AuthServer::new(auth))
        .add_service(ClanServer::with_interceptor(clans, jwt_interceptor))
        .add_service(PlayerServer::with_interceptor(players, jwt_interceptor))
        .add_service(BattleServer::with_interceptor(battle, jwt_interceptor))
        .serve(addr)
        .await?;

    Ok(())
}
