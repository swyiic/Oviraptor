//! Verify exact built-in Root-creation and model-delivery emitters.
use rusqlite::{Connection, OptionalExtension};
pub(crate) fn verify(db: &Connection) -> Result<(), String> {
    verify_named(db, "agent_collaboration_run_insert", "web_finance_emitter_contract_conflict")
}

pub(crate) fn verify_model_delivery(db: &Connection) -> Result<(), String> {
    verify_named(db, "agent_collaboration_directive_delivery", "specialist_receipt_emitter_contract_conflict")
}

fn verify_named(db: &Connection, name: &str, error: &'static str) -> Result<(), String> {
    let actual: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
            [name],
            |r| r.get(0),
        )
        .map_err(|_| error)?;
    let shadow: Option<String> = db
        .query_row(
            "SELECT name FROM sqlite_temp_master WHERE type='trigger' AND name=?1",
            [name],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let start = super::COLLABORATION_EVENT_SCHEMA
        .find(&format!("CREATE TRIGGER IF NOT EXISTS {name}\n"))
        .ok_or(error)?;
    let tail = &super::COLLABORATION_EVENT_SCHEMA[start..];
    let end = tail
        .find("\nEND;")
        .ok_or(error)?
        + 5;
    if shadow.is_some() || canonical(&actual) != canonical(&tail[..end]) {
        return Err(error.into());
    }
    Ok(())
}
fn canonical(sql: &str) -> String {
    let sql = sql.trim().trim_end_matches(';').replacen(
        "CREATE TRIGGER IF NOT EXISTS ",
        "CREATE TRIGGER ",
        1,
    );
    let mut out = String::new();
    let mut quoted = false;
    let mut blank = false;
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if blank && !out.is_empty() {
                out.push(' ');
            }
            blank = false;
            out.push(c);
            if quoted && chars.peek() == Some(&'\'') {
                out.push(chars.next().unwrap());
            } else {
                quoted = !quoted;
            }
        } else if !quoted && c.is_ascii_whitespace() {
            blank = true;
        } else {
            if blank && !out.is_empty() {
                out.push(' ');
            }
            blank = false;
            out.push(c);
        }
    }
    out
}
