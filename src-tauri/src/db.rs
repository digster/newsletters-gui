use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;
use log::{info, warn};

/// Schema for the `emails` table.
///
/// ## Identity model
///
/// `id` is a **label-scoped composite key**, `"<label>/<message_id>"` — *not* the
/// on-disk directory name. Directory names are only unique within a label (and,
/// historically, not even that: they were 8-char truncations of the Gmail message ID,
/// which collide for messages delivered close together). Keying rows on a bare
/// directory name made `INSERT OR REPLACE` overwrite one newsletter with another and
/// let read/bookmark state bleed between unrelated emails.
///
/// The three identity-ish columns each have exactly one job:
///
/// - `id`          — primary key + the opaque handle handed to the frontend
/// - `dir_name`    — on-disk directory, used to rebuild the path to the HTML file
/// - `message_id`  — full Gmail message ID (front matter `id:`, else the .html
///                   filename stem, else the directory name)
///
/// Keep them separate: the moment a single value has to be both "globally unique row
/// identity" and "whatever the folder is called", the corruption comes back.
pub const EMAILS_TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS emails (
        id TEXT PRIMARY KEY,
        label TEXT NOT NULL,
        dir_name TEXT NOT NULL DEFAULT '',
        message_id TEXT NOT NULL DEFAULT '',
        subject TEXT NOT NULL DEFAULT '',
        from_addr TEXT NOT NULL DEFAULT '',
        body TEXT NOT NULL DEFAULT '',
        date TEXT,
        html_filename TEXT NOT NULL,
        md_filename TEXT,
        is_read INTEGER NOT NULL DEFAULT 0,
        is_bookmarked INTEGER NOT NULL DEFAULT 0
    );
";

/// Indexes for the `emails` table.
///
/// `idx_emails_identity` is the structural guard against the collision bug: one row per
/// physical HTML file on disk. It must be created *after* the `dir_name` backfill, or
/// legacy rows (all with `dir_name = ''`) would violate it.
pub const EMAILS_INDEX_SQL: &str = "
    CREATE INDEX IF NOT EXISTS idx_emails_label_date
        ON emails(label, date DESC);

    CREATE INDEX IF NOT EXISTS idx_emails_bookmarked
        ON emails(is_bookmarked) WHERE is_bookmarked = 1;

    CREATE UNIQUE INDEX IF NOT EXISTS idx_emails_identity
        ON emails(label, dir_name, html_filename);
";

/// Schema for the FTS5 external-content index.
///
/// Deliberately without `IF NOT EXISTS`: callers check for existence first, and the
/// re-index path drops and recreates it (external-content FTS5 tables can't be cleared
/// with a plain DELETE).
pub const FTS_TABLE_SQL: &str = "
    CREATE VIRTUAL TABLE emails_fts USING fts5(
        subject, body, from_addr, label,
        content='emails', content_rowid='rowid',
        tokenize='porter unicode61'
    );
";

/// Drop and recreate the derived index tables ahead of a full re-scan.
///
/// The FTS5 table is dropped rather than deleted: with `content='emails'` it shares
/// storage with `emails`, and a plain `DELETE FROM emails_fts` misbehaves.
pub fn reset_index_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS emails_fts;\nDELETE FROM emails;\n{}",
        FTS_TABLE_SQL
    ))
    .map_err(|e| format!("Clear tables error: {}", e))
}

/// Thread-safe database wrapper
pub struct Database {
    pub conn: Mutex<Connection>,
}

impl Database {
    /// Open (or create) the SQLite database and run migrations
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path)
            .map_err(|e| format!("Failed to open database: {}", e))?;

        // Performance pragmas for desktop use
        // journal_mode returns a result row, so must use query_row
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(|e| format!("Failed to set journal_mode: {}", e))?;
        conn.execute_batch("
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA cache_size = -64000;
        ").map_err(|e| format!("Failed to set pragmas: {}", e))?;

        let db = Database { conn: Mutex::new(conn) };
        db.migrate()?;
        Ok(db)
    }

    /// Create tables if they don't exist, then bring older databases up to date.
    fn migrate(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        conn.execute_batch(EMAILS_TABLE_SQL)
            .map_err(|e| format!("Failed to create tables: {}", e))?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT
            );
        ").map_err(|e| format!("Failed to create settings table: {}", e))?;

        // Migration: add body column to existing databases that lack it
        if !Self::has_column(&conn, "body") {
            conn.execute("ALTER TABLE emails ADD COLUMN body TEXT NOT NULL DEFAULT ''", [])
                .map_err(|e| format!("Failed to add body column: {}", e))?;
            info!("Migrated emails table: added body column");
        }

        // Migration: split identity into (id, dir_name, message_id).
        //
        // Pre-migration rows keyed `id` on the bare directory name. Promote them to the
        // composite form so a user's existing read/bookmark state survives the upgrade
        // instead of being reset. Order matters — dir_name must be captured from the old
        // id *before* the id itself is rewritten. The `instr(id, '/') = 0` guard makes the
        // rewrite idempotent (a label is a single path component, so it never contains '/').
        if !Self::has_column(&conn, "dir_name") {
            conn.execute_batch("
                ALTER TABLE emails ADD COLUMN dir_name TEXT NOT NULL DEFAULT '';
                ALTER TABLE emails ADD COLUMN message_id TEXT NOT NULL DEFAULT '';
                UPDATE emails SET dir_name = id, message_id = id WHERE dir_name = '';
                UPDATE emails SET id = label || '/' || id WHERE instr(id, '/') = 0;
            ").map_err(|e| format!("Failed to migrate email identity columns: {}", e))?;
            info!("Migrated emails table: label-scoped composite ids (dir_name, message_id)");
        }

        // Indexes come last: the unique identity index can only hold once dir_name is
        // populated. A pre-existing database that somehow carries duplicates shouldn't
        // block startup — the tables are a rebuildable cache, and the next scan fixes it.
        if let Err(e) = conn.execute_batch(EMAILS_INDEX_SQL) {
            warn!("Could not create email indexes (will retry after next re-index): {}", e);
        }

        // Create FTS5 virtual table if it doesn't exist
        // We check first because CREATE VIRTUAL TABLE IF NOT EXISTS isn't always reliable
        let has_fts: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='emails_fts'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_fts {
            conn.execute_batch(FTS_TABLE_SQL)
                .map_err(|e| format!("Failed to create FTS table: {}", e))?;
            info!("Created FTS5 table");
        }

        Ok(())
    }

    /// Whether the `emails` table already has the given column
    fn has_column(conn: &Connection, column: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('emails') WHERE name = ?1",
            params![column],
            |row| row.get(0),
        ).unwrap_or(false)
    }

    /// Get a setting value by key
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")
            .map_err(|e| e.to_string())?;
        let result = stmt.query_row(params![key], |row| row.get(0)).ok();
        Ok(result)
    }

    /// Set a setting value
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Get total email count
    pub fn email_count(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row("SELECT COUNT(*) FROM emails", [], |row| row.get(0))
            .map_err(|e| e.to_string())
    }

    /// Clear all emails and FTS data (used before re-indexing)
    pub fn clear_emails(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        reset_index_tables(&conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database created from the current schema constants
    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(EMAILS_TABLE_SQL).unwrap();
        conn.execute_batch(EMAILS_INDEX_SQL).unwrap();
        conn.execute_batch(FTS_TABLE_SQL).unwrap();
        conn
    }

    #[test]
    fn identity_index_rejects_duplicate_physical_emails() {
        let conn = fresh_conn();
        conn.execute(
            "INSERT INTO emails (id, label, dir_name, message_id, html_filename)
             VALUES ('Alpha/aaa', 'Alpha', '1786999a', 'aaa', 'email.html')",
            [],
        ).unwrap();

        // Same label + directory + file is the same physical email; a second row for it
        // would mean the index has double-counted one message.
        let err = conn.execute(
            "INSERT INTO emails (id, label, dir_name, message_id, html_filename)
             VALUES ('Alpha/bbb', 'Alpha', '1786999a', 'bbb', 'email.html')",
            [],
        );
        assert!(err.is_err(), "duplicate (label, dir_name, html_filename) must be rejected");

        // The same directory name under a *different* label is a different email entirely.
        conn.execute(
            "INSERT INTO emails (id, label, dir_name, message_id, html_filename)
             VALUES ('Beta/bbb', 'Beta', '1786999a', 'bbb', 'email.html')",
            [],
        ).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM emails", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn legacy_database_migrates_to_composite_ids() {
        let dir = std::env::temp_dir().join(format!("newsletters-db-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legacy.db");

        // Build a pre-migration database: id = bare directory name, no dir_name column.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("
                CREATE TABLE emails (
                    id TEXT PRIMARY KEY,
                    label TEXT NOT NULL,
                    subject TEXT NOT NULL DEFAULT '',
                    from_addr TEXT NOT NULL DEFAULT '',
                    date TEXT,
                    html_filename TEXT NOT NULL,
                    md_filename TEXT,
                    is_read INTEGER NOT NULL DEFAULT 0,
                    is_bookmarked INTEGER NOT NULL DEFAULT 0
                );
            ").unwrap();
            conn.execute(
                "INSERT INTO emails (id, label, subject, html_filename, is_read, is_bookmarked)
                 VALUES ('1786999a', 'Alpha', 'Old subject', 'email.html', 1, 1)",
                [],
            ).unwrap();
        }

        let db = Database::open(&path).unwrap();
        let conn = db.conn.lock().unwrap();
        let (id, dir_name, message_id, is_read, is_bookmarked): (String, String, String, bool, bool) =
            conn.query_row(
                "SELECT id, dir_name, message_id, is_read, is_bookmarked FROM emails",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            ).unwrap();

        assert_eq!(id, "Alpha/1786999a", "legacy id should be promoted to the composite form");
        assert_eq!(dir_name, "1786999a", "the on-disk directory must still be recoverable");
        assert_eq!(message_id, "1786999a");
        assert!(is_read && is_bookmarked, "user state must survive the migration");

        drop(conn);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
