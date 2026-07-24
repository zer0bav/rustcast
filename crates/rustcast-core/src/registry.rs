//! Holds every provider, routes a query to the right ones, and arranges the
//! results the way Raycast does: best match first, grouped under section
//! headers, with your habits ([`crate::frecency`]) breaking the ties.

use crate::model::{Action, Item};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use std::cell::RefCell;
use std::rc::Rc;

pub const MAX_RESULTS: usize = 80;

/// Section names used across providers. Kept here so the strings can't drift
/// apart (grouping is by exact name).
pub mod section {
    pub const FAVORITES: &str = "Favorites";
    pub const SUGGESTIONS: &str = "Suggestions";
    pub const APPLICATIONS: &str = "Applications";
    pub const COMMANDS: &str = "Commands";
    pub const QUICKLINKS: &str = "Quicklinks";
    pub const ALIASES: &str = "Aliases";
    pub const SNIPPETS: &str = "Snippets";
    pub const SYSTEM: &str = "System";
    pub const CALCULATOR: &str = "Calculator";
    pub const FILES: &str = "Files";
    pub const PINNED: &str = "Pinned";
    pub const HISTORY: &str = "History";
    pub const WEB: &str = "Web Search";
}

/// How many habitual items lead the empty root as "Suggestions".
const SUGGESTIONS: usize = 6;

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
        mode: Option<&str>,
    ) -> Vec<Item> {
        let mut collected: Vec<Item> = Vec::new();
        // Prefix tools and command modes are single-provider views: no usage
        // reordering and no section headers, so e.g. the kill-process list stays
        // in the order that provider chose.
        let mut grouped = false;

        if let Some(mode_id) = mode {
            // Inside a command mode: the whole query is a filter for one
            // provider — no prefix parsing, so nothing collides with it.
            if let Some(p) = self.providers.iter().find(|p| p.id() == mode_id) {
                let query = raw.trim();
                let ctx = QueryCtx { raw, query, active_tab, matcher, mode };
                collected.extend(p.query(&ctx));
            }
        } else if let Some((provider, plen)) = self.prefix_match(raw) {
            let query = raw.trim_start()[plen..].trim();
            let ctx = QueryCtx { raw, query, active_tab, matcher, mode: None };
            collected.extend(provider.query(&ctx));
        } else {
            grouped = true;
            let query = raw.trim();
            for p in &self.providers {
                if p.tab() == active_tab {
                    let ctx = QueryCtx { raw, query, active_tab, matcher, mode: None };
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
                if query.is_empty() {
                    promote_suggestions(&mut collected, &fr, now);
                }
            }
            // Nothing matched an Apps-tab query → offer to search the web.
            if collected.is_empty() && active_tab == Tab::Apps && !query.is_empty() {
                collected.extend(web_fallback(query));
            }
        }

        collected.sort_by(|a, b| b.score.cmp(&a.score));
        collected.truncate(MAX_RESULTS);
        if grouped {
            collected = insert_section_headers(collected);
        }
        collected
    }
}

/// Lift the most-used items into a leading "Suggestions" group on the empty
/// root, the way Raycast does — so the launcher opens on what you actually use
/// instead of an alphabetical wall. Already-pinned items are left alone (they're
/// in Favorites; showing them twice is noise).
fn promote_suggestions(items: &mut [Item], fr: &crate::frecency::Frecency, now: i64) {
    let pinned: std::collections::HashSet<String> = items
        .iter()
        .filter(|it| it.section == section::FAVORITES)
        .filter_map(|it| crate::pins::pin_key(&it.action))
        .collect();

    // (index, boost) for every used, non-pinned item.
    let mut ranked: Vec<(usize, i64)> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| it.section != section::FAVORITES && !it.header)
        .filter_map(|(i, it)| {
            let k = crate::pins::pin_key(&it.action)?;
            if pinned.contains(&k) {
                return None;
            }
            match fr.boost(&k, now) {
                0 => None,
                b => Some((i, b)),
            }
        })
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    // Scored just under Favorites (20_000) and above every other empty-root item.
    for (rank, (idx, _)) in ranked.into_iter().take(SUGGESTIONS).enumerate() {
        items[idx].section = section::SUGGESTIONS.to_string();
        items[idx].score = 15_000 - rank as i64;
    }
}

/// Insert a header row before each named group. Input must already be sorted by
/// score, which makes a section's first appearance its best hit — so walking the
/// list in order and emitting a header on each new section name yields groups
/// ordered by relevance, with the single best result still on the first row.
///
/// Items with an empty section stay ungrouped and keep their place.
fn insert_section_headers(items: Vec<Item>) -> Vec<Item> {
    if !items.iter().any(|it| !it.section.is_empty()) {
        return items;
    }
    let mut out: Vec<Item> = Vec::with_capacity(items.len() + 4);
    let mut seen: Vec<String> = Vec::new();
    for it in items {
        if !it.section.is_empty() && !seen.iter().any(|s| *s == it.section) {
            seen.push(it.section.clone());
            out.push(Item::section_header(it.section.clone()));
        }
        out.push(it);
    }
    out
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
        )
        .in_section(section::WEB),
        Item::new(
            format!("Search DuckDuckGo for \u{201c}{query}\u{201d}"),
            "open in your browser",
            "web-browser",
            "web",
            9,
            Action::OpenUrl(format!("https://duckduckgo.com/?q={enc}")),
        )
        .in_section(section::WEB),
    ]
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, sec: &str, score: i64) -> Item {
        Item::new(title, "", "", "t", score, Action::None).in_section(sec)
    }

    #[test]
    fn headers_follow_the_best_hit_of_each_group() {
        let items = vec![
            item("Signal", section::APPLICATIONS, 8_000),
            item("Kill Process", section::COMMANDS, 6_400),
            item("Slack", section::APPLICATIONS, 6_000),
        ];
        let out = insert_section_headers(items);
        let shape: Vec<(&str, bool)> = out.iter().map(|i| (i.title.as_str(), i.header)).collect();
        assert_eq!(
            shape,
            vec![
                ("Applications", true),
                ("Signal", false),
                ("Commands", true),
                ("Kill Process", false),
                // A later Applications item doesn't repeat the header; it just
                // sorts where its score put it.
                ("Slack", false),
            ]
        );
    }

    #[test]
    fn ungrouped_results_get_no_headers() {
        let items = vec![item("a", "", 2), item("b", "", 1)];
        let out = insert_section_headers(items);
        assert!(out.iter().all(|i| !i.header));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn suggestions_lift_used_items_and_skip_pins() {
        let mut fr = crate::frecency::Frecency::new();
        for _ in 0..5 {
            fr.record("launch:brave");
        }
        fr.record("launch:kitty");
        let now = crate::frecency::now_unix();

        let mut items = vec![
            Item::new("Brave", "", "", "app", 400, Action::Launch("brave".into()))
                .in_section(section::APPLICATIONS),
            Item::new("Kitty", "", "", "app", 400, Action::Launch("kitty".into()))
                .in_section(section::APPLICATIONS),
            Item::new("Zed", "", "", "app", 400, Action::Launch("zed".into()))
                .in_section(section::APPLICATIONS),
            // Brave is also pinned → must not appear in Suggestions as well.
            Item::new("Brave", "", "", "pinned", 20_000, Action::Launch("brave".into()))
                .in_section(section::FAVORITES),
        ];
        promote_suggestions(&mut items, &fr, now);

        assert_eq!(items[0].section, section::APPLICATIONS, "pinned Brave stays out");
        assert_eq!(items[1].section, section::SUGGESTIONS, "used Kitty is promoted");
        assert_eq!(items[2].section, section::APPLICATIONS, "unused Zed is not");
    }
}
