use openssl::{hash::MessageDigest, memcmp, pkey::PKey, sign::Signer};
use std::fmt;

#[derive(Debug)]
pub struct SignedPayloadError;

impl fmt::Display for SignedPayloadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "failed to validate payload")
    }
}

impl std::error::Error for SignedPayloadError {}

pub fn sign(secret: &str, payload: &[u8]) -> Vec<u8> {
    let key = PKey::hmac(secret.as_bytes()).unwrap();
    let mut signer = Signer::new(MessageDigest::sha1(), &key).unwrap();
    signer.update(&payload).unwrap();
    signer.sign_to_vec().unwrap()
}

pub fn assert_signed(signature: &str, payload: &[u8]) -> Result<(), SignedPayloadError> {
    let signature = signature.get("sha1=".len()..).ok_or(SignedPayloadError)?;
    let signature = match hex::decode(&signature) {
        Ok(e) => e,
        Err(e) => {
            tracing::trace!("hex decode failed for {:?}: {:?}", signature, e);
            return Err(SignedPayloadError);
        }
    };

    let secret = std::env::var("GITHUB_WEBHOOK_SECRET").expect("Missing GITHUB_WEBHOOK_SECRET");
    let hmac = sign(&secret, payload);

    if !memcmp::eq(&hmac, &signature) {
        return Err(SignedPayloadError);
    }
    Ok(())
}
