pub mod services;

use jsonwebtoken::{DecodingKey, Validation};
use tonic::{Request, Status};

//Put this in any service, except Auth
pub fn jwt_interceptor(mut req: Request<()>) -> Result<Request<()>, Status> {
    let token = match req.metadata().get("authorization") {
        Some(token) => token.to_str(),
        None => return Err(Status::unauthenticated("JWT token not found")),
    };

    if let Ok(token) = token {
        let claims = jsonwebtoken::decode::<services::auth::Claims>(
            token,
            &DecodingKey::from_base64_secret(&std::env::var("JWT_SECRET").unwrap()).unwrap(),
            &Validation::default(),
        )
        .map_err(|e| Status::unauthenticated(e.to_string()))?;

        req.extensions_mut().insert(claims.claims);
    } else {
        return Err(Status::unauthenticated(
            "Key \"authorization\" was invalid string",
        ));
    }
    Ok(req)
}
