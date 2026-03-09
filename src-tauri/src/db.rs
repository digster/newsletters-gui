use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;
use log::info;

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

    /// Create tables if they don't exist
    fn migrate(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS emails (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                subject TEXT NOT NULL DEFAULT '',
                from_addr TEXT NOT NULL DEFAULT '',
                body TEXT NOT NULL DEFAULT '',
                date TEXT,
                html_filename TEXT NOT NULL,
                md_filename TEXT,
                is_read INTEGER NOT NULL DEFAULT 0,
                is_bookmarked INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_emails_label_date
                ON emails(label, date DESC);

            CREATE INDEX IF NOT EXISTS idx_emails_bookmarked
                ON emails(is_bookmarked) WHERE is_bookmarked = 1;

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT
            );
        ").map_err(|e| format!("Failed to create tables: {}", e))?;

        // Migration: add body column to existing databases that lack it
        let has_body: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('emails') WHERE name='body'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);
        if !has_body {
            conn.execute("ALTER TABLE emails ADD COLUMN body TEXT NOT NULL DEFAULT ''", [])
                .map_err(|e| format!("Failed to add body column: {}", e))?;
            info!("Migrated emails table: added body column");
        }

        // Create FTS5 virtual table if it doesn't exist
        // We check first because CREATE VIRTUAL TABLE IF NOT EXISTS isn't always reliable
        let has_fts: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='emails_fts'",
            [],
            |row| row.get(0),
        ).unwrap_or(false);

        if !has_fts {
            conn.execute_batch("
                CREATE VIRTUAL TABLE emails_fts USING fts5(
                    subject, body, from_addr, label,
                    content='emails', content_rowid='rowid',
                    tokenize='porter unicode61'
                );
            ").map_err(|e| format!("Failed to create FTS table: {}", e))?;
            info!("Created FTS5 table");
        }

        Ok(())
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
        conn.execute_batch("
            DELETE FROM emails_fts;
            DELETE FROM emails;
        ").map_err(|e| format!("Failed to clear emails: {}", e))?;
        Ok(())
    }
}
