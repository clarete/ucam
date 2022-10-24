use std::time::{Duration, SystemTime};

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde_derive::{Deserialize, Serialize};

use crate::err::Error;
use crate::model::Config;

#[derive(Debug, Deserialize, Serialize)]
struct Claims {
    sub: String,
    role: String,
    exp: usize,
}

pub(crate) fn create_token(config: &Config, jid: &str) -> Result<String, Error> {
    let claims = Claims {
        sub: jid.to_string(),
        role: "peer".to_string(),
        exp: token_expiration(config.userauth.token_validity)? as usize,
    };

    let header = Header::new(Algorithm::HS512);

    let key = EncodingKey::from_secret(config.userauth.jwt_secret.as_bytes());

    Ok(encode(&header, &claims, &key)?)
}

fn token_expiration(seconds: u64) -> Result<u64, Error> {
    let epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
    epoch
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(Error::new)?;
    Ok(epoch.as_secs())
}
