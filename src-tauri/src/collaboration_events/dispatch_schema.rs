//! Exact built-in child-issuance emitters. No replaced or temporary names.
use rusqlite::{Connection, OptionalExtension};

pub(crate) const EMITTERS: [&str; 4] = [
    "agent_collaboration_run_insert",
    "agent_collaboration_run_update",
    "agent_collaboration_assignment_insert",
    "agent_collaboration_assignment_update",
];

pub(crate) fn verify(db: &Connection) -> Result<(), String> {
    for name in EMITTERS {
        let actual: String = db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|_| "root_dispatch_emitter_contract_conflict")?;
        let shadow: Option<String> = db
            .query_row(
                "SELECT name FROM sqlite_temp_master WHERE type='trigger' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let needle = format!("CREATE TRIGGER IF NOT EXISTS {name}\n");
        let start = super::COLLABORATION_EVENT_SCHEMA
            .find(&needle)
            .ok_or("root_dispatch_emitter_contract_conflict")?;
        let tail = &super::COLLABORATION_EVENT_SCHEMA[start..];
        let end = tail
            .find("\nEND;")
            .ok_or("root_dispatch_emitter_contract_conflict")?
            + 5;
        if shadow.is_some() || canonical(&actual) != canonical(&tail[..end]) {
            return Err("root_dispatch_emitter_contract_conflict".into());
        }
    }
    Ok(())
}

// SQLite omits IF NOT EXISTS and trailing ';'. Quoted literal bytes stay exact.
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
