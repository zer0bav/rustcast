//! Core data model shared by every provider and the GUI.
//!
//! Grown from the original `Item`/`Action`/`Prev` in the single-file prototype,
//! but with a richer action set (URLs, shell templates, file ops) and secondary
//! actions for the Cmd-K style actions menu.

/// A single action performed when the user activates a result.
#[derive(Clone, Debug)]
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
    /// Set the active cyber target (host[:port]).
    SetTarget(String),
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
    /// Decode a `cliphist list` line to a temp image (legacy clipboard).
    ClipImage(String),
    /// Load an image straight from an absolute path (thumbnails, files).
    ImagePath(String),
    /// Rich file metadata block plus an optional text head.
    File { path: String, meta: String, head: Option<String> },
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
    /// Right-aligned type label ("app", "clip", "codec", …).
    pub tag: String,
    pub score: i64,
    pub action: Action,
    pub prev: Prev,
    pub actions: Vec<SecondaryAction>,
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
}
