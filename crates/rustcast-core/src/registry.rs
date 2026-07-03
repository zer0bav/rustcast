//! Holds every provider and routes a query to the right ones.
//!
//! Replaces the hardcoded provider dispatch that used to live inside the GUI's
//! `rebuild` closure.

use crate::model::Item;
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};

pub const MAX_RESULTS: usize = 80;

const DEFAULT_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "open" },
    ActionHint { keys: "⌃K", label: "actions" },
    ActionHint { keys: "⇥", label: "switch tab" },
    ActionHint { keys: "esc", label: "close" },
];

pub struct Registry {
    providers: Vec<Box<dyn Provider>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry { providers: Vec::new() }
    }

    pub fn register(&mut self, p: Box<dyn Provider>) {
        self.providers.push(p);
    }

    /// Find a provider whose prefix begins `raw` (followed by end or space).
    fn prefix_match(&self, raw: &str) -> Option<(&dyn Provider, usize)> {
        let raw_t = raw.trim_start();
        for p in &self.providers {
            if let Some(px) = p.prefix() {
                if raw_t == px || raw_t.starts_with(&format!("{px} ")) {
                    return Some((p.as_ref(), px.len()));
                }
            }
        }
        None
    }

    /// Placeholder + tab for the active tab's first provider (for the entry).
    pub fn placeholder_for(&self, tab: Tab) -> &'static str {
        self.providers
            .iter()
            .find(|p| p.tab() == tab)
            .map(|p| p.placeholder())
            .unwrap_or("Search…")
    }

    /// Footer hint chips for the active tab (first provider that supplies any).
    pub fn footer_hints_for(&self, tab: Tab) -> &'static [crate::provider::ActionHint] {
        self.providers
            .iter()
            .filter(|p| p.tab() == tab)
            .map(|p| p.footer_hints())
            .find(|h| !h.is_empty())
            .unwrap_or(DEFAULT_HINTS)
    }

    /// Route and collect ranked items.
    ///
    /// `raw` is the full entry text; `active_tab` is the focused tab. An inline
    /// prefix (e.g. `= 2+2`) overrides the tab and targets a single provider.
    pub fn route(
        &self,
        raw: &str,
        active_tab: Tab,
        matcher: &fuzzy_matcher::skim::SkimMatcherV2,
        target: Option<&str>,
    ) -> Vec<Item> {
        let mut collected: Vec<Item> = Vec::new();

        if let Some((provider, plen)) = self.prefix_match(raw) {
            let query = raw.trim_start()[plen..].trim();
            let ctx = QueryCtx { raw, query, active_tab, matcher, target };
            collected.extend(provider.query(&ctx));
        } else {
            let query = raw.trim();
            for p in &self.providers {
                if p.tab() == active_tab {
                    let ctx = QueryCtx { raw, query, active_tab, matcher, target };
                    collected.extend(p.query(&ctx));
                }
            }
        }

        collected.sort_by(|a, b| b.score.cmp(&a.score));
        collected.truncate(MAX_RESULTS);
        collected
    }
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new()
    }
}
