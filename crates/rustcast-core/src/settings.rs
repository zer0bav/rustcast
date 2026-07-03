//! A minimal in-launcher settings surface (Extensions tab). Shows current config
//! values and a few actions: open the config file, open the stylesheet, and clear
//! clipboard history.

use crate::config::Config;
use crate::model::{Action, Item};
use crate::provider::{Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;

pub struct SettingsProvider {
    lines: Vec<(String, String)>,
}

impl SettingsProvider {
    pub fn new(cfg: &Config) -> Self {
        let lines = vec![
            ("Default tab".into(), cfg.general.default_tab.clone()),
            ("Terminal".into(), cfg.general.terminal.clone()),
            ("Animations".into(), cfg.ui.animations.to_string()),
            ("Window size".into(), format!("{}×{}", cfg.ui.width, cfg.ui.height)),
            ("Clipboard".into(), format!("enabled={} · cap={}", cfg.clipboard.enabled, cfg.clipboard.max_entries)),
            ("File roots".into(), cfg.files.roots.join(", ")),
            ("Quicklinks".into(), cfg.quicklinks.len().to_string()),
            ("Snippets".into(), cfg.snippets.len().to_string()),
        ];
        SettingsProvider { lines }
    }
}

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
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query.trim().to_lowercase();
        let mut out = Vec::new();

        let actions: [(&str, &str, &str, Action); 4] = [
            ("Open config file", "edit ~/.config/rustcast/config.toml", "text-editor", Action::OpenConfig),
            (
                "Open stylesheet",
                "custom theme at ~/.config/rustcast/style.css",
                "preferences-desktop-theme",
                Action::RunShell(
                    "d=\"$HOME/.config/rustcast\"; mkdir -p \"$d\"; f=\"$d/style.css\"; [ -f \"$f\" ] || : > \"$f\"; xdg-open \"$f\""
                        .into(),
                ),
            ),
            ("Clear clipboard history", "delete all non-pinned entries", "edit-clear-all", Action::ClipClear),
            (
                "Reload apps",
                "rescan .desktop files (restart)",
                "view-refresh",
                Action::None,
            ),
        ];

        for (title, sub, icon, action) in actions {
            let matches = q.is_empty() || format!("{title} {sub}").to_lowercase().contains(&q)
                || ctx.matcher.fuzzy_match(&title.to_lowercase(), &q).is_some();
            if matches {
                out.push(Item::new(title, sub, icon, "settings", 500, action));
            }
        }

        for (k, v) in &self.lines {
            let matches = q.is_empty() || format!("{k} {v}").to_lowercase().contains(&q);
            if matches {
                out.push(Item::new(
                    format!("{k}: {v}"),
                    "config value (edit in config file)",
                    "emblem-system",
                    "settings",
                    100,
                    Action::None,
                ));
            }
        }
        out
    }
}
