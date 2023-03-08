use std::time::Duration;

use animal_combat_grpc::{
    jwt_interceptor,
    services::{
        auth::{AuthServer, AuthService},
        clans::{ClanServer, ClanService},
    },
};
use sqlx::postgres::PgPoolOptions;
use tonic::{transport::Server, Request};

use http::Method;
use tonic_web::GrpcWebLayer;
use tower_http::cors::{Any, CorsLayer};

#[tokio::main()]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv()?;

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&std::env::var("DATABASE_URL").unwrap())
        .await?;

    let addr = "0.0.0.0:3009".parse().unwrap();
    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<AuthServer<AuthService>>()
        .await;

    //Create services
    let auth = AuthService::default();
    let clans = ClanService::default();

    //Add cors support
    let cors_layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any)
        .expose_headers([
            "grpc-message".parse().unwrap(),
            "grpc-status".parse().unwrap(),
        ])
        .allow_origin(Any);

    let layer = tower::ServiceBuilder::new()
        .timeout(Duration::from_secs(30))
        .layer(tonic::service::interceptor(move |mut req: Request<()>| {
            req.extensions_mut().insert(pool.clone());
            Ok(req)
        }))
        .into_inner();

    Server::builder()
        .accept_http1(true)
        .layer(cors_layer)
        .layer(GrpcWebLayer::new())
        .layer(layer)
        .add_service(health_service)
        .add_service(AuthServer::new(auth))
        .add_service(ClanServer::with_interceptor(clans, jwt_interceptor))
        .serve(addr)
        .await?;

    Ok(())
}
