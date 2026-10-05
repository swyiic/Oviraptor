//! Durable asset log records. Notifications contain identity only, never text.
use crate::{log_display, models::LogEntry};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogHint {
    pub log_id: i64,
    pub run_id: i64,
    pub project_id: Option<i64>,
}

pub(crate) fn append(
    connection: &mut Connection,
    run_id: i64,
    level: &str,
    stage: &str,
    message: &str,
    notify: impl FnOnce(&LogHint),
) -> Result<(), String> {
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO logs(run_id,level,stage,message) VALUES(?1,?2,?3,?4)",
            params![
                run_id,
                log_display::text(level, 32),
                log_display::text(stage, 120),
                log_display::line(message)
            ],
        )
        .map_err(|error| error.to_string())?;
    let hint = LogHint {
        log_id: transaction.last_insert_rowid(),
        run_id,
        project_id: transaction
            .query_row("SELECT project_id FROM runs WHERE id=?1", [run_id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|error| error.to_string())?,
    };
    transaction.commit().map_err(|error| error.to_string())?;
    notify(&hint);
    Ok(())
}

pub(crate) fn read(
    connection: &Connection,
    run_id: Option<i64>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<LogEntry>, String> {
    let mut statement = connection
        .prepare(
            "SELECT l.id,l.run_id,l.level,l.stage,l.message,l.created_at
         FROM logs l LEFT JOIN runs r ON r.id=l.run_id
         WHERE (?1 IS NULL OR l.run_id=?1) AND (?2 IS NULL OR r.project_id=?2)
         ORDER BY l.id DESC LIMIT ?3",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![run_id, project_id, limit.unwrap_or(500).clamp(1, 2000)],
            |row| {
                Ok(LogEntry {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    level: log_display::text(&row.get::<_, String>(2)?, 32),
                    stage: log_display::text(&row.get::<_, String>(3)?, 120),
                    message: log_display::line(&row.get::<_, String>(4)?),
                    created_at: row.get(5)?,
                })
            },
        )
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "asset_logs_tests.rs"]
mod tests;
