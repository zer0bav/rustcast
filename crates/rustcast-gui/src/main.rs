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

/// Append a line to `$XDG_CACHE_HOME/rustcast/daemon.log` when `RUSTCAST_LOG`
/// is set — a no-op otherwise, so it costs nothing in normal use. Records every
/// show and every key press to diagnose the intermittent "keys stop after
/// hours" report: if a failed keypress leaves no `key …` line the compositor
/// stopped delivering input (focus/layer-shell); if the line is there but
/// nothing happened the fault is in our handling.
fn dlog(msg: &str) {
    use std::io::Write;
    if std::env::var_os("RUSTCAST_LOG").is_none() {
        return;
    }
    let Some(dir) = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .map(|c| c.join("rustcast"))
    else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("daemon.log")) {
        let _ = writeln!(f, "{secs} {msg}");
    }
}

/// `$XDG_RUNTIME_DIR/rustcast` (or `~/.cache/rustcast`) — holds the daemon
/// pidfile so the recovery script can hard-kill a hung primary regardless of
/// how it was launched (bare `rustcast` that became primary from a hotkey, or
/// an explicit `--daemon`).
fn daemon_runtime_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .map(|d| d.join("rustcast"))
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
             \x20 --tab <apps|clipboard|files|cheat|windows|extensions>  open on a tab\n\
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

    // Record the resident PID once we're the primary (connect_startup fires only
    // for the primary registration, never for a remote/forwarding invocation) so
    // recovery can force-replace a hung daemon by PID. Cleared on shutdown.
    app.connect_startup(|_| {
        if let Some(dir) = daemon_runtime_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("daemon.pid"), std::process::id().to_string());
        }
    });
    app.connect_shutdown(|_| {
        if let Some(dir) = daemon_runtime_dir() {
            let _ = std::fs::remove_file(dir.join("daemon.pid"));
        }
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

/// True if `pid` is a live process whose command line is our own clipboard
/// daemon (`--clip-daemon`). Guards against a recycled pid being mistaken for a
/// running daemon after a reboot/crash left a stale lockfile behind.
fn clip_daemon_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    match std::fs::read(format!("/proc/{pid}/cmdline")) {
        // cmdline is NUL-separated argv; a live rustcast daemon carries the flag.
        Ok(raw) => raw.split(|&b| b == 0).any(|arg| arg == b"--clip-daemon"),
        Err(_) => false,
    }
}

/// Spawn the two `wl-paste --watch` processes that feed the clipboard store.
/// Guarded by a lockfile so only one daemon runs.
fn run_clip_daemon() {
    use std::process::Command;
    let Some(dir) = Config::data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let lock = dir.join("clip-daemon.lock");
    // Single-instance guard: bail only if the recorded pid is *our* daemon still
    // alive. Checking `/proc/<pid>` alone is not enough — after a reboot the OS
    // can recycle that pid for an unrelated process, which would make us think
    // the daemon is up and never spawn the watchers (clipboard silently dead).
    if let Ok(pid) = std::fs::read_to_string(&lock) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            if clip_daemon_alive(pid) {
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
        // Short busy-timeout: this store is read on the GTK main thread, so it
        // must never block the UI for seconds during a WAL checkpoint.
        Store::open_reader().ok().map(Rc::new)
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

    let (css_provider, theme_idx) = load_css(&cfg);
    // Shared theme state so Ctrl+T can cycle bundled themes at runtime.
    let css_provider = Rc::new(css_provider);
    let theme_idx = Rc::new(Cell::new(theme_idx));

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
    // Metadata table pinned under the body, Raycast-style: one `label   value`
    // line per fact, filled by `set_meta_rows`.
    let prev_meta = GtkBox::new(Orientation::Vertical, 3);
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
                set_meta_rows(&prev_meta, &[]);
                prev_img.set_pixel_size(0);
                prev_img.clear();
                return;
            };
            set_meta_rows(&prev_meta, &[]);
            match &it.prev {
                Prev::None => {
                    prev_lbl.set_text("");
                    prev_img.set_pixel_size(120);
                    if it.icon.starts_with('/') {
                        set_scaled_image(&prev_img, &it.icon, 128);
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
                        set_scaled_image(&prev_img, &p, 320);
                    }
                }
                Prev::ImagePath(p) => {
                    prev_lbl.set_text("");
                    prev_img.set_pixel_size(320);
                    set_scaled_image(&prev_img, p, 320);
                    set_meta_rows(&prev_meta, &[("".into(), it.subtitle.clone())]);
                }
                Prev::WindowIcon(icon) => {
                    // Just the app's logo, shown large, plus the title/workspace.
                    prev_lbl.set_text("");
                    prev_img.set_pixel_size(128);
                    if icon.starts_with('/') {
                        set_scaled_image(&prev_img, icon, 128);
                    } else {
                        prev_img.set_icon_name(Some(icon));
                    }
                    set_meta_rows(&prev_meta, &[("".into(), it.subtitle.clone())]);
                }
                Prev::Rich { image, text, meta } => {
                    // Body (image or text) on top, metadata table at the bottom.
                    match image {
                        Some(p) => {
                            prev_img.set_pixel_size(300);
                            set_scaled_image(&prev_img, p, 300);
                        }
                        None => {
                            prev_img.set_pixel_size(0);
                            prev_img.clear();
                        }
                    }
                    prev_lbl.set_text(text.as_deref().unwrap_or(""));
                    set_meta_rows(&prev_meta, meta);
                }
                Prev::File { path, meta, head } => {
                    // content on top, metadata pinned at the bottom (Raycast style)
                    set_meta_rows(&prev_meta, &[("".into(), meta.clone())]);
                    if let Some(h) = head {
                        prev_img.set_pixel_size(0);
                        prev_img.clear();
                        prev_lbl.set_text(h);
                    } else if !path.is_empty() && is_image_path(path) {
                        prev_lbl.set_text("");
                        prev_img.set_pixel_size(300);
                        set_scaled_image(&prev_img, path, 300);
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
            let mode = state.mode.borrow();
            let mode_id = mode.as_ref().map(|(id, _)| id.as_str());
            let collected = state.registry.borrow().route(query, tab, &state.matcher, mode_id);
            drop(mode);

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
            match select_first(&list) {
                Some(i) => update_preview(i),
                None => update_preview(usize::MAX),
            }
        }
    };
    let rebuild: Rc<dyn Fn(&str)> = Rc::new(rebuild);

    // ── tab switching ───────────────────────────────────────
    // Preview pane is only meaningful on content tabs; hide it elsewhere for a
    // minimal single-column (rofi-style) look. Clipboard always keeps it.
    fn tab_shows_preview(tab: Tab) -> bool {
        matches!(tab, Tab::Clipboard | Tab::Files | Tab::Cheat | Tab::Win)
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

    // ── click / double-click a row ──────────────────────────
    {
        let state = state.clone();
        let entry_c = entry.clone();
        let win_c = window.clone();
        let rebuild_c = rebuild.clone();
        let list_c = list.clone();
        list.connect_row_activated(move |_, _| {
            activate_selected(&state, &list_c, &entry_c, &win_c, &rebuild_c);
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
        let css_provider = css_provider.clone();
        let theme_idx = theme_idx.clone();
        let key = EventControllerKey::new();
        key.set_propagation_phase(gtk4::PropagationPhase::Capture);
        key.connect_key_pressed(move |_, keyval, _, mods| {
            let ctrl = mods.contains(ModifierType::CONTROL_MASK);
            let shift = mods.contains(ModifierType::SHIFT_MASK);
            dlog(&format!(
                "key {} ctrl={ctrl} shift={shift}",
                keyval.name().map(|s| s.to_string()).unwrap_or_default()
            ));
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
                // direct tab jump: Ctrl+1..6
                Key::_1 | Key::_2 | Key::_3 | Key::_4 | Key::_5 | Key::_6 if ctrl => {
                    let n = match keyval {
                        Key::_1 => 0,
                        Key::_2 => 1,
                        Key::_3 => 2,
                        Key::_4 => 3,
                        Key::_5 => 4,
                        _ => 5,
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
                // clipboard: delete / pin the highlighted entry in place
                Key::d | Key::D if ctrl => {
                    apply_selected_secondary(&state, &list, &entry, &win, &rebuild, |a| {
                        matches!(a, Action::ClipDelete(_))
                    });
                    Propagation::Stop
                }
                Key::Delete | Key::BackSpace if shift => {
                    apply_selected_secondary(&state, &list, &entry, &win, &rebuild, |a| {
                        matches!(a, Action::ClipDelete(_))
                    });
                    Propagation::Stop
                }
                Key::s | Key::S if ctrl => {
                    apply_selected_secondary(&state, &list, &entry, &win, &rebuild, |a| {
                        matches!(a, Action::ClipPin(_))
                    });
                    Propagation::Stop
                }
                // clear
                Key::u if ctrl => {
                    entry.set_text("");
                    Propagation::Stop
                }
                // cycle bundled themes live: caelestia → cyberdeck → tokyonight → catppuccin → …
                Key::t | Key::T if ctrl => {
                    let next = (theme_idx.get() + 1) % BUNDLED_THEMES.len();
                    theme_idx.set(next);
                    css_provider.load_from_data(BUNDLED_THEMES[next].1);
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

    // Close the show-time keyboard-focus race. On a layer-shell surface the
    // compositor only grants keyboard focus once the surface is mapped, but
    // `show_with` asserts KeyboardMode::Exclusive + grab_focus *before*
    // `present()` (i.e. before the map). Across the resident hide/show cycle
    // that first assertion is sometimes dropped, so the first ↑/↓/Enter after
    // opening lands on a surface hyprland hasn't focused yet and is swallowed —
    // "sometimes the arrow key works, sometimes it doesn't". Re-asserting on
    // every `map` (which fires each time the window becomes visible) makes the
    // focus deterministic.
    {
        let entry = entry.clone();
        let clip_enabled = cfg.clipboard.enabled;
        window.connect_map(move |w| {
            // Same scheduled re-assert as show_with, so an external
            // set_visible(true) path is covered too.
            reassert_focus(w, &entry, layer);
            // Respawn the clipboard watcher if it has died (compositor restart,
            // Wayland disconnect, crash). `ensure_clip_daemon` is guarded by the
            // lockfile, so this is a cheap no-op while the daemon is alive.
            if clip_enabled {
                ensure_clip_daemon();
            }
        });
    }

    // Live refresh ticker (700 ms): while the Clipboard tab is open, pick up
    // newly copied entries; while the Cheat tab is open and a tldr download is
    // running, repaint when it finishes — both without needing a keystroke.
    {
        let state = state.clone();
        let entry_r = entry.clone();
        let rebuild_r = rebuild.clone();
        let window_t = window.clone();
        let last_id = std::cell::Cell::new(
            state.clip_store.as_ref().and_then(|s| s.recent(1).first().map(|r| r.id)).unwrap_or(-1),
        );
        let was_downloading = std::cell::Cell::new(false);
        glib::timeout_add_local(std::time::Duration::from_millis(700), move || {
            // Do nothing while the launcher is hidden. In resident mode the
            // process lives for days; without this guard, every clipboard copy
            // (with the Clipboard tab last-active) rebuilds the list behind the
            // invisible window — and rebuild reloads the selected entry's image
            // preview into a texture via set_from_file. Copying images all day
            // then grows RSS unbounded (observed ~300 MB after hours) and, once
            // the box bloats, the compositor/main-loop starves and input starts
            // dropping. Repaint on the next real show instead.
            if resident && !window_t.is_visible() {
                return glib::ControlFlow::Continue;
            }
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

/// Re-assert layer-shell keyboard focus on a short fixed schedule after the
/// surface maps. This is the root fix for the "launcher visible but swallows
/// keys" freeze: on hyprland the compositor sometimes finishes moving keyboard
/// focus onto the newly-mapped exclusive layer *after* our pre-present
/// `grab_focus`, and the old single 20 ms shot fired once — so if the compositor
/// was still mid-transition (more likely after hours of uptime / many surfaces)
/// focus was never recaptured until the next show.
///
/// We deliberately do NOT verify `entry.has_focus()` and loop: GTK exposes no
/// signal for whether the compositor is delivering keys to a layer surface
/// (and for a composite `Entry` `has_focus()` is false anyway — focus lives on
/// the inner GtkText delegate). Instead we re-request the exclusive grab + entry
/// focus a few times over ~400 ms (covering a late focus transition), and finish
/// with a `KeyboardMode::None -> Exclusive` toggle that forces the compositor to
/// drop and re-grant the grab if it got stuck — the strongest lever, applied
/// once, without a visible flicker.
fn reassert_focus(window: &ApplicationWindow, entry: &Entry, layer: bool) {
    if layer {
        window.set_keyboard_mode(KeyboardMode::Exclusive);
    }
    entry.grab_focus();
    dlog("reassert: immediate grab");
    // A few timed re-asserts; the last also toggles the grab to force re-eval.
    const SCHEDULE_MS: [u64; 3] = [80, 200, 400];
    for (i, &delay) in SCHEDULE_MS.iter().enumerate() {
        let (w, e) = (window.clone(), entry.clone());
        let last = i == SCHEDULE_MS.len() - 1;
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
            if layer {
                w.set_keyboard_mode(KeyboardMode::Exclusive);
            }
            e.grab_focus();
            if last && layer {
                w.set_keyboard_mode(KeyboardMode::None);
                w.set_keyboard_mode(KeyboardMode::Exclusive);
                e.grab_focus();
                dlog("reassert: toggled None->Exclusive to force re-eval");
            }
        });
    }
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

        // Reset any command mode back to a clean root.
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
        dlog(&format!(
            "show: present visible={} focus={}",
            self.window.is_visible(),
            self.entry.has_focus()
        ));

        // Confirmed failure mode (hyprland): the overlay maps but the compositor
        // leaves keyboard focus on whatever was focused before (e.g. Signal), so
        // the launcher is visible yet swallows every key. Setting Exclusive +
        // grab_focus before present isn't always honoured because the surface
        // isn't mapped yet. Re-assert (and re-check) after the map until the
        // re-request the exclusive grab + entry focus on a short schedule — see
        // `reassert_focus`.
        reassert_focus(&self.window, &self.entry, self.layer);
    }

    fn hide(&self) {
        if self.resident {
            self.window.set_visible(false);
        } else {
            self.window.close();
        }
    }
}

/// Run the selected item's first secondary action for which `pick` returns true
/// — the clipboard's ⌃D (delete) / ⌃S (pin) shortcuts, without a trip through
/// the actions menu. Returns whether such an action existed.
///
/// The list is rebuilt by the action itself (an entry disappears, or its pin
/// flips), so the selection is restored to the same position afterwards: deleting
/// five entries in a row shouldn't send you back to the top each time.
fn apply_selected_secondary(
    state: &Rc<State>,
    list: &ListBox,
    entry: &Entry,
    win: &ApplicationWindow,
    rebuild: &Rc<dyn Fn(&str)>,
    pick: impl Fn(&Action) -> bool,
) -> bool {
    let Some(row) = list.selected_row() else { return false };
    let i = row.index();
    if i < 0 {
        return false;
    }
    let action = {
        let items = state.items.borrow();
        let Some(it) = items.get(i as usize) else { return false };
        it.actions.iter().find(|sa| pick(&sa.action)).map(|sa| sa.action.clone())
    };
    let Some(action) = action else { return false };
    apply_action(state, &action, entry, win, rebuild);

    // Land on whatever took the old row's place — scanning down a couple of rows
    // in case a section header slid in, then back up if the list got shorter.
    let selectable = |n: i32| list.row_at_index(n).filter(|r| r.is_selectable());
    match (i..=i + 2).find_map(selectable).or_else(|| (0..=i).rev().find_map(selectable)) {
        Some(r) => list.select_row(Some(&r)),
        None => {
            select_first(list);
        }
    }
    true
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
        Action::SetQuery(text) => {
            // Drop a prefix into the box (e.g. the calculator's `= `).
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
    // A Popover parented via set_parent is NOT auto-removed on popdown — without
    // this every Ctrl+K leaves a detached widget on the entry (the focus target),
    // a slow multi-day leak that can itself degrade keyboard focus. Unparent on
    // close and hand focus back to the entry so keys aren't swallowed afterwards.
    {
        let entry_p = entry.clone();
        popover.connect_closed(move |p| {
            p.unparent();
            entry_p.grab_focus();
        });
    }

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

/// Fill the preview's metadata table: one `label            value` line per
/// entry, dim label on the left, value right-aligned. An entry with an empty
/// label renders as a single full-width line (used for the plain-string metadata
/// the file preview still produces). An empty slice clears the table.
fn set_meta_rows(container: &GtkBox, rows: &[(String, String)]) {
    while let Some(c) = container.first_child() {
        container.remove(&c);
    }
    container.set_visible(!rows.is_empty());
    for (label, value) in rows {
        let line = GtkBox::new(Orientation::Horizontal, 10);
        if label.is_empty() {
            let v = Label::builder()
                .label(value)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk4::pango::WrapMode::WordChar)
                .max_width_chars(46)
                .build();
            v.add_css_class("meta-value");
            line.append(&v);
        } else {
            let k = Label::builder().label(label).xalign(0.0).build();
            k.add_css_class("meta-key");
            let v = Label::builder()
                .label(value)
                .xalign(1.0)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                .build();
            v.add_css_class("meta-value");
            line.append(&k);
            line.append(&v);
        }
        container.append(&line);
    }
}

/// Build one empty, reusable result row. Content is filled by [`update_row`].
fn make_row() -> RowWidget {
    let row = ListBoxRow::new();
    // Single-line rows (icon · title · dim subtitle · right-aligned tag): more
    // results fit on screen and the eye scans one column of titles.
    let hb = GtkBox::new(Orientation::Horizontal, 10);
    hb.add_css_class("row-inner");
    let icon = Image::new();
    icon.set_pixel_size(20);
    let title = Label::builder()
        .xalign(0.0)
        .valign(Align::Center)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    title.add_css_class("app-title");
    let sub = Label::builder()
        .xalign(0.0)
        .valign(Align::Center)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    sub.add_css_class("app-sub");
    let tag = Label::builder().xalign(1.0).valign(Align::Center).build();
    tag.add_css_class("app-tag");
    hb.append(&icon);
    hb.append(&title);
    hb.append(&sub);
    hb.append(&tag);
    row.set_child(Some(&hb));
    RowWidget { row, icon, title, sub, tag, drag: RefCell::new(None) }
}

/// Update a pooled row to show `it` — mutates labels/icon in place instead of
/// rebuilding widgets, and swaps the file drag-source as needed.
fn update_row(rw: &RowWidget, it: &Item) {
    // Group headers ("Applications", "Pinned"…) are captions, not results: no
    // icon, no selection, skipped by the arrow keys (see `move_sel`).
    if it.header {
        rw.row.add_css_class("section-header");
        rw.row.set_selectable(false);
        rw.row.set_activatable(false);
        rw.icon.clear();
        rw.icon.set_visible(false);
        // GTK CSS has no text-transform, so the caps happen here.
        rw.title.set_ellipsize(gtk4::pango::EllipsizeMode::None);
        rw.title.set_text(&it.title.to_uppercase());
        rw.title.remove_css_class("app-title");
        rw.title.add_css_class("section-title");
        rw.sub.set_visible(false);
        rw.tag.set_text("");
        if let Some(old) = rw.drag.borrow_mut().take() {
            rw.row.remove_controller(&old);
        }
        return;
    }
    rw.row.remove_css_class("section-header");
    rw.row.set_selectable(true);
    rw.row.set_activatable(true);
    rw.icon.set_visible(true);
    rw.title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    rw.title.remove_css_class("section-title");
    rw.title.add_css_class("app-title");

    if it.icon.starts_with('/') {
        rw.icon.set_from_file(Some(&it.icon));
    } else if it.icon.is_empty() {
        rw.icon.clear();
    } else {
        rw.icon.set_icon_name(Some(&it.icon));
    }
    rw.title.set_text(&it.title);
    // The subtitle stays in the layout even when empty: it is the expanding
    // spacer that keeps the tag pinned to the right edge.
    rw.sub.set_text(&it.subtitle);
    rw.sub.set_visible(true);
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

/// Show an image from `path`, decoded down to at most `max_px` on its longest
/// side. `GtkImage::set_from_file` always decodes at full resolution
/// (`set_pixel_size` scales only the *display*, not the backing texture), so a
/// preview of a full-screen screenshot or photo costs ~9 MB of RGBA. Over a
/// session of browsing image clips that grows RSS into the hundreds of MB and
/// eventually starves the compositor/main loop, which drops input. Decoding to
/// the preview size keeps each image at a few hundred KB.
fn set_scaled_image(img: &Image, path: &str, max_px: i32) {
    match gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(path, max_px, max_px, true) {
        Ok(pb) => img.set_paintable(Some(&gtk4::gdk::Texture::for_pixbuf(&pb))),
        Err(_) => img.clear(),
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

/// Move the selection by `delta`, stepping over the non-selectable section
/// headers so ↑/↓ only ever lands on a real result.
fn move_sel(list: &ListBox, scroll: &ScrolledWindow, delta: i32) {
    // No selection yet (e.g. list just repopulated): the first ↑/↓ press should
    // land on the first real row rather than being a dead key. Without this,
    // Down (delta=+1) recovered via next=1 while Up (delta=-1) computed next=-1
    // and returned — "the up arrow doesn't work, down does".
    let Some(cur_row) = list.selected_row() else {
        select_first(list);
        if let Some(r) = list.selected_row() {
            scroll_into_view(scroll, &r);
        }
        return;
    };
    let cur = cur_row.index();
    let step = if delta >= 0 { 1 } else { -1 };
    let mut next = cur + delta;
    let row = loop {
        if next < 0 {
            return;
        }
        let Some(row) = list.row_at_index(next) else { return };
        if row.is_selectable() {
            break row;
        }
        next += step;
    };
    list.select_row(Some(&row));
    dlog(&format!(
        "move_sel d={delta} cur={cur} -> next={next} now={:?}",
        list.selected_row().map(|r| r.index())
    ));
    scroll_into_view(scroll, &row);
}

/// Scroll `row` into the viewport, keeping a section header visible above the
/// first row of its group.
fn scroll_into_view(scroll: &ScrolledWindow, row: &ListBoxRow) {
    let adj = scroll.vadjustment();
    let alloc = row.allocation();
    let (y, h) = (alloc.y() as f64, alloc.height() as f64);
    let (val, page) = (adj.value(), adj.page_size());
    // A little headroom so the group caption above the row stays on screen.
    const HEADROOM: f64 = 26.0;
    if y - HEADROOM < val {
        adj.set_value((y - HEADROOM).max(0.0));
    } else if y + h > val + page {
        adj.set_value((y + h - page).max(0.0));
    }
}

/// Select the first selectable row (skipping a leading section header) and
/// return its index, or `None` when the list holds no results.
fn select_first(list: &ListBox) -> Option<usize> {
    let mut i = 0;
    while let Some(row) = list.row_at_index(i) {
        if row.is_selectable() {
            list.select_row(Some(&row));
            return Some(i as usize);
        }
        i += 1;
    }
    None
}

const CAELESTIA_CSS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/theme-caelestia.css"));
const CYBERDECK_CSS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/theme-cyberdeck.css"));
const TOKYONIGHT_CSS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/theme-tokyonight.css"));
const CATPPUCCIN_CSS: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/theme-catppuccin.css"));

/// Bundled themes, in the order Ctrl+T cycles through them. `caelestia` (soft
/// lavender dashboard) is the default (index 0); `cyberdeck` (sharp cyan) is
/// index 1; `tokyonight` and `catppuccin` are the square-cornered dark themes.
const BUNDLED_THEMES: &[(&str, &str)] = &[
    ("caelestia", CAELESTIA_CSS),
    ("cyberdeck", CYBERDECK_CSS),
    ("tokyonight", TOKYONIGHT_CSS),
    ("catppuccin", CATPPUCCIN_CSS),
];

/// Resolve a `[ui].theme` value to a bundled theme index, treating aliases
/// gracefully. Returns `None` for empty/custom-path values (→ index 0).
fn theme_index(name: &str) -> Option<usize> {
    match name.trim().to_ascii_lowercase().as_str() {
        "caelestia" | "soft" | "dashboard" => Some(0),
        "cyberdeck" | "deck" => Some(1),
        "tokyonight" | "tokyo" | "night" => Some(2),
        "catppuccin" | "mocha" | "catppuccin-mocha" => Some(3),
        _ => None,
    }
}

/// Load the stylesheet into a fresh persistent provider and return it together
/// with the active bundled-theme index (for Ctrl+T cycling). `[ui].theme`
/// accepts a bundled theme *name* (caelestia | cyberdeck | tokyonight |
/// catppuccin, plus aliases) or a path to a custom `.css`; empty falls back to
/// `~/.config/rustcast/style.css`
/// then the bundled default. All bundled themes are embedded at compile time so
/// the binary is self-contained (a packaged install has no source tree to read).
fn load_css(cfg: &Config) -> (CssProvider, usize) {
    let provider = CssProvider::new();
    let theme = cfg.ui.theme.trim();
    let idx = match theme_index(theme) {
        Some(i) => {
            // the default (index 0) may still be overridden by the user's own style.css
            if i == 0 {
                match Config::user_css().filter(|p| p.exists()) {
                    Some(p) => provider.load_from_path(&p),
                    None => provider.load_from_data(BUNDLED_THEMES[0].1),
                }
            } else {
                provider.load_from_data(BUNDLED_THEMES[i].1);
            }
            i
        }
        None if !theme.is_empty() && std::path::Path::new(theme).exists() => {
            provider.load_from_path(theme);
            0
        }
        None => {
            match Config::user_css().filter(|p| p.exists()) {
                Some(p) => provider.load_from_path(&p),
                None => provider.load_from_data(BUNDLED_THEMES[0].1),
            }
            0
        }
    };
    if let Some(d) = Display::default() {
        gtk4::style_context_add_provider_for_display(&d, &provider, STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    (provider, idx)
}
