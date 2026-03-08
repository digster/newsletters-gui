use crate::db::Database;
use rusqlite::params;
use serde::Serialize;
use tauri::State;

/// Search result with snippet highlighting
#[derive(Serialize)]
pub struct SearchResult {
    pub id: String,
    pub label: String,
    pub subject: String,
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
        SELECT e.id, e.label, e.subject, e.date,
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
    let results = stmt.query_map(params![fts_query, limit], |row| {
        Ok(SearchResult {
            id: row.get(0)?,
            label: row.get(1)?,
            subject: row.get(2)?,
            date: row.get(3)?,
            snippet: row.get(4)?,
            is_read: row.get(5)?,
            is_bookmarked: row.get(6)?,
            rank: row.get(7)?,
        })
    }).map_err(|e| format!("Search error: {}", e))?
    .filter_map(|r| r.ok())
    .collect();

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
        .map(|w| w.replace(|c: char| "(){}[]^~*:\"".contains(c), ""))
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
}
