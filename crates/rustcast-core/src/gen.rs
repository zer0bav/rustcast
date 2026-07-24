//! Secret generator: strong passwords, hex/base64 tokens, UUIDv4, PINs.
//!
//! Randomness comes straight from `/dev/urandom` (no crate needed). Because
//! `query()` runs on every keystroke, the values re-roll as you type and settle
//! once you stop — whatever is shown is what Enter copies.

use crate::model::{Action, Item, Prev};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};

const GEN_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "copy" },
    ActionHint { keys: "type", label: "gen [length] · password · hex · base64 · uuid · pin" },
    ActionHint { keys: "esc", label: "close" },
];

/// Password alphabet: unambiguous-ish alnum plus a safe symbol set.
const PW_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*-_=+";

pub struct GenProvider;

impl GenProvider {
    pub fn new() -> Self {
        GenProvider
    }
}

impl Default for GenProvider {
    fn default() -> Self {
        GenProvider::new()
    }
}

impl Provider for GenProvider {
    fn id(&self) -> &'static str {
        "gen"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn placeholder(&self) -> &'static str {
        "Generate a secret… (type a length, e.g. 24)"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        GEN_HINTS
    }

    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        // Only inside the Generate Secret mode.
        if ctx.mode != Some(self.id()) {
            return Vec::new();
        }
        // Optional length argument, e.g. "32". Defaults to 20.
        let len: usize = ctx.query.split_whitespace().next().and_then(|t| t.parse().ok()).unwrap_or(20);
        let len = len.clamp(4, 256);

        let entries = [
            (format!("Password ({len} chars)"), "letters, digits & symbols", "password", password(len)),
            (format!("Hex token ({len} bytes)"), "lowercase hex", "hex", hex_token(len)),
            (format!("Base64 token ({len} bytes)"), "url-safe base64", "base64", base64_token(len)),
            ("UUID v4".to_string(), "random 128-bit identifier", "uuid", uuidv4()),
            ("PIN (6 digits)".to_string(), "numeric", "pin", pin(6)),
        ];

        entries
            .into_iter()
            .enumerate()
            .map(|(i, (title, sub, tag, value))| {
                Item::new(
                    format!("{title}: {}", truncate(&value, 96)),
                    sub,
                    "dialog-password",
                    tag,
                    9000 - i as i64,
                    Action::Copy(value.clone()),
                )
                .with_prev(Prev::Text(value))
            })
            .collect()
    }
}

/// Read `n` bytes of cryptographic randomness from `/dev/urandom`.
fn random_bytes(n: usize) -> Vec<u8> {
    use std::io::Read as _;
    let mut buf = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    buf
}

fn password(len: usize) -> String {
    // Rejection sampling to avoid modulo bias over the alphabet.
    let n = PW_CHARS.len() as u8;
    let limit = 256 - (256 % PW_CHARS.len());
    let mut out = String::with_capacity(len);
    while out.len() < len {
        for b in random_bytes(len * 2) {
            if (b as usize) < limit {
                out.push(PW_CHARS[(b % n) as usize] as char);
                if out.len() == len {
                    break;
                }
            }
        }
    }
    out
}

fn hex_token(bytes: usize) -> String {
    hex::encode(random_bytes(bytes))
}

fn base64_token(bytes: usize) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(bytes))
}

fn uuidv4() -> String {
    let mut b = random_bytes(16);
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 1
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

fn pin(len: usize) -> String {
    random_bytes(len).into_iter().map(|b| char::from(b'0' + b % 10)).collect()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_has_requested_length_and_valid_chars() {
        let p = password(24);
        assert_eq!(p.chars().count(), 24);
        assert!(p.bytes().all(|b| PW_CHARS.contains(&b)));
    }

    #[test]
    fn uuid_is_well_formed_v4() {
        let u = uuidv4();
        assert_eq!(u.len(), 36);
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert!(u.as_bytes()[14] == b'4'); // version nibble
    }

    #[test]
    fn hex_and_pin_lengths() {
        assert_eq!(hex_token(16).len(), 32);
        let p = pin(6);
        assert_eq!(p.len(), 6);
        assert!(p.bytes().all(|b| b.is_ascii_digit()));
    }
}
