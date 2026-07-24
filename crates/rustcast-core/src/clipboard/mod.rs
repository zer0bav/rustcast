//! Native clipboard history — a background `wl-paste --watch` daemon feeds a
//! SQLite store; this provider reads it. Replaces the cliphist-backed provider.

pub mod store;

use crate::model::{Action, Item, Prev, SecondaryAction};
use crate::provider::{ActionHint, Provider, QueryCtx, Tab};
use fuzzy_matcher::FuzzyMatcher;
use std::rc::Rc;
use store::{human_age, human_size, ClipRow, Store};

const CLIP_HINTS: &[ActionHint] = &[
    ActionHint { keys: "↵", label: "copy" },
    ActionHint { keys: "⌃D", label: "delete" },
    ActionHint { keys: "⌃S", label: "pin" },
    ActionHint { keys: "⌃K", label: "actions" },
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

/// What kind of thing a text clip looks like, for the preview's Type row.
/// Purely cosmetic — a wrong guess costs nothing.
fn text_kind(t: &str) -> &'static str {
    let s = t.trim();
    if s.is_empty() {
        return "Text";
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return "Link";
    }
    if s.starts_with('/') && !s.contains('\n') && s.len() < 4096 && std::path::Path::new(s).exists()
    {
        return "Path";
    }
    if (s.starts_with('{') && s.ends_with('}')) || (s.starts_with('[') && s.ends_with(']')) {
        return "JSON";
    }
    if s.len() > 20 && !s.contains(char::is_whitespace) && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return "Hex";
    }
    if s.contains('\n') && s.lines().count() > 2 {
        return "Multiline text";
    }
    "Text"
}

/// Absolute local timestamp for the preview ("2026-07-24 09:53").
fn stamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()
        })
        .unwrap_or_default()
}

fn row_to_item(r: &ClipRow) -> Item {
    let is_image = r.kind == "image";
    let title = if is_image {
        r.preview.clone()
    } else {
        let t = r.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        if t.is_empty() {
            r.preview.clone()
        } else {
            t.chars().take(100).collect()
        }
    };
    let kind = if is_image { "Image" } else { text_kind(&r.text) };
    // One-line summary under the title in the list.
    let subtitle = format!(
        "{} · {} · {}{}",
        kind,
        human_size(r.bytes),
        human_age(r.ts),
        if r.pinned { " · pinned" } else { "" },
    );

    // Raycast-style metadata table under the preview body.
    let mut meta: Vec<(String, String)> = vec![("Type".into(), kind.into())];
    if is_image {
        meta.push(("Format".into(), r.mime.clone()));
    } else {
        let chars = r.text.chars().count();
        meta.push(("Characters".into(), chars.to_string()));
        meta.push(("Words".into(), r.text.split_whitespace().count().to_string()));
        let lines = r.text.lines().count();
        if lines > 1 {
            meta.push(("Lines".into(), lines.to_string()));
        }
    }
    meta.push(("Size".into(), human_size(r.bytes)));
    meta.push(("Copied".into(), format!("{} · {}", human_age(r.ts), stamp(r.ts))));
    if r.pinned {
        meta.push(("Pinned".into(), "yes".into()));
    }

    let action = if is_image {
        Action::CopyImage { path: r.blob_path.clone(), mime: r.mime.clone() }
    } else {
        Action::Copy(r.text.clone())
    };
    let prev = Prev::Rich {
        image: (is_image && !r.blob_path.is_empty()).then(|| r.blob_path.clone()),
        text: (!is_image).then(|| r.text.clone()),
        meta,
    };

    let mut actions = vec![
        SecondaryAction {
            label: if r.pinned { "Unpin  ⌃S".into() } else { "Pin  ⌃S".into() },
            action: Action::ClipPin(r.id),
        },
        SecondaryAction { label: "Delete  ⌃D".into(), action: Action::ClipDelete(r.id) },
    ];
    if !is_image {
        actions.push(SecondaryAction {
            label: "Copy as single line".into(),
            action: Action::Copy(r.preview.clone()),
        });
    }
    // Images can be run through OCR (tesseract) and the extracted text copied.
    if is_image && !r.blob_path.is_empty() {
        actions.push(SecondaryAction {
            label: "Extract text (OCR)".into(),
            action: Action::RunShell(ocr_command(&r.blob_path)),
        });
        actions.push(SecondaryAction {
            label: "Open image".into(),
            action: Action::OpenFile(r.blob_path.clone()),
        });
    }
    actions.push(SecondaryAction {
        label: "Clear history (keeps pinned)".into(),
        action: Action::ClipClear,
    });

    Item::new(
        title,
        subtitle,
        if is_image { "image-x-generic" } else { "edit-paste" },
        if is_image { "image" } else { "text" },
        0,
        action,
    )
    .in_section(if r.pinned {
        crate::registry::section::PINNED
    } else {
        crate::registry::section::HISTORY
    })
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
        let q = ctx.query.trim().to_lowercase();
        let mut out = Vec::new();
        for (rank, r) in rows.iter().enumerate() {
            // Recency is the clipboard's natural order, so it stays the base and
            // the query only filters/boosts. Pinned entries keep their own band.
            let recency = 4_000 - rank as i64;
            let score = if q.is_empty() {
                recency
            } else {
                let hay = if r.text.is_empty() { &r.preview } else { &r.text };
                let lc = hay.to_lowercase();
                // Substring first: clipboard search is "find that thing I copied",
                // where a literal hit is always what you meant. Fuzzy is the
                // fallback so a typo still finds something.
                match lc.find(&q) {
                    // Earlier hits rank higher; recency breaks ties.
                    Some(at) => 8_000 - (at.min(400) as i64) + recency / 10,
                    None => match ctx.matcher.fuzzy_match(&lc, &q) {
                        Some(s) => 2_000 + s.min(400) + recency / 20,
                        None => continue,
                    },
                }
            };
            let mut it = row_to_item(r);
            it.score = score + if r.pinned { 10_000 } else { 0 };
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
