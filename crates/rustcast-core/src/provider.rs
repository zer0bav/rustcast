//! The `Provider` abstraction that replaces the hardcoded query closures.
//!
//! GTK runs on a single thread, so `query()` is called on the main loop and must
//! be cheap. Providers that do genuinely slow work (network scans) return
//! action-only items and spawn detached processes instead of blocking here.

use crate::model::Item;
use fuzzy_matcher::skim::SkimMatcherV2;

/// The mode tabs across the top of the launcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Apps,
    Clipboard,
    Files,
    Cyber,
    Cheat,
    Win,
    Extensions,
}

impl Tab {
    pub const ALL: [Tab; 7] =
        [Tab::Apps, Tab::Clipboard, Tab::Files, Tab::Cyber, Tab::Cheat, Tab::Win, Tab::Extensions];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Apps => "Apps",
            Tab::Clipboard => "Clipboard",
            Tab::Files => "Files",
            Tab::Cyber => "Cyber",
            Tab::Cheat => "Cheats",
            Tab::Win => "Windows",
            Tab::Extensions => "Extensions",
        }
    }

    pub fn from_index(i: usize) -> Option<Tab> {
        Tab::ALL.get(i).copied()
    }

    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|&t| t == self).unwrap_or(0)
    }
}

/// Shared state threaded into every query: the raw entry text, the query with
/// any tab/prefix scoping stripped, the active tab, a fuzzy matcher, and the
/// current cyber target.
pub struct QueryCtx<'a> {
    pub raw: &'a str,
    pub query: &'a str,
    pub active_tab: Tab,
    pub matcher: &'a SkimMatcherV2,
    pub target: Option<&'a str>,
    /// When set, an isolated command mode is active and the query routes only to
    /// the provider whose [`Provider::id`] equals this. Tool providers (process
    /// killer, window switcher, secret generator, port inspector) respond only
    /// in their mode, so typing a word like `kill` never collides with an app.
    pub mode: Option<&'a str>,
}

/// A hint chip shown in the animated footer.
#[derive(Clone, Debug)]
pub struct ActionHint {
    pub keys: &'static str,
    pub label: &'static str,
}

pub trait Provider {
    /// Stable identifier (used for prefix routing / logging).
    fn id(&self) -> &'static str;

    /// The tab this provider feeds.
    fn tab(&self) -> Tab;

    /// Optional inline prefix (e.g. "=", "b64") that routes to this provider
    /// from any tab.
    fn prefix(&self) -> Option<&'static str> {
        None
    }

    /// Placeholder shown in the entry when this provider's tab is active.
    fn placeholder(&self) -> &'static str {
        "Search…"
    }

    /// Footer hint chips for this provider's tab.
    fn footer_hints(&self) -> &'static [ActionHint] {
        &[]
    }

    /// Produce ranked items. Called on the GTK main thread — keep it fast.
    fn query(&self, ctx: &QueryCtx) -> Vec<Item>;
}
