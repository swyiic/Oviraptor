// Operator switch for the single-controller ("one brain") execution model,
// per scan attempt (design Option A / B3-1, B3-2). Two independent, default-OFF
// toggles frozen for the run:
//   * conversation_continuity — thread the controller's planning reasoning into
//     the execution run's opening conversation (B3.3c Option a). Pure
//     conversation seeding; it grants NO capability and changes NO execution
//     authorization. Consumed by native_controller_execution_planning.
//   * phase_leasing — let the coordinator enter the safest controller PHASES
//     (Planner: zero capability; and, later/B3-3, a read-only phase) on its own
//     run, budget-neutral, bound to the live coordinator lease epoch + fencing
//     (B3-2). Consumed by the coordinator preparation path.
//
// The row is IMMUTABLE once written (no update / replace / delete triggers,
// exactly like scan_capability_authorizations), and the agent has NO tool that
// writes this table, so a (possibly prompt-injected) agent — or a target page
// driving the agent — can never set or flip it: there is no injection surface.
// Absent table or row ⇒ BOTH toggles OFF ⇒ runtime behavior is byte-identical to
// the pre-single-controller build. This operator record is the production,
// frozen-plan replacement for the dev-only OVIRAPTOR_CONTROLLER_EXECUTION env
// var; the env var stays as an operator-only dev/test fallback (also uninjectable
// — a page cannot set env).
const SCAN_CONTROLLER_EXECUTION_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS scan_controller_execution (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    conversation_continuity INTEGER NOT NULL CHECK(conversation_continuity IN (0,1)),
    phase_leasing INTEGER NOT NULL CHECK(phase_leasing IN (0,1)),
    attested_by TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number)
);
CREATE TRIGGER IF NOT EXISTS scan_controller_execution_no_update
BEFORE UPDATE ON scan_controller_execution BEGIN SELECT RAISE(ABORT,'scan_controller_execution_immutable'); END;
CREATE TRIGGER IF NOT EXISTS scan_controller_execution_no_replace
BEFORE INSERT ON scan_controller_execution
WHEN EXISTS(SELECT 1 FROM scan_controller_execution WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)
BEGIN SELECT RAISE(ABORT,'scan_controller_execution_immutable'); END;
CREATE TRIGGER IF NOT EXISTS scan_controller_execution_no_delete
BEFORE DELETE ON scan_controller_execution
WHEN EXISTS(SELECT 1 FROM sentinel_scans WHERE id=OLD.scan_id)
BEGIN SELECT RAISE(ABORT,'scan_controller_execution_immutable'); END;
"#;

// Shared reader. Returns the requested column's boolean, fail-closed: a missing
// table, a missing row, or any read error means OFF. `column` is a fixed
// identifier chosen by the two public readers below — never caller-supplied —
// so there is no SQL-identifier injection.
fn scan_controller_execution_flag(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    column: &'static str,
) -> Result<bool, String> {
    let storage: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='scan_controller_execution')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !storage {
        return Ok(false);
    }
    let value = connection.query_row(
        &format!(
            "SELECT {column} FROM scan_controller_execution WHERE scan_id=?1 AND attempt_number=?2"
        ),
        params![scan_id, attempt_number],
        |r| r.get::<_, i64>(0),
    );
    match value {
        Ok(flag) => Ok(flag == 1),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

/// Whether this scan attempt froze the controller-planning conversation
/// continuity ON (B3-1). Operator-only, immutable, default-OFF. Fail-closed.
pub(crate) fn scan_controller_conversation_continuity_on(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<bool, String> {
    scan_controller_execution_flag(connection, scan_id, attempt_number, "conversation_continuity")
}

/// Whether this scan attempt froze controller phase-leasing ON (B3-2).
/// Operator-only, immutable, default-OFF. Fail-closed. Gates only the safest
/// phases (Planner zero-capability; read-only later) on the coordinator's own
/// run; it never relaxes any per-call authorization wall.
pub(crate) fn scan_controller_phase_leasing_on(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<bool, String> {
    scan_controller_execution_flag(connection, scan_id, attempt_number, "phase_leasing")
}

fn set_scan_controller_execution_on(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    conversation_continuity: bool,
    phase_leasing: bool,
    attested_by: &str,
) -> Result<(), String> {
    if scan_id.trim().is_empty()
        || scan_id.contains('/')
        || scan_id.contains('\\')
        || scan_id.contains("..")
        || scan_id.chars().any(char::is_control)
    {
        return Err("任务 ID 非法".into());
    }
    if attempt_number <= 0 {
        return Err("attempt_number 非法".into());
    }
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)",
            [scan_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("任务不存在".into());
    }
    connection
        .execute_batch(SCAN_CONTROLLER_EXECUTION_SCHEMA)
        .map_err(|_| "scan_controller_execution_storage_unavailable")?;
    let changed = connection
        .execute(
            "INSERT INTO scan_controller_execution(scan_id,attempt_number,conversation_continuity,phase_leasing,attested_by) VALUES(?1,?2,?3,?4,?5)",
            params![
                scan_id,
                attempt_number,
                i64::from(conversation_continuity),
                i64::from(phase_leasing),
                attested_by
            ],
        )
        .map_err(|e| {
            if e.to_string().contains("scan_controller_execution_immutable") {
                "该任务的单主脑执行模式已设定且不可更改".to_string()
            } else {
                e.to_string()
            }
        })?;
    if changed != 1 {
        return Err("单主脑执行模式写入未确认".into());
    }
    Ok(())
}

fn set_scan_controller_execution_inner(
    db_path: &Path,
    scan_id: &str,
    attempt_number: Option<i64>,
    conversation_continuity: bool,
    phase_leasing: bool,
    attested_by: &str,
) -> Result<(), String> {
    let mut connection = db::open(db_path)?;
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let attempt = match attempt_number {
        Some(value) => value,
        None => {
            let current: i64 = tx
                .query_row(
                    "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
                    [scan_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            current.max(1)
        }
    };
    set_scan_controller_execution_on(
        &tx,
        scan_id,
        attempt,
        conversation_continuity,
        phase_leasing,
        attested_by,
    )?;
    tx.commit().map_err(|e| e.to_string())
}

/// Operator sets, once, whether this scan attempt runs under the single-
/// controller model (conversation continuity and/or phase leasing). Immutable:
/// a second call for the same scan+attempt is refused. The agent has no tool
/// that writes this table, so it can never self-enable the single-controller
/// model; absent ⇒ both OFF ⇒ behavior unchanged.
#[tauri::command]
pub fn set_scan_controller_execution(
    state: State<AppState>,
    scan_id: String,
    attempt_number: Option<i64>,
    conversation_continuity: bool,
    phase_leasing: bool,
    attested_by: Option<String>,
) -> Result<(), String> {
    set_scan_controller_execution_inner(
        &state.db_path,
        &scan_id,
        attempt_number,
        conversation_continuity,
        phase_leasing,
        attested_by.as_deref().unwrap_or(""),
    )
}

#[tauri::command]
pub fn get_scan_controller_execution(
    state: State<AppState>,
    scan_id: String,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    let attempt: i64 = connection
        .query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |row| row.get(0),
        )
        .unwrap_or(0)
        .max(1);
    Ok(serde_json::json!({
        "conversationContinuity": scan_controller_conversation_continuity_on(&connection, &scan_id, attempt).unwrap_or(false),
        "phaseLeasing": scan_controller_phase_leasing_on(&connection, &scan_id, attempt).unwrap_or(false),
    }))
}

#[cfg(test)]
mod scan_controller_execution_tests {
    use super::{
        scan_controller_conversation_continuity_on, scan_controller_phase_leasing_on,
        set_scan_controller_execution_on,
    };
    use rusqlite::Connection;

    fn db() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY);
                 INSERT INTO sentinel_scans(id) VALUES('scan-a');",
            )
            .unwrap();
        connection
    }

    #[test]
    fn both_toggles_off_by_default() {
        // No table at all (fresh DB) ⇒ fail-closed OFF.
        let bare = Connection::open_in_memory().unwrap();
        assert!(!scan_controller_conversation_continuity_on(&bare, "scan-a", 1).unwrap());
        assert!(!scan_controller_phase_leasing_on(&bare, "scan-a", 1).unwrap());
        // Table present but no row for this attempt ⇒ OFF.
        let c = db();
        set_scan_controller_execution_on(&c, "scan-a", 1, true, true, "op").unwrap();
        assert!(!scan_controller_conversation_continuity_on(&c, "scan-a", 2).unwrap());
        assert!(!scan_controller_phase_leasing_on(&c, "scan-a", 2).unwrap());
    }

    #[test]
    fn reads_back_each_toggle_independently() {
        let c = db();
        // conversation ON, phase OFF.
        set_scan_controller_execution_on(&c, "scan-a", 1, true, false, "op").unwrap();
        assert!(scan_controller_conversation_continuity_on(&c, "scan-a", 1).unwrap());
        assert!(!scan_controller_phase_leasing_on(&c, "scan-a", 1).unwrap());
    }

    #[test]
    fn record_is_immutable() {
        let c = db();
        assert!(set_scan_controller_execution_on(&c, "scan-a", 1, true, false, "op").is_ok());
        let again = set_scan_controller_execution_on(&c, "scan-a", 1, false, true, "op");
        assert!(again.is_err());
        // The first write still stands unchanged.
        assert!(scan_controller_conversation_continuity_on(&c, "scan-a", 1).unwrap());
        assert!(!scan_controller_phase_leasing_on(&c, "scan-a", 1).unwrap());
    }

    #[test]
    fn unknown_scan_rejected() {
        let c = db();
        assert!(set_scan_controller_execution_on(&c, "ghost", 1, true, true, "op").is_err());
    }

    #[test]
    fn illegal_attempt_rejected() {
        let c = db();
        assert!(set_scan_controller_execution_on(&c, "scan-a", 0, true, true, "op").is_err());
    }
}
