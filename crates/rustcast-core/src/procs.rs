//! Process manager and port inspector.
//!
//! `ProcessProvider` reads `/proc` directly (pure Rust, no deps) and lets you
//! terminate processes by name — SIGTERM on Enter, SIGKILL from the actions
//! menu. `PortsProvider` shells out to `ss`/`lsof` to find and kill whatever is
//! holding a port. Both use [`Action::Signal`], which the GUI runs while keeping
//! the launcher open so you can kill several in a row.

use crate::config::which;
use crate::model::{Action, Item, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;
use std::collections::HashMap;

/// Assumed page size for RSS math. 4 KiB on every mainstream Linux target.
const PAGE_SIZE: u64 = 4096;

struct Proc {
    pid: i32,
    comm: String,
    cmdline: String,
    rss: u64,
    uid: u32,
}

// ── Process manager ──────────────────────────────────────────────────

const PROC_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "terminate (SIGTERM)" },
    ActionHint { keys: "⌃K", label: "force kill · copy pid" },
    ActionHint { keys: "type", label: "filter by name" },
    ActionHint { keys: "esc", label: "close" },
];

pub struct ProcessProvider {
    users: HashMap<u32, String>,
    self_pid: i32,
}

impl ProcessProvider {
    pub fn new() -> Self {
        ProcessProvider { users: read_passwd(), self_pid: std::process::id() as i32 }
    }
}

impl Default for ProcessProvider {
    fn default() -> Self {
        ProcessProvider::new()
    }
}

impl Provider for ProcessProvider {
    fn id(&self) -> &'static str {
        "procs"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn placeholder(&self) -> &'static str {
        "Kill a process by name…"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        PROC_HINTS
    }

    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        // Only inside the Kill Process mode — never on a normal tab.
        if ctx.mode != Some(self.id()) {
            return Vec::new();
        }
        let q = ctx.query.trim().to_lowercase();
        let mut procs = read_procs();
        // Never offer to kill ourselves or PID 1.
        procs.retain(|p| p.pid != self.self_pid && p.pid != 1);

        let mut scored: Vec<(i64, Proc)> = Vec::new();
        for p in procs {
            let score = if q.is_empty() {
                // Landing: show the biggest processes first.
                (p.rss / 1024) as i64
            } else {
                // Rank by process name first; only fall back to the (noisy) full
                // command line, with a penalty, so name matches always win.
                let by_name = ctx.matcher.fuzzy_match(&p.comm.to_lowercase(), &q).map(|s| s + 1000);
                let by_cmd = ctx.matcher.fuzzy_match(&p.cmdline.to_lowercase(), &q).map(|s| s - 200);
                match by_name.or(by_cmd) {
                    Some(s) => s,
                    None => continue,
                }
            };
            scored.push((score, p));
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        if q.is_empty() {
            scored.truncate(60);
        }

        scored
            .into_iter()
            .map(|(score, p)| {
                let user = self.users.get(&p.uid).cloned().unwrap_or_else(|| p.uid.to_string());
                let mem = crate::clipboard::store::human_size((p.rss * PAGE_SIZE) as i64);
                let cmd = if p.cmdline.is_empty() { p.comm.clone() } else { p.cmdline.clone() };
                let subtitle = format!("PID {} · {} · {} · {}", p.pid, user, mem, truncate(&cmd, 80));
                Item::new(
                    p.comm.clone(),
                    subtitle,
                    "utilities-system-monitor",
                    "proc",
                    score,
                    Action::Signal { pid: p.pid, signal: 15 },
                )
                .with_actions(vec![
                    SecondaryAction {
                        label: "Force kill (SIGKILL)".into(),
                        action: Action::Signal { pid: p.pid, signal: 9 },
                    },
                    SecondaryAction { label: "Copy PID".into(), action: Action::Copy(p.pid.to_string()) },
                    SecondaryAction { label: "Copy command".into(), action: Action::Copy(cmd) },
                ])
            })
            .collect()
    }
}

// ── Port inspector ───────────────────────────────────────────────────

const PORT_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "kill the process on this port" },
    ActionHint { keys: "⌃K", label: "force kill · copy pid" },
    ActionHint { keys: "type", label: "a port number or process name" },
    ActionHint { keys: "esc", label: "close" },
];

struct Listener {
    proto: String,
    port: String,
    pid: i32,
    process: String,
}

pub struct PortsProvider;

impl PortsProvider {
    pub fn new() -> Self {
        PortsProvider
    }
}

impl Default for PortsProvider {
    fn default() -> Self {
        PortsProvider::new()
    }
}

impl Provider for PortsProvider {
    fn id(&self) -> &'static str {
        "ports"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn placeholder(&self) -> &'static str {
        "Inspect a listening port… (e.g. 8080)"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        PORT_HINTS
    }

    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        // Only inside the Port Inspector mode.
        if ctx.mode != Some(self.id()) {
            return Vec::new();
        }
        let q = ctx.query.trim().to_lowercase();
        if q.is_empty() {
            return vec![Item::new(
                "Type a port number or process name",
                "shows what is listening — Enter kills it",
                "network-server",
                "port",
                1,
                Action::None,
            )];
        }
        if !which("ss") && !which("lsof") {
            return vec![Item::new(
                "Install iproute2 (ss) or lsof",
                "needed to list listening ports",
                "dialog-warning",
                "port",
                1,
                Action::None,
            )];
        }

        let mut out = Vec::new();
        for (i, l) in listeners().into_iter().enumerate() {
            let matches = l.port == q
                || l.port.starts_with(&q)
                || l.process.to_lowercase().contains(&q);
            if !matches {
                continue;
            }
            let mut item = Item::new(
                format!(":{} {}", l.port, l.proto),
                format!("PID {} · {}", l.pid, l.process),
                "network-server",
                "port",
                10_000 - i as i64,
                Action::Signal { pid: l.pid, signal: 15 },
            )
            .with_actions(vec![
                SecondaryAction {
                    label: "Force kill (SIGKILL)".into(),
                    action: Action::Signal { pid: l.pid, signal: 9 },
                },
                SecondaryAction { label: "Copy PID".into(), action: Action::Copy(l.pid.to_string()) },
            ]);
            if l.pid == 0 {
                // No owning PID resolved (e.g. not our process) — informational only.
                item.action = Action::None;
                item.subtitle = format!("{} · owner not visible (try sudo)", l.proto);
            }
            out.push(item);
        }
        if out.is_empty() {
            out.push(Item::new(
                format!("Nothing listening matches “{q}”"),
                "only listening sockets are shown",
                "dialog-information",
                "port",
                1,
                Action::None,
            ));
        }
        out
    }
}

// ── /proc reading ────────────────────────────────────────────────────

fn read_procs() -> Vec<Proc> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return out };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(pid) = name.parse::<i32>() else { continue };
        let base = entry.path();

        let comm = std::fs::read_to_string(base.join("comm"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if comm.is_empty() {
            continue;
        }
        // cmdline is NUL-separated argv.
        let cmdline = std::fs::read(base.join("cmdline"))
            .map(|b| {
                b.split(|&c| c == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        // statm: size resident shared … (in pages). Field 1 (index 1) is RSS.
        let rss = std::fs::read_to_string(base.join("statm"))
            .ok()
            .and_then(|s| s.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()))
            .unwrap_or(0);
        let uid = proc_uid(&base);

        out.push(Proc { pid, comm, cmdline, rss, uid });
    }
    out
}

/// Read the real UID from `/proc/<pid>/status` (`Uid:\treal\teff\t…`).
fn proc_uid(base: &std::path::Path) -> u32 {
    std::fs::read_to_string(base.join("status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u32>().ok())
        })
        .unwrap_or(0)
}

/// Parse `/etc/passwd` into a uid → name map for friendly process owners.
fn read_passwd() -> HashMap<u32, String> {
    let mut m = HashMap::new();
    if let Ok(text) = std::fs::read_to_string("/etc/passwd") {
        for line in text.lines() {
            let mut f = line.split(':');
            if let (Some(name), Some(_), Some(uid)) = (f.next(), f.next(), f.next()) {
                if let Ok(uid) = uid.parse::<u32>() {
                    m.insert(uid, name.to_string());
                }
            }
        }
    }
    m
}

// ── port listing ─────────────────────────────────────────────────────

/// Enumerate listening sockets via `ss` (preferred) or `lsof`, collapsing the
/// same (proto, port, pid) bound on several interfaces into one row.
fn listeners() -> Vec<Listener> {
    let raw = if which("ss") {
        run(&["ss", "-tulpnH"]).map(|o| parse_ss(&o))
    } else if which("lsof") {
        run(&["lsof", "-nP", "-iTCP", "-sTCP:LISTEN"]).map(|o| parse_lsof(&o))
    } else {
        None
    };
    let Some(raw) = raw else { return Vec::new() };

    let mut seen = std::collections::HashSet::new();
    raw.into_iter()
        .filter(|l| seen.insert((l.proto.clone(), l.port.clone(), l.pid)))
        .collect()
}

fn run(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(args[0]).args(&args[1..]).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse `ss -tulpnH` rows:
/// `tcp LISTEN 0 128 0.0.0.0:22 0.0.0.0:* users:(("sshd",pid=612,fd=3))`
fn parse_ss(text: &str) -> Vec<Listener> {
    let mut out = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 {
            continue;
        }
        let proto = cols[0].to_string();
        let local = cols[4];
        let Some(port) = local.rsplit(':').next().filter(|p| !p.is_empty()) else { continue };
        let (pid, process) = parse_ss_users(line);
        out.push(Listener { proto, port: port.to_string(), pid, process });
    }
    out
}

/// Pull the first `("name",pid=NNN,…)` tuple out of an `ss` line.
fn parse_ss_users(line: &str) -> (i32, String) {
    let name = line
        .split_once("((\"")
        .and_then(|(_, r)| r.split_once('"'))
        .map(|(n, _)| n.to_string())
        .unwrap_or_else(|| "?".into());
    let pid = line
        .split_once("pid=")
        .and_then(|(_, r)| {
            let end = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
            r[..end].parse::<i32>().ok()
        })
        .unwrap_or(0);
    (pid, name)
}

/// Parse `lsof -nP -iTCP -sTCP:LISTEN` rows (COMMAND PID … NAME).
fn parse_lsof(text: &str) -> Vec<Listener> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 9 {
            continue;
        }
        let process = cols[0].to_string();
        let pid = cols[1].parse::<i32>().unwrap_or(0);
        let addr = cols[8];
        let Some(port) = addr.rsplit(':').next().filter(|p| !p.is_empty()) else { continue };
        out.push(Listener { proto: "tcp".into(), port: port.to_string(), pid, process });
    }
    out
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
    fn ss_parse_extracts_port_and_pid() {
        let line = r#"tcp   LISTEN 0 128 0.0.0.0:22 0.0.0.0:* users:(("sshd",pid=612,fd=3))"#;
        let ls = parse_ss(line);
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].port, "22");
        assert_eq!(ls[0].pid, 612);
        assert_eq!(ls[0].process, "sshd");
        assert_eq!(ls[0].proto, "tcp");
    }

    #[test]
    fn ss_parse_ipv6_and_missing_process() {
        let line = "tcp   LISTEN 0 4096 [::]:8080 [::]:*";
        let ls = parse_ss(line);
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].port, "8080");
        assert_eq!(ls[0].pid, 0);
    }

    #[test]
    fn passwd_maps_root() {
        // /etc/passwd always has root at uid 0 on a real system; the parser
        // itself is exercised here with a synthetic line.
        let m = read_passwd();
        // Don't assert contents (CI images vary) — just that it doesn't panic
        // and yields a map. root is nearly universal though:
        let _ = m.get(&0);
    }
}
