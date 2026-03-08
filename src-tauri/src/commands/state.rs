use crate::db::Database;
use rusqlite::params;
use serde::Serialize;
use tauri::State;

/// Response from a toggle operation
#[derive(Serialize)]
pub struct ToggleResult {
    pub id: String,
    pub value: bool,
}

/// Toggle the bookmark state of an email
#[tauri::command]
pub fn toggle_bookmark(
    db: State<'_, Database>,
    email_id: String,
) -> Result<ToggleResult, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE emails SET is_bookmarked = CASE WHEN is_bookmarked = 1 THEN 0 ELSE 1 END WHERE id = ?1",
        params![email_id],
    ).map_err(|e| e.to_string())?;

    let new_value: bool = conn.query_row(
        "SELECT is_bookmarked FROM emails WHERE id = ?1",
        params![email_id],
        |row| row.get(0),
    ).map_err(|e| e.to_string())?;

    Ok(ToggleResult { id: email_id, value: new_value })
}

/// Mark an email as read
#[tauri::command]
pub fn mark_read(
    db: State<'_, Database>,
    email_id: String,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE emails SET is_read = 1 WHERE id = ?1",
        params![email_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// Toggle the read state of an email
#[tauri::command]
pub fn toggle_read(
    db: State<'_, Database>,
    email_id: String,
) -> Result<ToggleResult, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE emails SET is_read = CASE WHEN is_read = 1 THEN 0 ELSE 1 END WHERE id = ?1",
        params![email_id],
    ).map_err(|e| e.to_string())?;

    let new_value: bool = conn.query_row(
        "SELECT is_read FROM emails WHERE id = ?1",
        params![email_id],
        |row| row.get(0),
    ).map_err(|e| e.to_string())?;

    Ok(ToggleResult { id: email_id, value: new_value })
}

/// Get all bookmarked emails
#[tauri::command]
pub fn get_bookmarked_emails(
    db: State<'_, Database>,
    offset: i64,
    limit: i64,
) -> Result<Vec<super::emails::EmailSummary>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare(
        "SELECT id, label, subject, from_addr, date, is_read, is_bookmarked
         FROM emails
         WHERE is_bookmarked = 1
         ORDER BY date DESC NULLS LAST
         LIMIT ?1 OFFSET ?2"
    ).map_err(|e| e.to_string())?;

    let emails = stmt.query_map(params![limit, offset], |row| {
        Ok(super::emails::EmailSummary {
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
