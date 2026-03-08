use crate::db::Database;
use crate::parser;
use rusqlite::params;
use std::fs;
use std::path::Path;
use tauri::{AppHandle, Emitter, State};
use log::{info, warn};
use serde::Serialize;

/// Progress event payload sent to the frontend during indexing
#[derive(Clone, Serialize)]
pub struct IndexProgress {
    pub current: usize,
    pub total: usize,
    pub label: String,
    pub phase: String, // "scanning" | "indexing" | "done"
}

/// Parsed email data ready for DB insertion
struct ParsedEmail {
    id: String,
    label: String,
    subject: String,
    from_addr: String,
    date: Option<String>,
    html_filename: String,
    md_filename: String,
    body_text: String,
}

/// Walk the newsletters directory and index all emails into SQLite + FTS5.
/// Emits "index-progress" events so the frontend can show a progress bar.
#[tauri::command]
pub async fn scan_and_index(
    app: AppHandle,
    db: State<'_, Database>,
) -> Result<usize, String> {
    use std::sync::atomic::Ordering;

    // Prevent concurrent indexing operations
    if crate::INDEXING_IN_PROGRESS.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err("Indexing already in progress".to_string());
    }

    // Ensure flag is cleared on exit (even on error)
    let _guard = scopeguard(|| {
        crate::INDEXING_IN_PROGRESS.store(false, Ordering::SeqCst);
    });

    let newsletters_path = db.get_setting("newsletters_path")?
        .ok_or("No newsletters path configured")?;
    let base_path = newsletters_path.clone();
    let base = Path::new(&base_path);

    if !base.exists() {
        return Err(format!("Path does not exist: {}", newsletters_path));
    }

    info!("Starting index of: {}", newsletters_path);

    let _ = app.emit("index-progress", IndexProgress {
        current: 0, total: 0, label: String::new(),
        phase: "scanning".to_string(),
    });

    // Phase 1: Scan directories and parse all files (NO DB lock held)
    // This does all the expensive I/O work without blocking other commands
    let mut parsed_emails: Vec<ParsedEmail> = Vec::new();

    let label_dirs: Vec<_> = fs::read_dir(base)
        .map_err(|e| format!("Cannot read directory: {}", e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .collect();

    for label_entry in &label_dirs {
        let label_name = label_entry.file_name().to_string_lossy().to_string();
        let label_path = label_entry.path();

        let email_dirs: Vec<_> = fs::read_dir(&label_path)
            .map_err(|e| format!("Cannot read label dir: {}", e))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .collect();

        for email_entry in email_dirs {
            let email_id = email_entry.file_name().to_string_lossy().to_string();
            let email_path = email_entry.path();

            let mut md_path: Option<String> = None;
            let mut html_filename: Option<String> = None;

            if let Ok(files) = fs::read_dir(&email_path) {
                for file in files.filter_map(|f| f.ok()) {
                    let name = file.file_name().to_string_lossy().to_string();
                    if name.ends_with(".md") && md_path.is_none() {
                        md_path = Some(file.path().to_string_lossy().to_string());
                    }
                    if name.ends_with(".html") && html_filename.is_none() {
                        html_filename = Some(name);
                    }
                }
            }

            let Some(html) = html_filename else { continue };

            // Parse the .md frontmatter now (while no lock is held)
            let mut subject = String::new();
            let mut from_addr = String::new();
            let mut date: Option<String> = None;
            let mut body_text = String::new();
            let md_filename = md_path.clone().unwrap_or_default();

            if let Some(ref md) = md_path {
                if let Ok(content) = fs::read_to_string(md) {
                    match parser::parse_frontmatter(&content) {
                        Ok((fm, body)) => {
                            subject = fm.subject;
                            from_addr = fm.from;
                            date = fm.date.and_then(|d| parser::normalize_date(&d));
                            // Truncate body safely at a char boundary
                            body_text = if body.len() > 2000 {
                                let mut end = 2000;
                                while !body.is_char_boundary(end) && end > 0 {
                                    end -= 1;
                                }
                                body[..end].to_string()
                            } else {
                                body
                            };
                        }
                        Err(e) => {
                            warn!("Parse error for {}/{}: {}", label_name, email_id, e);
                        }
                    }
                }
            }

            parsed_emails.push(ParsedEmail {
                id: email_id,
                label: label_name.clone(),
                subject,
                from_addr,
                date,
                html_filename: html,
                md_filename,
                body_text,
            });
        }
    }

    let total = parsed_emails.len();
    info!("Parsed {} emails across {} labels, starting DB insert", total, label_dirs.len());

    // Phase 2: DB operations — acquire lock, do fast in-memory inserts, release
    {
        let conn = match db.conn.try_lock() {
            Ok(c) => c,
            Err(std::sync::TryLockError::WouldBlock) => {
                db.conn.lock().map_err(|e| e.to_string())?
            }
            Err(e) => {
                return Err(format!("DB lock error: {}", e));
            }
        };

        // Load existing user state before clearing
        let mut existing_state: std::collections::HashMap<String, (bool, bool)> = std::collections::HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT id, is_read, is_bookmarked FROM emails")
                .map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            }).map_err(|e| e.to_string())?;
            for row in rows.flatten() {
                existing_state.insert(row.0, (row.1, row.2));
            }
        }

        // Drop and recreate the FTS5 table (external content FTS can't be simply DELETEd)
        conn.execute_batch("
            DROP TABLE IF EXISTS emails_fts;
            DELETE FROM emails;
            CREATE VIRTUAL TABLE emails_fts USING fts5(
                subject, body, label,
                content='emails', content_rowid='rowid',
                tokenize='porter unicode61'
            );
        ").map_err(|e| format!("Clear tables error: {}", e))?;

        // Batch insert — all data is already parsed, so this is pure DB writes (fast)
        let batch_size = 500;
        let mut indexed = 0;

        for (chunk_idx, chunk) in parsed_emails.chunks(batch_size).enumerate() {
            conn.execute_batch("BEGIN TRANSACTION;").map_err(|e| e.to_string())?;

            for email in chunk {
                let (is_read, is_bookmarked) = existing_state
                    .get(&email.id)
                    .copied()
                    .unwrap_or((false, false));

                conn.execute(
                    "INSERT OR REPLACE INTO emails (id, label, subject, from_addr, date, html_filename, md_filename, is_read, is_bookmarked)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        email.id, email.label, email.subject, email.from_addr, email.date,
                        email.html_filename, email.md_filename,
                        is_read as i32, is_bookmarked as i32,
                    ],
                ).map_err(|e| format!("Insert error: {}", e))?;

                // Insert into FTS5 index
                let rowid: i64 = conn.query_row(
                    "SELECT rowid FROM emails WHERE id = ?1",
                    params![email.id],
                    |row| row.get(0),
                ).map_err(|e| format!("Rowid lookup error: {}", e))?;

                conn.execute(
                    "INSERT INTO emails_fts (rowid, subject, body, label) VALUES (?1, ?2, ?3, ?4)",
                    params![rowid, email.subject, email.body_text, email.label],
                ).map_err(|e| format!("FTS insert error: {}", e))?;

                indexed += 1;
            }

            conn.execute_batch("COMMIT;").map_err(|e| e.to_string())?;

            if chunk_idx % 5 == 0 {
                info!("Batch {}: {}/{} emails inserted", chunk_idx, indexed, total);
            }

            let current_label = chunk.first().map(|e| e.label.clone()).unwrap_or_default();
            let _ = app.emit("index-progress", IndexProgress {
                current: indexed,
                total,
                label: current_label,
                phase: "indexing".to_string(),
            });
        }

        // Save timestamp while we still have the lock (via direct SQL)
        let now = chrono_now();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('last_indexed', ?1)",
            params![now],
        ).map_err(|e| e.to_string())?;

        info!("Indexing complete: {} emails", indexed);
    } // Lock released here

    let _ = app.emit("index-progress", IndexProgress {
        current: total, total,
        label: String::new(),
        phase: "done".to_string(),
    });

    Ok(total)
}

/// Simple RAII guard that runs a closure on drop
struct ScopeGuard<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() { f(); }
    }
}
fn scopeguard<F: FnOnce()>(f: F) -> ScopeGuard<F> {
    ScopeGuard(Some(f))
}

/// Simple timestamp without chrono dependency
fn chrono_now() -> String {
    use std::time::SystemTime;
    let since_epoch = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", since_epoch.as_secs())
}
