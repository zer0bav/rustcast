//! Window switcher with several backends:
//!
//! - **Hyprland** / **Sway** — the compositor's own IPC (`hyprctl -j clients`,
//!   `swaymsg -t get_tree`).
//! - **GNOME (Wayland)** — the "Window Calls" shell extension's D-Bus API
//!   (`org.gnome.Shell.Extensions.Windows`), when installed.
//! - **X11** (any desktop) — `wmctrl`.
//!
//! Lists open windows, focuses one on Enter, and can close it from the actions
//! menu. Everything dispatches through [`Action::RunShell`], which closes the
//! launcher — exactly what you want when jumping to a window.

use crate::model::{Action, Item, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;

const WIN_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "focus window" },
    ActionHint { keys: "⌃K", label: "close window" },
    ActionHint { keys: "type", label: "filter by title or app" },
    ActionHint { keys: "esc", label: "close" },
];

/// Window Calls (GNOME shell extension) D-Bus endpoint.
const GNOME_DEST: &str = "org.gnome.Shell";
const GNOME_PATH: &str = "/org/gnome/Shell/Extensions/Windows";
const GNOME_IFACE: &str = "org.gnome.Shell.Extensions.Windows";
const WINDOW_CALLS_URL: &str = "https://extensions.gnome.org/extension/4724/window-calls/";

#[derive(Clone, Copy, PartialEq)]
enum Backend {
    Hyprland,
    Sway,
    Gnome,
    X11,
    None,
}

struct Win {
    /// Backend handle: Hyprland address, Sway con_id, GNOME window id, or X11 id.
    handle: String,
    title: String,
    app: String,
    workspace: String,
}

pub struct WindowsProvider {
    backend: Backend,
}

impl WindowsProvider {
    pub fn new() -> Self {
        WindowsProvider { backend: detect_backend() }
    }

    fn focus_cmd(&self, w: &Win) -> String {
        match self.backend {
            Backend::Hyprland => format!("hyprctl dispatch focuswindow address:{}", w.handle),
            Backend::Sway => format!("swaymsg [con_id={}] focus", w.handle),
            Backend::Gnome => gnome_call("Activate", &w.handle),
            Backend::X11 => format!("wmctrl -ia {}", w.handle),
            Backend::None => String::new(),
        }
    }

    fn close_cmd(&self, w: &Win) -> String {
        match self.backend {
            Backend::Hyprland => format!("hyprctl dispatch closewindow address:{}", w.handle),
            Backend::Sway => format!("swaymsg [con_id={}] kill", w.handle),
            Backend::Gnome => gnome_call("Close", &w.handle),
            Backend::X11 => format!("wmctrl -ic {}", w.handle),
            Backend::None => String::new(),
        }
    }
}

impl Default for WindowsProvider {
    fn default() -> Self {
        WindowsProvider::new()
    }
}

/// Pick a backend from the environment. wlroots IPC wins when present; otherwise
/// GNOME (Wayland) or an X11 session with `wmctrl`.
fn detect_backend() -> Backend {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Backend::Hyprland;
    }
    if std::env::var_os("SWAYSOCK").is_some() {
        return Backend::Sway;
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default().to_lowercase();
    if session == "wayland" && desktop.contains("gnome") {
        return Backend::Gnome;
    }
    // X11 (any desktop) — wmctrl drives it. Covers GNOME/KDE/XFCE on X11.
    let on_x11 = session == "x11" || (session.is_empty() && std::env::var_os("DISPLAY").is_some());
    if on_x11 && crate::config::which("wmctrl") {
        return Backend::X11;
    }
    Backend::None
}

impl Provider for WindowsProvider {
    fn id(&self) -> &'static str {
        "windows"
    }
    fn tab(&self) -> Tab {
        Tab::Win
    }
    fn placeholder(&self) -> &'static str {
        "Switch to an open window…"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        WIN_HINTS
    }

    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        // Show the window list on the Windows tab, or inside its command mode.
        if ctx.active_tab != Tab::Win && ctx.mode != Some(self.id()) {
            return Vec::new();
        }

        let wins = match self.backend {
            Backend::Hyprland => Some(hyprland_windows()),
            Backend::Sway => Some(sway_windows()),
            Backend::Gnome => gnome_windows(), // None = extension not available
            Backend::X11 => Some(x11_windows()),
            Backend::None => None,
        };

        let Some(wins) = wins else {
            return vec![unsupported_item()];
        };

        let q = ctx.query.trim().to_lowercase();
        let mut scored: Vec<(i64, Win)> = Vec::new();
        for (i, w) in wins.into_iter().enumerate() {
            let score = if q.is_empty() {
                1000 - i as i64
            } else {
                let hay = format!("{} {}", w.title, w.app).to_lowercase();
                match ctx.matcher.fuzzy_match(&hay, &q) {
                    Some(s) => s,
                    None => continue,
                }
            };
            scored.push((score, w));
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));

        scored
            .into_iter()
            .map(|(score, w)| {
                let title = if w.title.is_empty() { w.app.clone() } else { w.title.clone() };
                let subtitle = if w.workspace.is_empty() {
                    w.app.clone()
                } else {
                    format!("{} · workspace {}", w.app, w.workspace)
                };
                Item::new(title, subtitle, "window", "win", score, Action::RunShell(self.focus_cmd(&w)))
                    .with_actions(vec![SecondaryAction {
                        label: "Close window".into(),
                        action: Action::RunShell(self.close_cmd(&w)),
                    }])
            })
            .collect()
    }
}

/// The row shown when no backend is available — tailored to the desktop so the
/// fix is actionable (install the GNOME extension, or run under X11).
fn unsupported_item() -> Item {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default().to_lowercase();
    if desktop.contains("gnome") && session == "wayland" {
        Item::new(
            "Install the “Window Calls” GNOME extension",
            "enables window switching on GNOME Wayland — press Enter to open its page",
            "dialog-information",
            "win",
            1,
            Action::OpenUrl(WINDOW_CALLS_URL.into()),
        )
    } else if desktop.contains("kde") && session == "wayland" {
        Item::new(
            "Window switching isn't available on KDE Wayland",
            "KWin exposes no listing IPC; log into an X11 session for wmctrl support",
            "dialog-warning",
            "win",
            1,
            Action::None,
        )
    } else {
        Item::new(
            "Window switching needs Hyprland, Sway, GNOME (Window Calls), or X11",
            "no supported backend detected",
            "dialog-warning",
            "win",
            1,
            Action::None,
        )
    }
}

fn run(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(args[0]).args(&args[1..]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ── Hyprland ────────────────────────────────────────────────────

fn hyprland_windows() -> Vec<Win> {
    let Some(text) = run(&["hyprctl", "-j", "clients"]) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    let Some(arr) = v.as_array() else { return Vec::new() };
    arr.iter()
        .filter_map(|c| {
            let handle = c.get("address")?.as_str()?.to_string();
            let title = c.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let app = c.get("class").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let workspace = c
                .get("workspace")
                .and_then(|w| w.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("?")
                .to_string();
            // Skip special/hidden scratchpad entries with empty class and title.
            if app.is_empty() && title.is_empty() {
                return None;
            }
            Some(Win { handle, title, app, workspace })
        })
        .collect()
}

// ── Sway ────────────────────────────────────────────────────────

fn sway_windows() -> Vec<Win> {
    let Some(text) = run(&["swaymsg", "-t", "get_tree"]) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    let mut out = Vec::new();
    collect_sway(&v, "?", &mut out);
    out
}

/// Recurse the Sway tree, pulling leaf windows and tracking the workspace name.
fn collect_sway(node: &serde_json::Value, workspace: &str, out: &mut Vec<Win>) {
    let ws = if node.get("type").and_then(|t| t.as_str()) == Some("workspace") {
        node.get("name").and_then(|n| n.as_str()).unwrap_or(workspace)
    } else {
        workspace
    };

    let ntype = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let is_window = matches!(ntype, "con" | "floating_con")
        && node.get("nodes").map(|n| n.as_array().map(|a| a.is_empty()).unwrap_or(true)).unwrap_or(true)
        && node.get("name").and_then(|n| n.as_str()).map(|s| !s.is_empty()).unwrap_or(false);

    if is_window {
        if let Some(id) = node.get("id").and_then(|i| i.as_i64()) {
            let title = node.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            let app = node
                .get("app_id")
                .and_then(|a| a.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    node.get("window_properties")
                        .and_then(|w| w.get("class"))
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();
            out.push(Win { handle: id.to_string(), title, app, workspace: ws.to_string() });
        }
    }

    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = node.get(key).and_then(|n| n.as_array()) {
            for child in children {
                collect_sway(child, ws, out);
            }
        }
    }
}

// ── GNOME (Window Calls extension) ──────────────────────────────

/// A `gdbus` command line that invokes a Window Calls method with one uint arg.
fn gnome_call(method: &str, id: &str) -> String {
    format!(
        "gdbus call --session --dest {GNOME_DEST} --object-path {GNOME_PATH} \
         --method {GNOME_IFACE}.{method} {id}"
    )
}

/// List windows via the Window Calls extension. `None` when the extension isn't
/// installed / the D-Bus call fails (so the caller can prompt to install it).
fn gnome_windows() -> Option<Vec<Win>> {
    let raw = run(&[
        "gdbus", "call", "--session", "--dest", GNOME_DEST, "--object-path", GNOME_PATH, "--method",
        &format!("{GNOME_IFACE}.List"),
    ])?;
    Some(parse_gnome_list(&raw))
}

/// Parse the gdbus reply `('[{…}]',)` into windows. The JSON payload sits between
/// the first `[` and the last `]` of the reply.
fn parse_gnome_list(raw: &str) -> Vec<Win> {
    let (Some(start), Some(end)) = (raw.find('['), raw.rfind(']')) else { return Vec::new() };
    let json = &raw[start..=end];
    let Ok(arr) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let Some(arr) = arr.as_array() else { return Vec::new() };
    arr.iter()
        .filter_map(|w| {
            let id = w.get("id")?;
            // id is a number in the JSON; stringify for the handle.
            let handle = id.as_u64().map(|n| n.to_string()).or_else(|| id.as_str().map(str::to_string))?;
            let app = w.get("wm_class").and_then(|c| c.as_str()).unwrap_or("").to_string();
            // Some Window Calls versions omit titles from List; fall back to class.
            let title = w.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
            let workspace = w
                .get("workspace")
                .and_then(|n| n.as_u64())
                .map(|n| (n + 1).to_string())
                .unwrap_or_default();
            if app.is_empty() && title.is_empty() {
                return None;
            }
            Some(Win { handle, title, app, workspace })
        })
        .collect()
}

// ── X11 (wmctrl) ────────────────────────────────────────────────

fn x11_windows() -> Vec<Win> {
    let Some(text) = run(&["wmctrl", "-lx"]) else { return Vec::new() };
    text.lines().filter_map(parse_wmctrl_line).collect()
}

/// Parse one `wmctrl -lx` row: `<id> <desktop> <class> <host> <title…>`.
/// The title can contain spaces, so we peel the first four fields off the front
/// (tolerating runs of whitespace) and keep the remainder as the title.
fn parse_wmctrl_line(line: &str) -> Option<Win> {
    let r = line.trim_start();
    let (handle, r) = r.split_once(char::is_whitespace)?;
    let (desktop, r) = r.trim_start().split_once(char::is_whitespace)?;
    let (class, r) = r.trim_start().split_once(char::is_whitespace)?;
    let (_host, r) = r.trim_start().split_once(char::is_whitespace)?;
    let title = r.trim().to_string();
    // wmctrl class is "instance.Class"; show the readable part.
    let app = class.split('.').next_back().unwrap_or(class).to_string();
    // Desktop -1 is "sticky/all"; show a friendlier workspace label.
    let workspace = if desktop == "-1" { "all".into() } else { desktop.to_string() };
    if app.is_empty() && title.is_empty() {
        return None;
    }
    Some(Win { handle: handle.to_string(), title, app, workspace })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyprland_json_parses() {
        let json = r#"[
            {"address":"0x55a","title":"nvim","class":"kitty","workspace":{"id":1,"name":"1"}},
            {"address":"0x55b","title":"","class":"","workspace":{"id":1,"name":"1"}}
        ]"#;
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut wins = Vec::new();
        for c in v.as_array().unwrap() {
            if let (Some(a), Some(cl), Some(t)) = (
                c.get("address").and_then(|x| x.as_str()),
                c.get("class").and_then(|x| x.as_str()),
                c.get("title").and_then(|x| x.as_str()),
            ) {
                if cl.is_empty() && t.is_empty() {
                    continue;
                }
                wins.push(a.to_string());
            }
        }
        assert_eq!(wins, vec!["0x55a".to_string()]);
    }

    #[test]
    fn sway_tree_collects_leaf_windows() {
        let json = r#"{"type":"root","name":"root","nodes":[
            {"type":"workspace","name":"2","nodes":[
                {"type":"con","id":42,"name":"Firefox","app_id":"firefox","nodes":[]}
            ],"floating_nodes":[]}
        ],"floating_nodes":[]}"#;
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut out = Vec::new();
        collect_sway(&v, "?", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].handle, "42");
        assert_eq!(out[0].title, "Firefox");
        assert_eq!(out[0].app, "firefox");
        assert_eq!(out[0].workspace, "2");
    }

    #[test]
    fn gnome_list_parses_from_gdbus_wrapper() {
        // Mimics the `gdbus call` reply: a tuple wrapping the JSON string.
        let raw = r#"('[{"wm_class":"kitty","title":"nvim","id":123,"workspace":0},{"wm_class":"","title":"","id":9,"workspace":1}]',)"#;
        let wins = parse_gnome_list(raw);
        assert_eq!(wins.len(), 1); // the empty-class/empty-title entry is skipped
        assert_eq!(wins[0].handle, "123");
        assert_eq!(wins[0].title, "nvim");
        assert_eq!(wins[0].app, "kitty");
        assert_eq!(wins[0].workspace, "1"); // workspace 0 → shown 1-based
    }

    #[test]
    fn x11_wmctrl_line_parses() {
        let out = parse_wmctrl_line("0x03400007  2 kitty.kitty            host Terminal — nvim").unwrap();
        assert_eq!(out.handle, "0x03400007");
        assert_eq!(out.app, "kitty");
        assert_eq!(out.workspace, "2");
        assert_eq!(out.title, "Terminal — nvim");
        // sticky window (-1) → "all"
        let sticky = parse_wmctrl_line("0x01 -1 Polybar.polybar host bar").unwrap();
        assert_eq!(sticky.workspace, "all");
    }
}
