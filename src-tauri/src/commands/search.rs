use crate::db::Database;
use rusqlite::params;
use serde::Serialize;
use tauri::State;

/// Search result with snippet highlighting
#[derive(Serialize, Debug)]
pub struct SearchResult {
    pub id: String,
    pub label: String,
    pub subject: String,
    pub from_addr: String,
    pub date: Option<String>,
    pub snippet: String,
    pub is_read: bool,
    pub is_bookmarked: bool,
    pub rank: f64,
}

/// Full-text search across emails using FTS5 MATCH with snippet highlighting.
/// The query supports FTS5 syntax (AND, OR, NOT, phrase "quotes", prefix*).
#[tauri::command]
pub fn search_emails(
    db: State<'_, Database>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<SearchResult>, String> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let limit = limit.unwrap_or(50);

    // Sanitize the query for FTS5: escape special characters, add prefix matching
    let fts_query = sanitize_fts_query(&query);
    if fts_query.is_empty() {
        return Ok(vec![]);
    }

    let sql = "
        SELECT e.id, e.label, e.subject, e.from_addr, e.date,
               snippet(emails_fts, 1, '<mark>', '</mark>', '...', 40) as snip,
               e.is_read, e.is_bookmarked,
               rank
        FROM emails_fts
        JOIN emails e ON emails_fts.rowid = e.rowid
        WHERE emails_fts MATCH ?1
        ORDER BY rank
        LIMIT ?2
    ";

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let results: Vec<SearchResult> = stmt.query_map(params![fts_query, limit], |row| {
        Ok(SearchResult {
            id: row.get(0)?,
            label: row.get(1)?,
            subject: row.get(2)?,
            from_addr: row.get(3)?,
            date: row.get(4)?,
            snippet: row.get(5)?,
            is_read: row.get(6)?,
            is_bookmarked: row.get(7)?,
            rank: row.get(8)?,
        })
    }).map_err(|e| format!("Search query error: {}", e))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("Search result error: {}", e))?;

    Ok(results)
}

/// Sanitize user input for FTS5 query syntax.
/// Converts a plain-text query into a safe FTS5 expression with prefix matching.
fn sanitize_fts_query(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // If user entered explicit FTS5 operators, pass through (advanced usage)
    if trimmed.contains('"') || trimmed.contains(" OR ") || trimmed.contains(" NOT ") {
        return trimmed.to_string();
    }

    // Split into words and add prefix matching to the last word (for as-you-type search)
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }

    // Escape each word (remove FTS5 special chars)
    let clean_words: Vec<String> = words.iter()
        .map(|w| w.replace(|c: char| "(){}[]^~*:\"-".contains(c), ""))
        .filter(|w| !w.is_empty())
        .collect();

    if clean_words.is_empty() {
        return String::new();
    }

    // Add prefix matching to the last word for incremental search
    let mut parts = clean_words;
    if let Some(last) = parts.last_mut() {
        *last = format!("{}*", last);
    }

    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    // --- Sanitize query tests ---

    #[test]
    fn test_sanitize_basic() {
        assert_eq!(sanitize_fts_query("hello world"), "hello world*");
    }

    #[test]
    fn test_sanitize_single_word() {
        assert_eq!(sanitize_fts_query("test"), "test*");
    }

    #[test]
    fn test_sanitize_empty() {
        assert_eq!(sanitize_fts_query(""), "");
        assert_eq!(sanitize_fts_query("   "), "");
    }

    #[test]
    fn test_sanitize_special_chars() {
        assert_eq!(sanitize_fts_query("hello (world)"), "hello world*");
    }

    #[test]
    fn test_passthrough_quotes() {
        assert_eq!(sanitize_fts_query("\"exact phrase\""), "\"exact phrase\"");
    }

    // --- Integration tests for FTS5 search pipeline ---

    /// Create an in-memory SQLite database with emails + FTS5 tables and sample data
    fn create_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();

        conn.execute_batch("
            CREATE TABLE emails (
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

            CREATE VIRTUAL TABLE emails_fts USING fts5(
                subject, body, from_addr, label,
                content='emails', content_rowid='rowid',
                tokenize='porter unicode61'
            );
        ").unwrap();

        // Insert sample emails
        let samples = vec![
            ("id001", "tech-digest", "Rust async patterns", "Tech Weekly <tech@example.com>",
             "Learn about async/await patterns in Rust including futures and tokio runtime.", "2024-01-15", false, false),
            ("id002", "design-weekly", "Modern CSS Grid layouts", "Design News <design@example.com>",
             "CSS Grid provides powerful two-dimensional layout capabilities for web design.", "2024-01-20", true, false),
            ("id003", "tech-digest", "Python data science tools", "Tech Weekly <tech@example.com>",
             "Exploring pandas, numpy, and matplotlib for data analysis.", "2024-02-01", false, true),
            ("id004", "startup-news", "AI fundraising trends", "Startup Radar <hello@startup.com>",
             "Venture capital investment in artificial intelligence startups has doubled.", "2024-02-10", false, false),
        ];

        for (id, label, subject, from_addr, body, date, is_read, is_bookmarked) in &samples {
            conn.execute(
                "INSERT INTO emails (id, label, subject, from_addr, body, date, html_filename, is_read, is_bookmarked)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'email.html', ?7, ?8)",
                params![id, label, subject, from_addr, body, date, *is_read as i32, *is_bookmarked as i32],
            ).unwrap();

            let rowid: i64 = conn.query_row(
                "SELECT rowid FROM emails WHERE id = ?1", params![id], |row| row.get(0),
            ).unwrap();

            conn.execute(
                "INSERT INTO emails_fts (rowid, subject, body, from_addr, label) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![rowid, subject, body, from_addr, label],
            ).unwrap();
        }

        conn
    }

    /// Helper to run a search query against a test connection
    fn run_search(conn: &Connection, query: &str, limit: i64) -> Vec<SearchResult> {
        let fts_query = sanitize_fts_query(query);
        if fts_query.is_empty() {
            return vec![];
        }

        let sql = "
            SELECT e.id, e.label, e.subject, e.from_addr, e.date,
                   snippet(emails_fts, 1, '<mark>', '</mark>', '...', 40) as snip,
                   e.is_read, e.is_bookmarked,
                   rank
            FROM emails_fts
            JOIN emails e ON emails_fts.rowid = e.rowid
            WHERE emails_fts MATCH ?1
            ORDER BY rank
            LIMIT ?2
        ";

        let mut stmt = conn.prepare(sql).unwrap();
        stmt.query_map(params![fts_query, limit], |row| {
            Ok(SearchResult {
                id: row.get(0)?,
                label: row.get(1)?,
                subject: row.get(2)?,
                from_addr: row.get(3)?,
                date: row.get(4)?,
                snippet: row.get(5)?,
                is_read: row.get(6)?,
                is_bookmarked: row.get(7)?,
                rank: row.get(8)?,
            })
        }).unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    }

    #[test]
    fn test_search_returns_results() {
        let conn = create_test_db();
        let results = run_search(&conn, "rust", 50);
        assert!(!results.is_empty(), "Search for 'rust' should return results");
        assert_eq!(results[0].id, "id001");
        assert!(!results[0].from_addr.is_empty());
    }

    #[test]
    fn test_search_snippet_highlighting() {
        let conn = create_test_db();
        let results = run_search(&conn, "async", 50);
        assert!(!results.is_empty());
        // snippet() wraps matched terms with <mark> tags
        assert!(results[0].snippet.contains("<mark>"), "Snippet should contain <mark> tags: {}", results[0].snippet);
    }

    #[test]
    fn test_search_matches_subject() {
        let conn = create_test_db();
        let results = run_search(&conn, "CSS Grid", 50);
        assert!(!results.is_empty(), "Should match text in subject");
        assert_eq!(results[0].id, "id002");
    }

    #[test]
    fn test_search_matches_body() {
        let conn = create_test_db();
        // "pandas" only appears in body text of id003
        let results = run_search(&conn, "pandas", 50);
        assert!(!results.is_empty(), "Should match text only in body");
        assert_eq!(results[0].id, "id003");
    }

    #[test]
    fn test_search_matches_from_addr() {
        let conn = create_test_db();
        // "Startup Radar" only appears in from_addr of id004
        let results = run_search(&conn, "Startup Radar", 50);
        assert!(!results.is_empty(), "Should match sender name");
        assert_eq!(results[0].id, "id004");
    }

    #[test]
    fn test_search_matches_label() {
        let conn = create_test_db();
        // FTS5 tokenizes "design-weekly" into "design" and "weekly" tokens,
        // so searching for either word matches via the label column
        let results = run_search(&conn, "startup news", 50);
        assert!(!results.is_empty(), "Should match label name tokens");
        assert_eq!(results[0].label, "startup-news");
    }

    #[test]
    fn test_search_empty_query() {
        let conn = create_test_db();
        let results = run_search(&conn, "", 50);
        assert!(results.is_empty(), "Empty query should return no results");

        let results = run_search(&conn, "   ", 50);
        assert!(results.is_empty(), "Whitespace query should return no results");
    }

    #[test]
    fn test_search_no_results() {
        let conn = create_test_db();
        let results = run_search(&conn, "xyznonexistent", 50);
        assert!(results.is_empty(), "Non-matching query should return no results");
    }
}
