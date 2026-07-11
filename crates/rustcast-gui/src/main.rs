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
    /// Pinned favorites, shared live with the registry's `PinsProvider`.
    pins: rustcast_core::pins::PinList,
    /// Usage/recency ranking, shared with the registry.
    frecency: Rc<RefCell<rustcast_core::frecency::Frecency>>,
    /// Resident (daemon) vs one-shot: decides whether "close" hides or destroys.
    resident: bool,
    /// Reusable result-row widgets. Rows are updated in place per keystroke
    /// instead of destroyed+recreated, and only the head `attached` of them are
    /// in the ListBox at any time.
    row_pool: RefCell<Vec<RowWidget>>,
    /// How many pooled rows are currently attached to the ListBox.
    attached: Cell<usize>,
}

/// One reusable result row: the outer row plus the widgets we mutate per query.
struct RowWidget {
    row: ListBoxRow,
    icon: Image,
    title: Label,
    sub: Label,
    tag: Label,
    /// Live drag source for file rows; removed/re-added as the row is reused.
    drag: RefCell<Option<gtk4::DragSource>>,
}

/// Hide the window in resident mode, or close it (quitting) in one-shot mode.
fn dismiss(state: &Rc<State>, win: &ApplicationWindow) {
    if state.resident {
        win.set_visible(false);
    } else {
        win.close();
    }
}

/// What a (possibly forwarded) invocation asks the resident instance to do.
enum Invocation {
    /// Show/toggle the window, optionally on a tab / with a pre-filled query.
    Show { tab: Option<String>, query: Option<String> },
    /// Build the window but stay hidden (autostart pre-warm).
    Daemon,
    /// Quit the resident instance.
    Quit,
}

/// Parse a flat argument list (own argv, or a forwarded one) into an invocation.
fn parse_invocation(args: &[String]) -> Invocation {
    if args.iter().any(|a| a == "--quit") {
        return Invocation::Quit;
    }
    if args.iter().any(|a| a == "--daemon") {
        return Invocation::Daemon;
    }
    let mut tab = args
        .iter()
        .position(|a| a == "--tab")
        .and_then(|i| args.get(i + 1).cloned());
    let mut query = args
        .iter()
        .position(|a| a == "--query")
        .and_then(|i| args.get(i + 1).cloned());
    if tab.is_none() {
        match std::env::var("RUSTCAST_MODE").as_deref() {
            Ok("clipboard") | Ok("clip") | Ok("cb") => tab = Some("clipboard".into()),
            Ok("files") | Ok("file") => tab = Some("files".into()),
            Ok("cyber") => tab = Some("cyber".into()),
            Ok("calc") => {
                tab = Some("apps".into());
                if query.is_none() {
                    query = Some("=".into());
                }
            }
            _ => {}
        }
    }
    Invocation::Show { tab, query }
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
             \x20 --tab <apps|clipboard|files|cyber|cheat|windows|extensions>  open on a tab\n\
             \x20 --query <text>   pre-fill the search box\n\
             \x20 --daemon         start resident, hidden (for autostart)\n\
             \x20 --quit           stop the resident instance\n\
             \x20 --no-daemon      one-shot mode (no resident process)\n\
             \x20 --version        print version\n\
             \x20 --help           show this help\n\n\
             rustcast runs as a single resident process: bind `rustcast` to a\n\
             global hotkey and it toggles the window instantly. Pass --tab to\n\
             open straight onto a tab (e.g. Super+V → `rustcast --tab clipboard`).\n"
        );
        return;
    }

    // Escape hatch: one-shot mode (old behaviour) for compositors where the
    // layer-shell hide/show cycle misbehaves.
    if args.iter().any(|a| a == "--no-daemon") {
        run_oneshot(parse_invocation(&args));
        return;
    }

    run_daemon();
}

/// Resident single-instance mode: the first `rustcast` builds the window and
/// holds the process alive; every later `rustcast` forwards its argv to this
/// instance (over D-Bus) and returns, toggling the window with zero cold-start.
fn run_daemon() {
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    // Built lazily on the first command line, then reused for the process life.
    let ui_cell: Rc<RefCell<Option<Rc<Ui>>>> = Rc::new(RefCell::new(None));

    app.connect_command_line(move |app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        match parse_invocation(&args) {
            Invocation::Quit => {
                app.quit();
            }
            Invocation::Daemon => {
                // Pre-warm: build the window but leave it hidden.
                let mut cell = ui_cell.borrow_mut();
                if cell.is_none() {
                    *cell = Some(build_ui(app, true));
                }
            }
            Invocation::Show { tab, query } => {
                let ui = {
                    let mut cell = ui_cell.borrow_mut();
                    if cell.is_none() {
                        *cell = Some(build_ui(app, true));
                    }
                    cell.as_ref().unwrap().clone()
                };
                // Same hotkey while open (no explicit tab/query) → hide.
                if ui.window.is_visible() && tab.is_none() && query.is_none() {
                    ui.hide();
                } else {
                    ui.show_with(tab, query);
                }
            }
        }
        0
    });

    app.run();
}

/// One-shot fallback (`--no-daemon`): behave like the pre-daemon build — a fresh
/// process that closes on Escape / focus loss.
fn run_oneshot(inv: Invocation) {
    let (tab, query) = match inv {
        Invocation::Show { tab, query } => (tab, query),
        _ => (None, None),
    };
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |a| {
        let ui = build_ui(a, false);
        ui.show_with(tab.clone(), query.clone());
    });
    app.run_with_args::<String>(&[]);
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

/// The whole launcher UI, built once and kept resident. `show_with`/`hide`
/// toggle visibility so a hotkey press never pays GTK/registry startup cost.
struct Ui {
    window: ApplicationWindow,
    entry: Entry,
    state: Rc<State>,
    switch_tab: Rc<dyn Fn(Tab)>,
    /// Whether this instance stays resident (daemon) or closes on hide.
    resident: bool,
    /// True when the window is a wlr-layer-shell surface.
    layer: bool,
    /// Default tab (from config) used when no `--tab` is given.
    default_tab: Cell<Tab>,
    /// mtime of the config file at last load, to detect edits between shows.
    cfg_mtime: Cell<Option<std::time::SystemTime>>,
    /// Keeps the GApplication alive across hide/show in resident mode. Dropping
    /// this releases the app, so it must live as long as the `Ui`.
    _hold: Option<gtk4::gio::ApplicationHoldGuard>,
}

fn config_mtime() -> Option<std::time::SystemTime> {
    Config::config_path()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
}

fn build_ui(app: &Application, resident: bool) -> Rc<Ui> {
    let cfg = Config::load();
    let initial_tab = tab_from_config(&cfg.general.default_tab);

    // Native clipboard store + background watcher daemon.
    let clip_store = if cfg.clipboard.enabled {
        Store::open().ok().map(Rc::new)
    } else {
        None
    };
    // Defer the non-critical clipboard startup work (prune + spawn watcher) off
    // the pre-present path so the first paint is instant.
    if clip_store.is_some() {
        let store = clip_store.clone();
        let cap = cfg.clipboard.max_entries;
        glib::idle_add_local_once(move || {
            if let Some(store) = &store {
                let _ = store.prune(cap);
            }
            ensure_clip_daemon();
        });
    }

    let pins: rustcast_core::pins::PinList =
        Rc::new(RefCell::new(rustcast_core::pins::load()));
    let frecency = Rc::new(RefCell::new(rustcast_core::frecency::Frecency::load()));
    let mut registry = rustcast_core::default_registry(&cfg, clip_store.clone(), pins.clone());
    registry.set_frecency(frecency.clone());
    let state = Rc::new(State {
        registry: RefCell::new(registry),
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
        pins,
        frecency,
        resident,
        row_pool: RefCell::new(Vec::new()),
        attached: Cell::new(0),
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
        // Hide (resident) or close (one-shot) when the launcher loses focus,
        // once it has been focused.
        let seen_active = std::cell::Cell::new(false);
        let win_close = window.clone();
        window.connect_is_active_notify(move |w| {
            if w.is_active() {
                seen_active.set(true);
            } else if seen_active.get() {
                if resident {
                    w.set_visible(false);
                } else {
                    win_close.close();
                }
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
            let tab = state.active_tab.get();
            let target = state.target.borrow();
            let mode = state.mode.borrow();
            let mode_id = mode.as_ref().map(|(id, _)| id.as_str());
            let collected =
                state.registry.borrow().route(query, tab, &state.matcher, target.as_deref(), mode_id);
            drop(mode);
            drop(target);

            // Reuse pooled row widgets: update the first N in place, attach any
            // we're missing, and detach the surplus — no per-keystroke teardown.
            {
                let mut pool = state.row_pool.borrow_mut();
                let attached = state.attached.get();
                let n = collected.len();
                for (i, it) in collected.iter().enumerate() {
                    if i >= pool.len() {
                        pool.push(make_row());
                    }
                    if i >= attached {
                        list.append(&pool[i].row);
                    }
                    update_row(&pool[i], it);
                }
                for i in n..attached {
                    list.remove(&pool[i].row);
                }
                state.attached.set(n);
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
    // Preview pane is only meaningful on content tabs; hide it elsewhere for a
    // minimal single-column (rofi-style) look. Clipboard always keeps it.
    fn tab_shows_preview(tab: Tab) -> bool {
        matches!(tab, Tab::Clipboard | Tab::Files | Tab::Cheat | Tab::Cyber)
    }

    let switch_tab = {
        let state = state.clone();
        let entry = entry.clone();
        let tab_buttons = tab_buttons.clone();
        let rebuild = rebuild.clone();
        let rebuild_footer = rebuild_footer.clone();
        let preview = preview.clone();
        move |tab: Tab| {
            state.active_tab.set(tab);
            preview.set_visible(tab_shows_preview(tab));
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
    let switch_tab: Rc<dyn Fn(Tab)> = Rc::new(switch_tab);

    for (i, b) in tab_buttons.iter().enumerate() {
        let switch_tab = switch_tab.clone();
        b.connect_clicked(move |_| {
            if let Some(t) = Tab::from_index(i) {
                switch_tab(t);
            }
        });
    }

    // Initial preview visibility matches the starting tab.
    preview.set_visible(tab_shows_preview(state.active_tab.get()));

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
                        dismiss(&state, &win);
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

    // Live refresh ticker (700 ms): while the Clipboard tab is open, pick up
    // newly copied entries; while the Cheat tab is open and a tldr download is
    // running, repaint when it finishes — both without needing a keystroke.
    {
        let state = state.clone();
        let entry_r = entry.clone();
        let rebuild_r = rebuild.clone();
        let last_id = std::cell::Cell::new(
            state.clip_store.as_ref().and_then(|s| s.recent(1).first().map(|r| r.id)).unwrap_or(-1),
        );
        let was_downloading = std::cell::Cell::new(false);
        glib::timeout_add_local(std::time::Duration::from_millis(700), move || {
            match state.active_tab.get() {
                Tab::Clipboard => {
                    if let Some(store) = &state.clip_store {
                        let newest = store.recent(1).first().map(|r| r.id).unwrap_or(-1);
                        if newest != last_id.get() {
                            last_id.set(newest);
                            rebuild_r(&entry_r.text());
                        }
                    }
                }
                Tab::Cheat => {
                    // Repaint once when the download starts and once when it ends
                    // so the "Downloading…" row and the results both appear live.
                    let now = rustcast_core::tldr::downloading();
                    if now != was_downloading.get() {
                        was_downloading.set(now);
                        rebuild_r(&entry_r.text());
                    }
                }
                _ => {}
            }
            glib::ControlFlow::Continue
        });
    }

    // Keep the process alive across hide/show in resident mode. The guard must
    // be stored — dropping it releases the application.
    let _hold = if resident { Some(app.hold()) } else { None };
    let _ = &rebuild; // kept alive by the closures; not needed on the struct
    Rc::new(Ui {
        window,
        entry,
        state,
        switch_tab,
        resident,
        layer,
        default_tab: Cell::new(initial_tab),
        cfg_mtime: Cell::new(config_mtime()),
        _hold,
    })
}

impl Ui {
    /// Present the window fresh: reload config if it changed, reset mode/target,
    /// switch to the requested (or default) tab, pre-fill the query, refresh the
    /// background indexes, and grab focus.
    fn show_with(&self, tab: Option<String>, query: Option<String>) {
        // Reload config + rebuild the registry if the file changed since we last
        // looked (so edits apply without a restart).
        let mt = config_mtime();
        if mt != self.cfg_mtime.get() {
            self.cfg_mtime.set(mt);
            let cfg = Config::load();
            self.default_tab.set(tab_from_config(&cfg.general.default_tab));
            let mut reg =
                rustcast_core::default_registry(&cfg, self.state.clip_store.clone(), self.state.pins.clone());
            reg.set_frecency(self.state.frecency.clone());
            *self.state.registry.borrow_mut() = reg;
        }

        // Reset any command mode / target back to a clean root.
        *self.state.mode.borrow_mut() = None;
        self.entry.remove_css_class("mode-active");

        let target_tab = tab.map(|t| tab_from_config(&t)).unwrap_or_else(|| self.default_tab.get());
        (self.switch_tab)(target_tab);

        match query {
            Some(q) => {
                self.entry.set_text(&q);
                self.entry.set_position(-1);
            }
            None => self.entry.set_text(""),
        }

        // Kick background index refreshes (apps/files/tldr) for this show.
        self.state.registry.borrow().refresh_all();

        // Re-assert layer-shell keyboard focus before each present (older
        // gtk4-layer-shell can drop it across an unmap/remap cycle).
        if self.layer {
            self.window.set_keyboard_mode(KeyboardMode::Exclusive);
        }
        self.entry.grab_focus();
        self.window.present();
    }

    fn hide(&self) {
        if self.resident {
            self.window.set_visible(false);
        } else {
            self.window.close();
        }
    }
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
                let mut reg =
                    rustcast_core::default_registry(&cfg, state.clip_store.clone(), state.pins.clone());
                reg.set_frecency(state.frecency.clone());
                *state.registry.borrow_mut() = reg;
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
        Action::SetConfig { section, key, value } => {
            // Edit the config in place, then reload so the change shows now.
            if Config::set_value(section, key, value).is_ok() {
                let cfg = Config::load();
                let mut reg =
                    rustcast_core::default_registry(&cfg, state.clip_store.clone(), state.pins.clone());
                reg.set_frecency(state.frecency.clone());
                *state.registry.borrow_mut() = reg;
            }
            rebuild(&entry.text());
            return;
        }
        Action::Refresh => {
            // Trigger provider refresh (e.g. start/retry the tldr download) and
            // re-query so progress/results appear.
            state.registry.borrow().refresh_all();
            rebuild(&entry.text());
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
            dismiss(state, win);
            return;
        }
        _ => {}
    }
    // Record usage for launch-like actions so frequently-used items float up
    // (never for high-churn Copy — clipboard/tldr copies would pollute it).
    if matches!(
        action,
        Action::Launch(_)
            | Action::OpenUrl(_)
            | Action::OpenFile(_)
            | Action::RunShell(_)
            | Action::RunInTerminal(_)
            | Action::EnterMode { .. }
    ) {
        if let Some(k) = rustcast_core::pins::pin_key(action) {
            state.frecency.borrow_mut().record(&k);
            state.frecency.borrow().save();
        }
    }
    if do_action(action, &state.env) {
        dismiss(state, win);
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
    // Snapshot what we need from the selected item.
    let (title, subtitle, icon, item_action, secondaries) = {
        let items = state.items.borrow();
        let Some(it) = items.get(i as usize) else { return };
        (it.title.clone(), it.subtitle.clone(), it.icon.clone(), it.action.clone(), it.actions.clone())
    };

    // Build the menu: the item's own secondary actions, plus a Pin/Unpin toggle
    // for anything pinnable. `None` marks the pin row (handled specially).
    use rustcast_core::pins::{pin_key, PinnedItem};
    let key = pin_key(&item_action);
    let is_pinned = key
        .as_ref()
        .map(|k| state.pins.borrow().iter().any(|p| pin_key(&p.action).as_deref() == Some(k)))
        .unwrap_or(false);

    let mut entries: Vec<(String, Option<Action>)> =
        secondaries.iter().map(|sa| (sa.label.clone(), Some(sa.action.clone()))).collect();
    if key.is_some() {
        let label = if is_pinned { "Unpin from top".to_string() } else { "★ Pin to top".to_string() };
        entries.push((label, None));
    }
    if entries.is_empty() {
        return;
    }

    let popover = Popover::new();
    popover.add_css_class("actions-menu");
    let menu = ListBox::new();
    menu.set_selection_mode(SelectionMode::Single);
    for (label, _) in &entries {
        let r = ListBoxRow::new();
        let l = Label::builder().label(label).xalign(0.0).build();
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
        if idx < 0 {
            return;
        }
        let Some((_, act)) = entries.get(idx as usize) else { return };
        popover2.popdown();
        match act {
            Some(action) => apply_action(&state2, action, &entry2, &win2, &rebuild2),
            None => {
                // Toggle the pin, persist, and refresh live (PinsProvider shares
                // the same list).
                {
                    let mut pins = state2.pins.borrow_mut();
                    if is_pinned {
                        if let Some(k) = &key {
                            pins.retain(|p| pin_key(&p.action).as_deref() != Some(k.as_str()));
                        }
                    } else {
                        pins.push(PinnedItem {
                            title: title.clone(),
                            subtitle: subtitle.clone(),
                            icon: icon.clone(),
                            action: item_action.clone(),
                        });
                    }
                    let snapshot = pins.clone();
                    drop(pins);
                    rustcast_core::pins::save(&snapshot);
                }
                rebuild2(&entry2.text());
            }
        }
    });
    popover.popup();
}

/// Build one empty, reusable result row. Content is filled by [`update_row`].
fn make_row() -> RowWidget {
    let row = ListBoxRow::new();
    let hb = GtkBox::new(Orientation::Horizontal, 12);
    hb.add_css_class("row-inner");
    let icon = Image::new();
    icon.set_pixel_size(26);
    let tb = GtkBox::new(Orientation::Vertical, 0);
    tb.set_hexpand(true);
    let title = Label::builder().xalign(0.0).ellipsize(gtk4::pango::EllipsizeMode::End).build();
    title.add_css_class("app-title");
    let sub = Label::builder().xalign(0.0).ellipsize(gtk4::pango::EllipsizeMode::End).build();
    sub.add_css_class("app-sub");
    tb.append(&title);
    tb.append(&sub);
    let tag = Label::builder().xalign(1.0).valign(Align::Center).build();
    tag.add_css_class("app-tag");
    hb.append(&icon);
    hb.append(&tb);
    hb.append(&tag);
    row.set_child(Some(&hb));
    RowWidget { row, icon, title, sub, tag, drag: RefCell::new(None) }
}

/// Update a pooled row to show `it` — mutates labels/icon in place instead of
/// rebuilding widgets, and swaps the file drag-source as needed.
fn update_row(rw: &RowWidget, it: &Item) {
    if it.icon.starts_with('/') {
        rw.icon.set_from_file(Some(&it.icon));
    } else if it.icon.is_empty() {
        rw.icon.clear();
    } else {
        rw.icon.set_icon_name(Some(&it.icon));
    }
    rw.title.set_text(&it.title);
    if it.subtitle.is_empty() {
        rw.sub.set_visible(false);
    } else {
        rw.sub.set_text(&it.subtitle);
        rw.sub.set_visible(true);
    }
    rw.tag.set_text(&it.tag);

    // Drag-out: file results can be dragged into other apps as a real file.
    // Remove any drag source left from a previous item this row showed.
    if let Some(old) = rw.drag.borrow_mut().take() {
        rw.row.remove_controller(&old);
    }
    if let Action::OpenFile(p) = &it.action {
        let src = file_drag_source(p);
        rw.row.add_controller(src.clone());
        *rw.drag.borrow_mut() = Some(src);
    }
}

/// A GTK4 drag source that exports `path` as a draggable file (advertises both
/// the GTK file-list type and `text/uri-list`).
fn file_drag_source(path: &str) -> gtk4::DragSource {
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
    src
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
            out.push_str(&format!("<span size='large' weight='bold' foreground='#fb4934'>{}</span>", pango_escape(rest)));
        } else if let Some(rest) = line.strip_prefix("# ") {
            if !seen_title {
                seen_title = true;
                out.push_str(&format!("<span size='x-large' weight='bold' foreground='#fb4934'>{}</span>", pango_escape(rest)));
            } else {
                out.push_str(&format!("<span foreground='#928374'>{}</span>", pango_escape(line)));
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
        // Embed the default theme at compile time so the binary is self-contained
        // (a packaged/installed binary has no source tree to read from).
        const DEFAULT_CSS: &str =
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/style.css"));
        provider.load_from_data(DEFAULT_CSS);
    }
    if let Some(d) = Display::default() {
        gtk4::style_context_add_provider_for_display(&d, &provider, STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
