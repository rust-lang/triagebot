use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::fmt;

#[derive(Debug)]
pub struct SignedPayloadError;

impl fmt::Display for SignedPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "failed to validate payload")
    }
}

impl std::error::Error for SignedPayloadError {}

pub fn assert_signed(signature: &str, payload: &[u8]) -> Result<(), SignedPayloadError> {
    let signature = signature.get("sha1=".len()..).ok_or(SignedPayloadError)?;
    let signature = match hex::decode(&signature) {
        Ok(e) => e,
        Err(e) => {
            tracing::trace!("hex decode failed for {:?}: {:?}", signature, e);
            return Err(SignedPayloadError);
        }
    };

    let mut mac = Hmac::<Sha1>::new_from_slice(
        std::env::var("GITHUB_WEBHOOK_SECRET")
            .expect("Missing GITHUB_WEBHOOK_SECRET")
            .as_bytes(),
    )
    .unwrap();
    mac.update(&payload);
    mac.verify_slice(&signature).map_err(|_| SignedPayloadError)
}
