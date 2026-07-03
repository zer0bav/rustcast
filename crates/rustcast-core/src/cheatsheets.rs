//! Cheatsheets: a browsable tab of command references.
//!
//! Ships a curated set of security/dev cheatsheets compiled into the binary
//! (`assets/cheatsheets/*.md`), and also loads any user sheets dropped into
//! `~/.config/rustcast/cheatsheets/*.md`. The full text renders in the preview
//! pane; Enter copies the whole sheet, and user sheets can be opened for editing.

use crate::model::{Action, Item, Prev, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;

const CHEAT_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "copy sheet" },
    ActionHint { keys: "⌃K", label: "open source file" },
    ActionHint { keys: "type", label: "filter by name or content" },
    ActionHint { keys: "esc", label: "close" },
];

/// Bundled cheatsheets: (display name, markdown body). Compiled into the binary.
const BUILTIN: &[(&str, &str)] = &[
    ("nmap", include_str!("../../../assets/cheatsheets/nmap.md")),
    ("netcat", include_str!("../../../assets/cheatsheets/netcat.md")),
    ("tcpdump", include_str!("../../../assets/cheatsheets/tcpdump.md")),
    ("ffuf & gobuster", include_str!("../../../assets/cheatsheets/ffuf-gobuster.md")),
    ("hashcat & john", include_str!("../../../assets/cheatsheets/hashcat-john.md")),
    ("sqlmap", include_str!("../../../assets/cheatsheets/sqlmap.md")),
    ("linux privesc", include_str!("../../../assets/cheatsheets/linux-privesc.md")),
    ("tmux", include_str!("../../../assets/cheatsheets/tmux.md")),
    ("vim", include_str!("../../../assets/cheatsheets/vim.md")),
    ("gdb", include_str!("../../../assets/cheatsheets/gdb.md")),
    ("ssh", include_str!("../../../assets/cheatsheets/ssh.md")),
    ("curl", include_str!("../../../assets/cheatsheets/curl.md")),
    ("git", include_str!("../../../assets/cheatsheets/git.md")),
];

struct Sheet {
    name: String,
    body: String,
    /// Present only for user sheets — lets us offer "Open source file".
    source: Option<String>,
}

pub struct CheatsheetProvider {
    sheets: Vec<Sheet>,
}

impl CheatsheetProvider {
    pub fn new() -> Self {
        let mut sheets: Vec<Sheet> = BUILTIN
            .iter()
            .map(|(name, body)| Sheet { name: name.to_string(), body: body.to_string(), source: None })
            .collect();
        sheets.extend(load_user_sheets());
        CheatsheetProvider { sheets }
    }
}

impl Default for CheatsheetProvider {
    fn default() -> Self {
        CheatsheetProvider::new()
    }
}

impl Provider for CheatsheetProvider {
    fn id(&self) -> &'static str {
        "cheatsheets"
    }
    fn tab(&self) -> Tab {
        Tab::Cheat
    }
    fn placeholder(&self) -> &'static str {
        "Search cheatsheets… (nmap, tmux, vim, curl…)"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        CHEAT_HINTS
    }

    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query.trim().to_lowercase();
        let mut scored: Vec<(i64, &Sheet)> = Vec::new();
        for (i, s) in self.sheets.iter().enumerate() {
            let score = if q.is_empty() {
                1000 - i as i64
            } else {
                // Name matches rank far above content matches.
                let name = ctx.matcher.fuzzy_match(&s.name.to_lowercase(), &q).map(|v| v + 1000);
                let body = if s.body.to_lowercase().contains(&q) { Some(50) } else { None };
                match name.or(body) {
                    Some(v) => v,
                    None => continue,
                }
            };
            scored.push((score, s));
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));

        scored
            .into_iter()
            .map(|(score, s)| {
                let first = s.body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches('#').trim();
                let tag = if s.source.is_some() { "custom" } else { "cheat" };
                let mut actions = Vec::new();
                if let Some(src) = &s.source {
                    actions.push(SecondaryAction { label: "Open source file".into(), action: Action::OpenFile(src.clone()) });
                }
                actions.push(SecondaryAction { label: "Copy sheet".into(), action: Action::Copy(s.body.clone()) });
                Item::new(&s.name, first, "accessories-dictionary", tag, score, Action::Copy(s.body.clone()))
                    .with_prev(Prev::Markdown(s.body.clone()))
                    .with_actions(actions)
            })
            .collect()
    }
}

/// Load `*.md` / `*.txt` from `~/.config/rustcast/cheatsheets/`.
fn load_user_sheets() -> Vec<Sheet> {
    let Some(dir) = crate::config::Config::config_dir().map(|d| d.join("cheatsheets")) else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "md" | "txt") {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else { continue };
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        out.push(Sheet { name, body, source: Some(path.to_string_lossy().into_owned()) });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_non_empty() {
        assert!(!BUILTIN.is_empty());
        assert!(BUILTIN.iter().all(|(n, b)| !n.is_empty() && b.contains('#')));
    }

    #[test]
    fn query_filters_by_name() {
        let p = CheatsheetProvider { sheets: vec![
            Sheet { name: "nmap".into(), body: "# nmap\nscan".into(), source: None },
            Sheet { name: "vim".into(), body: "# vim\nedit".into(), source: None },
        ]};
        let m = crate::ranking::matcher();
        let ctx = QueryCtx { raw: "nmap", query: "nmap", active_tab: Tab::Cheat, matcher: &m, target: None, mode: None };
        let r = p.query(&ctx);
        assert_eq!(r[0].title, "nmap");
    }
}
