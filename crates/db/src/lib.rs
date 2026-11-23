//! SQLite + FTS5 index: location, connection setup and schema versioning.
//!
//! The database always lives on a local disk under `<data>/<vault-hash>/index.db`,
//! never inside the vault: `<data>` is `$XDG_DATA_HOME/sage` on Linux and
//! `%LOCALAPPDATA%\sage` on Windows. WAL and file locking are unreliable on 9p
//! (`/mnt/c`), and keeping the index out of the vault stops OneDrive from
//! syncing it.
//!
//! Every connection replaces SQLite's `lower()` and `NOCASE`, which fold ASCII
//! letters only, with Unicode case folding: notes are written in every
//! language, and `file:żółw` must find `Żółw.md`. No index uses `NOCASE`, so
//! replacing it cannot leave one out of order.

use anyhow::{Context, Result};
use rusqlite::config::DbConfig;
use rusqlite::functions::FunctionFlags;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

pub use rusqlite;

pub const SCHEMA: &str = include_str!("schema.sql");
/// Bump on any schema change, and whenever the scanner starts producing
/// different rows. A mismatch rebuilds the index from the vault.
pub const SCHEMA_VERSION: i32 = 9;

/// Where the index for `vault` is stored.
pub fn index_path_for(vault: &Path) -> Result<PathBuf> {
    let vault = sage_watcher::canonical(vault)
        .with_context(|| format!("vault path {} is not accessible", vault.display()))?;
    let hash = blake3::hash(vault.as_os_str().as_encoded_bytes()).to_hex();
    let base = sage_watcher::data_dir().context("no folder for Sage's data: set HOME, or LOCALAPPDATA on Windows")?;
    Ok(base.join(&hash.as_str()[..16]).join("index.db"))
}

/// Opens the read-write connection. Exactly one of these exists per vault and it
/// belongs to the indexer thread.
pub fn open_writer(db_path: &Path) -> Result<Connection> {
    if let Some(dir) = db_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(db_path)?;
    configure(&conn)?;
    if user_version(&conn)? != SCHEMA_VERSION {
        // Disposable cache: emptied and built again. Emptied in place rather
        // than deleted: on Windows the UI's reader, open since the start,
        // keeps the file from being deleted.
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_RESET_DATABASE, true)?;
        conn.execute("VACUUM", [])?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_RESET_DATABASE, false)?;
        configure(&conn)?;
        conn.execute_batch(&format!(
            "BEGIN;\n{SCHEMA}\nPRAGMA user_version = {SCHEMA_VERSION};\nCOMMIT;"
        ))?;
    }
    Ok(conn)
}

/// Opens a read-only connection for the UI. Readers never block the writer in
/// WAL mode and see a consistent snapshot per statement.
pub fn open_reader(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_millis(250))?;
    conn.pragma_update(None, "query_only", true)?;
    fold_case(&conn)?;
    Ok(conn)
}

/// Unicode `lower()` and `NOCASE` in place of SQLite's ASCII-only ones.
fn fold_case(conn: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    conn.create_scalar_function("lower", 1, flags, |ctx| {
        Ok(match ctx.get_raw(0) {
            ValueRef::Text(text) => Value::Text(String::from_utf8_lossy(text).to_lowercase()),
            // As SQLite's own `lower()`, which returns text.
            ValueRef::Integer(n) => Value::Text(n.to_string()),
            ValueRef::Real(x) => Value::Real(x),
            ValueRef::Blob(bytes) => Value::Blob(bytes.to_vec()),
            ValueRef::Null => Value::Null,
        })
    })?;
    conn.create_collation("NOCASE", |a, b| a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase)))
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // Losing the last transaction on power failure is fine for a cache.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "cache_size", -32_000)?; // 32 MB
    fold_case(conn)?;
    Ok(())
}

fn user_version(conn: &Connection) -> Result<i32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Turns free text typed into OmniSearch into a safe FTS5 prefix query:
/// every word is quoted, the last one matches as a prefix.
pub fn fts_query(input: &str) -> Option<String> {
    let words: Vec<String> = input
        .split_whitespace()
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    if words.is_empty() {
        return None;
    }
    Some(format!("{}*", words.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_index_is_rebuilt_while_a_reader_has_it_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let old = Connection::open(&path).unwrap();
        old.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE meta (key TEXT PRIMARY KEY); PRAGMA user_version = 1;")
            .unwrap();
        drop(old);
        // The UI opens its reader at start, before the indexer opens the writer.
        let reader = open_reader(&path).unwrap();
        let writer = open_writer(&path).unwrap();
        assert_eq!(user_version(&writer).unwrap(), SCHEMA_VERSION);
        let files: i64 = reader.query_row("SELECT count(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(files, 0);
    }

    #[test]
    fn case_folds_beyond_ascii() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        drop(open_writer(&path).unwrap());
        let reader = open_reader(&path).unwrap();
        let text = |sql: &str| reader.query_row(sql, [], |r| r.get::<_, String>(0)).unwrap();
        assert_eq!(text("SELECT lower('ŻÓŁW Środa')"), "żółw środa");
        assert_eq!(text("SELECT lower(12)"), "12");
        let sorted = text(
            "SELECT group_concat(w, ' ') FROM (SELECT w FROM (SELECT 'Zebra' AS w UNION ALL SELECT 'łąka' \
             UNION ALL SELECT 'apple' UNION ALL SELECT 'Łódź' UNION ALL SELECT 'Banana') ORDER BY w COLLATE NOCASE)",
        );
        // Case folds; letters still go by code point, so `Ł` sorts after `z`.
        assert_eq!(sorted, "apple Banana Zebra Łódź łąka");
        let same: bool = reader.query_row("SELECT 'ŻÓŁW' = 'żółw' COLLATE NOCASE", [], |r| r.get(0)).unwrap();
        assert!(same);
    }
}
