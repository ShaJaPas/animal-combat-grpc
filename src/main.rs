use std::time::Duration;

use animal_combat_grpc::services::auth::{Auth, AuthServer};
use sqlx::postgres::PgPoolOptions;
use tonic::{transport::Server, Request};

#[tokio::main()]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv()?;

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&std::env::var("DATABASE_URL").unwrap())
        .await?;

    let addr = "[::1]:3009".parse().unwrap();
    let auth = Auth::default();

    let layer = tower::ServiceBuilder::new()
        .timeout(Duration::from_secs(30))
        .layer(tonic::service::interceptor(move |mut req: Request<()>| {
            req.extensions_mut().insert(pool.clone());
            Ok(req)
        }))
        .into_inner();

    Server::builder()
        .layer(layer)
        .add_service(AuthServer::new(auth))
        .serve(addr)
        .await?;

    Ok(())
}
