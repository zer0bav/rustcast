//! Pinned favorites — items the user sticks to the top of the root list.
//!
//! Pins are stored as full item snapshots in `~/.local/share/rustcast/pins.json`
//! so they can be re-rendered without re-querying. The GUI toggles them from the
//! Ctrl+K actions menu; [`PinsProvider`] shows them at the very top of the Apps
//! root (empty query only, so searching still shows fresh results, no dupes).

use crate::config::Config;
use crate::model::{Action, Item};
use crate::provider::{Provider, QueryCtx, Tab};
use std::cell::RefCell;
use std::rc::Rc;

/// A pinned item, persisted so it survives restarts.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PinnedItem {
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub icon: String,
    pub action: Action,
}

/// Shared, mutable list of pins held by both the GUI and the provider.
pub type PinList = Rc<RefCell<Vec<PinnedItem>>>;

/// A stable identity for an item, or `None` if it can't be pinned (transient
/// actions like signals, target-setting, clipboard mutations).
pub fn pin_key(a: &Action) -> Option<String> {
    Some(match a {
        Action::Launch(e) => format!("launch:{e}"),
        Action::OpenUrl(u) => format!("url:{u}"),
        Action::OpenFile(p) => format!("file:{p}"),
        Action::RunShell(c) => format!("shell:{c}"),
        Action::RunInTerminal(c) => format!("term:{c}"),
        Action::Copy(t) => format!("copy:{t}"),
        Action::EnterMode { id, .. } => format!("mode:{id}"),
        _ => return None,
    })
}

fn path() -> Option<std::path::PathBuf> {
    Config::data_dir().map(|d| d.join("pins.json"))
}

/// Load pins from disk (empty on any error).
pub fn load() -> Vec<PinnedItem> {
    let Some(p) = path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(p) else { return Vec::new() };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Persist pins to disk.
pub fn save(pins: &[PinnedItem]) {
    let Some(p) = path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(pins) {
        let _ = std::fs::write(p, text);
    }
}

pub struct PinsProvider {
    pins: PinList,
}

impl PinsProvider {
    pub fn new(pins: PinList) -> Self {
        PinsProvider { pins }
    }
}

impl Provider for PinsProvider {
    fn id(&self) -> &'static str {
        "pins"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        // Favorites lead the empty root; once you type, normal providers answer
        // (so a pinned app isn't shown twice).
        if !ctx.query.trim().is_empty() {
            return Vec::new();
        }
        self.pins
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, p)| {
                Item::new(
                    p.title.clone(),
                    p.subtitle.clone(),
                    p.icon.clone(),
                    "pinned",
                    20_000 - i as i64,
                    p.action.clone(),
                )
                .in_section(crate::registry::section::FAVORITES)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_key_stable_and_selective() {
        assert_eq!(pin_key(&Action::Launch("firefox".into())), Some("launch:firefox".into()));
        assert_eq!(pin_key(&Action::OpenUrl("https://x".into())), Some("url:https://x".into()));
        assert_eq!(pin_key(&Action::None), None);
        assert_eq!(pin_key(&Action::Signal { pid: 1, signal: 9 }), None);
    }

    #[test]
    fn provider_shows_pins_only_on_empty_query() {
        let pins: PinList = Rc::new(RefCell::new(vec![PinnedItem {
            title: "Firefox".into(),
            subtitle: "web".into(),
            icon: "firefox".into(),
            action: Action::Launch("firefox".into()),
        }]));
        let p = PinsProvider::new(pins);
        let m = crate::ranking::matcher();
        let empty = QueryCtx { raw: "", query: "", active_tab: Tab::Apps, matcher: &m, mode: None };
        assert_eq!(p.query(&empty).len(), 1);
        let typed = QueryCtx { raw: "fire", query: "fire", active_tab: Tab::Apps, matcher: &m, mode: None };
        assert_eq!(p.query(&typed).len(), 0);
    }
}
