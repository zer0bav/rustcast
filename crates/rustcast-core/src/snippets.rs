//! Text-expansion snippets from config. Triggered by keyword or name from the
//! Apps (root) tab; activating copies the (token-expanded) text.

use crate::config::Snippet;
use crate::model::{Action, Item};
use crate::provider::{Provider, QueryCtx, Tab};

pub struct SnippetsProvider {
    snippets: Vec<Snippet>,
}

impl SnippetsProvider {
    pub fn new(snippets: Vec<Snippet>) -> Self {
        SnippetsProvider { snippets }
    }
}

/// Expand `{date}` / `{time}` / `{datetime}` / `{clipboard}` tokens in snippet
/// text. `{clipboard}` is read from the live clipboard only when present (so
/// snippets without it never shell out).
pub fn expand(text: &str) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let now = chrono::Local::now();
    let mut out = text
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H:%M").to_string())
        .replace("{datetime}", &now.format("%Y-%m-%d %H:%M").to_string());
    if out.contains("{clipboard}") {
        out = out.replace("{clipboard}", &crate::action::paste_text());
    }
    out
}

impl Provider for SnippetsProvider {
    fn id(&self) -> &'static str {
        "snippets"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for s in &self.snippets {
            let name = if s.name.is_empty() { &s.keyword } else { &s.name };
            let kw_match = !s.keyword.is_empty() && s.keyword.to_lowercase() == q;
            let hidden = format!("{} {}", s.keyword, s.text);
            let score = if kw_match {
                crate::ranking::EXACT + 1_000
            } else {
                match crate::ranking::score(ctx.matcher, name, &hidden, &q) {
                    Some(sc) => sc,
                    None => continue,
                }
            };
            let expanded = expand(&s.text);
            let item = Item::new(
                name.clone(),
                expanded.replace('\n', " ").chars().take(80).collect::<String>(),
                "text-x-generic",
                "snippet",
                score,
                Action::Copy(expanded.clone()),
            )
            .in_section(crate::registry::section::SNIPPETS)
            .with_prev(crate::model::Prev::Text(expanded));
            out.push(item);
        }
        out
    }
}
