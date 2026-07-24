//! User configuration loaded from `~/.config/rustcast/config.toml`.
//!
//! Every field has a default, so a missing or partial config still works. The
//! bundled `config.example.toml` documents the schema.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    pub width: i32,
    pub height: i32,
    pub animations: bool,
    /// Optional path to a custom stylesheet; empty = bundled theme.
    pub theme: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig { width: 900, height: 560, animations: true, theme: String::new() }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub default_tab: String,
    pub terminal: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        GeneralConfig { default_tab: "apps".into(), terminal: default_terminal() }
    }
}

fn default_terminal() -> String {
    for (bin, tmpl) in [
        ("kitty", "kitty -e"),
        ("alacritty", "alacritty -e"),
        ("foot", "foot"),
        ("wezterm", "wezterm start --"),
        ("xterm", "xterm -e"),
    ] {
        if which(bin) {
            return tmpl.into();
        }
    }
    "xterm -e".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ClipboardConfig {
    pub enabled: bool,
    pub max_entries: usize,
    pub max_image_mb: u64,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        ClipboardConfig { enabled: true, max_entries: 500, max_image_mb: 50 }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct FilesConfig {
    pub enabled: bool,
    pub roots: Vec<String>,
    pub ignore: Vec<String>,
}

impl Default for FilesConfig {
    fn default() -> Self {
        FilesConfig {
            enabled: true,
            roots: vec!["~".into()],
            ignore: vec![
                ".cache".into(),
                "node_modules".into(),
                ".git".into(),
                ".cargo".into(),
                "target".into(),
            ],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Quicklink {
    pub name: String,
    pub template: String,
    /// "url" (default, opens in browser) or "shell".
    #[serde(default = "quicklink_kind_url")]
    pub kind: String,
    #[serde(default)]
    pub icon: String,
}

fn quicklink_kind_url() -> String {
    "url".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snippet {
    pub keyword: String,
    pub text: String,
    #[serde(default)]
    pub name: String,
}

/// A short trigger word that launches an app, opens a URL, or runs a command —
/// e.g. `ff` → Firefox. Raycast-style aliases.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Alias {
    /// The trigger word typed in the box (e.g. "ff").
    pub keyword: String,
    /// What it does: an exec line, a URL, or a shell command (per `kind`).
    pub target: String,
    /// Display name; defaults to `target` when empty.
    #[serde(default)]
    pub name: String,
    /// "launch" (default), "url", or "shell".
    #[serde(default = "alias_kind_launch")]
    pub kind: String,
    #[serde(default)]
    pub icon: String,
}

fn alias_kind_launch() -> String {
    "launch".into()
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Config {
    pub ui: UiConfig,
    pub general: GeneralConfig,
    pub clipboard: ClipboardConfig,
    pub files: FilesConfig,
    // Skip when empty so `save()` never writes an inline `quicklinks = []`,
    // which would clash with the `[[quicklinks]]` blocks `append_quicklink` adds.
    #[serde(rename = "quicklinks", skip_serializing_if = "Vec::is_empty")]
    pub quicklinks: Vec<Quicklink>,
    #[serde(rename = "snippets", skip_serializing_if = "Vec::is_empty")]
    pub snippets: Vec<Snippet>,
    #[serde(rename = "aliases", skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<Alias>,
}

impl Config {
    /// `~/.config/rustcast/config.toml`
    pub fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("dev", "zer0bav", "rustcast")
            .map(|d| d.config_dir().join("config.toml"))
    }

    /// `~/.config/rustcast`
    pub fn config_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("dev", "zer0bav", "rustcast")
            .map(|d| d.config_dir().to_path_buf())
    }

    /// `~/.local/share/rustcast`
    pub fn data_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("dev", "zer0bav", "rustcast")
            .map(|d| d.data_dir().to_path_buf())
    }

    /// Custom stylesheet path, if the user dropped one in the config dir.
    pub fn user_css() -> Option<PathBuf> {
        directories::ProjectDirs::from("dev", "zer0bav", "rustcast")
            .map(|d| d.config_dir().join("style.css"))
            .filter(|p| p.exists())
    }

    /// Load config, falling back to defaults on any error.
    pub fn load() -> Config {
        let Some(path) = Config::config_path() else { return Config::default() };
        let Ok(text) = std::fs::read_to_string(&path) else { return Config::default() };
        match toml::from_str(&text) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("rustcast: config parse error ({e}); using defaults");
                Config::default()
            }
        }
    }

    /// Append a `[[quicklinks]]` block to the config file, preserving everything
    /// already there (comments included — we never rewrite the whole file).
    /// Creates the file if missing.
    pub fn append_quicklink(name: &str, template: &str, kind: &str) -> anyhow::Result<()> {
        let path = Config::config_path().ok_or_else(|| anyhow::anyhow!("no config dir"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let mut block = String::from("\n[[quicklinks]]\n");
        block.push_str(&format!("name = \"{}\"\n", esc(name)));
        block.push_str(&format!("template = \"{}\"\n", esc(template)));
        if kind != "url" {
            block.push_str(&format!("kind = \"{}\"\n", esc(kind)));
        }
        // Read existing content and strip a conflicting inline `quicklinks = []`
        // / `snippets = []` (left by an older `save()`), then append. Without
        // this, mixing inline arrays with `[[quicklinks]]` is invalid TOML.
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let mut out: String = existing
            .lines()
            .filter(|l| {
                let t = l.trim();
                t != "quicklinks = []" && t != "quicklinks = [ ]"
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&block);
        std::fs::write(&path, out)?;
        Ok(())
    }

    /// Set a single `key = value` under `[section]` in the config file, editing
    /// in place so comments and everything else are preserved. `value` is a TOML
    /// literal (`true`, `42`, or a quoted string). Creates the section/key/file
    /// if missing.
    pub fn set_value(section: &str, key: &str, value: &str) -> anyhow::Result<()> {
        let path = Config::config_path().ok_or_else(|| anyhow::anyhow!("no config dir"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let header = format!("[{section}]");
        let mut lines: Vec<String> =
            std::fs::read_to_string(&path).unwrap_or_default().lines().map(String::from).collect();

        let mut in_section = false;
        let mut done = false;
        let mut header_idx = None;
        for (idx, line) in lines.iter_mut().enumerate() {
            let t = line.trim();
            if t.starts_with('[') && t.ends_with(']') {
                in_section = t == header;
                if in_section {
                    header_idx = Some(idx);
                }
                continue;
            }
            if in_section && !done {
                if let Some((lhs, _)) = t.split_once('=') {
                    if lhs.trim() == key {
                        *line = format!("{key} = {value}");
                        done = true;
                    }
                }
            }
        }
        if !done {
            match header_idx {
                Some(i) => lines.insert(i + 1, format!("{key} = {value}")),
                None => {
                    if !lines.is_empty() {
                        lines.push(String::new());
                    }
                    lines.push(header);
                    lines.push(format!("{key} = {value}"));
                }
            }
        }
        std::fs::write(&path, lines.join("\n") + "\n")?;
        Ok(())
    }

    /// Persist the config back to disk (used by the settings view).
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Config::config_path().ok_or_else(|| anyhow::anyhow!("no config dir"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text)?;
        Ok(())
    }
}

/// Is a binary on PATH?
pub fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file())
        })
        .unwrap_or(false)
}

/// Expand a leading `~` to the home directory.
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if p == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(p)
}
