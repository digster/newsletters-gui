use serde::Deserialize;
use serde::de::Deserializer;

/// YAML frontmatter from .md files
#[derive(Debug, Deserialize, Default)]
pub struct Frontmatter {
    /// Full Gmail message ID, emitted by gmail-ingestor as `id: "<message_id>"`.
    ///
    /// Deserialized leniently rather than as a plain `String`: a minority of historical
    /// files carry the value unquoted, and all-digit IDs (e.g. 1637675546614607) then
    /// parse as a YAML integer, which would fail a strict String field and silently
    /// drop the ID for exactly those messages.
    #[serde(default, deserialize_with = "deserialize_scalar_as_string")]
    pub id: Option<String>,
    #[serde(default)]
    pub subject: String,
    #[serde(default, alias = "from")]
    pub from: String,
    #[serde(default)]
    pub to: String,
    /// Date field — serde_yaml parses bare "2021-05-11 16:34:38" as a timestamp,
    /// so we use a custom deserializer to always convert it to a String.
    #[serde(default, deserialize_with = "deserialize_scalar_as_string")]
    pub date: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub label_ids: Vec<String>,
}

/// Custom deserializer that accepts any YAML scalar and converts it to a String.
/// Handles: string "2021-05-11", timestamp 2021-05-11 16:34:38, bare integers, null, etc.
fn deserialize_scalar_as_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde_yaml::Value;
    let v = Option::<Value>::deserialize(deserializer)?;
    match v {
        None => Ok(None),
        Some(Value::String(s)) => {
            if s.is_empty() { Ok(None) } else { Ok(Some(s)) }
        }
        Some(Value::Number(n)) => Ok(Some(n.to_string())),
        Some(Value::Null) => Ok(None),
        // serde_yaml represents timestamps as tagged values or strings depending on version
        Some(other) => {
            // Fall back to formatting whatever we got
            let s = format!("{}", serde_yaml::to_string(&other).unwrap_or_default().trim());
            if s.is_empty() || s == "null" || s == "~" {
                Ok(None)
            } else {
                // serde_yaml may quote the string, strip surrounding quotes
                let cleaned = s.trim_matches('\'').trim_matches('"').to_string();
                Ok(Some(cleaned))
            }
        }
    }
}

/// Parse YAML frontmatter from a markdown file's content.
/// Expects the file to start with "---" and have a closing "---".
/// Returns (frontmatter, body_text).
pub fn parse_frontmatter(content: &str) -> Result<(Frontmatter, String), String> {
    let content = content.trim_start_matches('\u{feff}'); // Strip BOM

    if !content.starts_with("---") {
        return Err("No frontmatter delimiter found".to_string());
    }

    // Find the closing "---" delimiter
    let rest = &content[3..];
    let end_pos = rest.find("\n---")
        .ok_or_else(|| "No closing frontmatter delimiter".to_string())?;

    let yaml_str = &rest[..end_pos];
    let body = &rest[end_pos + 4..]; // Skip past "\n---"

    let fm: Frontmatter = serde_yaml::from_str(yaml_str)
        .map_err(|e| format!("YAML parse error: {}", e))?;

    // Clean up the body text: strip markdown formatting for FTS indexing
    let body_text = body.trim().to_string();

    Ok((fm, body_text))
}

/// Format a date string for consistent storage.
/// Passes through valid-looking dates as-is.
pub fn normalize_date(date_str: &str) -> Option<String> {
    let trimmed = date_str.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
subject: "Test Subject"
from: "Sender <test@example.com>"
to: ""
date: 2021-05-11 16:34:38
labels: ["Test", "INBOX"]
label_ids: ["Label_123", "INBOX"]
---
This is the body text of the email.
More content here.
"#;
        let (fm, body) = parse_frontmatter(content).unwrap();
        assert_eq!(fm.subject, "Test Subject");
        assert_eq!(fm.from, "Sender <test@example.com>");
        assert!(fm.date.is_some());
        assert!(body.contains("body text"));
    }

    #[test]
    fn test_no_frontmatter() {
        let content = "Just plain text without frontmatter";
        assert!(parse_frontmatter(content).is_err());
    }
}
