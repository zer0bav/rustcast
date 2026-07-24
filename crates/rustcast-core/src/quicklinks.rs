//! Quicklinks: user-defined URL / shell templates with a `{query}` placeholder.
//! Triggered by name from the Apps (root) tab.

use crate::config::Quicklink;
use crate::model::{Action, Item, SecondaryAction};
use crate::provider::{Provider, QueryCtx, Tab};

pub struct QuicklinksProvider {
    links: Vec<Quicklink>,
}

impl QuicklinksProvider {
    pub fn new(links: Vec<Quicklink>) -> Self {
        QuicklinksProvider { links }
    }
}

/// Substitute `{query}` / `{argument}`, percent-encoding for URL templates.
pub fn substitute(template: &str, query: &str, is_url: bool) -> String {
    let value = if is_url {
        urlencoding::encode(query).into_owned()
    } else {
        query.to_string()
    };
    template.replace("{query}", &value).replace("{argument}", &value)
}

impl Provider for QuicklinksProvider {
    fn id(&self) -> &'static str {
        "quicklinks"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let raw = ctx.query.trim();
        if raw.is_empty() {
            return Vec::new();
        }
        // A quicklink triggers when the first word matches its name; the rest is {query}.
        let (first, rest) = match raw.split_once(' ') {
            Some((a, b)) => (a, b.trim()),
            None => (raw, ""),
        };
        let mut out = Vec::new();
        for ql in &self.links {
            let is_url = ql.kind == "url" || ql.template.starts_with("http");
            let name_match = ql.name.eq_ignore_ascii_case(first);
            let partial = crate::ranking::score_name(ctx.matcher, &ql.name, raw);
            if !name_match && partial.is_none() {
                continue;
            }
            let arg = if name_match { rest } else { "" };
            let resolved = substitute(&ql.template, arg, is_url);
            let action = if is_url {
                Action::OpenUrl(resolved.clone())
            } else {
                Action::RunShell(resolved.clone())
            };
            // An exact trigger is the most explicit thing the user can type, so
            // it sits above every other exact match; a partial one just ranks in
            // the normal tiers.
            let score = if name_match {
                crate::ranking::EXACT + 2_000
            } else {
                partial.unwrap_or(0)
            };
            let icon = if ql.icon.is_empty() {
                if is_url { "web-browser".to_string() } else { "utilities-terminal".to_string() }
            } else {
                ql.icon.clone()
            };
            out.push(
                Item::new(
                    ql.name.clone(),
                    resolved.clone(),
                    icon,
                    "quicklink",
                    score,
                    action,
                )
                .in_section(crate::registry::section::QUICKLINKS)
                .with_actions(vec![SecondaryAction {
                    label: "Copy".into(),
                    action: Action::Copy(resolved),
                }]),
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_substitution_encodes() {
        assert_eq!(
            substitute("https://x/search?q={query}", "a b", true),
            "https://x/search?q=a%20b"
        );
    }

    #[test]
    fn shell_substitution_raw() {
        assert_eq!(substitute("rg {query} ~", "foo", false), "rg foo ~");
    }
}
