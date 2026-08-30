use crate::db::{self, Database};
use crate::parser;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
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

/// How an email body should be rendered in the viewer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFormat {
    /// A `.html` body, rendered as-is in the iframe
    Html,
    /// A `.txt` body from a newsletter that ships no HTML part, wrapped for display
    Text,
}

impl BodyFormat {
    /// Value stored in `emails.body_format`
    pub fn as_str(self) -> &'static str {
        match self {
            BodyFormat::Html => "html",
            BodyFormat::Text => "text",
        }
    }
}

/// One indexable email — that is, **one body file**, not one directory.
///
/// A directory usually holds a single email, but historically a pair of messages whose
/// truncated IDs collided were written into the same directory. Treating the directory
/// as the unit silently published one message's body under the other's headline and
/// dropped the loser entirely, so the unit here is the body file.
#[derive(Debug, Clone)]
pub struct ParsedEmail {
    /// Primary key: `"<label>/<message_id>"`. Label-scoped by construction, so two
    /// identically-named directories under different labels can never collide.
    pub uid: String,
    /// Full Gmail message ID when it can be determined (front matter > .html stem > dir).
    pub message_id: String,
    pub label: String,
    /// On-disk directory name. Kept separate from `uid` because it is the only thing
    /// that can rebuild the path to the HTML file, and it is not globally unique.
    pub dir_name: String,
    pub subject: String,
    pub from_addr: String,
    pub date: Option<String>,
    /// File rendered in the viewer: a `.html`, or a `.txt` for text-only newsletters
    pub body_filename: String,
    pub body_format: BodyFormat,
    pub md_filename: String,
    pub body_text: String,
}

impl ParsedEmail {
    /// The three columns that together identify one physical email on disk
    fn location(&self) -> (String, String, String) {
        (self.label.clone(), self.dir_name.clone(), self.body_filename.clone())
    }
}

/// Parsed metadata from one .md sidecar file
struct MdMeta {
    /// Absolute path (stored in `md_filename`, matching prior behaviour)
    path: String,
    /// File name on disk, used for the `<slug>_<message_id>.md` matching fallback
    filename: String,
    /// Full message ID from the `id:` front-matter field, when present
    fm_id: Option<String>,
    subject: String,
    from_addr: String,
    date: Option<String>,
    body_text: String,
}

/// Read/bookmark state carried across a re-index.
///
/// Every key here is label-scoped, which is what makes cross-label bleed impossible:
/// there is no lookup path that can reach a row belonging to a different label.
#[derive(Default)]
pub struct ExistingState {
    /// Primary lookup, by composite uid
    by_uid: HashMap<String, (bool, bool)>,
    /// Fallback lookup by (label, dir_name, body_filename), so state survives a change in
    /// how the uid is derived — e.g. front matter gaining an `id:` field promotes a uid
    /// from `Label/<dir>` to `Label/<full_message_id>` while the files stay put.
    by_location: HashMap<(String, String, String), (bool, bool)>,
}

impl ExistingState {
    /// Snapshot the current user state before the tables are cleared
    pub fn load(conn: &Connection) -> Result<Self, String> {
        let mut state = ExistingState::default();
        let mut stmt = conn
            .prepare("SELECT id, label, dir_name, body_filename, is_read, is_bookmarked FROM emails")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, bool>(5)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        for (uid, label, dir_name, body_filename, is_read, is_bookmarked) in rows.flatten() {
            state.by_uid.insert(uid, (is_read, is_bookmarked));
            state
                .by_location
                .insert((label, dir_name, body_filename), (is_read, is_bookmarked));
        }
        Ok(state)
    }

    /// Restore `(is_read, is_bookmarked)` for a freshly scanned email
    pub fn lookup(&self, email: &ParsedEmail) -> (bool, bool) {
        self.by_uid
            .get(&email.uid)
            .or_else(|| self.by_location.get(&email.location()))
            .copied()
            .unwrap_or((false, false))
    }
}

/// Build the primary key for an email.
///
/// The label prefix is what makes the key robust to directory naming: even if every
/// directory on disk were named `email`, rows would still be distinct per label, and the
/// per-directory suffix keeps them distinct within a label.
pub fn make_uid(label: &str, message_id: &str) -> String {
    format!("{}/{}", label, message_id)
}

/// Whether a string looks like a Gmail message ID (hex, at least the legacy 8 chars).
///
/// Used to decide whether a body filename stem is meaningful identity or just a
/// generic name like `email.html`.
fn looks_like_message_id(s: &str) -> bool {
    s.len() >= 8 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// File name minus its extension
fn stem_of(filename: &str) -> &str {
    filename.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(filename)
}

/// Walk the newsletters tree and parse every email found, without touching the database.
///
/// Iteration is fully sorted at every level (labels, directories, files) so the result is
/// byte-for-byte identical across machines and repeated scans — `read_dir` order is
/// filesystem-dependent and was previously letting an arbitrary .html file win.
pub fn scan_newsletters(base: &Path) -> Result<Vec<ParsedEmail>, String> {
    let mut label_names: Vec<String> = fs::read_dir(base)
        .map_err(|e| format!("Cannot read directory: {}", e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with('.'))
        .collect();
    label_names.sort();

    let mut emails: Vec<ParsedEmail> = Vec::new();
    // Guards against two scanned emails claiming the same primary key. With correct data
    // this never fires; it exists so bad data degrades to a logged warning rather than a
    // silent overwrite.
    let mut seen_uids: HashSet<String> = HashSet::new();

    for label_name in &label_names {
        let label_path = base.join(label_name);

        let mut dir_names: Vec<String> = match fs::read_dir(&label_path) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|name| !name.starts_with('.'))
                .collect(),
            Err(e) => {
                warn!("Cannot read label dir {}: {}", label_name, e);
                continue;
            }
        };
        dir_names.sort();

        for dir_name in &dir_names {
            scan_email_dir(
                label_name,
                dir_name,
                &label_path.join(dir_name),
                &mut seen_uids,
                &mut emails,
            );
        }
    }

    Ok(emails)
}

/// Parse a single email directory, emitting one `ParsedEmail` per body file it contains.
fn scan_email_dir(
    label: &str,
    dir_name: &str,
    dir_path: &Path,
    seen_uids: &mut HashSet<String>,
    out: &mut Vec<ParsedEmail>,
) {
    let Ok(entries) = fs::read_dir(dir_path) else {
        warn!("Cannot read email dir {}/{}", label, dir_name);
        return;
    };

    let mut html_files: Vec<String> = Vec::new();
    let mut txt_files: Vec<String> = Vec::new();
    let mut md_files: Vec<String> = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".html") {
            html_files.push(name);
        } else if lower.ends_with(".txt") {
            txt_files.push(name);
        } else if lower.ends_with(".md") {
            md_files.push(name);
        }
    }
    // Deterministic selection: never depend on filesystem enumeration order.
    html_files.sort();
    txt_files.sort();
    md_files.sort();

    if html_files.is_empty() && txt_files.is_empty() {
        return; // Nothing renderable here
    }
    if html_files.len() > 1 {
        // Not an error — a colliding pair of messages can share a directory. Index each
        // one rather than picking a winner, but say so, because it means upstream wrote
        // two messages into one folder.
        info!(
            "{}/{}: {} .html files present, indexing each separately: {:?}",
            label, dir_name, html_files.len(), html_files
        );
    }

    // Parse each sidecar once, then match them to bodies below.
    let mds: Vec<MdMeta> = md_files
        .iter()
        .map(|name| parse_md(dir_path.join(name), name, label, dir_name))
        .collect();

    // Pass 1 — every .html is a body.
    let mut bodies: Vec<ResolvedBody> = Vec::new();
    let html_pairing = html_files.len() == 1 && mds.len() == 1;
    for filename in &html_files {
        let (md, message_id) = resolve_body(
            filename, &mds, html_pairing, dir_name, html_files.len(), label,
        );
        bodies.push(ResolvedBody {
            filename: filename.clone(),
            format: BodyFormat::Html,
            md,
            message_id,
        });
    }

    // Pass 2 — promote .txt files for messages that have no HTML body.
    //
    // Nearly every .txt is just the plain-text alternative of an .html sitting beside it
    // (verified across the whole archive), so indexing them unconditionally would double
    // every email. A .txt earns its own row only when the *message* it names isn't already
    // covered: always when the directory has no HTML at all — some newsletters ship no
    // HTML part, which previously made them invisible — and otherwise only when its stem
    // is a message ID that no HTML body resolved to, which is the shape a colliding pair
    // would take if one member arrived without HTML.
    let covered: HashSet<String> = bodies
        .iter()
        .flat_map(|b| [stem_of(&b.filename).to_string(), b.message_id.clone()])
        .collect();
    let text_candidates: Vec<&String> = txt_files
        .iter()
        .filter(|name| {
            let stem = stem_of(name);
            if html_files.is_empty() {
                true
            } else if looks_like_message_id(stem) && !covered.contains(stem) {
                info!(
                    "{}/{}: {} names a message with no HTML body, indexing as plain text",
                    label, dir_name, name
                );
                true
            } else {
                false
            }
        })
        .collect();

    let text_pairing = html_files.is_empty() && text_candidates.len() == 1 && mds.len() == 1;
    let text_count = text_candidates.len();
    for filename in text_candidates {
        let (md, message_id) = resolve_body(
            filename, &mds, text_pairing, dir_name, text_count, label,
        );
        bodies.push(ResolvedBody {
            filename: filename.clone(),
            format: BodyFormat::Text,
            md,
            message_id,
        });
    }

    // Pass 3 — assign keys and emit.
    for body in bodies {
        let md = body.md.map(|i| &mds[i]);

        let mut uid = make_uid(label, &body.message_id);
        if !seen_uids.insert(uid.clone()) {
            // Two emails derived the same key. Fall back to the physical location, which
            // the filesystem guarantees is unique, so neither email is lost.
            let fallback = format!("{}/{}#{}", label, dir_name, body.filename);
            warn!(
                "Duplicate email key {:?} ({}/{}); falling back to {:?}",
                uid, label, dir_name, fallback
            );
            if !seen_uids.insert(fallback.clone()) {
                warn!("Skipping {}/{}/{}: key still not unique", label, dir_name, body.filename);
                continue;
            }
            uid = fallback;
        }

        out.push(ParsedEmail {
            uid,
            message_id: body.message_id,
            label: label.to_string(),
            dir_name: dir_name.to_string(),
            subject: md.map(|m| m.subject.clone()).unwrap_or_default(),
            from_addr: md.map(|m| m.from_addr.clone()).unwrap_or_default(),
            date: md.and_then(|m| m.date.clone()),
            body_filename: body.filename,
            body_format: body.format,
            md_filename: md.map(|m| m.path.clone()).unwrap_or_default(),
            body_text: md.map(|m| m.body_text.clone()).unwrap_or_default(),
        });
    }
}

/// A body file matched to its metadata, before a key is assigned
struct ResolvedBody {
    filename: String,
    format: BodyFormat,
    /// Index into the directory's parsed .md files
    md: Option<usize>,
    message_id: String,
}

/// Match one body file to its .md sidecar and work out the message ID it belongs to.
///
/// `single_pairing` says whether this body is the only one of its kind next to exactly one
/// sidecar; `siblings` is how many bodies of its kind the directory holds, which decides
/// the shape of the last-resort key.
fn resolve_body(
    filename: &str,
    mds: &[MdMeta],
    single_pairing: bool,
    dir_name: &str,
    siblings: usize,
    label: &str,
) -> (Option<usize>, String) {
    let stem = stem_of(filename);
    let stem_is_id = looks_like_message_id(stem);

    // Match a body to its metadata, most-specific rule first:
    //   1. front matter `id:` equals the body's stem — exact, and the only rule that
    //      can safely disambiguate two messages sharing a directory
    //   2. the .md filename embeds that ID (`<slug>_<message_id>.md`), only trusted
    //      when the stem actually looks like an ID
    //   3. one body + one sidecar in the directory — unambiguous by elimination,
    //      and how legacy `email.html` layouts pair up
    let md = mds
        .iter()
        .position(|m| m.fm_id.as_deref() == Some(stem))
        .or_else(|| {
            if stem_is_id {
                mds.iter().position(|m| m.filename.contains(stem))
            } else {
                None
            }
        })
        .or_else(|| if single_pairing { Some(0) } else { None });

    if md.is_none() {
        warn!(
            "{}/{}: no .md metadata matched {} — indexing body without metadata",
            label, dir_name, filename
        );
    }

    // Identity, best available source first. Every fallback is unique within a label:
    // directory names are unique within a label, and file names within a directory.
    let message_id = md
        .and_then(|i| mds[i].fm_id.clone())
        .or_else(|| stem_is_id.then(|| stem.to_string()))
        .unwrap_or_else(|| {
            if siblings == 1 {
                dir_name.to_string()
            } else {
                format!("{}-{}", dir_name, stem)
            }
        });

    (md, message_id)
}

/// Read and parse one .md sidecar. Parse failures degrade to empty metadata (logged)
/// rather than dropping the email, which would hide it from the archive entirely.
fn parse_md(path: std::path::PathBuf, filename: &str, label: &str, dir_name: &str) -> MdMeta {
    let mut meta = MdMeta {
        path: path.to_string_lossy().to_string(),
        filename: filename.to_string(),
        fm_id: None,
        subject: String::new(),
        from_addr: String::new(),
        date: None,
        body_text: String::new(),
    };

    let Ok(content) = fs::read_to_string(&path) else {
        warn!("Cannot read {}/{}/{}", label, dir_name, filename);
        return meta;
    };

    match parser::parse_frontmatter(&content) {
        Ok((fm, body)) => {
            meta.fm_id = fm.id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            meta.subject = fm.subject;
            meta.from_addr = fm.from;
            meta.date = fm.date.and_then(|d| parser::normalize_date(&d));
            meta.body_text = truncate_body(body);
        }
        Err(e) => {
            warn!("Parse error for {}/{}/{}: {}", label, dir_name, filename, e);
        }
    }

    meta
}

/// Truncate the plaintext body for FTS indexing, respecting UTF-8 char boundaries
/// (smart quotes in newsletters make a naive `&body[..2000]` panic).
fn truncate_body(body: String) -> String {
    const MAX: usize = 2000;
    if body.len() <= MAX {
        return body;
    }
    let mut end = MAX;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    body[..end].to_string()
}

/// Insert one batch of emails plus their FTS rows, inside a single transaction.
///
/// Uses a plain `INSERT` rather than `INSERT OR REPLACE`: with keys now unique by
/// construction, a conflict means something is genuinely wrong, and quietly replacing a
/// row is exactly the behaviour that corrupted the index before. A conflicting row is
/// logged and skipped so one bad directory can't abort a 17k-email scan.
pub fn insert_chunk(
    conn: &Connection,
    chunk: &[ParsedEmail],
    existing: &ExistingState,
) -> Result<usize, String> {
    conn.execute_batch("BEGIN TRANSACTION;").map_err(|e| e.to_string())?;
    let mut inserted = 0usize;

    for email in chunk {
        let (is_read, is_bookmarked) = existing.lookup(email);

        let result = conn.execute(
            "INSERT INTO emails (id, label, dir_name, message_id, subject, from_addr, body, date, body_filename, body_format, md_filename, is_read, is_bookmarked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                email.uid, email.label, email.dir_name, email.message_id,
                email.subject, email.from_addr, email.body_text, email.date,
                email.body_filename, email.body_format.as_str(), email.md_filename,
                is_read as i32, is_bookmarked as i32,
            ],
        );

        if let Err(e) = result {
            warn!("Skipping {} ({}): {}", email.uid, email.body_filename, e);
            continue;
        }

        // The row we just wrote — no lookup by id needed, and no chance of resolving to
        // some other row.
        let rowid = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO emails_fts (rowid, subject, body, from_addr, label) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![rowid, email.subject, email.body_text, email.from_addr, email.label],
        ).map_err(|e| format!("FTS insert error: {}", e))?;

        inserted += 1;
    }

    conn.execute_batch("COMMIT;").map_err(|e| e.to_string())?;
    Ok(inserted)
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
    let parsed_emails = scan_newsletters(base)?;

    let total = parsed_emails.len();
    let label_count = parsed_emails
        .iter()
        .map(|e| e.label.as_str())
        .collect::<HashSet<_>>()
        .len();
    info!("Parsed {} emails across {} labels, starting DB insert", total, label_count);

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
        let existing_state = ExistingState::load(&conn)?;

        // Drop and recreate the FTS5 table (external content FTS can't be simply DELETEd)
        db::reset_index_tables(&conn)?;

        // Batch insert — all data is already parsed, so this is pure DB writes (fast)
        let batch_size = 500;
        let mut indexed = 0;

        for (chunk_idx, chunk) in parsed_emails.chunks(batch_size).enumerate() {
            indexed += insert_chunk(&conn, chunk, &existing_state)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // --- Test fixtures -----------------------------------------------------------

    /// Throwaway newsletters tree on disk, removed when the test ends.
    /// Hand-rolled rather than pulling in `tempfile`, which isn't a dependency here
    /// (the build must stay resolvable offline).
    struct TempTree(std::path::PathBuf);

    impl TempTree {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "newsletters-scan-test-{}-{}-{}",
                std::process::id(),
                tag,
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempTree(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        /// Create `<label>/<dir_name>/` and write the given files into it
        fn email_dir(&self, label: &str, dir_name: &str, files: &[(&str, String)]) {
            let dir = self.0.join(label).join(dir_name);
            fs::create_dir_all(&dir).unwrap();
            for (name, content) in files {
                fs::write(dir.join(name), content).unwrap();
            }
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A .md sidecar in the shape gmail-ingestor emits (quoted id, per its LEARNINGS)
    fn md_with_id(id: &str, subject: &str) -> String {
        format!(
            "---\nid: \"{id}\"\nsubject: \"{subject}\"\nfrom: \"Sender <s@example.com>\"\nto: \"\"\ndate: 2024-01-15 10:00:00\nlabels: [\"INBOX\"]\n---\nBody text for {subject}.\n"
        )
    }

    /// A legacy sidecar with no `id:` field at all
    fn md_without_id(subject: &str) -> String {
        format!(
            "---\nsubject: \"{subject}\"\nfrom: \"Sender <s@example.com>\"\ndate: 2024-01-15 10:00:00\n---\nBody text for {subject}.\n"
        )
    }

    /// In-memory database built from the production schema constants
    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(db::EMAILS_TABLE_SQL).unwrap();
        conn.execute_batch(db::EMAILS_INDEX_SQL).unwrap();
        conn.execute_batch(db::FTS_TABLE_SQL).unwrap();
        conn
    }

    fn find<'a>(emails: &'a [ParsedEmail], label: &str) -> &'a ParsedEmail {
        emails.iter().find(|e| e.label == label).expect("email for label")
    }

    // --- Regression: colliding directory names across labels ---------------------

    /// The core data-corruption regression. Two labels each hold a directory named
    /// `1786999a` — the shape produced by 8-char truncated message IDs, where the same
    /// name legitimately appeared under multiple labels. Keyed on the directory name
    /// alone, one row overwrote the other; both must now survive as distinct emails.
    #[test]
    fn colliding_dir_names_across_labels_index_as_separate_emails() {
        let tree = TempTree::new("collision");
        tree.email_dir("Alpha", "1786999a", &[
            ("1786999a3f2b1c4d.html", "<p>Alpha body</p>".to_string()),
            ("alpha-post_1786999a3f2b1c4d.md", md_with_id("1786999a3f2b1c4d", "Alpha Subject")),
        ]);
        tree.email_dir("Beta", "1786999a", &[
            ("1786999abc9d0e1f.html", "<p>Beta body</p>".to_string()),
            ("beta-post_1786999abc9d0e1f.md", md_with_id("1786999abc9d0e1f", "Beta Subject")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 2, "both colliding directories must be indexed");

        let alpha = find(&emails, "Alpha");
        let beta = find(&emails, "Beta");
        assert_ne!(alpha.uid, beta.uid, "keys must differ across labels");
        assert_eq!(alpha.uid, "Alpha/1786999a3f2b1c4d");
        assert_eq!(beta.uid, "Beta/1786999abc9d0e1f");
        assert_eq!(alpha.subject, "Alpha Subject");
        assert_eq!(beta.subject, "Beta Subject");
        // The on-disk directory is still recoverable for path building, collision and all.
        assert_eq!(alpha.dir_name, "1786999a");
        assert_eq!(beta.dir_name, "1786999a");

        // And they survive the round trip through the real insert path.
        let conn = test_conn();
        let inserted = insert_chunk(&conn, &emails, &ExistingState::default()).unwrap();
        assert_eq!(inserted, 2);

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM emails", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2, "neither row may overwrite the other");

        let subject: String = conn.query_row(
            "SELECT subject FROM emails WHERE label = 'Beta'", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(subject, "Beta Subject", "Beta must not carry Alpha's headline");
    }

    /// Same collision, but with no `id:` in the front matter, so the key falls back to the
    /// directory name. The label prefix alone has to keep the rows apart.
    #[test]
    fn colliding_dir_names_stay_distinct_without_front_matter_ids() {
        let tree = TempTree::new("collision-nofm");
        tree.email_dir("Alpha", "1786999a", &[
            ("email.html", "<p>Alpha body</p>".to_string()),
            ("post.md", md_without_id("Alpha Subject")),
        ]);
        tree.email_dir("Beta", "1786999a", &[
            ("email.html", "<p>Beta body</p>".to_string()),
            ("post.md", md_without_id("Beta Subject")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 2);
        assert_eq!(find(&emails, "Alpha").uid, "Alpha/1786999a");
        assert_eq!(find(&emails, "Beta").uid, "Beta/1786999a");

        let conn = test_conn();
        assert_eq!(insert_chunk(&conn, &emails, &ExistingState::default()).unwrap(), 2);
    }

    /// Read/bookmark state restored on re-index must not cross label boundaries.
    #[test]
    fn user_state_does_not_bleed_between_labels() {
        let tree = TempTree::new("state-bleed");
        tree.email_dir("Alpha", "1786999a", &[
            ("email.html", "<p>Alpha body</p>".to_string()),
            ("post.md", md_without_id("Alpha Subject")),
        ]);
        tree.email_dir("Beta", "1786999a", &[
            ("email.html", "<p>Beta body</p>".to_string()),
            ("post.md", md_without_id("Beta Subject")),
        ]);
        let emails = scan_newsletters(tree.path()).unwrap();

        // First index, then mark only Alpha's copy read + bookmarked.
        let conn = test_conn();
        insert_chunk(&conn, &emails, &ExistingState::default()).unwrap();
        conn.execute(
            "UPDATE emails SET is_read = 1, is_bookmarked = 1 WHERE label = 'Alpha'", [],
        ).unwrap();

        // Re-index exactly as the command does: snapshot state, clear, reinsert.
        let state = ExistingState::load(&conn).unwrap();
        assert_eq!(state.lookup(find(&emails, "Alpha")), (true, true));
        assert_eq!(
            state.lookup(find(&emails, "Beta")),
            (false, false),
            "Beta shares a directory name with Alpha but must keep its own state"
        );

        db::reset_index_tables(&conn).unwrap();
        insert_chunk(&conn, &emails, &state).unwrap();

        let beta: (bool, bool) = conn.query_row(
            "SELECT is_read, is_bookmarked FROM emails WHERE label = 'Beta'",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(beta, (false, false), "state bled across labels on re-index");

        let alpha: (bool, bool) = conn.query_row(
            "SELECT is_read, is_bookmarked FROM emails WHERE label = 'Alpha'",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(alpha, (true, true), "the owner's state must be preserved");
    }

    // --- HTML selection ----------------------------------------------------------

    /// A directory holding two colliding messages must yield two emails, each paired with
    /// its own metadata — previously one was published under the other's headline and the
    /// second was dropped.
    #[test]
    fn multiple_html_files_in_one_dir_are_all_indexed() {
        let tree = TempTree::new("multi-html");
        tree.email_dir("Alpha", "1786999a", &[
            ("1786999a3f2b1c4d.html", "<p>first</p>".to_string()),
            ("1786999abc9d0e1f.html", "<p>second</p>".to_string()),
            ("first-post_1786999a3f2b1c4d.md", md_with_id("1786999a3f2b1c4d", "First Subject")),
            ("second-post_1786999abc9d0e1f.md", md_with_id("1786999abc9d0e1f", "Second Subject")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 2, "no email may be silently discarded");

        let first = emails.iter().find(|e| e.message_id == "1786999a3f2b1c4d").unwrap();
        let second = emails.iter().find(|e| e.message_id == "1786999abc9d0e1f").unwrap();
        assert_eq!(first.subject, "First Subject");
        assert_eq!(second.subject, "Second Subject");
        assert_eq!(first.body_filename, "1786999a3f2b1c4d.html");
        assert_eq!(second.body_filename, "1786999abc9d0e1f.html");

        let conn = test_conn();
        assert_eq!(insert_chunk(&conn, &emails, &ExistingState::default()).unwrap(), 2);
    }

    /// Which .html wins must not depend on filesystem enumeration order.
    #[test]
    fn html_selection_is_sorted_and_stable() {
        let tree = TempTree::new("determinism");
        tree.email_dir("Alpha", "somedir", &[
            ("c.html", "<p>c</p>".to_string()),
            ("a.html", "<p>a</p>".to_string()),
            ("b.html", "<p>b</p>".to_string()),
        ]);

        let first_pass = scan_newsletters(tree.path()).unwrap();
        let second_pass = scan_newsletters(tree.path()).unwrap();

        let names: Vec<&str> = first_pass.iter().map(|e| e.body_filename.as_str()).collect();
        assert_eq!(names, vec!["a.html", "b.html", "c.html"], "files must be sorted");

        let uids: Vec<&str> = first_pass.iter().map(|e| e.uid.as_str()).collect();
        let uids_again: Vec<&str> = second_pass.iter().map(|e| e.uid.as_str()).collect();
        assert_eq!(uids, uids_again, "repeated scans must produce identical keys");

        // Non-ID stems still key uniquely within the label.
        assert_eq!(uids.iter().collect::<HashSet<_>>().len(), 3);
    }

    /// Labels and directories are walked in sorted order too, so progress and row order
    /// are reproducible across machines.
    #[test]
    fn labels_and_dirs_are_walked_in_sorted_order() {
        let tree = TempTree::new("sorted-walk");
        for (label, dir) in [("Zulu", "bbb"), ("Alpha", "zzz"), ("Alpha", "aaa")] {
            tree.email_dir(label, dir, &[("email.html", "<p>x</p>".to_string())]);
        }
        let emails = scan_newsletters(tree.path()).unwrap();
        let seen: Vec<(&str, &str)> = emails
            .iter()
            .map(|e| (e.label.as_str(), e.dir_name.as_str()))
            .collect();
        assert_eq!(seen, vec![("Alpha", "aaa"), ("Alpha", "zzz"), ("Zulu", "bbb")]);
    }

    // --- Identity resolution -----------------------------------------------------

    /// Front matter is the best source of identity: a legacy 8-char directory still keys
    /// on the full message ID, which is what makes the key survive a corpus rebuild.
    #[test]
    fn uid_prefers_full_message_id_from_front_matter() {
        let tree = TempTree::new("fm-id");
        tree.email_dir("Alpha", "1786999a", &[
            ("email.html", "<p>body</p>".to_string()),
            ("post_1786999a3f2b1c4d.md", md_with_id("1786999a3f2b1c4d", "Subject")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].uid, "Alpha/1786999a3f2b1c4d");
        assert_eq!(emails[0].message_id, "1786999a3f2b1c4d");
        assert_eq!(emails[0].dir_name, "1786999a", "path building still needs the directory");
    }

    /// With no front matter at all, the .html filename supplies the message ID.
    #[test]
    fn uid_falls_back_to_body_filename_stem() {
        let tree = TempTree::new("stem-id");
        tree.email_dir("Alpha", "1786999a", &[
            ("1786999a3f2b1c4d.html", "<p>body</p>".to_string()),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 1, "a body without metadata is still an email");
        assert_eq!(emails[0].uid, "Alpha/1786999a3f2b1c4d");
        assert_eq!(emails[0].subject, "", "no metadata available, but not dropped");
    }

    /// All-digit Gmail IDs must not be lost to YAML integer coercion
    /// (gmail-ingestor LEARNINGS: ~30 live IDs are all digits).
    #[test]
    fn numeric_unquoted_front_matter_id_is_still_read() {
        let tree = TempTree::new("numeric-id");
        let unquoted = "---\nid: 1637675546614607\nsubject: \"Numeric\"\nfrom: \"S <s@example.com>\"\n---\nBody.\n";
        tree.email_dir("Alpha", "somedir", &[
            ("email.html", "<p>body</p>".to_string()),
            ("post.md", unquoted.to_string()),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails[0].message_id, "1637675546614607");
        assert_eq!(emails[0].uid, "Alpha/1637675546614607");
    }

    #[test]
    fn directory_without_any_body_is_skipped() {
        let tree = TempTree::new("no-body");
        // Metadata with no body file at all — nothing to render.
        tree.email_dir("Alpha", "empty", &[("post.md", md_without_id("Orphan"))]);
        tree.email_dir("Alpha", "real", &[
            ("email.html", "<p>body</p>".to_string()),
            ("post.md", md_without_id("Real")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].dir_name, "real");
    }

    // --- Plain-text newsletters --------------------------------------------------

    /// Some newsletters ship no HTML part. Those directories hold only a .txt and a .md,
    /// and used to be skipped entirely — 400 emails invisible in the live archive.
    #[test]
    fn text_only_email_is_indexed() {
        let tree = TempTree::new("text-only");
        tree.email_dir("Quincy", "198f3e01cfede00e", &[
            ("198f3e01cfede00e.txt", "Plain text newsletter body.".to_string()),
            ("learn-devops_198f3e01cfede00e.md", md_with_id("198f3e01cfede00e", "Learn DevOps")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 1, "a text-only newsletter is still an email");
        assert_eq!(emails[0].body_format, BodyFormat::Text);
        assert_eq!(emails[0].body_filename, "198f3e01cfede00e.txt");
        assert_eq!(emails[0].subject, "Learn DevOps");
        assert_eq!(emails[0].uid, "Quincy/198f3e01cfede00e");
        assert!(!emails[0].body_text.is_empty(), "body must still be indexed for search");

        let conn = test_conn();
        assert_eq!(insert_chunk(&conn, &emails, &ExistingState::default()).unwrap(), 1);
        let format: String = conn.query_row(
            "SELECT body_format FROM emails", [], |r| r.get(0)).unwrap();
        assert_eq!(format, "text");
    }

    /// The common case: a .txt sitting next to an .html is the *same* message's plain-text
    /// alternative. Indexing it would double every email in the archive.
    #[test]
    fn text_alternative_beside_html_is_not_double_indexed() {
        let tree = TempTree::new("txt-alternative");
        tree.email_dir("Alpha", "19aa81c5f42e4d56", &[
            ("19aa81c5f42e4d56.html", "<p>html body</p>".to_string()),
            ("19aa81c5f42e4d56.txt", "same body as plain text".to_string()),
            ("post_19aa81c5f42e4d56.md", md_with_id("19aa81c5f42e4d56", "Subject")),
        ]);
        // Legacy generic naming, same situation.
        tree.email_dir("Alpha", "1786999a", &[
            ("email.html", "<p>html body</p>".to_string()),
            ("email.txt", "same body as plain text".to_string()),
            ("post.md", md_without_id("Legacy Subject")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 2, "each directory holds one email, not two");
        assert!(emails.iter().all(|e| e.body_format == BodyFormat::Html));
    }

    /// A colliding pair where only one member has an HTML part: the other's .txt names a
    /// message no HTML body covers, so it must be indexed rather than dropped.
    #[test]
    fn text_body_for_message_without_html_is_promoted() {
        let tree = TempTree::new("txt-uncovered");
        tree.email_dir("Alpha", "1786999a", &[
            ("1786999a3f2b1c4d.html", "<p>first, with html</p>".to_string()),
            ("1786999a3f2b1c4d.txt", "first, as text".to_string()),
            ("1786999abc9d0e1f.txt", "second, text only".to_string()),
            ("first_1786999a3f2b1c4d.md", md_with_id("1786999a3f2b1c4d", "First Subject")),
            ("second_1786999abc9d0e1f.md", md_with_id("1786999abc9d0e1f", "Second Subject")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 2, "the HTML-less message must not be dropped");

        let first = emails.iter().find(|e| e.message_id == "1786999a3f2b1c4d").unwrap();
        let second = emails.iter().find(|e| e.message_id == "1786999abc9d0e1f").unwrap();
        assert_eq!(first.body_format, BodyFormat::Html, "prefer HTML when the message has it");
        assert_eq!(first.body_filename, "1786999a3f2b1c4d.html");
        assert_eq!(second.body_format, BodyFormat::Text);
        assert_eq!(second.subject, "Second Subject");

        let conn = test_conn();
        assert_eq!(insert_chunk(&conn, &emails, &ExistingState::default()).unwrap(), 2);
    }

    /// The same message covered by an .html whose stem is generic: the .txt is named by
    /// the real message ID, but front matter proves they are one message.
    #[test]
    fn text_matching_the_html_message_id_is_not_promoted() {
        let tree = TempTree::new("txt-covered-by-fm");
        tree.email_dir("Alpha", "1786999a", &[
            ("email.html", "<p>body</p>".to_string()),
            ("1786999a3f2b1c4d.txt", "same body as text".to_string()),
            ("post.md", md_with_id("1786999a3f2b1c4d", "Subject")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 1, "front matter identifies both files as one message");
        assert_eq!(emails[0].body_format, BodyFormat::Html);
    }

    /// Text-only emails keep their state across a re-index like any other row.
    #[test]
    fn text_only_email_state_survives_reindex() {
        let tree = TempTree::new("text-state");
        tree.email_dir("Quincy", "198f3e01cfede00e", &[
            ("198f3e01cfede00e.txt", "Plain text.".to_string()),
            ("post_198f3e01cfede00e.md", md_with_id("198f3e01cfede00e", "Subject")),
        ]);
        let emails = scan_newsletters(tree.path()).unwrap();

        let conn = test_conn();
        insert_chunk(&conn, &emails, &ExistingState::default()).unwrap();
        conn.execute("UPDATE emails SET is_bookmarked = 1", []).unwrap();

        let state = ExistingState::load(&conn).unwrap();
        db::reset_index_tables(&conn).unwrap();
        insert_chunk(&conn, &emails, &state).unwrap();

        let bookmarked: bool = conn.query_row(
            "SELECT is_bookmarked FROM emails", [], |r| r.get(0)).unwrap();
        assert!(bookmarked, "bookmark on a text-only email must survive re-indexing");
    }

    #[test]
    fn hidden_directories_are_ignored() {
        let tree = TempTree::new("hidden");
        tree.email_dir(".git", "objects", &[("email.html", "<p>x</p>".to_string())]);
        tree.email_dir("Alpha", ".cache", &[("email.html", "<p>x</p>".to_string())]);
        tree.email_dir("Alpha", "real", &[("email.html", "<p>x</p>".to_string())]);

        let emails = scan_newsletters(tree.path()).unwrap();
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].dir_name, "real");
    }

    // --- FTS alignment -----------------------------------------------------------

    /// The FTS row must point at the email it was built from. This is where the old
    /// `SELECT rowid ... WHERE id = ?` lookup could resolve to the wrong row when ids
    /// collided, mismatching search snippets against headlines.
    #[test]
    fn fts_rows_align_with_their_emails() {
        let tree = TempTree::new("fts-align");
        tree.email_dir("Alpha", "1786999a", &[
            ("email.html", "<p>a</p>".to_string()),
            ("post.md", md_with_id("1786999a3f2b1c4d", "Alpha Unique Headline")),
        ]);
        tree.email_dir("Beta", "1786999a", &[
            ("email.html", "<p>b</p>".to_string()),
            ("post.md", md_with_id("1786999abc9d0e1f", "Beta Unique Headline")),
        ]);

        let emails = scan_newsletters(tree.path()).unwrap();
        let conn = test_conn();
        insert_chunk(&conn, &emails, &ExistingState::default()).unwrap();

        let (label, subject): (String, String) = conn.query_row(
            "SELECT e.label, e.subject FROM emails_fts
             JOIN emails e ON emails_fts.rowid = e.rowid
             WHERE emails_fts MATCH 'Beta'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(label, "Beta");
        assert_eq!(subject, "Beta Unique Headline");
    }

    // --- Helpers -----------------------------------------------------------------

    #[test]
    fn message_id_shape_detection() {
        assert!(looks_like_message_id("1786999a3f2b1c4d"));
        assert!(looks_like_message_id("1786999a"));
        assert!(!looks_like_message_id("email"), "generic names are not identity");
        assert!(!looks_like_message_id("abc"), "too short to be a message ID");
        assert!(!looks_like_message_id("newsletter-2024"));
    }

    #[test]
    fn body_truncation_respects_char_boundaries() {
        // A multi-byte char straddling the 2000-byte cut would panic on a naive slice.
        let body = format!("{}\u{201c}tail", "a".repeat(1999));
        let truncated = truncate_body(body);
        assert!(truncated.len() <= 2000);
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
