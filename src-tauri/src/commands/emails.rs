use crate::db::Database;
use rusqlite::params;
use serde::Serialize;
use std::fs;
use std::path::Path;
use tauri::State;

/// Label with email count and unread count
#[derive(Serialize)]
pub struct LabelInfo {
    pub name: String,
    pub count: i64,
    pub unread_count: i64,
}

/// Email summary for list view (no HTML body)
#[derive(Serialize)]
pub struct EmailSummary {
    pub id: String,
    pub label: String,
    pub subject: String,
    pub from_addr: String,
    pub date: Option<String>,
    pub is_read: bool,
    pub is_bookmarked: bool,
}

/// Get all labels with their email counts and unread counts
#[tauri::command]
pub fn get_labels(db: State<'_, Database>) -> Result<Vec<LabelInfo>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT label, COUNT(*) as cnt,
                SUM(CASE WHEN is_read = 0 THEN 1 ELSE 0 END) as unread
         FROM emails
         GROUP BY label
         ORDER BY label COLLATE NOCASE"
    ).map_err(|e| e.to_string())?;

    let labels = stmt.query_map([], |row| {
        Ok(LabelInfo {
            name: row.get(0)?,
            count: row.get(1)?,
            unread_count: row.get(2)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    Ok(labels)
}

/// Get paginated emails for a label, with sort and filter options
#[tauri::command]
pub fn get_emails_by_label(
    db: State<'_, Database>,
    label: String,
    offset: i64,
    limit: i64,
    sort: Option<String>,    // "date_desc" (default), "date_asc", "subject"
    filter: Option<String>,  // "unread", "bookmarked", or empty
) -> Result<Vec<EmailSummary>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // Build query dynamically based on sort/filter
    let mut where_clauses = vec!["label = ?1".to_string()];
    match filter.as_deref() {
        Some("unread") => where_clauses.push("is_read = 0".to_string()),
        Some("bookmarked") => where_clauses.push("is_bookmarked = 1".to_string()),
        _ => {}
    }

    let order = match sort.as_deref() {
        Some("date_asc") => "date ASC NULLS LAST",
        Some("subject") => "subject COLLATE NOCASE ASC",
        _ => "date DESC NULLS LAST",
    };

    let sql = format!(
        "SELECT id, label, subject, from_addr, date, is_read, is_bookmarked
         FROM emails
         WHERE {}
         ORDER BY {}
         LIMIT ?2 OFFSET ?3",
        where_clauses.join(" AND "),
        order
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let emails = stmt.query_map(params![label, limit, offset], |row| {
        Ok(EmailSummary {
            id: row.get(0)?,
            label: row.get(1)?,
            subject: row.get(2)?,
            from_addr: row.get(3)?,
            date: row.get(4)?,
            is_read: row.get(5)?,
            is_bookmarked: row.get(6)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    Ok(emails)
}

/// Get the total count of emails for a label (with optional filter)
#[tauri::command]
pub fn get_email_count(
    db: State<'_, Database>,
    label: String,
    filter: Option<String>,
) -> Result<i64, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let mut where_clauses = vec!["label = ?1".to_string()];
    match filter.as_deref() {
        Some("unread") => where_clauses.push("is_read = 0".to_string()),
        Some("bookmarked") => where_clauses.push("is_bookmarked = 1".to_string()),
        _ => {}
    }

    let sql = format!(
        "SELECT COUNT(*) FROM emails WHERE {}",
        where_clauses.join(" AND ")
    );

    conn.query_row(&sql, params![label], |row| row.get(0))
        .map_err(|e| e.to_string())
}

/// Read the HTML email file and return its content as a string for iframe srcdoc
#[tauri::command]
pub fn get_email_html(
    db: State<'_, Database>,
    email_id: String,
) -> Result<String, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // The path components come from dedicated columns, never from the id itself: the id
    // is a label-scoped composite key ("<label>/<message_id>"), which is deliberately
    // decoupled from what the directory happens to be called on disk.
    let (label, dir_name, body_filename, body_format): (String, String, String, String) = conn.query_row(
        "SELECT label, dir_name, body_filename, body_format FROM emails WHERE id = ?1",
        params![email_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).map_err(|e| format!("Email not found: {}", e))?;

    let newsletters_path = {
        // Need to release conn lock before calling get_setting
        drop(conn);
        db.get_setting("newsletters_path")?
            .ok_or("No newsletters path configured")?
    };

    let body_path = Path::new(&newsletters_path)
        .join(&label)
        .join(&dir_name)
        .join(&body_filename);

    let content = fs::read_to_string(&body_path)
        .map_err(|e| format!("Failed to read email body at {:?}: {}", body_path, e))?;

    // Plain-text newsletters are wrapped here rather than in the frontend: the result goes
    // straight into an iframe `srcdoc`, so escaping has to happen before the string leaves
    // Rust — handing raw text to the viewer would make it parse as markup.
    Ok(match body_format.as_str() {
        "text" => render_text_body(&content),
        _ => content,
    })
}

/// Wrap a plain-text email body in a minimal HTML document for the viewer iframe.
///
/// No colours are set: the viewer injects `color-scheme` into the document, which makes
/// the UA's default text colour follow the active theme on its own. Only layout is
/// specified here, so light and dark both stay readable.
fn render_text_body(text: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><style>\n\
         body {{ margin: 0; padding: 32px 40px; }}\n\
         pre.plaintext {{\n\
         margin: 0; max-width: 78ch;\n\
         font-family: ui-monospace, SFMono-Regular, \"SF Mono\", Menlo, Consolas, monospace;\n\
         font-size: 13px; line-height: 1.65;\n\
         white-space: pre-wrap; overflow-wrap: anywhere;\n\
         }}\n</style></head><body><pre class=\"plaintext\">{}</pre></body></html>",
        escape_html(text)
    )
}

/// Escape text for interpolation into an HTML text node
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Get a single email's details
#[tauri::command]
pub fn get_email(
    db: State<'_, Database>,
    email_id: String,
) -> Result<EmailSummary, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, label, subject, from_addr, date, is_read, is_bookmarked
         FROM emails WHERE id = ?1",
        params![email_id],
        |row| Ok(EmailSummary {
            id: row.get(0)?,
            label: row.get(1)?,
            subject: row.get(2)?,
            from_addr: row.get(3)?,
            date: row.get(4)?,
            is_read: row.get(5)?,
            is_bookmarked: row.get(6)?,
        }),
    ).map_err(|e| format!("Email not found: {}", e))
}

/// Get the newsletters path from settings
#[tauri::command]
pub fn get_newsletters_path(db: State<'_, Database>) -> Result<Option<String>, String> {
    db.get_setting("newsletters_path")
}

/// Set the newsletters path in settings
#[tauri::command]
pub fn set_newsletters_path(
    db: State<'_, Database>,
    path: String,
) -> Result<(), String> {
    db.set_setting("newsletters_path", &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_bodies_are_escaped_not_rendered_as_markup() {
        // A plain-text newsletter that happens to contain angle brackets must not be able
        // to inject markup or script into the viewer iframe.
        let html = render_text_body("Reply to <script>alert(1)</script> & co.");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>"), "raw script tag leaked into the document");
        assert!(html.contains("&amp; co."));
    }

    #[test]
    fn text_bodies_preserve_layout_and_wrap_in_a_document() {
        let html = render_text_body("line one\nline two");
        assert!(html.starts_with("<!DOCTYPE html>"));
        // The viewer injects its theme <style> before </head>, so the document needs one.
        assert!(html.contains("</head>"), "theme injection depends on a </head>");
        assert!(html.contains("white-space: pre-wrap"), "newlines must survive display");
        assert!(html.contains("line one\nline two"), "body text should be untouched");
    }

    #[test]
    fn escape_html_leaves_ordinary_text_alone() {
        assert_eq!(escape_html("plain text, 100% fine"), "plain text, 100% fine");
        assert_eq!(escape_html("a & b < c > d"), "a &amp; b &lt; c &gt; d");
    }
}
