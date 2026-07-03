//! A minimal in-launcher settings surface (Extensions tab). Shows current config
//! values, quick actions (open config/stylesheet/folders, rebuild the file index,
//! clear clipboard), and a discoverable reference of every prefix command.

use crate::config::{which, Config, Quicklink};
use crate::model::{Action, Item, SecondaryAction};
use crate::provider::{Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;

/// Optional external tools rustcast can use, and what breaks without each.
/// Each entry is (any-of these binaries, feature label, install hint).
const DEP_CHECKS: &[(&[&str], &str, &str)] = &[
    (&["wl-copy", "xclip", "xsel"], "Clipboard copy", "install wl-clipboard (Wayland) or xclip"),
    (&["wl-paste"], "Clipboard history & {clipboard} snippets", "install wl-clipboard"),
    (&["xdg-open"], "Open URLs & files", "install xdg-utils"),
    (&["curl", "wget"], "tldr download", "install curl"),
    (&["bsdtar", "unzip"], "tldr extract", "install libarchive or unzip"),
    (&["tesseract"], "Clipboard image OCR", "install tesseract"),
    (&["qalc"], "Advanced calculator (built-in fallback works without it)", "install libqalculate"),
    (&["hyprctl", "swaymsg"], "Window switcher", "wlroots compositor only (Hyprland/Sway)"),
    (&["ss", "lsof"], "Port inspector", "install iproute2 or lsof"),
];

/// Rows describing which optional tools are present/missing.
fn dependency_rows() -> Vec<(String, String, &'static str, i64, Action)> {
    DEP_CHECKS
        .iter()
        .map(|(bins, label, hint)| {
            let present = bins.iter().any(|b| which(b));
            let names = bins.join(" / ");
            if present {
                (
                    format!("{label}: OK"),
                    format!("using {names}"),
                    "emblem-ok",
                    140,
                    Action::None,
                )
            } else {
                (
                    format!("{label}: missing"),
                    format!("{hint}  ·  needs one of: {names}"),
                    "dialog-warning",
                    // Missing deps rank above OK ones so they surface first.
                    600,
                    Action::Copy(hint.to_string()),
                )
            }
        })
        .collect()
}

pub struct SettingsProvider {
    lines: Vec<(String, String)>,
    quicklinks: Vec<Quicklink>,
    animations: bool,
    clip_enabled: bool,
    clip_cap: usize,
}

impl SettingsProvider {
    pub fn new(cfg: &Config) -> Self {
        let cfg_dir = Config::config_dir().map(|p| p.display().to_string()).unwrap_or_default();
        let data_dir = Config::data_dir().map(|p| p.display().to_string()).unwrap_or_default();
        let target = if cfg.cyber.default_target.is_empty() {
            "none".into()
        } else {
            cfg.cyber.default_target.clone()
        };
        let lines = vec![
            ("Version".into(), env!("CARGO_PKG_VERSION").to_string()),
            ("Default tab".into(), cfg.general.default_tab.clone()),
            ("Terminal".into(), cfg.general.terminal.clone()),
            ("Window size".into(), format!("{}×{}", cfg.ui.width, cfg.ui.height)),
            ("File roots".into(), cfg.files.roots.join(", ")),
            ("Cyber target".into(), target),
            ("Quicklinks".into(), cfg.quicklinks.len().to_string()),
            ("Snippets".into(), cfg.snippets.len().to_string()),
            ("Config dir".into(), cfg_dir),
            ("Data dir".into(), data_dir),
        ];
        SettingsProvider {
            lines,
            quicklinks: cfg.quicklinks.clone(),
            animations: cfg.ui.animations,
            clip_enabled: cfg.clipboard.enabled,
            clip_cap: cfg.clipboard.max_entries,
        }
    }
}

fn onoff(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

/// Build the action rows. Paths are resolved here so the shell commands are
/// self-contained (create-then-open, never failing on a missing folder).
fn action_rows() -> Vec<(String, String, &'static str, Action)> {
    let mut rows: Vec<(String, String, &'static str, Action)> = vec![
        (
            "Add Quicklink".into(),
            "create a URL/command shortcut without editing files".into(),
            "list-add",
            Action::EnterMode { id: "add-quicklink".into(), label: "Add Quicklink".into() },
        ),
        ("Open config file".into(), "edit ~/.config/rustcast/config.toml".into(), "text-editor", Action::OpenConfig),
        (
            "Open stylesheet".into(),
            "custom theme at ~/.config/rustcast/style.css".into(),
            "preferences-desktop-theme",
            Action::RunShell(
                "d=\"$HOME/.config/rustcast\"; mkdir -p \"$d\"; f=\"$d/style.css\"; [ -f \"$f\" ] || : > \"$f\"; xdg-open \"$f\""
                    .into(),
            ),
        ),
        (
            "Open cheatsheets folder".into(),
            "drop your own *.md sheets here".into(),
            "accessories-dictionary",
            Action::RunShell(
                "d=\"$HOME/.config/rustcast/cheatsheets\"; mkdir -p \"$d\"; xdg-open \"$d\"".into(),
            ),
        ),
        (
            "Open plugins folder".into(),
            "add script extensions here".into(),
            "application-x-addon",
            Action::RunShell("d=\"$HOME/.config/rustcast/plugins\"; mkdir -p \"$d\"; xdg-open \"$d\"".into()),
        ),
        (
            "Clear clipboard history".into(),
            "delete all non-pinned entries".into(),
            "edit-clear-all",
            Action::ClipClear,
        ),
    ];

    // Rebuild the file index by deleting the on-disk cache; it re-walks on the
    // next launch. Path resolved so we don't depend on a fixed data dir.
    if let Some(idx) = Config::data_dir().map(|d| d.join("files-index.tsv")) {
        rows.push((
            "Rebuild file index".into(),
            "clear the cache — reindexes on next launch".into(),
            "view-refresh",
            Action::RunShell(format!("rm -f {}", crate::action::shell_quote(&idx.display().to_string()))),
        ));
    }
    if let Some(cfg) = Config::config_path() {
        rows.push((
            "Copy config path".into(),
            cfg.display().to_string(),
            "edit-copy",
            Action::Copy(cfg.display().to_string()),
        ));
    }
    rows
}

/// Discoverable command reference. The middle field is either `@<provider-id>`
/// (Enter enters that isolated mode) or a literal prefix like `= ` (Enter drops
/// it into the box for the cyber toolkit).
const COMMANDS: &[(&str, &str, &str)] = &[
    ("Kill Process", "@procs", "browse & terminate processes"),
    ("Window Switcher", "@windows", "jump to an open window"),
    ("Port Inspector", "@ports", "find & kill whatever holds a port"),
    ("Generate Secret", "@gen", "passwords, tokens, UUIDs, PINs"),
    ("Search Cheatsheets", "@cheatsheets", "nmap, tmux, vim, curl…"),
    ("Calculator", "= ", "quick math from any tab"),
    ("Encode / Decode", "b64 ", "base64 / hex / url / rot13"),
    ("Hash", "hash ", "md5 / sha1 / sha256 / sha512"),
    ("JWT decode", "jwt ", "inspect a JSON Web Token"),
    ("Reverse shell", "rev ", "one-liners for host:port"),
];

impl Provider for SettingsProvider {
    fn id(&self) -> &'static str {
        "settings"
    }
    fn tab(&self) -> Tab {
        Tab::Extensions
    }
    fn prefix(&self) -> Option<&'static str> {
        Some("settings")
    }
    fn placeholder(&self) -> &'static str {
        "Settings, actions & command reference…"
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query.trim().to_lowercase();
        let mut out = Vec::new();

        for (title, sub, icon, action) in action_rows() {
            let matches = q.is_empty()
                || format!("{title} {sub}").to_lowercase().contains(&q)
                || ctx.matcher.fuzzy_match(&title.to_lowercase(), &q).is_some();
            if matches {
                out.push(Item::new(title, sub, icon, "settings", 500, action));
            }
        }

        // Dependency status — missing tools surface prominently; searching
        // "deps"/"dependencies"/a tool name lists them all.
        let want_deps = q.contains("dep") || q.contains("missing") || q.contains("tool");
        for (title, sub, icon, score, action) in dependency_rows() {
            let missing = icon == "dialog-warning";
            let matches = want_deps
                || (q.is_empty() && missing)
                || format!("{title} {sub}").to_lowercase().contains(&q);
            if matches {
                out.push(Item::new(title, sub, icon, "deps", score, action));
            }
        }

        // Live-toggleable settings — Enter flips the value in the config file.
        let toggles: [(String, Action); 2] = [
            (
                format!("Animations: {}", onoff(self.animations)),
                Action::SetConfig {
                    section: "ui".into(),
                    key: "animations".into(),
                    value: (!self.animations).to_string(),
                },
            ),
            (
                format!("Clipboard history: {} · cap {}", onoff(self.clip_enabled), self.clip_cap),
                Action::SetConfig {
                    section: "clipboard".into(),
                    key: "enabled".into(),
                    value: (!self.clip_enabled).to_string(),
                },
            ),
        ];
        for (title, action) in toggles {
            let matches = q.is_empty() || title.to_lowercase().contains(&q);
            if matches {
                out.push(Item::new(title, "press Enter to toggle", "emblem-system", "settings", 450, action));
            }
        }

        // Existing quicklinks, each manageable (Ctrl+K: copy template / edit).
        for ql in &self.quicklinks {
            let matches = q.is_empty()
                || format!("quicklink {} {}", ql.name, ql.template).to_lowercase().contains(&q);
            if matches {
                let icon = if ql.template.starts_with("http") { "web-browser" } else { "utilities-terminal" };
                out.push(
                    Item::new(
                        format!("Quicklink: {}", ql.name),
                        ql.template.clone(),
                        icon,
                        "quicklink",
                        300,
                        Action::Copy(ql.template.clone()),
                    )
                    .with_actions(vec![
                        SecondaryAction { label: "Copy template".into(), action: Action::Copy(ql.template.clone()) },
                        SecondaryAction { label: "Edit in config file".into(), action: Action::OpenConfig },
                    ]),
                );
            }
        }

        for (k, v) in &self.lines {
            let matches = q.is_empty() || format!("{k} {v}").to_lowercase().contains(&q);
            if matches {
                // Enter opens the config so the value can actually be changed;
                // Ctrl+K copies it.
                out.push(
                    Item::new(
                        format!("{k}: {v}"),
                        "press Enter to edit in the config file",
                        "emblem-system",
                        "settings",
                        100,
                        Action::OpenConfig,
                    )
                    .with_actions(vec![SecondaryAction {
                        label: "Copy value".into(),
                        action: Action::Copy(v.clone()),
                    }]),
                );
            }
        }

        for (cmd, enter, desc) in COMMANDS {
            let matches = q.is_empty() || format!("{cmd} {desc}").to_lowercase().contains(&q);
            if matches {
                // `@id` enters an isolated mode; anything else is a box prefix.
                let action = if let Some(id) = enter.strip_prefix('@') {
                    Action::EnterMode { id: id.to_string(), label: (*cmd).to_string() }
                } else {
                    Action::SetQuery((*enter).to_string())
                };
                out.push(Item::new(*cmd, *desc, "utilities-terminal", "command", 50, action));
            }
        }
        out
    }
}
