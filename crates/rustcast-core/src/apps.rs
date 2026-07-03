//! Application launcher provider — parses `.desktop` files and fuzzy-matches
//! them. Ported from the original single-file prototype.

use crate::model::{Action, Item, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;

#[derive(Clone)]
pub struct DesktopApp {
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub subtitle: String,
    pub haystack: String,
}

pub fn load_apps() -> Vec<DesktopApp> {
    let mut dirs: Vec<std::path::PathBuf> = vec![
        "/usr/share/applications".into(),
        "/usr/local/share/applications".into(),
    ];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(format!("{home}/.local/share/applications").into());
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_DIRS") {
        for d in xdg.split(':') {
            dirs.push(format!("{d}/applications").into());
        }
    }
    let mut apps = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            if let Some(a) = parse_desktop(&content) {
                if seen.insert(a.name.clone()) {
                    apps.push(a);
                }
            }
        }
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

pub fn parse_desktop(content: &str) -> Option<DesktopApp> {
    let mut in_entry = false;
    let (mut name, mut exec, mut icon, mut generic, mut keywords) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    let (mut no_display, mut hidden, mut is_app) = (false, false, false);
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "Name" if name.is_empty() => name = v.trim().to_string(),
            "Exec" if exec.is_empty() => exec = v.trim().to_string(),
            "Icon" if icon.is_empty() => icon = v.trim().to_string(),
            "GenericName" if generic.is_empty() => generic = v.trim().to_string(),
            "Keywords" => keywords = v.trim().to_string(),
            "Type" => is_app = v.trim() == "Application",
            "NoDisplay" => no_display = v.trim() == "true",
            "Hidden" => hidden = v.trim() == "true",
            _ => {}
        }
    }
    if !is_app || no_display || hidden || name.is_empty() || exec.is_empty() {
        return None;
    }
    let haystack = format!("{name} {generic} {keywords}").to_lowercase();
    let subtitle = if !generic.is_empty() { generic } else { keywords };
    Some(DesktopApp { name, exec, icon, subtitle, haystack })
}

pub fn clean_exec(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|t| !(t.starts_with('%') && t.len() == 2))
        .collect::<Vec<_>>()
        .join(" ")
}

const APP_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "open" },
    ActionHint { keys: "⌃K", label: "actions" },
    ActionHint { keys: "↑↓", label: "navigate" },
    ActionHint { keys: "esc", label: "close" },
];

pub struct AppsProvider {
    apps: Vec<DesktopApp>,
}

impl AppsProvider {
    pub fn new() -> Self {
        AppsProvider { apps: load_apps() }
    }
}

impl Default for AppsProvider {
    fn default() -> Self {
        AppsProvider::new()
    }
}

impl Provider for AppsProvider {
    fn id(&self) -> &'static str {
        "apps"
    }
    fn tab(&self) -> Tab {
        Tab::Apps
    }
    fn placeholder(&self) -> &'static str {
        "Search apps and commands…"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        APP_HINTS
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query;
        let mut out = Vec::new();
        for a in &self.apps {
            let score = if q.is_empty() {
                1
            } else {
                match ctx.matcher.fuzzy_match(&a.haystack, q) {
                    Some(s) => s,
                    None => continue,
                }
            };
            let icon = if a.icon.is_empty() {
                "application-x-executable".to_string()
            } else {
                a.icon.clone()
            };
            let item = Item::new(
                a.name.clone(),
                a.subtitle.clone(),
                icon,
                "app",
                score,
                Action::Launch(a.exec.clone()),
            )
            .with_actions(vec![SecondaryAction {
                label: "Copy launch command".into(),
                action: Action::Copy(clean_exec(&a.exec)),
            }]);
            out.push(item);
        }
        out
    }
}
