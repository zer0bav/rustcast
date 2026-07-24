//! Application launcher provider — parses `.desktop` files and fuzzy-matches
//! them. Ported from the original single-file prototype.

use crate::model::{Action, Item, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct DesktopApp {
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub subtitle: String,
    /// GenericName + Keywords= — matched only as a weak fallback, never as the
    /// primary haystack (see [`crate::ranking`]).
    pub keywords: String,
}

/// The XDG application directories, most-specific last (user overrides system).
fn app_dirs() -> Vec<std::path::PathBuf> {
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
    dirs
}

pub fn load_apps() -> Vec<DesktopApp> {
    let dirs = app_dirs();
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
    let hidden_words = format!("{generic} {keywords}").trim().to_lowercase();
    let subtitle = generic;
    Some(DesktopApp { name, exec, icon, subtitle, keywords: hidden_words })
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
    apps: Arc<RwLock<Vec<DesktopApp>>>,
    refreshing: Arc<AtomicBool>,
}

impl AppsProvider {
    /// Load the disk cache instantly, then kick a fresh background rescan so the
    /// first window paint never blocks on hundreds of `.desktop` reads.
    pub fn new() -> Self {
        let apps = Arc::new(RwLock::new(load_cache()));
        let p = AppsProvider { apps, refreshing: Arc::new(AtomicBool::new(false)) };
        p.spawn_rescan();
        p
    }

    /// Rescan the app dirs on a background thread, unless one is already running.
    fn spawn_rescan(&self) {
        if self.refreshing.swap(true, Ordering::SeqCst) {
            return; // a rescan is already in flight
        }
        let apps = self.apps.clone();
        let flag = self.refreshing.clone();
        std::thread::spawn(move || {
            let fresh = load_apps();
            save_cache(&fresh);
            if let Ok(mut w) = apps.write() {
                *w = fresh;
            }
            flag.store(false, Ordering::SeqCst);
        });
    }
}

impl Default for AppsProvider {
    fn default() -> Self {
        AppsProvider::new()
    }
}

/// The cache file name carries a format version: when the column meaning
/// changes (v2 dropped Keywords= from the subtitle and made it match-only), a
/// new name makes every install ignore its stale index instead of rendering it
/// with the wrong meaning until the app dirs happen to change.
fn cache_path() -> Option<std::path::PathBuf> {
    crate::config::Config::data_dir().map(|d| d.join("apps-index-v2.tsv"))
}

/// Newest mtime across the app dirs, for staleness checks.
fn dirs_mtime() -> Option<std::time::SystemTime> {
    app_dirs()
        .iter()
        .filter_map(|d| std::fs::metadata(d).ok())
        .filter_map(|m| m.modified().ok())
        .max()
}

fn load_cache() -> Vec<DesktopApp> {
    let Some(p) = cache_path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(p) else { return Vec::new() };
    text.lines()
        .filter_map(|line| {
            let mut it = line.split('\t');
            let name = it.next()?.to_string();
            let exec = it.next()?.to_string();
            let icon = it.next().unwrap_or("").to_string();
            let subtitle = it.next().unwrap_or("").to_string();
            let keywords = it.next().unwrap_or("").to_string();
            if name.is_empty() || exec.is_empty() {
                return None;
            }
            Some(DesktopApp { name, exec, icon, subtitle, keywords })
        })
        .collect()
}

fn save_cache(apps: &[DesktopApp]) {
    let Some(p) = cache_path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Tabs/newlines never legitimately appear in these fields; sanitise anyway so
    // the TSV can't be corrupted by a malformed .desktop value.
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut buf = String::with_capacity(apps.len() * 64);
    for a in apps {
        buf.push_str(&clean(&a.name));
        buf.push('\t');
        buf.push_str(&clean(&a.exec));
        buf.push('\t');
        buf.push_str(&clean(&a.icon));
        buf.push('\t');
        buf.push_str(&clean(&a.subtitle));
        buf.push('\t');
        buf.push_str(&clean(&a.keywords));
        buf.push('\n');
    }
    let _ = std::fs::write(p, buf);
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
    fn refresh(&self) {
        // Only rescan when the app dirs changed since the cache was written
        // (a handful of stats), so this is cheap to call on every window show.
        let stale = match (cache_path().and_then(|p| std::fs::metadata(p).ok().and_then(|m| m.modified().ok())), dirs_mtime()) {
            (Some(cache_mt), Some(dir_mt)) => dir_mt > cache_mt,
            _ => true, // no cache yet, or can't stat → rescan
        };
        if stale {
            self.spawn_rescan();
        }
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query;
        let Ok(apps) = self.apps.read() else { return Vec::new() };
        let mut out = Vec::new();
        for a in apps.iter() {
            // Match the visible name first and treat GenericName/Keywords as a
            // weak last resort, so "si" finds Signal rather than every app whose
            // keyword blob happens to contain an s and an i.
            let Some(score) = crate::ranking::score(ctx.matcher, &a.name, &a.keywords, q) else {
                continue;
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
            .in_section(crate::registry::section::APPLICATIONS)
            .with_actions(vec![SecondaryAction {
                label: "Copy launch command".into(),
                action: Action::Copy(clean_exec(&a.exec)),
            }]);
            out.push(item);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_line_roundtrips() {
        // Simulate one TSV line and parse it back, including a value that
        // contains a stray tab (must be sanitised to a space on save).
        let a = DesktopApp {
            name: "Firefox".into(),
            exec: "firefox %u".into(),
            icon: "firefox".into(),
            subtitle: "Web\tBrowser".into(),
            keywords: "web browser".into(),
        };
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        let line = format!(
            "{}\t{}\t{}\t{}\t{}",
            clean(&a.name), clean(&a.exec), clean(&a.icon), clean(&a.subtitle), clean(&a.keywords)
        );
        let mut it = line.split('\t');
        assert_eq!(it.next().unwrap(), "Firefox");
        assert_eq!(it.next().unwrap(), "firefox %u");
        assert_eq!(it.next().unwrap(), "firefox");
        assert_eq!(it.next().unwrap(), "Web Browser"); // tab became space
        assert_eq!(it.next().unwrap(), "web browser");
        assert!(it.next().is_none());
    }

    #[test]
    fn two_letter_query_puts_the_prefix_match_first() {
        // Regression: "si" used to bury Signal under every app whose keyword
        // blob contained an s followed by an i.
        let names = ["Extension Manager", "Qt Assistant", "Signal", "Obsidian"];
        let m = crate::ranking::matcher();
        let mut scored: Vec<(&str, i64)> = names
            .iter()
            .filter_map(|n| crate::ranking::score(&m, n, "", "si").map(|s| (*n, s)))
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        assert_eq!(scored[0].0, "Signal");
    }

    #[test]
    fn parse_desktop_skips_nodisplay() {
        let hidden = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n";
        assert!(parse_desktop(hidden).is_none());
        let ok = "[Desktop Entry]\nType=Application\nName=X\nExec=x\n";
        assert!(parse_desktop(ok).is_some());
    }
}
