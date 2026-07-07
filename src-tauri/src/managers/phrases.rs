//! DotFlow — the editable phrase LIBRARY (design §8: the user's own trigger → text-block inserts, the
//! Beeftext-simple model but fired during live dictation). This is the effectful shell (SQLite persistence
//! + CRUD) around the pure expansion core in `crate::dotflow::phrases`. It owns a cached, compiled
//! `PhraseTable` that the wedge (batch paste + streaming injection) reads live; the cache is rebuilt on any
//! edit so a newly-added phrase triggers on the very next dictation.

use anyhow::Result;
use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

use crate::dotflow::{starter_pack_phrases, Phrase, PhraseTable};

/// Resolve the wedge's compiled phrase table from managed state (the user's live library), falling back to
/// the built-in starter pack if the manager isn't up yet. Used by both the batch paste and the streaming
/// injection so a phrase edited in the UI triggers on the very next dictation.
pub fn wedge_table(app_handle: &AppHandle) -> Arc<PhraseTable> {
    app_handle
        .try_state::<Arc<PhraseManager>>()
        .map(|pm| pm.current_table())
        .unwrap_or_else(|| Arc::new(crate::dotflow::starter_pack()))
}

static MIGRATIONS: &[M] = &[M::up(
    "CREATE TABLE IF NOT EXISTS phrases (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        key TEXT NOT NULL,
        aliases TEXT NOT NULL DEFAULT '[]',
        expansion TEXT NOT NULL,
        sort_order INTEGER NOT NULL DEFAULT 0
    );",
)];

/// One editable phrase as the UI sees it: a dot-trigger `key` (typed `.key` / the primary trigger), zero+
/// spoken `aliases`, and the `expansion` text block it inserts.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PhraseRecord {
    pub id: i64,
    pub key: String,
    pub aliases: Vec<String>,
    pub expansion: String,
}

pub struct PhraseManager {
    db_path: PathBuf,
    /// Compiled table the wedge reads; swapped atomically on every edit so edits take effect immediately.
    cache: Mutex<Arc<PhraseTable>>,
}

impl PhraseManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        let db_path = crate::portable::app_data_dir(app_handle)?.join("phrases.db");
        let manager = Self {
            db_path,
            cache: Mutex::new(Arc::new(PhraseTable::default())),
        };
        manager.init_database()?;
        manager.seed_if_empty()?;
        manager.rebuild_cache()?;
        Ok(manager)
    }

    fn conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    fn init_database(&self) -> Result<()> {
        let mut conn = self.conn()?;
        let migrations = Migrations::new(MIGRATIONS.to_vec());
        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid phrase migrations");
        migrations.to_latest(&mut conn)?;
        Ok(())
    }

    /// First-run seed: fill an empty library with the starter pack so the user starts from examples they
    /// can edit or delete (never re-seeded once there is any row, so deletions stick).
    fn seed_if_empty(&self) -> Result<()> {
        let conn = self.conn()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM phrases", [], |r| r.get(0))?;
        if count == 0 {
            for (i, p) in starter_pack_phrases().into_iter().enumerate() {
                Self::insert_conn(&conn, &p.key, &p.aliases, &p.expansion, i as i64)?;
            }
        }
        Ok(())
    }

    /* ---------- read ---------- */

    pub fn list(&self) -> Result<Vec<PhraseRecord>> {
        Self::list_conn(&self.conn()?)
    }

    /// The compiled table for the wedge (cheap clone of an Arc; rebuilt only on edits).
    pub fn current_table(&self) -> Arc<PhraseTable> {
        Arc::clone(&self.cache.lock().unwrap())
    }

    /* ---------- write (each rebuilds the cache) ---------- */

    pub fn add(&self, key: String, aliases: Vec<String>, expansion: String) -> Result<PhraseRecord> {
        let conn = self.conn()?;
        let next_order: i64 = conn
            .query_row("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM phrases", [], |r| r.get(0))?;
        let id = Self::insert_conn(&conn, &key, &aliases, &expansion, next_order)?;
        self.rebuild_cache()?;
        Ok(PhraseRecord { id, key, aliases, expansion })
    }

    pub fn update(
        &self,
        id: i64,
        key: String,
        aliases: Vec<String>,
        expansion: String,
    ) -> Result<PhraseRecord> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE phrases SET key = ?1, aliases = ?2, expansion = ?3 WHERE id = ?4",
            params![key, serde_json::to_string(&aliases)?, expansion, id],
        )?;
        self.rebuild_cache()?;
        Ok(PhraseRecord { id, key, aliases, expansion })
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn()?.execute("DELETE FROM phrases WHERE id = ?1", params![id])?;
        self.rebuild_cache()?;
        Ok(())
    }

    fn rebuild_cache(&self) -> Result<()> {
        let phrases: Vec<Phrase> = self
            .list()?
            .into_iter()
            .map(|r| Phrase { key: r.key, aliases: r.aliases, expansion: r.expansion })
            .collect();
        *self.cache.lock().unwrap() = Arc::new(PhraseTable::new(&phrases));
        Ok(())
    }

    /* ---------- connection-level helpers (unit-testable without an AppHandle) ---------- */

    fn insert_conn(
        conn: &Connection,
        key: &str,
        aliases: &[String],
        expansion: &str,
        sort_order: i64,
    ) -> Result<i64> {
        conn.execute(
            "INSERT INTO phrases (key, aliases, expansion, sort_order) VALUES (?1, ?2, ?3, ?4)",
            params![key, serde_json::to_string(aliases)?, expansion, sort_order],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn list_conn(conn: &Connection) -> Result<Vec<PhraseRecord>> {
        let mut stmt =
            conn.prepare("SELECT id, key, aliases, expansion FROM phrases ORDER BY sort_order, id")?;
        let rows = stmt.query_map([], |row| {
            let aliases_json: String = row.get(2)?;
            let aliases: Vec<String> = serde_json::from_str(&aliases_json).unwrap_or_default();
            Ok(PhraseRecord {
                id: row.get(0)?,
                key: row.get(1)?,
                aliases,
                expansion: row.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        Migrations::new(MIGRATIONS.to_vec()).to_latest(&mut conn).unwrap();
        conn
    }

    #[test]
    fn insert_then_list_round_trips_aliases_and_order() {
        let conn = mem();
        PhraseManager::insert_conn(&conn, "copd", &["insert copd plan".into()], "COPD plan.", 0).unwrap();
        PhraseManager::insert_conn(&conn, "fu", &["insert follow up".into(), "insert fu".into()], "FU.", 1)
            .unwrap();
        let rows = PhraseManager::list_conn(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "copd");
        assert_eq!(rows[0].aliases, vec!["insert copd plan".to_string()]);
        assert_eq!(rows[1].aliases, vec!["insert follow up".to_string(), "insert fu".to_string()]);
        assert_eq!(rows[1].expansion, "FU.");
    }

    #[test]
    fn a_listed_phrase_compiles_into_a_table_that_expands() {
        let conn = mem();
        PhraseManager::insert_conn(&conn, "fu", &["insert follow up".into()], "Follow up in two weeks.", 0)
            .unwrap();
        let phrases: Vec<Phrase> = PhraseManager::list_conn(&conn)
            .unwrap()
            .into_iter()
            .map(|r| Phrase { key: r.key, aliases: r.aliases, expansion: r.expansion })
            .collect();
        let table = PhraseTable::new(&phrases);
        // the user's stored phrase drives real expansion (dot + spoken alias), punctuation-tolerant.
        assert_eq!(crate::dotflow::expand(".fu", &table), "Follow up in two weeks.");
        assert_eq!(crate::dotflow::expand("Insert follow up.", &table), "Follow up in two weeks.");
    }

    #[test]
    fn empty_library_has_no_triggers() {
        let conn = mem();
        let rows = PhraseManager::list_conn(&conn).unwrap();
        assert!(rows.is_empty());
    }
}
