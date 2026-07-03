//! In-app management modes. Currently: "Add Quicklink" — a guided flow that
//! parses `name | template` from the search box and saves it to the config.
//!
//! It only responds inside its command mode (id "add-quicklink"), entered from
//! the root palette or the Extensions tab, so it never appears otherwise.

use crate::model::{Action, Item, Prev};
use crate::provider::{Provider, QueryCtx, Tab};

pub struct AddQuicklinkProvider;

impl AddQuicklinkProvider {
    pub fn new() -> Self {
        AddQuicklinkProvider
    }
}

impl Default for AddQuicklinkProvider {
    fn default() -> Self {
        AddQuicklinkProvider::new()
    }
}

/// Split `name | template` and infer the kind (url when it looks like a link).
pub fn parse_quicklink(input: &str) -> Option<(String, String, String)> {
    let (name, template) = input.split_once('|')?;
    let name = name.trim();
    let template = template.trim();
    if name.is_empty() || template.is_empty() {
        return None;
    }
    let kind = if template.starts_with("http://") || template.starts_with("https://") {
        "url"
    } else {
        "shell"
    };
    Some((name.to_string(), template.to_string(), kind.to_string()))
}

impl Provider for AddQuicklinkProvider {
    fn id(&self) -> &'static str {
        "add-quicklink"
    }
    fn tab(&self) -> Tab {
        Tab::Extensions
    }
    fn placeholder(&self) -> &'static str {
        "name | https://site/{query}   (or a shell command)"
    }

    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        if ctx.mode != Some(self.id()) {
            return Vec::new();
        }
        let input = ctx.query.trim();
        if input.is_empty() {
            return vec![
                Item::new(
                    "Add a quicklink",
                    "type:  name | https://example.com/search?q={query}",
                    "list-add",
                    "quicklink",
                    100,
                    Action::None,
                ),
                Item::new(
                    "Example: gh | https://github.com/search?q={query}",
                    "opens GitHub search for whatever you type after the name",
                    "web-browser",
                    "quicklink",
                    50,
                    Action::None,
                ),
                Item::new(
                    "Example (shell): grep | rg {query} ~",
                    "runs a shell command with {query} substituted",
                    "utilities-terminal",
                    "quicklink",
                    40,
                    Action::None,
                ),
            ];
        }

        match parse_quicklink(input) {
            Some((name, template, kind)) => {
                let preview = format!(
                    "name: {name}\nkind: {kind}\ntemplate: {template}\n\nUse it later by typing:  {name} <your query>"
                );
                vec![Item::new(
                    format!("Save quicklink “{name}”"),
                    format!("{kind} · {template}"),
                    "document-save",
                    "quicklink",
                    9000,
                    Action::AddQuicklink { name, template, kind },
                )
                .with_prev(Prev::Text(preview))]
            }
            None => vec![Item::new(
                "Keep typing:  name | template",
                "separate the name and the template with a  |",
                "dialog-information",
                "quicklink",
                100,
                Action::None,
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url_quicklink() {
        let (n, t, k) = parse_quicklink("gh | https://github.com/search?q={query}").unwrap();
        assert_eq!(n, "gh");
        assert_eq!(t, "https://github.com/search?q={query}");
        assert_eq!(k, "url");
    }

    #[test]
    fn parses_shell_quicklink() {
        let (_, _, k) = parse_quicklink("grep | rg {query} ~").unwrap();
        assert_eq!(k, "shell");
    }

    #[test]
    fn rejects_incomplete() {
        assert!(parse_quicklink("noseparator").is_none());
        assert!(parse_quicklink("name |").is_none());
        assert!(parse_quicklink("| template").is_none());
    }
}
