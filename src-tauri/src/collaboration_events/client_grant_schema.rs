//! Exact emitters used by a ClientSide schedule; no arbitrary trigger authority.
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
            .map_err(|_| "client_side_grant_emitter_conflict")?;
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
            .ok_or("client_side_grant_emitter_conflict")?;
        let tail = &super::COLLABORATION_EVENT_SCHEMA[start..];
        let end = tail
            .find("\nEND;")
            .ok_or("client_side_grant_emitter_conflict")?
            + 5;
        if shadow.is_some()
            || super::closure_schema::canonical(&actual)
                != super::closure_schema::canonical(&tail[..end])
        {
            return Err("client_side_grant_emitter_conflict".into());
        }
    }
    Ok(())
}
