//! rustcast GUI — GTK4 + layer-shell launcher.
//!
//! Thin GTK shell over `rustcast-core`: it builds a provider registry, renders
//! the items a query returns, and dispatches actions. All matching/tool logic
//! lives in core.

use gtk4::gdk::{Display, Key, ModifierType};
use gtk4::glib::{self, Propagation};
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, Entry,
    EventControllerKey, Image, Label, ListBox, ListBoxRow, Orientation, Popover, ScrolledWindow,
    SelectionMode, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};

use rustcast_core::action::{do_action, Env};
use rustcast_core::clipboard::store::Store;
use rustcast_core::config::Config;
use rustcast_core::model::{Action, Item, Prev};
use rustcast_core::provider::Tab;
use rustcast_core::ranking;
use rustcast_core::registry::Registry;

use std::cell::{Cell, RefCell};
use std::io::Read;
use std::rc::Rc;

const APP_ID: &str = "dev.zer0bav.rustcast";

/// Shared, mutable app state passed into the GTK closures.
struct State {
    registry: RefCell<Registry>,
    matcher: fuzzy_matcher::skim::SkimMatcherV2,
    items: RefCell<Vec<Item>>,
    active_tab: Cell<Tab>,
    target: RefCell<Option<String>>,
    env: Env,
    clip_store: Option<Rc<Store>>,
    /// Active command mode: (provider id, display label). `None` = normal tab.
    mode: RefCell<Option<(String, String)>>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Headless clipboard subcommands run before any GTK init.
    match args.get(1).map(|s| s.as_str()) {
        Some("--clip-ingest") => {
            let mut buf = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut buf);
            let max = Config::load().clipboard.max_entries;
            let _ = rustcast_core::clipboard::ingest(&buf, max);
            return;
        }
        Some("--clip-daemon") => {
            run_clip_daemon();
            return;
        }
        _ => {}
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("rustcast {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!(
            "rustcast — a Raycast-class launcher for Linux\n\n\
             USAGE:\n  rustcast [--tab <name>] [--query <text>]\n\n\
             FLAGS:\n\
             \x20 --tab <apps|clipboard|files|cyber|cheat|extensions>  open on a tab\n\
             \x20 --query <text>   pre-fill the search box\n\
             \x20 --version        print version\n\
             \x20 --help           show this help\n\n\
             Bind these to a global hotkey in your compositor/desktop settings.\n"
        );
        return;
    }

    // Optional `--tab <name>` to open straight into a tab (e.g. Super+V → clipboard).
    // Falls back to the legacy `RUSTCAST_MODE` env var for backwards compatibility.
    let mut start_tab: Option<String> = args
        .iter()
        .position(|a| a == "--tab")
        .and_then(|i| args.get(i + 1).cloned());
    // Optional `--query <text>` to pre-fill the search.
    let mut start_query: Option<String> = args
        .iter()
        .position(|a| a == "--query")
        .and_then(|i| args.get(i + 1).cloned());

    if start_tab.is_none() {
        match std::env::var("RUSTCAST_MODE").as_deref() {
            Ok("clipboard") | Ok("clip") | Ok("cb") => start_tab = Some("clipboard".into()),
            Ok("files") | Ok("file") => start_tab = Some("files".into()),
            Ok("cyber") => start_tab = Some("cyber".into()),
            Ok("calc") => {
                start_tab = Some("apps".into());
                if start_query.is_none() {
                    start_query = Some("=".into());
                }
            }
            _ => {}
        }
    }

    // Toggle behaviour without a fragile `pkill -f` in the keybind (which would
    // match — and kill — its own launching shell): close any other GUI instance
    // from inside the process, where we can exclude our own pid and the daemon.
    kill_other_guis();

    // NON_UNIQUE: every invocation is its own process, so a running instance
    // never swallows a new launch's --tab/--query args.
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    // GTK would treat our flags as its own args; hand it nothing.
    app.connect_activate(move |a| build_ui(a, start_tab.clone(), start_query.clone()));
    app.run_with_args::<String>(&[]);
}

/// Terminate any other running rustcast **GUI** instance (so pressing the
/// keybind again re-opens fresh), while leaving the clipboard daemon and our own
/// process alone.
fn kill_other_guis() {
    let Ok(self_exe) = std::env::current_exe() else { return };
    let self_pid = std::process::id();
    let Ok(entries) = std::fs::read_dir("/proc") else { return };
    for e in entries.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        if pid == self_pid {
            continue;
        }
        // only processes running our exact binary
        match std::fs::read_link(format!("/proc/{pid}/exe")) {
            Ok(exe) if exe == self_exe => {}
            _ => continue,
        }
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let cmdline = String::from_utf8_lossy(&cmdline);
        // leave the background daemon / ingest helpers running
        if cmdline.contains("--clip-daemon") || cmdline.contains("--clip-ingest") {
            continue;
        }
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
    }
}

/// Spawn the two `wl-paste --watch` processes that feed the clipboard store.
/// Guarded by a lockfile so only one daemon runs.
fn run_clip_daemon() {
    use std::process::Command;
    let Some(dir) = Config::data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let lock = dir.join("clip-daemon.lock");
    // crude single-instance guard: bail if a live pid is recorded
    if let Ok(pid) = std::fs::read_to_string(&lock) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                return;
            }
        }
    }
    let _ = std::fs::write(&lock, std::process::id().to_string());

    let self_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "rustcast".into());

    // one watcher for regular (text) content, one for images
    let mut children = Vec::new();
    for typ in ["", "image/png"] {
        let mut c = Command::new("wl-paste");
        if !typ.is_empty() {
            c.args(["--type", typ]);
        }
        c.arg("--watch").arg(&self_bin).arg("--clip-ingest");
        if let Ok(child) = c.spawn() {
            children.push(child);
        }
    }
    for mut c in children {
        let _ = c.wait();
    }
}

/// Spawn the clipboard daemon detached if it isn't already running.
fn ensure_clip_daemon() {
    use std::process::Command;
    // Skip if wl-paste is missing — nothing to watch with.
    if !rustcast_core::config::which("wl-paste") {
        return;
    }
    if let Ok(bin) = std::env::current_exe() {
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!("setsid -f {} --clip-daemon >/dev/null 2>&1", bin.to_string_lossy()))
            .spawn();
    }
}

fn tab_from_config(s: &str) -> Tab {
    match s.to_lowercase().as_str() {
        "clipboard" | "clip" => Tab::Clipboard,
        "files" | "file" => Tab::Files,
        "cyber" => Tab::Cyber,
        "cheat" | "cheats" | "cheatsheets" => Tab::Cheat,
        "win" | "windows" | "window" => Tab::Win,
        "extensions" | "ext" => Tab::Extensions,
        _ => Tab::Apps,
    }
}

fn build_ui(app: &Application, start_tab: Option<String>, start_query: Option<String>) {
    let cfg = Config::load();
    let initial_tab = tab_from_config(start_tab.as_deref().unwrap_or(&cfg.general.default_tab));

    // Native clipboard store + background watcher daemon.
    let clip_store = if cfg.clipboard.enabled {
        Store::open().ok().map(Rc::new)
    } else {
        None
    };
    if let Some(store) = &clip_store {
        let _ = store.prune(cfg.clipboard.max_entries);
        ensure_clip_daemon();
    }

    let state = Rc::new(State {
        registry: RefCell::new(rustcast_core::default_registry(&cfg, clip_store.clone())),
        matcher: ranking::matcher(),
        items: RefCell::new(Vec::new()),
        active_tab: Cell::new(initial_tab),
        target: RefCell::new(if cfg.cyber.default_target.is_empty() {
            None
        } else {
            Some(cfg.cyber.default_target.clone())
        }),
        env: Env { terminal: cfg.general.terminal.clone() },
        clip_store,
        mode: RefCell::new(None),
    });

    let window = ApplicationWindow::builder()
        .application(app)
        .resizable(false)
        .default_width(cfg.ui.width)
        .default_height(cfg.ui.height)
        .build();
    // Prefer a wlr-layer-shell overlay (Hyprland, Sway, river, wayfire…). On
    // GNOME (Mutter), KDE (KWin) or X11 that protocol is absent, so fall back to
    // a normal undecorated window that closes when it loses focus.
    let layer = gtk4_layer_shell::is_supported();
    if layer {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("rustcast"));
        window.set_keyboard_mode(KeyboardMode::Exclusive);
    } else {
        window.set_decorated(false);
        // close when the launcher loses focus (once it has been focused)
        let seen_active = std::cell::Cell::new(false);
        let win_close = window.clone();
        window.connect_is_active_notify(move |w| {
            if w.is_active() {
                seen_active.set(true);
            } else if seen_active.get() {
                win_close.close();
            }
        });
    }

    load_css(&cfg);

    // ── layout ──────────────────────────────────────────────
    let root = GtkBox::new(Orientation::Vertical, 0);
    root.add_css_class("box-wrapper");

    // tab bar
    let tab_bar = GtkBox::new(Orientation::Horizontal, 6);
    tab_bar.add_css_class("tab-bar");
    let tab_buttons: Rc<Vec<Button>> = Rc::new(
        Tab::ALL
            .iter()
            .map(|t| {
                let b = Button::with_label(t.label());
                b.add_css_class("tab");
                b.set_focusable(false);
                tab_bar.append(&b);
                b
            })
            .collect(),
    );

    let entry = Entry::builder()
        .placeholder_text(state.registry.borrow().placeholder_for(initial_tab))
        .build();
    entry.add_css_class("input");

    let content = GtkBox::new(Orientation::Horizontal, 12);
    content.set_vexpand(true);

    let list = ListBox::new();
    list.add_css_class("list");
    list.set_selection_mode(SelectionMode::Single);
    let scroll = ScrolledWindow::builder().child(&list).hexpand(true).vexpand(true).build();
    scroll.add_css_class("list-scroll");

    let preview = GtkBox::new(Orientation::Vertical, 8);
    preview.add_css_class("preview");
    preview.set_size_request(360, -1);
    preview.set_hexpand(false);
    let prev_img = Image::new();
    prev_img.set_pixel_size(320);
    prev_img.set_vexpand(true);
    let prev_lbl = Label::builder()
        .wrap(true)
        .wrap_mode(gtk4::pango::WrapMode::WordChar)
        .xalign(0.0)
        .yalign(0.0)
        .selectable(true)
        .build();
    prev_lbl.add_css_class("preview-text");
    // A no-horizontal-scroll window of fixed width forces the label to wrap to
    // that width, so long lines / URLs can never widen the whole launcher.
    let text_scroll = ScrolledWindow::builder()
        .child(&prev_lbl)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .build();
    text_scroll.set_size_request(336, -1);
    let prev_meta = Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk4::pango::WrapMode::WordChar)
        .max_width_chars(46)
        .build();
    prev_meta.add_css_class("preview-meta");
    preview.append(&prev_img);
    preview.append(&text_scroll);
    preview.append(&prev_meta);

    content.append(&scroll);
    content.append(&preview);

    let footer = GtkBox::new(Orientation::Horizontal, 14);
    footer.add_css_class("footer");

    root.append(&tab_bar);
    root.append(&entry);
    root.append(&content);
    root.append(&footer);
    root.set_size_request(cfg.ui.width, cfg.ui.height);
    window.set_child(Some(&root));

    // ── preview updater ─────────────────────────────────────
    let update_preview = {
        let state = state.clone();
        let prev_img = prev_img.clone();
        let prev_lbl = prev_lbl.clone();
        let prev_meta = prev_meta.clone();
        move |idx: usize| {
            let items = state.items.borrow();
            let Some(it) = items.get(idx) else {
                prev_lbl.set_text("");
                prev_meta.set_text("");
                prev_img.set_pixel_size(0);
                prev_img.clear();
                return;
            };
            prev_meta.set_text("");
            match &it.prev {
                Prev::None => {
                    prev_lbl.set_text("");
                    prev_img.set_pixel_size(120);
                    if it.icon.starts_with('/') {
                        prev_img.set_from_file(Some(&it.icon));
                    } else {
                        prev_img.set_icon_name(Some(&it.icon));
                    }
                }
                Prev::Text(t) => {
                    prev_img.set_pixel_size(0);
                    prev_img.clear();
                    prev_lbl.set_text(t);
                }
                Prev::Markdown(t) => {
                    prev_img.set_pixel_size(0);
                    prev_img.clear();
                    prev_lbl.set_markup(&md_to_pango(t));
                }
                Prev::ClipImage(line) => {
                    prev_lbl.set_text("");
                    prev_img.set_pixel_size(320);
                    if let Some(p) = clip_image_temp(line) {
                        prev_img.set_from_file(Some(&p));
                    }
                }
                Prev::ImagePath(p) => {
                    prev_lbl.set_text("");
                    prev_img.set_pixel_size(320);
                    prev_img.set_from_file(Some(p));
                    prev_meta.set_text(&it.subtitle);
                }
                Prev::File { path, meta, head } => {
                    // content on top, metadata pinned at the bottom (Raycast style)
                    prev_meta.set_text(meta);
                    if let Some(h) = head {
                        prev_img.set_pixel_size(0);
                        prev_img.clear();
                        prev_lbl.set_text(h);
                    } else if !path.is_empty() && is_image_path(path) {
                        prev_lbl.set_text("");
                        prev_img.set_pixel_size(300);
                        prev_img.set_from_file(Some(path));
                    } else if !path.is_empty() {
                        prev_img.set_pixel_size(0);
                        prev_img.clear();
                        prev_lbl.set_text(&read_file_head(path));
                    } else {
                        prev_img.set_pixel_size(0);
                        prev_img.clear();
                        prev_lbl.set_text("");
                    }
                }
            }
        }
    };

    // ── footer builder ──────────────────────────────────────
    let rebuild_footer = {
        let footer = footer.clone();
        let state = state.clone();
        move || {
            while let Some(c) = footer.first_child() {
                footer.remove(&c);
            }
            let hints = state
                .registry
                .borrow()
                .footer_hints_for(state.active_tab.get());
            for h in hints {
                let chip = GtkBox::new(Orientation::Horizontal, 5);
                chip.add_css_class("chip");
                let k = Label::new(Some(h.keys));
                k.add_css_class("chip-key");
                let l = Label::new(Some(h.label));
                l.add_css_class("chip-label");
                chip.append(&k);
                chip.append(&l);
                footer.append(&chip);
            }
            // restart the slide-in animation
            footer.remove_css_class("slidein");
            let f = footer.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(10), move || {
                f.add_css_class("slidein");
            });
        }
    };

    // ── rebuild (query -> items -> rows) ────────────────────
    let rebuild = {
        let state = state.clone();
        let list = list.clone();
        let update_preview = update_preview.clone();
        move |query: &str| {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let tab = state.active_tab.get();
            let target = state.target.borrow();
            let mode = state.mode.borrow();
            let mode_id = mode.as_ref().map(|(id, _)| id.as_str());
            let collected =
                state.registry.borrow().route(query, tab, &state.matcher, target.as_deref(), mode_id);
            drop(mode);
            drop(target);

            for it in &collected {
                list.append(&build_row(it));
            }
            *state.items.borrow_mut() = collected;
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
                update_preview(0);
            } else {
                update_preview(usize::MAX);
            }
        }
    };
    let rebuild: Rc<dyn Fn(&str)> = Rc::new(rebuild);

    // ── tab switching ───────────────────────────────────────
    let switch_tab = {
        let state = state.clone();
        let entry = entry.clone();
        let tab_buttons = tab_buttons.clone();
        let rebuild = rebuild.clone();
        let rebuild_footer = rebuild_footer.clone();
        move |tab: Tab| {
            state.active_tab.set(tab);
            // Switching tabs always leaves any active command mode.
            *state.mode.borrow_mut() = None;
            entry.remove_css_class("mode-active");
            for (i, b) in tab_buttons.iter().enumerate() {
                if Tab::from_index(i) == Some(tab) {
                    b.add_css_class("tab-active");
                } else {
                    b.remove_css_class("tab-active");
                }
            }
            entry.set_placeholder_text(Some(state.registry.borrow().placeholder_for(tab)));
            rebuild(&entry.text());
            rebuild_footer();
        }
    };

    for (i, b) in tab_buttons.iter().enumerate() {
        let switch_tab = switch_tab.clone();
        b.connect_clicked(move |_| {
            if let Some(t) = Tab::from_index(i) {
                switch_tab(t);
            }
        });
    }

    // ── entry changes ───────────────────────────────────────
    {
        let rebuild = rebuild.clone();
        entry.connect_changed(move |e| rebuild(&e.text()));
    }

    // ── selection -> preview ────────────────────────────────
    {
        let update_preview = update_preview.clone();
        list.connect_row_selected(move |_, row| {
            if let Some(r) = row {
                let i = r.index();
                if i >= 0 {
                    update_preview(i as usize);
                }
            }
        });
    }

    // ── keyboard ────────────────────────────────────────────
    {
        let state = state.clone();
        let list = list.clone();
        let scroll = scroll.clone();
        let entry = entry.clone();
        let win = window.clone();
        let switch_tab = switch_tab.clone();
        let rebuild = rebuild.clone();
        let key = EventControllerKey::new();
        key.set_propagation_phase(gtk4::PropagationPhase::Capture);
        key.connect_key_pressed(move |_, keyval, _, mods| {
            let ctrl = mods.contains(ModifierType::CONTROL_MASK);
            let shift = mods.contains(ModifierType::SHIFT_MASK);
            match keyval {
                Key::Escape => {
                    let in_mode = state.mode.borrow().is_some();
                    if in_mode && entry.text().is_empty() {
                        // Back out of the command mode to the normal tab.
                        *state.mode.borrow_mut() = None;
                        entry.remove_css_class("mode-active");
                        entry.set_placeholder_text(Some(
                            state.registry.borrow().placeholder_for(state.active_tab.get()),
                        ));
                        rebuild("");
                    } else if !entry.text().is_empty() {
                        entry.set_text("");
                    } else {
                        win.close();
                    }
                    Propagation::Stop
                }
                // tab cycling
                Key::Tab if !ctrl => {
                    let cur = state.active_tab.get().index();
                    let next = (cur + 1) % Tab::ALL.len();
                    switch_tab(Tab::from_index(next).unwrap());
                    Propagation::Stop
                }
                Key::ISO_Left_Tab | Key::Tab if shift => {
                    let cur = state.active_tab.get().index();
                    let prev = (cur + Tab::ALL.len() - 1) % Tab::ALL.len();
                    switch_tab(Tab::from_index(prev).unwrap());
                    Propagation::Stop
                }
                // direct tab jump: Ctrl+1..7
                Key::_1 | Key::_2 | Key::_3 | Key::_4 | Key::_5 | Key::_6 | Key::_7 if ctrl => {
                    let n = match keyval {
                        Key::_1 => 0,
                        Key::_2 => 1,
                        Key::_3 => 2,
                        Key::_4 => 3,
                        Key::_5 => 4,
                        Key::_6 => 5,
                        _ => 6,
                    };
                    if let Some(t) = Tab::from_index(n) {
                        switch_tab(t);
                    }
                    Propagation::Stop
                }
                // navigation
                Key::Down => {
                    move_sel(&list, &scroll, 1);
                    Propagation::Stop
                }
                Key::Up => {
                    move_sel(&list, &scroll, -1);
                    Propagation::Stop
                }
                Key::n if ctrl => {
                    move_sel(&list, &scroll, 1);
                    Propagation::Stop
                }
                Key::p if ctrl => {
                    move_sel(&list, &scroll, -1);
                    Propagation::Stop
                }
                // actions menu
                Key::k if ctrl => {
                    open_actions_menu(&state, &list, &entry, &win, &rebuild);
                    Propagation::Stop
                }
                // clear
                Key::u if ctrl => {
                    entry.set_text("");
                    Propagation::Stop
                }
                Key::Return | Key::KP_Enter => {
                    activate_selected(&state, &list, &entry, &win, &rebuild);
                    Propagation::Stop
                }
                _ => Propagation::Proceed,
            }
        });
        window.add_controller(key);
    }

    // Live clipboard refresh: while the Clipboard tab is open, pick up newly
    // copied entries without needing a keystroke.
    if state.clip_store.is_some() {
        let state = state.clone();
        let entry_r = entry.clone();
        let rebuild_r = rebuild.clone();
        let last_id = std::cell::Cell::new(
            state.clip_store.as_ref().and_then(|s| s.recent(1).first().map(|r| r.id)).unwrap_or(-1),
        );
        glib::timeout_add_local(std::time::Duration::from_millis(700), move || {
            if state.active_tab.get() == Tab::Clipboard {
                if let Some(store) = &state.clip_store {
                    let newest = store.recent(1).first().map(|r| r.id).unwrap_or(-1);
                    if newest != last_id.get() {
                        last_id.set(newest);
                        rebuild_r(&entry_r.text());
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    // initial paint
    switch_tab(initial_tab);
    if let Some(q) = start_query {
        entry.set_text(&q);
        entry.set_position(-1);
    }
    entry.grab_focus();
    window.present();
}

/// Run the selected item's primary action.
fn activate_selected(
    state: &Rc<State>,
    list: &ListBox,
    entry: &Entry,
    win: &ApplicationWindow,
    rebuild: &Rc<dyn Fn(&str)>,
) {
    let Some(row) = list.selected_row() else { return };
    let i = row.index();
    if i < 0 {
        return;
    }
    let action = {
        let items = state.items.borrow();
        items.get(i as usize).map(|it| it.action.clone())
    };
    let Some(action) = action else { return };
    apply_action(state, &action, entry, win, rebuild);
}

/// Apply an action: SetTarget updates state and re-queries; everything else
/// dispatches through core and closes the window when appropriate.
fn apply_action(
    state: &Rc<State>,
    action: &Action,
    entry: &Entry,
    win: &ApplicationWindow,
    rebuild: &Rc<dyn Fn(&str)>,
) {
    match action {
        Action::SetTarget(t) => {
            *state.target.borrow_mut() = Some(t.clone());
            entry.set_text("");
            rebuild("");
            return;
        }
        Action::SetQuery(text) => {
            // Drop a prefix into the box (cyber toolkit: `= `, `b64 `, …).
            entry.set_text(text);
            entry.set_position(-1);
            entry.grab_focus();
            entry.set_position(-1);
            rebuild(text);
            return;
        }
        Action::EnterMode { id, label } => {
            // Enter an isolated command view: the box becomes a pure filter for
            // this provider, so no typed word can collide with an app name.
            *state.mode.borrow_mut() = Some((id.clone(), label.clone()));
            entry.add_css_class("mode-active");
            entry.set_placeholder_text(Some(&format!("{label} — type to filter · Esc to exit")));
            entry.set_text("");
            entry.grab_focus();
            rebuild("");
            return;
        }
        Action::AddQuicklink { name, template, kind } => {
            // Save to config, then reload the registry so it works immediately.
            if Config::append_quicklink(name, template, kind).is_ok() {
                let cfg = Config::load();
                *state.registry.borrow_mut() =
                    rustcast_core::default_registry(&cfg, state.clip_store.clone());
            }
            // Leave the add mode; the new quicklink is now live.
            *state.mode.borrow_mut() = None;
            entry.remove_css_class("mode-active");
            entry.set_placeholder_text(Some(
                state.registry.borrow().placeholder_for(state.active_tab.get()),
            ));
            entry.set_text("");
            rebuild("");
            return;
        }
        Action::ClipDelete(id) => {
            if let Some(store) = &state.clip_store {
                let _ = store.delete(*id);
            }
            rebuild(&entry.text());
            return;
        }
        Action::ClipPin(id) => {
            if let Some(store) = &state.clip_store {
                let _ = store.toggle_pin(*id);
            }
            rebuild(&entry.text());
            return;
        }
        Action::ClipClear => {
            if let Some(store) = &state.clip_store {
                let _ = store.clear();
            }
            rebuild(&entry.text());
            return;
        }
        Action::Signal { pid, signal } => {
            // Kill the target, then re-query so the row disappears and the
            // launcher stays open for the next one.
            let _ = std::process::Command::new("kill")
                .arg(format!("-{signal}"))
                .arg(pid.to_string())
                .status();
            rebuild(&entry.text());
            return;
        }
        Action::OpenConfig => {
            // ensure a config file exists (write defaults) then open it
            if let Some(path) = Config::config_path() {
                if !path.exists() {
                    let _ = Config::default().save();
                }
                rustcast_core::action::open(&path.to_string_lossy());
            }
            win.close();
            return;
        }
        _ => {}
    }
    if do_action(action, &state.env) {
        win.close();
    }
}

/// Cmd-K style popover listing the selected item's secondary actions.
fn open_actions_menu(
    state: &Rc<State>,
    list: &ListBox,
    entry: &Entry,
    win: &ApplicationWindow,
    rebuild: &Rc<dyn Fn(&str)>,
) {
    let Some(row) = list.selected_row() else { return };
    let i = row.index();
    if i < 0 {
        return;
    }
    let secondaries = {
        let items = state.items.borrow();
        match items.get(i as usize) {
            Some(it) if !it.actions.is_empty() => it.actions.clone(),
            _ => return,
        }
    };

    let popover = Popover::new();
    popover.add_css_class("actions-menu");
    let menu = ListBox::new();
    menu.set_selection_mode(SelectionMode::Single);
    for sa in &secondaries {
        let r = ListBoxRow::new();
        let l = Label::builder().label(&sa.label).xalign(0.0).build();
        l.add_css_class("action-row");
        r.set_child(Some(&l));
        menu.append(&r);
    }
    popover.set_child(Some(&menu));
    popover.set_parent(entry);

    let state2 = state.clone();
    let win2 = win.clone();
    let entry2 = entry.clone();
    let rebuild2 = rebuild.clone();
    let popover2 = popover.clone();
    menu.connect_row_activated(move |_, r| {
        let idx = r.index();
        if idx >= 0 {
            if let Some(sa) = secondaries.get(idx as usize) {
                popover2.popdown();
                apply_action(&state2, &sa.action, &entry2, &win2, &rebuild2);
            }
        }
    });
    popover.popup();
}

/// Build one result row widget.
fn build_row(it: &Item) -> ListBoxRow {
    let row = ListBoxRow::new();
    let hb = GtkBox::new(Orientation::Horizontal, 12);
    hb.add_css_class("row-inner");
    let img = if it.icon.starts_with('/') {
        Image::from_file(&it.icon)
    } else if it.icon.is_empty() {
        Image::new()
    } else {
        Image::from_icon_name(&it.icon)
    };
    img.set_pixel_size(26);
    let tb = GtkBox::new(Orientation::Vertical, 0);
    tb.set_hexpand(true);
    let title = Label::builder()
        .label(&it.title)
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    title.add_css_class("app-title");
    tb.append(&title);
    if !it.subtitle.is_empty() {
        let sub = Label::builder()
            .label(&it.subtitle)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        sub.add_css_class("app-sub");
        tb.append(&sub);
    }
    let tag = Label::builder().label(&it.tag).xalign(1.0).valign(Align::Center).build();
    tag.add_css_class("app-tag");
    hb.append(&img);
    hb.append(&tb);
    hb.append(&tag);
    row.set_child(Some(&hb));

    // Drag-out: file results can be dragged into other apps as a real file.
    if let Action::OpenFile(p) = &it.action {
        attach_file_drag(&row, p);
    }
    row
}

/// Attach a GTK4 drag source that exports `path` as a draggable file
/// (advertises both the GTK file-list type and `text/uri-list`).
fn attach_file_drag(row: &ListBoxRow, path: &str) {
    use gtk4::gdk;
    let src = gtk4::DragSource::new();
    src.set_actions(gdk::DragAction::COPY);
    let file = gtk4::gio::File::for_path(path);
    let flist = gdk::FileList::from_array(&[file]);
    let value = flist.to_value();
    let uri = format!("file://{}\r\n", path);
    src.connect_prepare(move |_, _, _| {
        let by_files = gdk::ContentProvider::for_value(&value);
        let by_uri = gdk::ContentProvider::for_bytes(
            "text/uri-list",
            &gtk4::glib::Bytes::from(uri.as_bytes()),
        );
        Some(gdk::ContentProvider::new_union(&[by_files, by_uri]))
    });
    row.add_controller(src);
}

/// Render lightweight Markdown to Pango markup for the preview label: headers
/// become larger/bold, `inline code` becomes monospace, everything else is
/// escaped and passed through (the body stays monospace via CSS).
fn md_to_pango(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + src.len() / 4);
    // Only the first non-empty `# …` is the document title; later single-`#`
    // lines are shell comments (`# on attacker`), rendered dimmed — not headers.
    let mut seen_title = false;
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("### ") {
            out.push_str(&format!("<span weight='bold'>{}</span>", pango_escape(rest)));
        } else if let Some(rest) = line.strip_prefix("## ") {
            out.push_str(&format!("<span size='large' weight='bold' foreground='#ff6b6b'>{}</span>", pango_escape(rest)));
        } else if let Some(rest) = line.strip_prefix("# ") {
            if !seen_title {
                seen_title = true;
                out.push_str(&format!("<span size='x-large' weight='bold' foreground='#ff6b6b'>{}</span>", pango_escape(rest)));
            } else {
                out.push_str(&format!("<span foreground='#8a8a8a'>{}</span>", pango_escape(line)));
            }
        } else {
            if !line.trim().is_empty() {
                seen_title = true;
            }
            out.push_str(&inline_code(&pango_escape(line)));
        }
        out.push('\n');
    }
    out
}

/// Escape the three characters Pango markup treats specially.
fn pango_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Wrap backtick-delimited spans of already-escaped text in `<tt>` (monospace).
fn inline_code(escaped: &str) -> String {
    if !escaped.contains('`') {
        return escaped.to_string();
    }
    let mut out = String::with_capacity(escaped.len());
    let mut in_code = false;
    for c in escaped.chars() {
        if c == '`' {
            out.push_str(if in_code { "</tt>" } else { "<tt>" });
            in_code = !in_code;
        } else {
            out.push(c);
        }
    }
    if in_code {
        out.push_str("</tt>");
    }
    out
}

fn is_image_path(p: &str) -> bool {
    let lower = p.to_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".ico"]
        .iter()
        .any(|e| lower.ends_with(e))
}

/// Read the first ~8 KB of a file (or a short directory listing) for preview.
fn read_file_head(path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        let mut names: Vec<String> = std::fs::read_dir(p)
            .map(|rd| {
                rd.flatten()
                    .take(200)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        return names.into_iter().take(60).collect::<Vec<_>>().join("\n");
    }
    use std::io::Read as _;
    let Ok(mut f) = std::fs::File::open(p) else { return String::new() };
    let mut buf = vec![0u8; 8192];
    let n = f.read(&mut buf).unwrap_or(0);
    buf.truncate(n);
    match String::from_utf8(buf) {
        Ok(s) => s,
        Err(_) => "(binary file)".into(),
    }
}

/// Legacy clipboard image decode (for `Prev::ClipImage`).
fn clip_image_temp(line: &str) -> Option<String> {
    use std::process::Command;
    let out = "/tmp/rustcast-preview.img";
    let ok = Command::new("sh")
        .env("L", line)
        .arg("-c")
        .arg(format!("printf '%s' \"$L\" | cliphist decode > {out}"))
        .status()
        .ok()?
        .success();
    if ok {
        Some(out.to_string())
    } else {
        None
    }
}

fn move_sel(list: &ListBox, scroll: &ScrolledWindow, delta: i32) {
    let cur = list.selected_row().map(|r| r.index()).unwrap_or(0);
    let next = (cur + delta).max(0);
    if let Some(row) = list.row_at_index(next) {
        list.select_row(Some(&row));
        let adj = scroll.vadjustment();
        let alloc = row.allocation();
        let (y, h) = (alloc.y() as f64, alloc.height() as f64);
        let (val, page) = (adj.value(), adj.page_size());
        if y < val {
            adj.set_value(y);
        } else if y + h > val + page {
            adj.set_value((y + h - page).max(0.0));
        }
    }
}

/// Load the stylesheet: user's `~/.config/rustcast/style.css`, an explicit
/// `[ui].theme` path, or the bundled default.
fn load_css(cfg: &Config) {
    let provider = CssProvider::new();
    if !cfg.ui.theme.is_empty() && std::path::Path::new(&cfg.ui.theme).exists() {
        provider.load_from_path(&cfg.ui.theme);
    } else if let Some(p) = Config::user_css() {
        provider.load_from_path(&p);
    } else {
        provider.load_from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/style.css"));
    }
    if let Some(d) = Display::default() {
        gtk4::style_context_add_provider_for_display(&d, &provider, STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
