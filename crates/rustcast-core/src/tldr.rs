//! Generic tldr-pages search (Cheat tab).
//!
//! Downloads the official [tldr-pages](https://github.com/tldr-pages/tldr)
//! English archive on first use, caches it offline under
//! `~/.local/share/rustcast/tldr/`, and exposes every command *example* as its
//! own searchable, copyable row — so "tar extract" gives you the one-line
//! `tar xf …` command, not a whole man page.
//!
//! No new crate dependency: download/unzip shell out to `curl`/`wget` and
//! `bsdtar`/`unzip`, matching how the rest of the codebase reaches the system
//! (qalc, wl-copy, cliphist).

use crate::config::{which, Config};
use crate::model::{Action, Item, Prev, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

const TLDR_URL: &str =
    "https://github.com/tldr-pages/tldr/releases/latest/download/tldr-pages.en.zip";
/// Re-download when the cache is older than this many days.
const REFRESH_DAYS: u64 = 30;

const TLDR_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "copy command" },
    ActionHint { keys: "⌃K", label: "copy raw · run · open page" },
    ActionHint { keys: "type", label: "a command (tar, ssh, curl…)" },
    ActionHint { keys: "esc", label: "close" },
];

#[derive(Clone)]
pub struct TldrExample {
    pub desc: String,
    /// Command with `{{placeholders}}` preserved.
    pub cmd: String,
}

#[derive(Clone)]
pub struct TldrPage {
    pub name: String,
    pub platform: String,
    pub description: String,
    pub body: String,
    pub examples: Vec<TldrExample>,
}

pub struct TldrProvider {
    pages: Arc<RwLock<Vec<TldrPage>>>,
    downloading: Arc<AtomicBool>,
}

impl TldrProvider {
    /// Build the provider and, if pages already exist on disk, load them on a
    /// background thread. Never downloads here — first download is user-triggered.
    pub fn new() -> Self {
        let p = TldrProvider {
            pages: Arc::new(RwLock::new(Vec::new())),
            downloading: Arc::new(AtomicBool::new(false)),
        };
        if have_pages() {
            p.spawn_load();
        }
        p
    }

    fn spawn_load(&self) {
        let pages = self.pages.clone();
        std::thread::spawn(move || {
            let loaded = load_pages();
            if let Ok(mut w) = pages.write() {
                *w = loaded;
            }
        });
    }

    /// Download (or refresh) the archive on a background thread, then reload.
    fn spawn_download(&self) {
        if self.downloading.swap(true, Ordering::SeqCst) {
            return;
        }
        let pages = self.pages.clone();
        let flag = self.downloading.clone();
        std::thread::spawn(move || {
            let _ = download_archive();
            let loaded = load_pages();
            if let Ok(mut w) = pages.write() {
                *w = loaded;
            }
            flag.store(false, Ordering::SeqCst);
        });
    }

    fn is_downloading(&self) -> bool {
        self.downloading.load(Ordering::SeqCst)
    }
}

impl Default for TldrProvider {
    fn default() -> Self {
        TldrProvider::new()
    }
}

impl Provider for TldrProvider {
    fn id(&self) -> &'static str {
        "tldr"
    }
    fn tab(&self) -> Tab {
        Tab::Cheat
    }
    fn placeholder(&self) -> &'static str {
        "Search commands & cheatsheets… (tar extract, ssh tunnel…)"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        TLDR_HINTS
    }

    fn refresh(&self) {
        // If pages exist but are stale, refresh in the background. Never triggers
        // the first download (that stays user-initiated to avoid surprise traffic).
        if have_pages() && cache_age_days().map(|d| d >= REFRESH_DAYS).unwrap_or(false) {
            self.spawn_download();
        }
    }

    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        // Missing pages → offer to download (or show progress).
        let have_pages = have_pages();
        if !have_pages || self.pages.read().map(|p| p.is_empty()).unwrap_or(true) {
            if self.is_downloading() {
                return vec![Item::new(
                    "Downloading tldr pages…",
                    "fetching the archive — results appear automatically",
                    "content-loading",
                    "tldr",
                    5,
                    Action::None,
                )];
            }
            if !have_pages {
                if !(which("curl") || which("wget")) || !(which("bsdtar") || which("unzip")) {
                    return vec![Item::new(
                        "tldr needs curl + bsdtar/unzip",
                        "install them, then press Enter to retry",
                        "dialog-warning",
                        "tldr",
                        5,
                        Action::Refresh,
                    )];
                }
                return vec![Item::new(
                    "Download tldr pages (~5 MB, one time)",
                    "10k+ community command examples · cached offline",
                    "folder-download",
                    "tldr",
                    5,
                    Action::Refresh,
                )];
            }
        }

        let Ok(pages) = self.pages.read() else { return Vec::new() };
        let q = ctx.query.trim().to_lowercase();
        if q.is_empty() {
            return vec![Item::new(
                "Type a command to search examples",
                "e.g. tar, ssh, curl, git, docker…",
                "utilities-terminal",
                "tldr",
                1,
                Action::None,
            )];
        }

        // First token, for an exact page-name boost ("tar x" pins tar to top).
        let first_tok = q.split_whitespace().next().unwrap_or("");
        let mut out: Vec<Item> = Vec::new();

        for page in pages.iter() {
            let name_lc = page.name.to_lowercase();
            let name_score = ctx.matcher.fuzzy_match(&name_lc, &q);
            let exact_name = name_lc == first_tok;

            // A "page" row (whole cheatsheet) when the name itself matches.
            if let Some(ns) = name_score {
                out.push(
                    Item::new(
                        page.name.clone(),
                        page.description.clone(),
                        "accessories-dictionary",
                        "tldr-page",
                        ns + 500 + if exact_name { 800 } else { 0 },
                        Action::None,
                    )
                    .with_prev(Prev::Markdown(page.body.clone())),
                );
            }

            // Per-example rows.
            for ex in &page.examples {
                let hay = format!("{} {}", page.name, ex.desc).to_lowercase();
                let base = match ctx.matcher.fuzzy_match(&hay, &q) {
                    Some(s) => s,
                    None => continue,
                };
                let score = base + if exact_name { 800 } else { 0 };
                let plain = strip_placeholders(&ex.cmd);
                out.push(
                    Item::new(
                        ex.desc.clone(),
                        plain.clone(),
                        "utilities-terminal",
                        "tldr",
                        score,
                        Action::Copy(plain.clone()),
                    )
                    .with_prev(Prev::Markdown(page.body.clone()))
                    .with_actions(vec![
                        SecondaryAction {
                            label: "Copy with placeholders".into(),
                            action: Action::Copy(ex.cmd.clone()),
                        },
                        SecondaryAction {
                            label: "Run in terminal".into(),
                            action: Action::RunInTerminal(plain.clone()),
                        },
                        SecondaryAction {
                            label: "Open page in browser".into(),
                            action: Action::OpenUrl(format!(
                                "https://tldr.inbrowser.app/pages/common/{}",
                                page.name
                            )),
                        },
                    ]),
                );
            }
        }
        out
    }
}

/// `{{path/to/file}}` → `path/to/file` — a literal `{{…}}` is never what you
/// want pasted into a shell, so the primary copy strips the braces.
pub fn strip_placeholders(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // find the closing }}
            if let Some(end) = cmd[i + 2..].find("}}") {
                out.push_str(&cmd[i + 2..i + 2 + end]);
                i = i + 2 + end + 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Parse one tldr-pages markdown file into a [`TldrPage`].
pub fn parse_page(name: &str, platform: &str, src: &str) -> TldrPage {
    let mut description = String::new();
    let mut examples: Vec<TldrExample> = Vec::new();
    let mut pending_desc: Option<String> = None;

    for line in src.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("> ") {
            let rest = rest.trim();
            // Skip the "More information:" pointer line.
            if !rest.to_lowercase().starts_with("more information")
                && description.is_empty()
            {
                description = rest.to_string();
            }
        } else if let Some(rest) = t.strip_prefix("- ") {
            pending_desc = Some(rest.trim_end_matches(':').trim().to_string());
        } else if t.starts_with('`') && t.ends_with('`') && t.len() >= 2 {
            let cmd = t.trim_matches('`').trim().to_string();
            if let Some(desc) = pending_desc.take() {
                if !cmd.is_empty() {
                    examples.push(TldrExample { desc, cmd });
                }
            }
        }
    }

    TldrPage {
        name: name.to_string(),
        platform: platform.to_string(),
        description,
        body: src.to_string(),
        examples,
    }
}

fn tldr_dir() -> Option<std::path::PathBuf> {
    Config::data_dir().map(|d| d.join("tldr"))
}

/// The archive extracts platform dirs (`common/`, `linux/`, …) directly into the
/// tldr data dir, so "pages exist" means the `common` dir is present.
fn have_pages() -> bool {
    tldr_dir().map(|d| d.join("common").is_dir()).unwrap_or(false)
}

/// True while a download thread is active (marker file), for the GUI ticker.
pub fn downloading() -> bool {
    tldr_dir().map(|d| d.join(".downloading").exists()).unwrap_or(false)
}

fn cache_age_days() -> Option<u64> {
    let stamp = tldr_dir()?.join(".last-updated");
    let text = std::fs::read_to_string(stamp).ok()?;
    let then: u64 = text.trim().parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(now.saturating_sub(then) / 86_400)
}

/// Walk `common/` and `linux/`, parsing every `*.md` into a page.
fn load_pages() -> Vec<TldrPage> {
    let Some(base) = tldr_dir() else { return Vec::new() };
    let mut out = Vec::new();
    for platform in ["common", "linux"] {
        let dir = base.join(platform);
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            let Ok(src) = std::fs::read_to_string(&path) else { continue };
            let page = parse_page(&name, platform, &src);
            if !page.examples.is_empty() {
                out.push(page);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Download the archive to a temp file and extract it into the tldr data dir.
/// Best-effort: returns Err if a needed tool is missing or a step fails.
fn download_archive() -> anyhow::Result<()> {
    let dir = tldr_dir().ok_or_else(|| anyhow::anyhow!("no data dir"))?;
    std::fs::create_dir_all(&dir)?;
    let marker = dir.join(".downloading");
    let _ = std::fs::write(&marker, "");
    // Ensure the marker is cleared however we leave this function.
    let _guard = MarkerGuard(marker.clone());

    let zip = dir.join("tldr.zip");
    let zip_s = crate::action::shell_quote(&zip.to_string_lossy());
    let dir_s = crate::action::shell_quote(&dir.to_string_lossy());

    // Download: curl, else wget.
    let dl = if which("curl") {
        format!("curl -fsSL --max-time 120 -o {zip_s} {}", crate::action::shell_quote(TLDR_URL))
    } else if which("wget") {
        format!("wget -q -O {zip_s} {}", crate::action::shell_quote(TLDR_URL))
    } else {
        anyhow::bail!("no curl or wget");
    };
    run_blocking(&dl)?;

    // Extract into <dir> (archive contains a top-level `pages/…`).
    let ex = if which("bsdtar") {
        format!("bsdtar -xf {zip_s} -C {dir_s}")
    } else if which("unzip") {
        format!("unzip -oq {zip_s} -d {dir_s}")
    } else {
        anyhow::bail!("no bsdtar or unzip");
    };
    run_blocking(&ex)?;
    let _ = std::fs::remove_file(&zip);

    // Stamp the update time.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::write(dir.join(".last-updated"), now.to_string());
    Ok(())
}

/// Run a shell command and wait for it, erroring on non-zero exit.
fn run_blocking(cmd: &str) -> anyhow::Result<()> {
    let status = std::process::Command::new("sh").arg("-c").arg(cmd).status()?;
    if !status.success() {
        anyhow::bail!("command failed: {cmd}");
    }
    Ok(())
}

struct MarkerGuard(std::path::PathBuf);
impl Drop for MarkerGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAR: &str = "# tar\n\n> Archiving utility.\n> More information: <https://example.com>.\n\n- Extract an archive:\n\n`tar xf {{path/to/file.tar}}`\n\n- Create an archive:\n\n`tar cf {{target.tar}} {{file1}} {{file2}}`\n";

    #[test]
    fn parses_name_desc_and_examples() {
        let p = parse_page("tar", "common", TAR);
        assert_eq!(p.name, "tar");
        assert_eq!(p.description, "Archiving utility.");
        assert_eq!(p.examples.len(), 2);
        assert_eq!(p.examples[0].desc, "Extract an archive");
        assert_eq!(p.examples[0].cmd, "tar xf {{path/to/file.tar}}");
    }

    #[test]
    fn strips_placeholders() {
        assert_eq!(strip_placeholders("tar xf {{path/to/file.tar}}"), "tar xf path/to/file.tar");
        assert_eq!(
            strip_placeholders("cp {{src}} {{dst}}"),
            "cp src dst"
        );
        assert_eq!(strip_placeholders("ls -la"), "ls -la");
        // unbalanced braces are left as-is
        assert_eq!(strip_placeholders("echo {{oops"), "echo {{oops");
    }

    #[test]
    fn loads_real_pages_if_installed() {
        // Only meaningful when the archive has been downloaded to the data dir;
        // a no-op otherwise, so it's safe in CI.
        if !have_pages() {
            return;
        }
        let pages = load_pages();
        assert!(pages.len() > 500, "expected many pages, got {}", pages.len());
        let tar = pages.iter().find(|p| p.name == "tar").expect("tar page");
        assert!(!tar.examples.is_empty());
        assert!(tar.examples.iter().all(|e| !e.cmd.is_empty()));
    }

    #[test]
    fn search_ranks_extract_example() {
        let p = TldrProvider {
            pages: Arc::new(RwLock::new(vec![parse_page("tar", "common", TAR)])),
            downloading: Arc::new(AtomicBool::new(false)),
        };
        // build a ctx by hand; pages exist in memory but the tldr dir won't exist
        // in the test env, so query would short-circuit to the download row.
        // Exercise ranking directly on the in-memory page instead.
        let page = &p.pages.read().unwrap()[0];
        let m = crate::ranking::matcher();
        let extract = page
            .examples
            .iter()
            .find(|e| e.desc.contains("Extract"))
            .unwrap();
        let hay = format!("{} {}", page.name, extract.desc).to_lowercase();
        assert!(m.fuzzy_match(&hay, "tar extract").is_some());
    }
}
