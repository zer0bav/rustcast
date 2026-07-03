//! Aliases — short trigger words that stand in for an app, URL, or command.
//!
//! Raycast-style: define `ff` → Firefox in config and typing `ff` floats it to
//! the very top of the root. An exact keyword match wins big; a partial match
//! still surfaces the alias for discovery.

use crate::config::Alias;
use crate::model::{Action, Item, SecondaryAction};
use crate::provider::{Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;

pub struct AliasProvider {
    aliases: Vec<Alias>,
}

impl AliasProvider {
    pub fn new(aliases: Vec<Alias>) -> Self {
        AliasProvider { aliases }
    }
}

/// Map an alias to the action it fires.
fn alias_action(a: &Alias) -> Action {
    match a.kind.as_str() {
        "url" => Action::OpenUrl(a.target.clone()),
        "shell" => Action::RunShell(a.target.clone()),
        _ => Action::Launch(a.target.clone()),
    }
}

impl Provider for AliasProvider {
    fn id(&self) -> &'static str {
        "aliases"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let first = q.split_whitespace().next().unwrap_or("");
        let mut out = Vec::new();
        for a in &self.aliases {
            let kw = a.keyword.to_lowercase();
            // Exact trigger dominates; otherwise fuzzy-match the keyword + name
            // so a partial type still finds the alias.
            let score = if kw == first {
                7_000
            } else {
                let hay = format!("{} {}", a.keyword, a.name).to_lowercase();
                match ctx.matcher.fuzzy_match(&hay, &q) {
                    Some(s) => s + 500,
                    None => continue,
                }
            };
            let name = if a.name.is_empty() { a.target.clone() } else { a.name.clone() };
            let icon = if a.icon.is_empty() {
                match a.kind.as_str() {
                    "url" => "web-browser".to_string(),
                    "shell" => "utilities-terminal".to_string(),
                    _ => "application-x-executable".to_string(),
                }
            } else {
                a.icon.clone()
            };
            let action = alias_action(a);
            out.push(
                Item::new(
                    name,
                    format!("alias: {} → {}", a.keyword, a.target),
                    icon,
                    "alias",
                    score,
                    action.clone(),
                )
                .with_actions(vec![SecondaryAction {
                    label: "Copy target".into(),
                    action: Action::Copy(a.target.clone()),
                }]),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ff() -> Alias {
        Alias {
            keyword: "ff".into(),
            target: "firefox".into(),
            name: "Firefox".into(),
            kind: "launch".into(),
            icon: String::new(),
        }
    }

    #[test]
    fn exact_keyword_scores_high() {
        let p = AliasProvider::new(vec![ff()]);
        let m = crate::ranking::matcher();
        let ctx = QueryCtx { raw: "ff", query: "ff", active_tab: Tab::Apps, matcher: &m, target: None, mode: None };
        let r = p.query(&ctx);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].score, 7_000);
        assert!(matches!(r[0].action, Action::Launch(_)));
    }

    #[test]
    fn empty_query_is_silent() {
        let p = AliasProvider::new(vec![ff()]);
        let m = crate::ranking::matcher();
        let ctx = QueryCtx { raw: "", query: "", active_tab: Tab::Apps, matcher: &m, target: None, mode: None };
        assert!(p.query(&ctx).is_empty());
    }
}
