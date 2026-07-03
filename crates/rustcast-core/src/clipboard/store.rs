//! SQLite-backed clipboard history. Text lives inline; images are written to
//! `blobs/<hash>.<ext>` and referenced by path so the DB stays small.

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ClipRow {
    pub id: i64,
    pub kind: String, // "text" | "image"
    pub mime: String,
    pub text: String,       // full text (empty for images)
    pub blob_path: String,  // absolute path (empty for text)
    pub preview: String,    // short one-line preview
    pub bytes: i64,
    pub ts: i64, // unix seconds
    pub pinned: bool,
}

pub struct Store {
    conn: Connection,
    blobs: PathBuf,
}

impl Store {
    /// Open (creating if needed) the clipboard DB under the data dir.
    pub fn open() -> anyhow::Result<Store> {
        let dir = crate::config::Config::data_dir()
            .ok_or_else(|| anyhow::anyhow!("no data dir"))?;
        std::fs::create_dir_all(&dir)?;
        let blobs = dir.join("blobs");
        std::fs::create_dir_all(&blobs)?;
        let conn = Connection::open(dir.join("clipboard.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_millis(3000))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS clips(
                id INTEGER PRIMARY KEY,
                kind TEXT NOT NULL,
                mime TEXT NOT NULL,
                hash TEXT UNIQUE NOT NULL,
                text TEXT NOT NULL DEFAULT '',
                blob_path TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                bytes INTEGER NOT NULL DEFAULT 0,
                ts INTEGER NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        Ok(Store { conn, blobs })
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Insert a new clipboard entry. Re-copying an identical value just bumps its
    /// timestamp (dedup by content hash).
    pub fn insert(&self, bytes: &[u8], mime: &str) -> anyhow::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let hash = hex::encode(Sha256::digest(bytes));
        let is_image = mime.starts_with("image/");
        let ts = Self::now();

        if is_image {
            let ext = mime.split('/').nth(1).unwrap_or("png");
            let path = self.blobs.join(format!("{hash}.{ext}"));
            if !path.exists() {
                std::fs::write(&path, bytes)?;
            }
            let preview = format!("Image · {} · {}", mime, human_size(bytes.len() as i64));
            self.conn.execute(
                "INSERT INTO clips(kind,mime,hash,text,blob_path,preview,bytes,ts)
                 VALUES('image',?1,?2,'',?3,?4,?5,?6)
                 ON CONFLICT(hash) DO UPDATE SET ts=?6",
                params![mime, hash, path.to_string_lossy(), preview, bytes.len() as i64, ts],
            )?;
        } else {
            let text = String::from_utf8_lossy(bytes).to_string();
            if text.trim().is_empty() {
                return Ok(());
            }
            let preview: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let preview: String = preview.chars().take(200).collect();
            self.conn.execute(
                "INSERT INTO clips(kind,mime,hash,text,blob_path,preview,bytes,ts)
                 VALUES('text',?1,?2,?3,'',?4,?5,?6)
                 ON CONFLICT(hash) DO UPDATE SET ts=?6",
                params!["text/plain", hash, text, preview, bytes.len() as i64, ts],
            )?;
        }
        Ok(())
    }

    /// Most-recent-first rows (pinned always first).
    pub fn recent(&self, limit: usize) -> Vec<ClipRow> {
        let mut stmt = match self.conn.prepare(
            "SELECT id,kind,mime,text,blob_path,preview,bytes,ts,pinned
             FROM clips ORDER BY pinned DESC, ts DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map(params![limit as i64], row_from).map(|r| r.flatten().collect());
        rows.unwrap_or_default()
    }

    pub fn get(&self, id: i64) -> Option<ClipRow> {
        self.conn
            .query_row(
                "SELECT id,kind,mime,text,blob_path,preview,bytes,ts,pinned FROM clips WHERE id=?1",
                params![id],
                row_from,
            )
            .ok()
    }

    pub fn delete(&self, id: i64) -> anyhow::Result<()> {
        if let Some(r) = self.get(id) {
            if !r.blob_path.is_empty() {
                let _ = std::fs::remove_file(&r.blob_path);
            }
        }
        self.conn.execute("DELETE FROM clips WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn toggle_pin(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .execute("UPDATE clips SET pinned = 1 - pinned WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        let _ = std::fs::remove_dir_all(&self.blobs);
        std::fs::create_dir_all(&self.blobs)?;
        self.conn.execute("DELETE FROM clips WHERE pinned=0", [])?;
        Ok(())
    }

    /// Evict oldest non-pinned rows beyond `max_entries`, unlinking their blobs.
    pub fn prune(&self, max_entries: usize) -> anyhow::Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM clips WHERE pinned=0 ORDER BY ts DESC LIMIT -1 OFFSET ?1",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![max_entries as i64], |r| r.get::<_, i64>(0))?
            .flatten()
            .collect();
        for id in ids {
            let _ = self.delete(id);
        }
        Ok(())
    }
}

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<ClipRow> {
    Ok(ClipRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        mime: r.get(2)?,
        text: r.get(3)?,
        blob_path: r.get(4)?,
        preview: r.get(5)?,
        bytes: r.get(6)?,
        ts: r.get(7)?,
        pinned: r.get::<_, i64>(8)? != 0,
    })
}

pub fn human_size(n: i64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut f = n as f64;
    let mut i = 0;
    while f >= 1024.0 && i < U.len() - 1 {
        f /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{f:.1} {}", U[i])
    }
}

pub fn human_age(ts: i64) -> String {
    let now = Store::now();
    let d = (now - ts).max(0);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => format!("{} h ago", d / 3600),
        _ => format!("{} d ago", d / 86400),
    }
}
