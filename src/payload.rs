//! This module implements the payload verification for GitHub webhook events.

use openssl::{hash::MessageDigest, memcmp, pkey::PKey, sign::Signer};
use rocket::{
    data::{self, Data, FromDataSimple},
    http::Status,
    request::Request,
    Outcome,
};
use std::{env, io::Read};

pub struct SignedPayload(Vec<u8>);

impl FromDataSimple for SignedPayload {
    type Error = String;
    fn from_data(req: &Request, data: Data) -> data::Outcome<Self, Self::Error> {
        let signature = match req.headers().get_one("X-Hub-Signature") {
            Some(s) => s,
            None => {
                return Outcome::Failure((
                    Status::Unauthorized,
                    format!("Unauthorized, no signature"),
                ));
            }
        };
        let signature = &signature["sha1=".len()..];
        let signature = match hex::decode(&signature) {
            Ok(e) => e,
            Err(e) => {
                return Outcome::Failure((
                    Status::BadRequest,
                    format!(
                        "failed to convert signature {:?} from hex: {:?}",
                        signature, e
                    ),
                ));
            }
        };

        let mut stream = data.open().take(1024 * 1024 * 5); // 5 Megabytes
        let mut buf = Vec::new();
        if let Err(err) = stream.read_to_end(&mut buf) {
            return Outcome::Failure((
                Status::InternalServerError,
                format!("failed to read request body to string: {:?}", err),
            ));
        }

        let key = PKey::hmac(env::var("GITHUB_WEBHOOK_SECRET").unwrap().as_bytes()).unwrap();
        let mut signer = Signer::new(MessageDigest::sha1(), &key).unwrap();
        signer.update(&buf).unwrap();
        let hmac = signer.sign_to_vec().unwrap();

        if !memcmp::eq(&hmac, &signature) {
            return Outcome::Failure((Status::Unauthorized, format!("HMAC not correct")));
        }

        Outcome::Success(SignedPayload(buf))
    }
}

impl SignedPayload {
    pub fn deserialize<T: serde::de::DeserializeOwned>(self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.0)
    }
}
