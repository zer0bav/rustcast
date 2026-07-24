//! File search provider. A background thread indexes the configured roots (via
//! the `ignore` crate, gitignore-aware); the index is cached to disk for instant
//! warm starts. Queries fuzzy-match file names.

use crate::config::expand_tilde;
use crate::model::{Action, Item, Prev, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

/// Minimum gap between background re-walks (daemon mode calls `refresh` on every
/// window show; walking the whole home dir every time would be wasteful).
const REWALK_THROTTLE: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Clone)]
pub struct FileEntry {
    pub path: String,
    pub name_lc: String,
    pub is_dir: bool,
    pub size: u64,
    pub depth: usize,
}

const FILE_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "open" },
    ActionHint { keys: "drag", label: "drag out to copy the file" },
    ActionHint { keys: "⌃K", label: "reveal · copy path" },
    ActionHint { keys: "esc", label: "close" },
];

pub struct FilesProvider {
    index: Arc<RwLock<Vec<FileEntry>>>,
    roots: Vec<String>,
    ignores: Vec<String>,
    walking: Arc<AtomicBool>,
    last_walk: Mutex<Option<Instant>>,
}

impl FilesProvider {
    /// Build the provider, loading any cached index immediately and kicking off
    /// a fresh background walk.
    pub fn new(roots: Vec<String>, ignores: Vec<String>) -> Self {
        let p = FilesProvider {
            index: Arc::new(RwLock::new(load_cache())),
            roots,
            ignores,
            walking: Arc::new(AtomicBool::new(false)),
            last_walk: Mutex::new(None),
        };
        p.spawn_walk();
        p
    }

    /// Re-walk the roots on a background thread, unless one is already running.
    fn spawn_walk(&self) {
        if self.walking.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Ok(mut lw) = self.last_walk.lock() {
            *lw = Some(Instant::now());
        }
        let idx = self.index.clone();
        let roots = self.roots.clone();
        let ignores = self.ignores.clone();
        let flag = self.walking.clone();
        std::thread::spawn(move || {
            let entries = walk(&roots, &ignores);
            save_cache(&entries);
            if let Ok(mut w) = idx.write() {
                *w = entries;
            }
            flag.store(false, Ordering::SeqCst);
        });
    }
}

impl Provider for FilesProvider {
    fn id(&self) -> &'static str {
        "files"
    }
    fn tab(&self) -> Tab {
        Tab::Files
    }
    fn placeholder(&self) -> &'static str {
        "Search files and folders…"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        FILE_HINTS
    }
    fn refresh(&self) {
        // Throttle: at most one re-walk per REWALK_THROTTLE, so repeated window
        // shows don't re-crawl the home directory.
        let due = self
            .last_walk
            .lock()
            .ok()
            .map(|lw| lw.map(|t| t.elapsed() >= REWALK_THROTTLE).unwrap_or(true))
            .unwrap_or(false);
        if due {
            self.spawn_walk();
        }
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let q = ctx.query.trim();
        if q.is_empty() {
            return vec![Item::new(
                "Type to search your files",
                "indexed in the background · gitignore-aware",
                "system-file-manager",
                "files",
                1,
                Action::None,
            )];
        }
        let Ok(index) = self.index.read() else { return Vec::new() };
        if index.is_empty() {
            return vec![Item::new(
                "Indexing files…",
                "first run builds the index — try again in a moment",
                "content-loading",
                "files",
                1,
                Action::None,
            )];
        }
        let ql = q.to_lowercase();
        let scored = search(&index, ctx.matcher, &ql);

        scored
            .into_iter()
            .map(|(s, e)| {
                let name = std::path::Path::new(&e.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| e.path.clone());
                let meta = format!(
                    "{} · {}",
                    if e.is_dir { "Folder".into() } else { crate::clipboard::store::human_size(e.size as i64) },
                    e.path,
                );
                Item::new(
                    name,
                    e.path.clone(),
                    if e.is_dir { "folder" } else { "text-x-generic" },
                    "file",
                    s,
                    Action::OpenFile(e.path.clone()),
                )
                .in_section(crate::registry::section::FILES)
                .with_prev(Prev::File { path: e.path.clone(), meta, head: None })
                .with_actions(vec![
                    SecondaryAction { label: "Reveal in file manager".into(), action: Action::RevealFile(e.path.clone()) },
                    SecondaryAction { label: "Copy path".into(), action: Action::Copy(e.path.clone()) },
                    SecondaryAction { label: "Copy as file URI".into(), action: Action::Copy(format!("file://{}", e.path)) },
                ])
            })
            .collect()
    }
}

/// Score an index against a lowercased query. A cheap subsequence prefilter
/// skips the expensive fuzzy scorer for most entries, and a hard evaluation
/// budget bounds worst-case cost (e.g. a one-char query matching everything),
/// so typing stays responsive even over a very large index.
fn search<'a>(
    index: &'a [FileEntry],
    matcher: &fuzzy_matcher::skim::SkimMatcherV2,
    ql: &str,
) -> Vec<(i64, &'a FileEntry)> {
    const FUZZY_BUDGET: usize = 4000;
    let mut budget = FUZZY_BUDGET;
    let mut scored: Vec<(i64, &FileEntry)> = Vec::new();
    for e in index.iter() {
        if !is_subsequence(&e.name_lc, ql) {
            continue;
        }
        if let Some(mut s) = matcher.fuzzy_match(&e.name_lc, ql) {
            s -= e.depth as i64 * 2; // prefer shallower paths
            scored.push((s, e));
        }
        budget -= 1;
        if budget == 0 {
            break;
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.truncate(60);
    scored
}

/// Is `needle` a subsequence of `hay` (both lowercase)? Cheap O(n) check.
fn is_subsequence(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut chars = needle.chars();
    let mut cur = chars.next();
    for h in hay.chars() {
        if let Some(c) = cur {
            if h == c {
                cur = chars.next();
                if cur.is_none() {
                    return true;
                }
            }
        }
    }
    cur.is_none()
}

fn walk(roots: &[String], ignores: &[String]) -> Vec<FileEntry> {
    let mut out = Vec::new();
    for root in roots {
        let base = expand_tilde(root);
        let mut builder = ignore::WalkBuilder::new(&base);
        // hidden(true) skips dotfiles/dotdirs (.cache, .local, .git, .mozilla…),
        // which is both what users expect from a file finder and a huge speedup.
        builder.hidden(true).git_ignore(true).follow_links(false);
        let ignores = ignores.to_vec();
        builder.filter_entry(move |e| {
            let name = e.file_name().to_string_lossy();
            !ignores.iter().any(|ig| name.as_ref() == ig.as_str())
        });
        for entry in builder.build().flatten() {
            let depth = entry.depth();
            let path = entry.path();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name_lc = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if name_lc.is_empty() {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(FileEntry {
                path: path.to_string_lossy().into_owned(),
                name_lc,
                is_dir,
                size,
                depth,
            });
            if out.len() >= 200_000 {
                return out;
            }
        }
    }
    out
}

fn cache_path() -> Option<std::path::PathBuf> {
    crate::config::Config::data_dir().map(|d| d.join("files-index.tsv"))
}

fn load_cache() -> Vec<FileEntry> {
    let Some(p) = cache_path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(p) else { return Vec::new() };
    text.lines()
        .filter_map(|line| {
            let mut it = line.split('\t');
            let path = it.next()?.to_string();
            let is_dir = it.next()? == "1";
            let size: u64 = it.next()?.parse().ok()?;
            let depth: usize = it.next().unwrap_or("0").parse().unwrap_or(0);
            let name_lc = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            Some(FileEntry { path, name_lc, is_dir, size, depth })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_index(n: usize) -> Vec<FileEntry> {
        let words = ["cargo", "main", "readme", "config", "index", "notes", "photo", "report"];
        (0..n)
            .map(|i| {
                let name = format!("{}_{}.txt", words[i % words.len()], i);
                FileEntry {
                    path: format!("/home/u/{name}"),
                    name_lc: name.to_lowercase(),
                    is_dir: false,
                    size: 100,
                    depth: 3,
                }
            })
            .collect()
    }

    #[test]
    fn subsequence_basics() {
        assert!(is_subsequence("cargo.toml", "cgo"));
        assert!(is_subsequence("cargo.toml", ""));
        assert!(!is_subsequence("cargo.toml", "xyz"));
    }

    #[test]
    fn search_is_fast_on_large_index() {
        let idx = fake_index(200_000);
        let m = crate::ranking::matcher();
        // worst case: single-char query matches almost everything
        let start = std::time::Instant::now();
        let r = search(&idx, &m, "a");
        let elapsed = start.elapsed();
        assert!(r.len() <= 60);
        // budget-bounded — must stay well under a frame even on 200k entries
        assert!(elapsed.as_millis() < 200, "query took {elapsed:?}");
    }
}

fn save_cache(entries: &[FileEntry]) {
    let Some(p) = cache_path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut buf = String::with_capacity(entries.len() * 48);
    for e in entries {
        buf.push_str(&e.path);
        buf.push('\t');
        buf.push(if e.is_dir { '1' } else { '0' });
        buf.push('\t');
        buf.push_str(&e.size.to_string());
        buf.push('\t');
        buf.push_str(&e.depth.to_string());
        buf.push('\n');
    }
    let _ = std::fs::write(p, buf);
}
