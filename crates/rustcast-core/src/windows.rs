//! Window switcher for wlroots compositors (Hyprland, Sway).
//!
//! Lists open windows via the compositor's IPC (`hyprctl -j clients`,
//! `swaymsg -t get_tree`), focuses one on Enter, and can close it from the
//! actions menu. Both dispatch through [`Action::RunShell`], which closes the
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

enum Compositor {
    Hyprland,
    Sway,
    None,
}

struct Win {
    /// Compositor handle: Hyprland address (`0x…`) or Sway con_id.
    handle: String,
    title: String,
    app: String,
    workspace: String,
}

pub struct WindowsProvider {
    comp: Compositor,
}

impl WindowsProvider {
    pub fn new() -> Self {
        let comp = if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            Compositor::Hyprland
        } else if std::env::var_os("SWAYSOCK").is_some() {
            Compositor::Sway
        } else {
            Compositor::None
        };
        WindowsProvider { comp }
    }

    fn focus_cmd(&self, w: &Win) -> String {
        match self.comp {
            Compositor::Hyprland => format!("hyprctl dispatch focuswindow address:{}", w.handle),
            Compositor::Sway => format!("swaymsg [con_id={}] focus", w.handle),
            Compositor::None => String::new(),
        }
    }

    fn close_cmd(&self, w: &Win) -> String {
        match self.comp {
            Compositor::Hyprland => format!("hyprctl dispatch closewindow address:{}", w.handle),
            Compositor::Sway => format!("swaymsg [con_id={}] kill", w.handle),
            Compositor::None => String::new(),
        }
    }
}

impl Default for WindowsProvider {
    fn default() -> Self {
        WindowsProvider::new()
    }
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
        if matches!(self.comp, Compositor::None) {
            return vec![Item::new(
                "Window switching needs Hyprland or Sway",
                "no supported compositor detected",
                "dialog-warning",
                "win",
                1,
                Action::None,
            )];
        }

        let wins = match self.comp {
            Compositor::Hyprland => hyprland_windows(),
            Compositor::Sway => sway_windows(),
            Compositor::None => Vec::new(),
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
                let subtitle = format!("{} · workspace {}", w.app, w.workspace);
                Item::new(title, subtitle, "window", "win", score, Action::RunShell(self.focus_cmd(&w)))
                    .with_actions(vec![SecondaryAction {
                        label: "Close window".into(),
                        action: Action::RunShell(self.close_cmd(&w)),
                    }])
            })
            .collect()
    }
}

fn run(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(args[0]).args(&args[1..]).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

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
}
