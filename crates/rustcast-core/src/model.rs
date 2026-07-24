//! Core data model shared by every provider and the GUI.
//!
//! Grown from the original `Item`/`Action`/`Prev` in the single-file prototype,
//! but with a richer action set (URLs, shell templates, file ops) and secondary
//! actions for the Cmd-K style actions menu.

/// A single action performed when the user activates a result.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Action {
    /// Launch a `.desktop` Exec line (detached via `setsid -f`).
    Launch(String),
    /// Put plain text on the clipboard.
    Copy(String),
    /// Copy a `cliphist list` line back to the clipboard (legacy backend).
    ClipCopy(String),
    /// Copy an image file back to the clipboard with its MIME type.
    CopyImage { path: String, mime: String },
    /// Delete a clipboard history entry by id (handled by the GUI).
    ClipDelete(i64),
    /// Toggle pin on a clipboard entry by id (handled by the GUI).
    ClipPin(i64),
    /// Clear all non-pinned clipboard history (handled by the GUI).
    ClipClear,
    /// Ensure the config file exists (writing defaults) then open it.
    OpenConfig,
    /// Open a URL in the default browser (`xdg-open`).
    OpenUrl(String),
    /// Run a shell command line, detached.
    RunShell(String),
    /// Run a command inside the user's terminal emulator.
    RunInTerminal(String),
    /// Open a file with its default handler.
    OpenFile(String),
    /// Reveal a file in the default file manager.
    RevealFile(String),
    /// Replace the search box text (used for prefix tools like `= `).
    SetQuery(String),
    /// Enter an isolated command mode routed to the provider whose id is `id`
    /// (e.g. "Kill Process" → id "procs"). The box clears and typing just
    /// filters within that mode; Esc backs out. `label` is shown to the user.
    EnterMode { id: String, label: String },
    /// Save a new quicklink to the config and reload it live. Handled GUI-side.
    AddQuicklink { name: String, template: String, kind: String },
    /// Set `[section] key = value` in the config and reload live. GUI-handled.
    SetConfig { section: String, key: String, value: String },
    /// Send a signal to a PID (SIGTERM=15, SIGKILL=9). Handled GUI-side: the
    /// launcher stays open and re-queries so you can kill several in a row.
    Signal { pid: i32, signal: i32 },
    /// Ask the registry to refresh its providers (e.g. trigger/retry the tldr
    /// download) and re-query. Handled GUI-side; the window stays open.
    Refresh,
    /// Do nothing (informational rows).
    None,
}

/// What to render in the right-hand preview pane for a result.
#[derive(Clone, Debug)]
pub enum Prev {
    /// No custom preview — the GUI falls back to a large icon.
    None,
    /// Show plain / monospace text (also used for pretty-printed JSON).
    Text(String),
    /// Render lightweight Markdown (headers, inline `code`) in the preview.
    Markdown(String),
    /// Decode a `cliphist list` line to a temp image (legacy clipboard).
    ClipImage(String),
    /// Load an image straight from an absolute path (thumbnails, files).
    ImagePath(String),
    /// Rich file metadata block plus an optional text head.
    File { path: String, meta: String, head: Option<String> },
    /// Raycast-style detail pane: an optional image, an optional body, and a
    /// key/value metadata table rendered as a grid under a separator.
    Rich {
        /// Absolute path to an image to show above the body, if any.
        image: Option<String>,
        /// Body text (monospace).
        text: Option<String>,
        /// Metadata rows shown as `label — value` pairs at the bottom.
        meta: Vec<(String, String)>,
    },
}

impl Default for Prev {
    fn default() -> Self {
        Prev::None
    }
}

/// A secondary action shown in the actions menu (Cmd-K style).
#[derive(Clone, Debug)]
pub struct SecondaryAction {
    pub label: String,
    pub action: Action,
}

/// One row in the result list.
#[derive(Clone, Debug)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    /// Icon name (freedesktop), absolute `/path`, or empty.
    pub icon: String,
    /// Right-aligned type label ("app", "clip", "file", …).
    pub tag: String,
    pub score: i64,
    pub action: Action,
    pub prev: Prev,
    pub actions: Vec<SecondaryAction>,
    /// Group this row belongs to ("Applications", "Commands", "Pinned"…).
    /// The registry sorts groups by their best hit and inserts a header row per
    /// group, the way Raycast splits its result list. Empty = ungrouped.
    pub section: String,
    /// True for the inserted group-header rows themselves: not selectable, not
    /// activatable, rendered as a small caption by the GUI.
    pub header: bool,
}

impl Item {
    /// Minimal constructor for the common case.
    pub fn new(
        title: impl Into<String>,
        subtitle: impl Into<String>,
        icon: impl Into<String>,
        tag: impl Into<String>,
        score: i64,
        action: Action,
    ) -> Self {
        Item {
            title: title.into(),
            subtitle: subtitle.into(),
            icon: icon.into(),
            tag: tag.into(),
            score,
            action,
            prev: Prev::None,
            actions: Vec::new(),
            section: String::new(),
            header: false,
        }
    }

    /// A non-selectable group header row.
    pub fn section_header(title: impl Into<String>) -> Self {
        let title = title.into();
        Item {
            title: title.clone(),
            subtitle: String::new(),
            icon: String::new(),
            tag: String::new(),
            score: 0,
            action: Action::None,
            prev: Prev::None,
            actions: Vec::new(),
            section: title,
            header: true,
        }
    }

    pub fn with_prev(mut self, prev: Prev) -> Self {
        self.prev = prev;
        self
    }

    pub fn with_actions(mut self, actions: Vec<SecondaryAction>) -> Self {
        self.actions = actions;
        self
    }

    /// Put this item in a named group (see [`Item::section`]).
    pub fn in_section(mut self, section: impl Into<String>) -> Self {
        self.section = section.into();
        self
    }
}
