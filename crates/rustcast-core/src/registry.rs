//! Holds every provider and routes a query to the right ones.
//!
//! Replaces the hardcoded provider dispatch that used to live inside the GUI's
//! `rebuild` closure.

use crate::model::{Action, Item};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use std::cell::RefCell;
use std::rc::Rc;

pub const MAX_RESULTS: usize = 80;

const DEFAULT_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "open" },
    ActionHint { keys: "⌃K", label: "actions" },
    ActionHint { keys: "⇥", label: "switch tab" },
    ActionHint { keys: "esc", label: "close" },
];

pub struct Registry {
    providers: Vec<Box<dyn Provider>>,
    /// Usage-based boost applied to items on the Apps root. Optional so headless
    /// tests can build a registry without it.
    frecency: Option<Rc<RefCell<crate::frecency::Frecency>>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry { providers: Vec::new(), frecency: None }
    }

    pub fn register(&mut self, p: Box<dyn Provider>) {
        self.providers.push(p);
    }

    /// Attach the shared frecency store so tab-routed results get a recency/usage
    /// boost. Re-called whenever the registry is rebuilt.
    pub fn set_frecency(&mut self, f: Rc<RefCell<crate::frecency::Frecency>>) {
        self.frecency = Some(f);
    }

    /// Refresh every provider's cached/background state (daemon window-show hook).
    pub fn refresh_all(&self) {
        for p in &self.providers {
            p.refresh();
        }
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
        mode: Option<&str>,
    ) -> Vec<Item> {
        let mut collected: Vec<Item> = Vec::new();

        if let Some(mode_id) = mode {
            // Inside a command mode: the whole query is a filter for one
            // provider — no prefix parsing, so nothing collides with it.
            if let Some(p) = self.providers.iter().find(|p| p.id() == mode_id) {
                let query = raw.trim();
                let ctx = QueryCtx { raw, query, active_tab, matcher, target, mode };
                collected.extend(p.query(&ctx));
            }
        } else if let Some((provider, plen)) = self.prefix_match(raw) {
            let query = raw.trim_start()[plen..].trim();
            let ctx = QueryCtx { raw, query, active_tab, matcher, target, mode: None };
            collected.extend(provider.query(&ctx));
        } else {
            let query = raw.trim();
            for p in &self.providers {
                if p.tab() == active_tab {
                    let ctx = QueryCtx { raw, query, active_tab, matcher, target, mode: None };
                    collected.extend(p.query(&ctx));
                }
            }
            // Usage/recency boost — only for the normal tab route (never inside a
            // command mode or prefix tool, where reordering by history would be
            // confusing, e.g. the kill-process list).
            if let Some(fr) = &self.frecency {
                let fr = fr.borrow();
                let now = crate::frecency::now_unix();
                for it in &mut collected {
                    if let Some(k) = crate::pins::pin_key(&it.action) {
                        it.score += fr.boost(&k, now);
                    }
                }
            }
            // Nothing matched an Apps-tab query → offer to search the web.
            if collected.is_empty() && active_tab == Tab::Apps && !query.is_empty() {
                collected.extend(web_fallback(query));
            }
        }

        collected.sort_by(|a, b| b.score.cmp(&a.score));
        collected.truncate(MAX_RESULTS);
        collected
    }
}

/// Two "search the web for <q>" rows, shown only when an Apps-tab query returns
/// no local results. `OpenUrl` opens the default browser via `xdg-open`.
fn web_fallback(query: &str) -> Vec<Item> {
    let enc = urlencoding::encode(query);
    vec![
        Item::new(
            format!("Search Google for \u{201c}{query}\u{201d}"),
            "open in your browser",
            "web-browser",
            "web",
            10,
            Action::OpenUrl(format!("https://www.google.com/search?q={enc}")),
        ),
        Item::new(
            format!("Search DuckDuckGo for \u{201c}{query}\u{201d}"),
            "open in your browser",
            "web-browser",
            "web",
            9,
            Action::OpenUrl(format!("https://duckduckgo.com/?q={enc}")),
        ),
    ]
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new()
    }
}
