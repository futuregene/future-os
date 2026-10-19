//! Read JWT expiry metadata. Authentication still belongs to the signed
//! platform grant and transport; this parser does not validate a signature.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Remote server returned an invalid JWT.")]
    Invalid,
    #[error("Remote JWT is missing its expiry.")]
    MissingExpiry,
}

pub fn expiry(jwt: &str) -> Result<i64, Error> {
    let payload = jwt.split('.').nth(1).ok_or(Error::Invalid)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| Error::Invalid)?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value.get("exp").and_then(Value::as_i64))
        .ok_or(Error::MissingExpiry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn jwt(expiry: i64) -> String {
        format!(
            "h.{}.s",
            URL_SAFE_NO_PAD.encode(json!({"exp": expiry}).to_string())
        )
    }
    #[test]
    fn jwt_expiry_rejects_malformed_tokens() {
        assert!(expiry("one-segment").is_err());
        assert!(expiry("two.segments").is_err()); // decodes, but not JSON
        assert!(expiry("a.!!!.b").is_err()); // invalid base64 payload → decode failure
        let no_exp = URL_SAFE_NO_PAD.encode(json!({ "sub": 1 }).to_string());
        assert!(expiry(&format!("h.{no_exp}.s")).is_err());
        assert_eq!(expiry(&jwt(42)).unwrap(), 42);
    }
}
