//! Executes an [`Action`]. All of this shells out to the environment (no GTK),
//! so it lives in core and can be called straight from the GUI's key handler.

use crate::model::Action;
use std::io::Write;
use std::process::{Command, Stdio};

/// The user's terminal command prefix, e.g. `"kitty -e"`. Threaded in from config.
#[derive(Clone)]
pub struct Env {
    pub terminal: String,
}

impl Default for Env {
    fn default() -> Self {
        Env { terminal: "xterm -e".into() }
    }
}

/// Launch a detached process from a `.desktop` Exec line.
pub fn launch(exec: &str) {
    let cmd = crate::apps::clean_exec(exec);
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("setsid -f {cmd} >/dev/null 2>&1"))
        .spawn();
}

/// Put plain text on the clipboard — `wl-copy` on Wayland, `xclip` on X11.
pub fn copy_text(text: &str) {
    let mut cmd = if crate::config::which("wl-copy") {
        Command::new("wl-copy")
    } else if crate::config::which("xclip") {
        let mut c = Command::new("xclip");
        c.args(["-selection", "clipboard"]);
        c
    } else if crate::config::which("xsel") {
        let mut c = Command::new("xsel");
        c.args(["--clipboard", "--input"]);
        c
    } else {
        return;
    };
    if let Ok(mut c) = cmd.stdin(Stdio::piped()).spawn() {
        if let Some(mut s) = c.stdin.take() {
            let _ = s.write_all(text.as_bytes());
        }
    }
}

/// Read the current clipboard text (`wl-paste`/`xclip`/`xsel`), empty on failure.
pub fn paste_text() -> String {
    let out = if crate::config::which("wl-paste") {
        Command::new("wl-paste").arg("--no-newline").output()
    } else if crate::config::which("xclip") {
        Command::new("xclip").args(["-selection", "clipboard", "-o"]).output()
    } else if crate::config::which("xsel") {
        Command::new("xsel").args(["--clipboard", "--output"]).output()
    } else {
        return String::new();
    };
    out.ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// Copy an image file back onto the clipboard with the right MIME type.
pub fn copy_image(path: &str, mime: &str) {
    let cmd = if crate::config::which("wl-copy") {
        format!("wl-copy --type {} < {}", shell_quote(mime), shell_quote(path))
    } else if crate::config::which("xclip") {
        format!("xclip -selection clipboard -t {} -i {}", shell_quote(mime), shell_quote(path))
    } else {
        return;
    };
    let _ = Command::new("sh").arg("-c").arg(cmd).spawn();
}

/// Legacy clipboard restore: decode a `cliphist list` line and re-copy it.
pub fn clip_copy(line: &str) {
    let _ = Command::new("sh")
        .env("L", line)
        .arg("-c")
        .arg("setsid -f sh -c 'printf %s \"$L\" | cliphist decode | wl-copy'")
        .spawn();
}

/// Open a URL / file with the default handler.
pub fn open(target: &str) {
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("setsid -f xdg-open {} >/dev/null 2>&1", shell_quote(target)))
        .spawn();
}

/// Run a shell command line, detached.
pub fn run_shell(cmd: &str) {
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("setsid -f sh -c {} >/dev/null 2>&1", shell_quote(cmd)))
        .spawn();
}

/// Run a command inside the user's terminal.
pub fn run_in_terminal(cmd: &str, env: &Env) {
    let full = format!("{} sh -c {}", env.terminal, shell_quote(&format!("{cmd}; exec $SHELL")));
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("setsid -f {full} >/dev/null 2>&1"))
        .spawn();
}

/// Reveal a file in the default file manager (falls back to opening its dir).
pub fn reveal(path: &str) {
    // Try the freedesktop file-manager D-Bus interface, else open the parent dir.
    let parent = std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".into());
    let script = format!(
        "dbus-send --session --dest=org.freedesktop.FileManager1 --type=method_call \
         /org/freedesktop/FileManager1 org.freedesktop.FileManager1.ShowItems \
         array:string:file://{} string:'' 2>/dev/null || setsid -f xdg-open {} >/dev/null 2>&1",
        shell_quote(path),
        shell_quote(&parent),
    );
    let _ = Command::new("sh").arg("-c").arg(script).spawn();
}

/// Dispatch an action. Returns whether the launcher window should close.
pub fn do_action(a: &Action, env: &Env) -> bool {
    match a {
        Action::Launch(e) => {
            launch(e);
            true
        }
        Action::Copy(t) => {
            copy_text(t);
            true
        }
        Action::ClipCopy(l) => {
            clip_copy(l);
            true
        }
        Action::CopyImage { path, mime } => {
            copy_image(path, mime);
            true
        }
        // Clipboard mutations are handled GUI-side (they need the store handle).
        Action::ClipDelete(_) | Action::ClipPin(_) | Action::ClipClear => false,
        // Settings action handled GUI-side.
        Action::OpenConfig => false,
        Action::OpenUrl(u) => {
            open(u);
            true
        }
        Action::RunShell(c) => {
            run_shell(c);
            true
        }
        Action::RunInTerminal(c) => {
            run_in_terminal(c, env);
            true
        }
        Action::OpenFile(p) => {
            open(p);
            true
        }
        Action::RevealFile(p) => {
            reveal(p);
            true
        }
        // Setting a target keeps the window open; the GUI handles the state update.
        Action::SetTarget(_) => false,
        // These are GUI-side view changes; the window stays open.
        Action::SetQuery(_) => false,
        Action::EnterMode { .. } => false,
        Action::AddQuicklink { .. } => false,
        Action::SetConfig { .. } => false,
        // Signals keep the window open; the GUI runs the kill then re-queries.
        Action::Signal { .. } => false,
        // Refresh is a GUI-side registry refresh + re-query.
        Action::Refresh => false,
        Action::None => false,
    }
}

/// Minimal single-quote shell escaping.
pub fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}
