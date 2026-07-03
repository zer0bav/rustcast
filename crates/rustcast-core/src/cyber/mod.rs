//! The Cyber tab: a single provider that fans out to every pure-Rust tool and
//! shows smart, live results as you type.

pub mod codec;
pub mod hash;
pub mod jwt;
pub mod net;
pub mod payload;

use crate::config::which;
use crate::model::{Action, Item, Prev, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};

const CYBER_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "copy / run" },
    ActionHint { keys: "⌃K", label: "actions" },
    ActionHint { keys: "type", label: "b64 · hash · jwt · cidr · ts · rev · defang · link · target" },
    ActionHint { keys: "esc", label: "close" },
];

pub struct CyberProvider;

impl CyberProvider {
    pub fn new() -> Self {
        CyberProvider
    }
}

impl Default for CyberProvider {
    fn default() -> Self {
        CyberProvider::new()
    }
}

/// Split a `host:port` / `host` target string.
fn split_target(t: &str) -> (String, String) {
    match t.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            (h.to_string(), p.to_string())
        }
        _ => (t.to_string(), String::new()),
    }
}

fn copy_item(title: String, subtitle: impl Into<String>, tag: &str, score: i64, value: String) -> Item {
    Item::new(title, subtitle, "utilities-terminal", tag, score, Action::Copy(value.clone()))
        .with_prev(Prev::Text(value))
}

impl Provider for CyberProvider {
    fn id(&self) -> &'static str {
        "cyber"
    }
    fn tab(&self) -> Tab {
        Tab::Cyber
    }
    fn placeholder(&self) -> &'static str {
        "Encode, hash, decode JWT, CIDR, target… (try: b64 hello)"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        CYBER_HINTS
    }

    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let raw = ctx.query.trim();
        if raw.is_empty() {
            return self.landing(ctx);
        }

        // Optional sub-keyword scoping: "b64 hello", "hash foo", "rev", "cidr 10/24".
        let (kw, arg) = match raw.split_once(' ') {
            Some((k, a)) => (k.to_lowercase(), a.trim()),
            None => (raw.to_lowercase(), ""),
        };
        let scoped: Option<&str> = match kw.as_str() {
            "b64" | "base64" | "hex" | "url" | "rot13" | "codec" => Some("codec"),
            "hash" | "md5" | "sha1" | "sha256" | "sha512" => Some("hash"),
            "hashid" | "identify" => Some("hashid"),
            "jwt" => Some("jwt"),
            "cidr" | "subnet" => Some("cidr"),
            "ts" | "epoch" | "time" => Some("epoch"),
            "defang" => Some("defang"),
            "refang" => Some("refang"),
            "link" | "osint" | "lookup" => Some("link"),
            "rev" | "revshell" | "payload" => Some("payload"),
            "pattern" | "cyclic" => Some("cyclic"),
            "target" | "set" => Some("target"),
            _ => None,
        };

        let mut out = Vec::new();
        let input = if scoped.is_some() { arg } else { raw };

        match scoped {
            Some("codec") => self.codec(input, &mut out, 9000),
            Some("hash") => self.hash(input, &mut out, 9000),
            Some("hashid") => self.hashid(input, &mut out, 9000),
            Some("jwt") => self.jwt(input, &mut out, 9000),
            Some("cidr") => self.cidr(input, &mut out, 9000),
            Some("epoch") => self.epoch(input, &mut out, 9000),
            Some("defang") => {
                out.push(copy_item(format!("defang → {}", net::defang(input)), "defanged indicator", "defang", 9000, net::defang(input)));
            }
            Some("refang") => {
                out.push(copy_item(format!("refang → {}", net::refang(input)), "refanged indicator", "refang", 9000, net::refang(input)));
            }
            Some("link") => self.links(input, &mut out, 9000),
            Some("payload") => self.payloads(if input.is_empty() { ctx.target.unwrap_or("") } else { input }, &mut out, 9000),
            Some("cyclic") => self.cyclic(input, &mut out, 9000),
            Some("target") => {
                out.push(Item::new(
                    format!("Set target → {input}"),
                    "reused by payloads and network templates",
                    "network-server",
                    "target",
                    9000,
                    Action::SetTarget(input.to_string()),
                ));
            }
            _ => {
                // Auto mode: detect the most specific tool, then offer the rest.
                if jwt::looks_like_jwt(raw) {
                    self.jwt(raw, &mut out, 9500);
                }
                if net::cidr_info(raw).is_some() {
                    self.cidr(raw, &mut out, 9400);
                }
                if raw.chars().all(|c| c.is_ascii_digit()) && raw.len() >= 9 {
                    self.epoch(raw, &mut out, 9300);
                }
                if !hash::identify(raw).is_empty() {
                    self.hashid(raw, &mut out, 9200);
                }
                if looks_like_indicator(raw) {
                    out.push(Item::new(
                        format!("Set target → {raw}"),
                        "use in payloads / network templates",
                        "network-server",
                        "target",
                        9100,
                        Action::SetTarget(raw.to_string()),
                    ));
                    self.links(raw, &mut out, 5000);
                    self.network_templates(raw, ctx, &mut out, 4800);
                }
                self.codec(raw, &mut out, 4000);
                self.hash(raw, &mut out, 3000);
            }
        }
        out
    }
}

impl CyberProvider {
    fn landing(&self, ctx: &QueryCtx) -> Vec<Item> {
        let mut out = Vec::new();
        if let Some(t) = ctx.target {
            out.push(Item::new(
                format!("Active target: {t}"),
                "clear by setting a new one",
                "network-server",
                "target",
                100,
                Action::None,
            ));
            self.network_templates(t, ctx, &mut out, 90);
            self.payloads(t, &mut out, 80);
        }
        for (kw, desc) in [
            ("b64 <text>", "base64 / hex / url / rot13 encode & decode"),
            ("hash <text>", "md5 / sha1 / sha256 / sha512"),
            ("jwt <token>", "decode & inspect a JWT"),
            ("cidr 10.0.0.0/24", "subnet calculator"),
            ("ts <epoch>", "epoch ↔ human time"),
            ("rev <host:port>", "reverse-shell one-liners"),
            ("defang <ioc>", "defang / refang an indicator"),
            ("link <ioc>", "VirusTotal / Shodan / CVE lookups"),
            ("target <host>", "set an active target"),
        ] {
            out.push(Item::new(kw, desc, "utilities-terminal", "cyber", 10, Action::None));
        }
        out
    }

    fn codec(&self, input: &str, out: &mut Vec<Item>, base: i64) {
        if input.is_empty() {
            return;
        }
        for (i, t) in codec::all(input).into_iter().enumerate() {
            out.push(copy_item(
                format!("{}: {}", t.label, truncate(&t.value, 120)),
                "copy result",
                "codec",
                base - i as i64,
                t.value,
            ));
        }
    }

    fn hash(&self, input: &str, out: &mut Vec<Item>, base: i64) {
        if input.is_empty() {
            return;
        }
        for (i, h) in hash::all(input).into_iter().enumerate() {
            out.push(copy_item(
                format!("{}: {}", h.label, h.value),
                "copy hash",
                "hash",
                base - i as i64,
                h.value,
            ));
        }
    }

    fn hashid(&self, input: &str, out: &mut Vec<Item>, base: i64) {
        let guesses = hash::identify(input);
        if guesses.is_empty() {
            return;
        }
        out.push(Item::new(
            format!("Possible hash type: {}", guesses.join(", ")),
            format!("{} hex chars", input.trim().len()),
            "dialog-question",
            "hashid",
            base,
            Action::Copy(guesses.join(", ")),
        ));
    }

    fn jwt(&self, input: &str, out: &mut Vec<Item>, base: i64) {
        let Some(d) = jwt::decode(input) else { return };
        let summary = d
            .summary
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("  ·  ");
        let body = format!("── header ──\n{}\n\n── payload ──\n{}", d.header, d.payload);
        out.push(
            Item::new(
                "JWT decoded",
                if summary.is_empty() { "header + payload".into() } else { summary },
                "application-certificate",
                "jwt",
                base,
                Action::Copy(d.payload.clone()),
            )
            .with_prev(Prev::Text(body))
            .with_actions(vec![
                SecondaryAction { label: "Copy header".into(), action: Action::Copy(d.header) },
                SecondaryAction { label: "Copy payload".into(), action: Action::Copy(d.payload) },
            ]),
        );
    }

    fn cidr(&self, input: &str, out: &mut Vec<Item>, base: i64) {
        let Some(info) = net::cidr_info(input) else { return };
        let body = info.iter().map(|(k, v)| format!("{k:>13}: {v}")).collect::<Vec<_>>().join("\n");
        let sub = info.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join("  ·  ");
        out.push(
            Item::new("CIDR", sub, "network-workgroup", "cidr", base, Action::Copy(body.clone()))
                .with_prev(Prev::Text(body)),
        );
    }

    fn epoch(&self, input: &str, out: &mut Vec<Item>, base: i64) {
        let Some(h) = net::epoch_to_human(input) else { return };
        out.push(copy_item(format!("epoch → {h}"), "copy time", "epoch", base, h));
    }

    fn links(&self, input: &str, out: &mut Vec<Item>, base: i64) {
        for (i, (name, url)) in net::links(input).into_iter().enumerate() {
            out.push(Item::new(
                format!("{name}: {input}"),
                url.clone(),
                "web-browser",
                "osint",
                base - i as i64,
                Action::OpenUrl(url),
            ));
        }
    }

    fn payloads(&self, target: &str, out: &mut Vec<Item>, base: i64) {
        let (host, port) = split_target(target);
        for (i, p) in payload::reverse_shells(&host, &port).into_iter().enumerate() {
            out.push(copy_item(
                format!("{}: {}", p.label, truncate(&p.value, 90)),
                "copy reverse shell",
                "payload",
                base - i as i64,
                p.value,
            ));
        }
    }

    fn cyclic(&self, input: &str, out: &mut Vec<Item>, base: i64) {
        if let Ok(len) = input.trim().parse::<usize>() {
            let p = payload::cyclic(len.min(20280));
            out.push(copy_item(format!("cyclic({len})"), "copy pattern", "cyclic", base, p));
        } else if !input.is_empty() {
            if let Some(off) = payload::cyclic_offset(input.trim()) {
                out.push(copy_item(format!("offset of {input} = {off}"), "copy offset", "cyclic", base, off.to_string()));
            }
        }
    }

    fn network_templates(&self, target: &str, ctx: &QueryCtx, out: &mut Vec<Item>, base: i64) {
        let (host, port) = split_target(target);
        let port = if port.is_empty() { "PORT".to_string() } else { port };
        let templates: &[(&str, String, bool)] = &[
            ("whois", format!("whois {host}"), which("whois")),
            ("dig", format!("dig {host}"), which("dig")),
            ("dig +short", format!("dig +short {host}"), which("dig")),
            ("nmap -sV", format!("nmap -sV {host}"), which("nmap")),
            ("nmap -sC -sV", format!("nmap -sC -sV {host}"), which("nmap")),
            ("nc", format!("nc {host} {port}"), which("nc")),
            ("curl -I", format!("curl -I https://{host}"), which("curl")),
        ];
        for (i, (label, cmd, available)) in templates.iter().enumerate() {
            let subtitle = if *available { "run in terminal".to_string() } else { "tool not installed — copies command".to_string() };
            let action = if *available {
                Action::RunInTerminal(cmd.clone())
            } else {
                Action::Copy(cmd.clone())
            };
            let _ = ctx;
            out.push(
                Item::new(format!("{label}  {cmd}"), subtitle, "utilities-terminal", "net", base - i as i64, action)
                    .with_actions(vec![SecondaryAction { label: "Copy command".into(), action: Action::Copy(cmd.clone()) }]),
            );
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

/// Rough check: an IP or a dotted domain (for target/link suggestions).
fn looks_like_indicator(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.contains(' ') {
        return false;
    }
    s.parse::<std::net::IpAddr>().is_ok()
        || (s.contains('.') && s.split('.').all(|p| !p.is_empty()) && s.split('.').count() >= 2)
        || s.to_uppercase().starts_with("CVE-")
}
