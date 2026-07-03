//! Legacy clipboard provider backed by `cliphist` — used in Phase 0 so behavior
//! is preserved. Phase 2 replaces this with a native clipboard store; the tab
//! and item shape stay the same so the GUI doesn't change.

use crate::config::which;
use crate::model::{Action, Item, Prev, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;
use std::process::Command;

const CLIP_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "paste to clipboard" },
    ActionHint { keys: "⌃K", label: "actions" },
    ActionHint { keys: "↑↓", label: "navigate" },
    ActionHint { keys: "esc", label: "close" },
];

pub struct ClipProvider {
    available: bool,
}

impl ClipProvider {
    pub fn new() -> Self {
        ClipProvider { available: which("cliphist") }
    }
}

impl Default for ClipProvider {
    fn default() -> Self {
        ClipProvider::new()
    }
}

impl Provider for ClipProvider {
    fn id(&self) -> &'static str {
        "clipboard"
    }
    fn tab(&self) -> Tab {
        Tab::Clipboard
    }
    fn placeholder(&self) -> &'static str {
        "Search clipboard history…"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        CLIP_HINTS
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        if !self.available {
            return vec![Item::new(
                "cliphist not found",
                "Install cliphist for clipboard history (native store coming soon)",
                "edit-paste",
                "clip",
                1,
                Action::None,
            )];
        }
        let Ok(out) = Command::new("cliphist").arg("list").output() else {
            return Vec::new();
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let q = ctx.query;
        let mut items = Vec::new();
        for (rank, line) in text.lines().enumerate() {
            let preview = line.split_once('\t').map(|(_, p)| p).unwrap_or(line).trim();
            let is_img = preview.contains("binary")
                && ["png", "jpeg", "jpg", "gif", "webp", "bmp", "image"]
                    .iter()
                    .any(|t| preview.contains(t));
            let display = if is_img {
                format!("image  [{}]", preview.trim_matches(|c| c == '[' || c == ']').trim())
            } else {
                preview.to_string()
            };
            let score = if q.is_empty() {
                2000 - rank as i64
            } else {
                match ctx.matcher.fuzzy_match(preview, q) {
                    Some(s) => s,
                    None => continue,
                }
            };
            let prev = if is_img {
                Prev::ClipImage(line.to_string())
            } else {
                Prev::Text(preview.to_string())
            };
            let item = Item::new(
                display.chars().take(90).collect::<String>(),
                "clipboard",
                if is_img { "image-x-generic" } else { "edit-paste" },
                "clip",
                score,
                Action::ClipCopy(line.to_string()),
            )
            .with_prev(prev)
            .with_actions(vec![SecondaryAction {
                label: "Copy text".into(),
                action: Action::Copy(preview.to_string()),
            }]);
            items.push(item);
        }
        items
    }
}
