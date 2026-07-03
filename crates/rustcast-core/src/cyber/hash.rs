//! Hashing (md5/sha1/sha256/sha512) — pure Rust via RustCrypto.

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

pub fn md5_hex(s: &str) -> String {
    let mut h = Md5::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

pub fn sha1_hex(s: &str) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

pub fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

pub fn sha512_hex(s: &str) -> String {
    let mut h = Sha512::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

pub struct Hash {
    pub label: &'static str,
    pub value: String,
}

pub fn all(input: &str) -> Vec<Hash> {
    vec![
        Hash { label: "md5", value: md5_hex(input) },
        Hash { label: "sha1", value: sha1_hex(input) },
        Hash { label: "sha256", value: sha256_hex(input) },
        Hash { label: "sha512", value: sha512_hex(input) },
    ]
}

/// Best-effort guess of a hash type from its length/charset.
pub fn identify(input: &str) -> Vec<&'static str> {
    let s = input.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Vec::new();
    }
    match s.len() {
        32 => vec!["MD5", "NTLM", "MD4"],
        40 => vec!["SHA1"],
        56 => vec!["SHA224"],
        64 => vec!["SHA256"],
        96 => vec!["SHA384"],
        128 => vec!["SHA512"],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(sha1_hex("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn identify_by_length() {
        assert!(identify("900150983cd24fb0d6963f7d28e17f72").contains(&"MD5"));
        assert!(identify("a9993e364706816aba3e25717850c26c9cd0d89d").contains(&"SHA1"));
        assert!(identify("nothex").is_empty());
    }
}
