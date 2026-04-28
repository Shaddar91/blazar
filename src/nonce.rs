//! HMAC-SHA256 nonce sign + verify.

use chrono::{DateTime, TimeZone, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Produce a signed nonce string of shape `<uuid>.<expires_unix>.<hex_hmac>`.
pub fn sign(secret_hex: &str, nonce_id: &str, expires_at: DateTime<Utc>) -> String {
    let key = decode_secret(secret_hex);
    let expires_unix = expires_at.timestamp();
    let payload = format!("{nonce_id}|{expires_unix}");

    let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());

    format!("{nonce_id}.{expires_unix}.{sig}")
}

/// Verify a nonce: check signature with constant-time compare, then check expiry.
pub fn verify(secret_hex: &str, token: &str) -> anyhow::Result<()> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        anyhow::bail!("token must be <uuid>.<expires_unix>.<hex_hmac>");
    }
    let nonce_id = parts[0];
    let expires_unix: i64 = parts[1].parse().map_err(|e| anyhow::anyhow!("bad ts: {e}"))?;
    let sig_hex = parts[2];

    let key = decode_secret(secret_hex);
    let payload = format!("{nonce_id}|{expires_unix}");
    let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    let expected = mac.finalize().into_bytes();

    let provided = hex::decode(sig_hex).map_err(|e| anyhow::anyhow!("sig not hex: {e}"))?;
    if !bool::from(expected.as_slice().ct_eq(provided.as_slice())) {
        anyhow::bail!("signature mismatch");
    }

    let expires_at = Utc
        .timestamp_opt(expires_unix, 0)
        .single()
        .ok_or_else(|| anyhow::anyhow!("invalid expiry ts"))?;
    if Utc::now() > expires_at {
        anyhow::bail!("nonce expired");
    }

    Ok(())
}

/// Decode the hex-encoded secret; falls back to the literal bytes if the value isn't valid hex.
fn decode_secret(secret: &str) -> Vec<u8> {
    match hex::decode(secret) {
        Ok(bytes) => bytes,
        Err(_) => secret.as_bytes().to_vec(),
    }
}
