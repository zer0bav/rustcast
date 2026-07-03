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

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct CyberConfig {
    pub default_target: String,
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

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Config {
    pub ui: UiConfig,
    pub general: GeneralConfig,
    pub clipboard: ClipboardConfig,
    pub files: FilesConfig,
    pub cyber: CyberConfig,
    #[serde(rename = "quicklinks")]
    pub quicklinks: Vec<Quicklink>,
    #[serde(rename = "snippets")]
    pub snippets: Vec<Snippet>,
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
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        f.write_all(block.as_bytes())?;
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
