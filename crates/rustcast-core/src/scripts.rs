//! Script-plugin runner (the Extensions tab). A plugin is a directory under
//! `~/.config/rustcast/plugins/<name>/` with a `manifest.toml` and an executable.
//! rustcast runs the executable with the query and parses `{ "items": [...] }`.

use crate::model::{Action, Item};
use crate::provider::{Provider, QueryCtx, Tab};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;

#[derive(Deserialize)]
struct Manifest {
    name: String,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    icon: String,
    exec: String,
}

struct Plugin {
    manifest: Manifest,
    dir: PathBuf,
}

#[derive(Deserialize)]
struct PluginOutput {
    #[serde(default)]
    items: Vec<PluginItem>,
}

#[derive(Deserialize)]
struct PluginItem {
    title: String,
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    action: PluginAction,
}

#[derive(Deserialize, Default)]
struct PluginAction {
    #[serde(default)]
    kind: String, // copy | open | shell | launch
    #[serde(default)]
    data: String,
}

pub struct ScriptProvider {
    plugins: Vec<Plugin>,
}

impl ScriptProvider {
    pub fn new() -> Self {
        let mut plugins = Vec::new();
        if let Some(dir) = plugins_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let pdir = e.path();
                    let manifest_path = pdir.join("manifest.toml");
                    if let Ok(text) = std::fs::read_to_string(&manifest_path) {
                        if let Ok(m) = toml::from_str::<Manifest>(&text) {
                            plugins.push(Plugin { manifest: m, dir: pdir });
                        }
                    }
                }
            }
        }
        ScriptProvider { plugins }
    }
}

impl Default for ScriptProvider {
    fn default() -> Self {
        ScriptProvider::new()
    }
}

fn plugins_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "zer0bav", "rustcast")
        .map(|d| d.config_dir().join("plugins"))
}

fn to_action(a: &PluginAction) -> Action {
    match a.kind.as_str() {
        "copy" => Action::Copy(a.data.clone()),
        "open" => Action::OpenUrl(a.data.clone()),
        "shell" => Action::RunShell(a.data.clone()),
        "launch" => Action::Launch(a.data.clone()),
        _ => Action::None,
    }
}

impl Provider for ScriptProvider {
    fn id(&self) -> &'static str {
        "scripts"
    }
    fn tab(&self) -> Tab {
        Tab::Extensions
    }
    fn placeholder(&self) -> &'static str {
        "Run an extension… (place plugins in ~/.config/rustcast/plugins)"
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let raw = ctx.query.trim();
        if self.plugins.is_empty() {
            return vec![Item::new(
                "No extensions installed",
                "press Enter to open the plugins folder",
                "application-x-addon",
                "ext",
                1,
                Action::RunShell("d=\"$HOME/.config/rustcast/plugins\"; mkdir -p \"$d\"; xdg-open \"$d\"".into()),
            )];
        }

        // Route: "<prefix> <args>" runs that plugin; empty query lists plugins.
        let (first, rest) = match raw.split_once(' ') {
            Some((a, b)) => (a, b.trim()),
            None => (raw, ""),
        };

        let mut out = Vec::new();
        for p in &self.plugins {
            let triggered = !raw.is_empty()
                && (p.manifest.prefix.eq_ignore_ascii_case(first)
                    || p.manifest.name.eq_ignore_ascii_case(first));
            if triggered {
                out.extend(run_plugin(p, rest));
            } else if raw.is_empty() || p.manifest.name.to_lowercase().contains(&raw.to_lowercase()) {
                let hint = if p.manifest.prefix.is_empty() {
                    format!("type '{} …'", p.manifest.name)
                } else {
                    format!("type '{} …'", p.manifest.prefix)
                };
                out.push(Item::new(
                    p.manifest.name.clone(),
                    hint,
                    if p.manifest.icon.is_empty() { "application-x-addon".into() } else { p.manifest.icon.clone() },
                    "ext",
                    100,
                    Action::None,
                ));
            }
        }
        out
    }
}

fn run_plugin(p: &Plugin, query: &str) -> Vec<Item> {
    let exec = if p.manifest.exec.starts_with('/') {
        p.manifest.exec.clone()
    } else {
        p.dir.join(&p.manifest.exec).to_string_lossy().into_owned()
    };
    let Ok(out) = Command::new(&exec).arg(query).current_dir(&p.dir).output() else {
        return vec![Item::new(
            format!("{}: failed to run", p.manifest.name),
            exec,
            "dialog-error",
            "ext",
            50,
            Action::None,
        )];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Ok(parsed) = serde_json::from_str::<PluginOutput>(&text) else {
        return vec![Item::new(
            format!("{}: bad output", p.manifest.name),
            "expected JSON { \"items\": [...] }",
            "dialog-error",
            "ext",
            50,
            Action::None,
        )];
    };
    parsed
        .items
        .into_iter()
        .enumerate()
        .map(|(i, it)| {
            Item::new(
                it.title,
                it.subtitle,
                if it.icon.is_empty() { "application-x-addon".into() } else { it.icon },
                "ext",
                1000 - i as i64,
                to_action(&it.action),
            )
        })
        .collect()
}
