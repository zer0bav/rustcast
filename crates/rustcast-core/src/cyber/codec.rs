//! Encoding/decoding transforms. All pure functions → instant live preview.

use base64::Engine;

pub fn b64_encode(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

pub fn b64_decode(s: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.trim()))
        .ok()?;
    String::from_utf8(bytes).ok()
}

pub fn hex_encode(s: &str) -> String {
    hex::encode(s.as_bytes())
}

pub fn hex_decode(s: &str) -> Option<String> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = hex::decode(cleaned).ok()?;
    String::from_utf8(bytes).ok()
}

pub fn url_encode(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

pub fn url_decode(s: &str) -> Option<String> {
    urlencoding::decode(s).ok().map(|c| c.into_owned())
}

pub fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            _ => c,
        })
        .collect()
}

/// A (label, value) transform result.
pub struct Transform {
    pub label: &'static str,
    pub value: String,
}

/// Every applicable transform for `input` (decodes that fail are omitted).
pub fn all(input: &str) -> Vec<Transform> {
    let mut out = vec![
        Transform { label: "base64 encode", value: b64_encode(input) },
        Transform { label: "hex encode", value: hex_encode(input) },
        Transform { label: "url encode", value: url_encode(input) },
        Transform { label: "rot13", value: rot13(input) },
    ];
    if let Some(v) = b64_decode(input) {
        out.push(Transform { label: "base64 decode", value: v });
    }
    if let Some(v) = hex_decode(input) {
        out.push(Transform { label: "hex decode", value: v });
    }
    if let Some(v) = url_decode(input) {
        if v != input {
            out.push(Transform { label: "url decode", value: v });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        assert_eq!(b64_encode("test"), "dGVzdA==");
        assert_eq!(b64_decode("dGVzdA==").as_deref(), Some("test"));
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(hex_encode("AB"), "4142");
        assert_eq!(hex_decode("4142").as_deref(), Some("AB"));
        assert_eq!(hex_decode("41 42").as_deref(), Some("AB"));
    }

    #[test]
    fn url_roundtrip() {
        assert_eq!(url_encode("a b&c"), "a%20b%26c");
        assert_eq!(url_decode("a%20b%26c").as_deref(), Some("a b&c"));
    }

    #[test]
    fn rot13_involution() {
        assert_eq!(rot13("Hello"), "Uryyb");
        assert_eq!(rot13(&rot13("Hello")), "Hello");
    }

    #[test]
    fn bad_decodes_omitted() {
        // "!!!" is not valid base64/hex text → those transforms absent.
        let labels: Vec<_> = all("!!!").into_iter().map(|t| t.label).collect();
        assert!(labels.contains(&"base64 encode"));
        assert!(!labels.contains(&"hex decode"));
    }
}
