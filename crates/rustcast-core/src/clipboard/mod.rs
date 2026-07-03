//! Native clipboard history — a background `wl-paste --watch` daemon feeds a
//! SQLite store; this provider reads it. Replaces the cliphist-backed provider.

pub mod store;

use crate::model::{Action, Item, Prev, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;
use std::rc::Rc;
use store::{human_age, human_size, ClipRow, Store};

const CLIP_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "copy to clipboard" },
    ActionHint { keys: "⌃K", label: "delete · pin" },
    ActionHint { keys: "↑↓", label: "navigate" },
    ActionHint { keys: "esc", label: "close" },
];

pub struct ClipboardProvider {
    store: Option<Rc<Store>>,
}

impl ClipboardProvider {
    pub fn new(store: Option<Rc<Store>>) -> Self {
        ClipboardProvider { store }
    }
}

fn row_to_item(r: &ClipRow) -> Item {
    let is_image = r.kind == "image";
    let title = if is_image {
        r.preview.clone()
    } else {
        let t = r.text.lines().next().unwrap_or("").trim();
        if t.is_empty() {
            r.preview.clone()
        } else {
            t.chars().take(100).collect()
        }
    };
    let meta = format!(
        "{} · {} · {}{}",
        if is_image { "Image" } else { "Text" },
        human_size(r.bytes),
        human_age(r.ts),
        if r.pinned { " · pinned" } else { "" },
    );

    let action = if is_image {
        Action::CopyImage { path: r.blob_path.clone(), mime: r.mime.clone() }
    } else {
        Action::Copy(r.text.clone())
    };
    let prev = if is_image {
        Prev::File { path: r.blob_path.clone(), meta: meta.clone(), head: None }
    } else {
        Prev::File {
            path: String::new(),
            meta: meta.clone(),
            head: Some(r.text.clone()),
        }
    };
    // Image previews still want the picture; encode via ImagePath when we have one.
    let prev = if is_image && !r.blob_path.is_empty() {
        Prev::ImagePath(r.blob_path.clone())
    } else {
        prev
    };

    let mut actions = vec![
        SecondaryAction {
            label: if r.pinned { "Unpin".into() } else { "Pin".into() },
            action: Action::ClipPin(r.id),
        },
        SecondaryAction { label: "Delete".into(), action: Action::ClipDelete(r.id) },
        SecondaryAction { label: "Copy preview text".into(), action: Action::Copy(r.preview.clone()) },
    ];
    // Images can be run through OCR (tesseract) and the extracted text copied.
    if is_image && !r.blob_path.is_empty() {
        actions.push(SecondaryAction {
            label: "Extract text (OCR)".into(),
            action: Action::RunShell(ocr_command(&r.blob_path)),
        });
    }

    Item::new(
        title,
        meta,
        if is_image { "image-x-generic" } else { "edit-paste" },
        "clip",
        0,
        action,
    )
    .with_prev(prev)
    .with_actions(actions)
}

/// Shell command that OCRs `path` with tesseract and copies the result to the
/// clipboard (wl-copy or xclip). Best-effort — no-op if tesseract is absent.
fn ocr_command(path: &str) -> String {
    let p = crate::action::shell_quote(path);
    format!(
        "t=$(tesseract {p} stdout 2>/dev/null); \
         if command -v wl-copy >/dev/null 2>&1; then printf '%s' \"$t\" | wl-copy; \
         elif command -v xclip >/dev/null 2>&1; then printf '%s' \"$t\" | xclip -selection clipboard; fi"
    )
}

impl Provider for ClipboardProvider {
    fn id(&self) -> &'static str {
        "clipboard"
    }
    fn tab(&self) -> Tab {
        Tab::Clipboard
    }
    fn placeholder(&self) -> &'static str {
        "Search clipboard history…"
    }
    fn footer_hints(&self) -> &'static [ActionHint] {
        CLIP_HINTS
    }
    fn query(&self, ctx: &QueryCtx) -> Vec<Item> {
        let Some(store) = &self.store else {
            return vec![Item::new(
                "Clipboard unavailable",
                "could not open the history database",
                "dialog-warning",
                "clip",
                1,
                Action::None,
            )];
        };
        let rows = store.recent(400);
        let q = ctx.query;
        let mut out = Vec::new();
        for (rank, r) in rows.iter().enumerate() {
            let score = if q.is_empty() {
                let base = if r.pinned { 10_000 } else { 5_000 };
                base - rank as i64
            } else {
                let hay = if r.text.is_empty() { &r.preview } else { &r.text };
                match ctx.matcher.fuzzy_match(hay, q) {
                    Some(s) => s,
                    None => continue,
                }
            };
            let mut it = row_to_item(r);
            it.score = score;
            out.push(it);
        }
        out
    }
}

/// Sniff a MIME type from the first bytes of clipboard content.
pub fn sniff_mime(bytes: &[u8]) -> &'static str {
    let b = bytes;
    if b.len() >= 8 && &b[0..8] == b"\x89PNG\r\n\x1a\n" {
        "image/png"
    } else if b.len() >= 3 && &b[0..3] == b"\xff\xd8\xff" {
        "image/jpeg"
    } else if b.len() >= 6 && (&b[0..6] == b"GIF87a" || &b[0..6] == b"GIF89a") {
        "image/gif"
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else if b.len() >= 2 && &b[0..2] == b"BM" {
        "image/bmp"
    } else {
        "text/plain"
    }
}

/// Ingest one clipboard payload from stdin bytes (called by `--clip-ingest`).
pub fn ingest(bytes: &[u8], max_entries: usize) -> anyhow::Result<()> {
    let mime = sniff_mime(bytes);
    let store = Store::open()?;
    store.insert(bytes, mime)?;
    store.prune(max_entries)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff() {
        assert_eq!(sniff_mime(b"\x89PNG\r\n\x1a\nrest"), "image/png");
        assert_eq!(sniff_mime(b"hello world"), "text/plain");
        assert_eq!(sniff_mime(b"\xff\xd8\xffdata"), "image/jpeg");
    }
}
