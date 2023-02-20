mod services;

use std::{env, time::Duration};

use dotenv::dotenv;
use jsonwebtoken::{DecodingKey, Validation};
use services::auth::{Auth, AuthServer};
use sqlx::postgres::PgPoolOptions;
use tonic::{transport::Server, Request, Status};

//Put this in any service, except Auth
fn jwt_interceptor(mut req: Request<()>) -> Result<Request<()>, Status> {
    let token = match req.metadata().get("authorization") {
        Some(token) => token.to_str(),
        None => return Err(Status::unauthenticated("JWT token not found")),
    };

    if let Ok(token) = token {
        let claims = match jsonwebtoken::decode::<services::auth::Claims>(
            token,
            &DecodingKey::from_base64_secret(&env::var("JWT_SECRET").unwrap()).unwrap(),
            &Validation::default(),
        ) {
            Ok(claims) => claims,
            Err(e) => {
                return Err(Status::unauthenticated(e.to_string()));
            }
        };
        req.extensions_mut().insert(claims);
    } else {
        return Err(Status::unauthenticated(
            "Key \"authorization\" was invalid string",
        ));
    }
    Ok(req)
}

#[tokio::main()]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&env::var("DATABASE_URL").unwrap())
        .await?;

    let addr = "0.0.0.0:3009".parse().unwrap();
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
