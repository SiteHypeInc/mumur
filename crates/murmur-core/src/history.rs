//! Local SQLite history of dictations. Nothing leaves the machine.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub id: i64,
    /// Unix milliseconds.
    pub created_at: i64,
    pub app: Option<String>,
    pub category: String,
    pub raw: String,
    pub text: String,
    pub audio_ms: i64,
    pub stt_ms: i64,
    pub polish_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct NewEntry<'a> {
    pub app: Option<&'a str>,
    pub category: &'a str,
    pub raw: &'a str,
    pub text: &'a str,
    pub audio_ms: u64,
    pub stt_ms: u64,
    pub polish_ms: u64,
}

pub struct History {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS dictations (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    app        TEXT,
    category   TEXT NOT NULL,
    raw        TEXT NOT NULL,
    text       TEXT NOT NULL,
    audio_ms   INTEGER NOT NULL DEFAULT 0,
    stt_ms     INTEGER NOT NULL DEFAULT 0,
    polish_ms  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS dictations_created ON dictations(created_at DESC);
";

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl History {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn add(&self, e: &NewEntry, keep: usize) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO dictations (created_at, app, category, raw, text, audio_ms, stt_ms, polish_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![now_ms(), e.app, e.category, e.raw, e.text, e.audio_ms as i64, e.stt_ms as i64, e.polish_ms as i64],
        )?;
        let id = self.conn.last_insert_rowid();
        self.prune(keep)?;
        Ok(id)
    }

    pub fn prune(&self, keep: usize) -> Result<()> {
        self.conn.execute(
            "DELETE FROM dictations WHERE id NOT IN (SELECT id FROM dictations ORDER BY id DESC LIMIT ?1)",
            params![keep as i64],
        )?;
        Ok(())
    }

    /// Newest first. `query` filters by substring over raw and final text.
    pub fn list(&self, query: Option<&str>, limit: usize) -> Result<Vec<Entry>> {
        let pattern = query.map(|q| format!("%{}%", q.replace('%', "\\%").replace('_', "\\_")));
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, app, category, raw, text, audio_ms, stt_ms, polish_ms FROM dictations
             WHERE ?1 IS NULL OR text LIKE ?1 ESCAPE '\\' OR raw LIKE ?1 ESCAPE '\\'
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |r| {
            Ok(Entry {
                id: r.get(0)?,
                created_at: r.get(1)?,
                app: r.get(2)?,
                category: r.get(3)?,
                raw: r.get(4)?,
                text: r.get(5)?,
                audio_ms: r.get(6)?,
                stt_ms: r.get(7)?,
                polish_ms: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM dictations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM dictations", [])?;
        Ok(())
    }

    /// Total words dictated, for the "you've saved N minutes of typing" stat.
    pub fn total_words(&self) -> Result<i64> {
        let mut stmt = self.conn.prepare("SELECT text FROM dictations")?;
        let words = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|t| t.ok())
            .map(|t| t.split_whitespace().count() as i64)
            .sum();
        Ok(words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry<'a>(text: &'a str) -> NewEntry<'a> {
        NewEntry { app: Some("Slack"), category: "work", raw: text, text, ..Default::default() }
    }

    #[test]
    fn add_list_search_prune() {
        let h = History::in_memory().unwrap();
        for t in ["first note", "second 100% done", "third_thing"] {
            h.add(&entry(t), 10).unwrap();
        }
        let all = h.list(None, 10).unwrap();
        assert_eq!(all.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["third_thing", "second 100% done", "first note"]);
        assert_eq!(h.list(Some("100%"), 10).unwrap().len(), 1);
        assert_eq!(h.list(Some("_"), 10).unwrap().len(), 1);
        h.prune(2).unwrap();
        assert_eq!(h.list(None, 10).unwrap().len(), 2);
        assert_eq!(h.total_words().unwrap(), 4);
        h.delete(all[0].id).unwrap();
        assert_eq!(h.list(None, 10).unwrap().len(), 1);
    }
}
