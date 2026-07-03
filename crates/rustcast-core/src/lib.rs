//! rustcast-core — pure launcher logic with no GTK dependency.
//!
//! The GUI crate builds a [`registry::Registry`] of [`provider::Provider`]s and
//! renders whatever [`model::Item`]s they return. Everything here is unit-testable
//! headlessly, which is why the cyber toolkit and ranking live on this side.

pub mod action;
pub mod apps;
pub mod calc;
pub mod cheatsheets;
pub mod clip;
pub mod commands;
pub mod clipboard;
pub mod config;
pub mod cyber;
pub mod files;
pub mod gen;
pub mod manage;
pub mod model;
pub mod procs;
pub mod provider;
pub mod quicklinks;
pub mod ranking;
pub mod registry;
pub mod scripts;
pub mod settings;
pub mod snippets;
pub mod system;
pub mod windows;

use config::Config;
use registry::Registry;
use std::rc::Rc;

/// Build the default provider registry from a loaded config.
///
/// `clip_store` is the native clipboard history handle; when `None`, the legacy
/// cliphist-backed provider is used as a fallback.
pub fn default_registry(cfg: &Config, clip_store: Option<Rc<clipboard::store::Store>>) -> Registry {
    let mut reg = Registry::new();
    reg.register(Box::new(commands::CommandsProvider::new()));
    reg.register(Box::new(apps::AppsProvider::new()));
    reg.register(Box::new(calc::CalcProvider::new()));
    reg.register(Box::new(quicklinks::QuicklinksProvider::new(cfg.quicklinks.clone())));
    reg.register(Box::new(snippets::SnippetsProvider::new(cfg.snippets.clone())));
    reg.register(Box::new(system::SystemProvider::new()));
    if clip_store.is_some() {
        reg.register(Box::new(clipboard::ClipboardProvider::new(clip_store)));
    } else {
        reg.register(Box::new(clip::ClipProvider::new()));
    }
    if cfg.files.enabled {
        reg.register(Box::new(files::FilesProvider::new(
            cfg.files.roots.clone(),
            cfg.files.ignore.clone(),
        )));
    }
    reg.register(Box::new(procs::ProcessProvider::new()));
    reg.register(Box::new(windows::WindowsProvider::new()));
    reg.register(Box::new(cyber::CyberProvider::new()));
    reg.register(Box::new(procs::PortsProvider::new()));
    reg.register(Box::new(gen::GenProvider::new()));
    reg.register(Box::new(cheatsheets::CheatsheetProvider::new()));
    // SettingsProvider first among Extensions-tab providers so its placeholder
    // wins; the mode-only AddQuicklink provider is registered last.
    reg.register(Box::new(settings::SettingsProvider::new(cfg)));
    reg.register(Box::new(scripts::ScriptProvider::new()));
    reg.register(Box::new(manage::AddQuicklinkProvider::new()));
    reg
}
