//! JWT decode/inspect (no signature verification — the common inspect case).

use base64::Engine;
use chrono::TimeZone;

pub struct DecodedJwt {
    pub header: String,
    pub payload: String,
    pub summary: Vec<(String, String)>,
    pub signature_len: usize,
}

fn b64url(seg: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(seg)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(seg))
        .ok()
}

fn pretty(bytes: &[u8]) -> String {
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| String::from_utf8_lossy(bytes).into()),
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Does this look like a three-part JWT?
pub fn looks_like_jwt(s: &str) -> bool {
    let s = s.trim();
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts[0].starts_with("eyJ")
        && parts.iter().all(|p| !p.is_empty())
}

pub fn decode(token: &str) -> Option<DecodedJwt> {
    let token = token.trim();
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let header_bytes = b64url(parts[0])?;
    let payload_bytes = b64url(parts[1])?;
    let header = pretty(&header_bytes);
    let payload = pretty(&payload_bytes);

    let mut summary = Vec::new();
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&header_bytes) {
        if let Some(alg) = v.get("alg").and_then(|a| a.as_str()) {
            summary.push(("alg".into(), alg.to_string()));
        }
        if let Some(typ) = v.get("typ").and_then(|a| a.as_str()) {
            summary.push(("typ".into(), typ.to_string()));
        }
    }
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&payload_bytes) {
        for (k, label) in [("sub", "sub"), ("iss", "iss"), ("aud", "aud")] {
            if let Some(s) = v.get(k).and_then(|a| a.as_str()) {
                summary.push((label.into(), s.to_string()));
            }
        }
        for (k, label) in [("iat", "issued"), ("exp", "expires"), ("nbf", "not before")] {
            if let Some(ts) = v.get(k).and_then(|a| a.as_i64()) {
                summary.push((label.into(), human_time(ts)));
            }
        }
    }

    let signature_len = b64url(parts[2]).map(|b| b.len()).unwrap_or(0);
    Some(DecodedJwt { header, payload, summary, signature_len })
}

fn human_time(epoch: i64) -> String {
    match chrono::Utc.timestamp_opt(epoch, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        None => epoch.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // {"alg":"HS256","typ":"JWT"}.{"sub":"1234567890","name":"John Doe","iat":1516239022}
    const TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

    #[test]
    fn detects_and_decodes() {
        assert!(looks_like_jwt(TOKEN));
        let d = decode(TOKEN).unwrap();
        assert!(d.payload.contains("John Doe"));
        assert!(d.summary.iter().any(|(k, v)| k == "alg" && v == "HS256"));
    }

    #[test]
    fn rejects_non_jwt() {
        assert!(!looks_like_jwt("hello.world"));
        assert!(decode("a.b").is_none());
    }
}
