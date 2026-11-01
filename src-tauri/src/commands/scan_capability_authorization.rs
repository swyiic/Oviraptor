// Operator authorization for high-risk pentest capability classes, per scan
// attempt (design §3.3 / §4b). The row is immutable once written, and the
// agent has no tool that writes this table, so a (possibly prompt-injected)
// agent can never self-authorize. The scheduling gate reads it to decide
// whether a high-risk capability class may be granted
// (child_contract.rs::scan_class_authorized); the per-action approval (a later
// step) sits on top of this opt-in, it does not replace it. Opting into any
// high-risk class requires the operator's explicit legal-authorization
// attestation; without an attested row every high-risk class stays refused.
const SCAN_CAPABILITY_AUTH_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS scan_capability_authorizations (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    legal_attestation INTEGER NOT NULL CHECK(legal_attestation IN (0,1)),
    authorized_classes TEXT NOT NULL CHECK(json_valid(authorized_classes)),
    attested_by TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number)
);
CREATE TRIGGER IF NOT EXISTS scan_capability_auth_no_update
BEFORE UPDATE ON scan_capability_authorizations BEGIN SELECT RAISE(ABORT,'scan_capability_authorization_immutable'); END;
CREATE TRIGGER IF NOT EXISTS scan_capability_auth_no_replace
BEFORE INSERT ON scan_capability_authorizations
WHEN EXISTS(SELECT 1 FROM scan_capability_authorizations WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)
BEGIN SELECT RAISE(ABORT,'scan_capability_authorization_immutable'); END;
CREATE TRIGGER IF NOT EXISTS scan_capability_auth_no_delete
BEFORE DELETE ON scan_capability_authorizations
WHEN EXISTS(SELECT 1 FROM sentinel_scans WHERE id=OLD.scan_id)
BEGIN SELECT RAISE(ABORT,'scan_capability_authorization_immutable'); END;
"#;

// Canonical high-risk class strings. Mirror of child_contract.rs
// CapabilityClass::as_str for the high-risk variants — keep in sync. A class
// string accepted here that the gate did not recognize would simply never match
// a capability, so the default-closed floor holds either way.
const HIGH_RISK_CLASS_STRINGS: &[&str] =
    &["code_exec", "target_write", "egress", "persistence", "exfil", "waf_evasion"];

fn set_scan_capability_authorization_on(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    legal_attestation: bool,
    authorized_classes: &[String],
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
    let mut classes: Vec<String> = authorized_classes.to_vec();
    classes.sort();
    classes.dedup();
    if classes
        .iter()
        .any(|c| !HIGH_RISK_CLASS_STRINGS.contains(&c.as_str()))
    {
        return Err("存在未知的高危能力类".into());
    }
    // The legal-authorization attestation is the hard precondition for opting
    // into any high-risk class. An empty authorization (no classes) is allowed
    // without it and simply keeps every high-risk class refused.
    if !classes.is_empty() && !legal_attestation {
        return Err("开启高危能力前必须先确认已获得对这些目标的合法授权".into());
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
        .execute_batch(SCAN_CAPABILITY_AUTH_SCHEMA)
        .map_err(|_| "scan_capability_authorization_storage_unavailable")?;
    let classes_json = serde_json::to_string(&classes).map_err(|e| e.to_string())?;
    let changed = connection
        .execute(
            "INSERT INTO scan_capability_authorizations(scan_id,attempt_number,legal_attestation,authorized_classes,attested_by) VALUES(?1,?2,?3,?4,?5)",
            params![scan_id, attempt_number, i64::from(legal_attestation), classes_json, attested_by],
        )
        .map_err(|e| {
            // The no_replace trigger fires when an authorization already exists.
            if e.to_string().contains("scan_capability_authorization_immutable") {
                "该任务的能力授权已设定且不可更改".to_string()
            } else {
                e.to_string()
            }
        })?;
    if changed != 1 {
        return Err("能力授权写入未确认".into());
    }
    Ok(())
}

fn set_scan_capability_authorization_inner(
    db_path: &Path,
    scan_id: &str,
    attempt_number: Option<i64>,
    legal_attestation: bool,
    authorized_classes: &[String],
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
    set_scan_capability_authorization_on(
        &tx,
        scan_id,
        attempt,
        legal_attestation,
        authorized_classes,
        attested_by,
    )?;
    tx.commit().map_err(|e| e.to_string())
}

/// Operator sets, once, which high-risk capability classes this scan attempt is
/// authorized for, with an explicit legal-authorization attestation. Immutable:
/// a second call for the same scan+attempt is refused. This is the opt-in that
/// the scheduling/per-call gates consult; it never itself executes anything.
#[tauri::command]
pub fn set_scan_capability_authorization(
    state: State<AppState>,
    scan_id: String,
    attempt_number: Option<i64>,
    legal_attestation: bool,
    authorized_classes: Vec<String>,
    attested_by: Option<String>,
) -> Result<(), String> {
    set_scan_capability_authorization_inner(
        &state.db_path,
        &scan_id,
        attempt_number,
        legal_attestation,
        &authorized_classes,
        attested_by.as_deref().unwrap_or(""),
    )
}

#[cfg(test)]
mod scan_capability_authorization_tests {
    use super::set_scan_capability_authorization_on;
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
    fn attestation_required_for_high_risk() {
        let c = db();
        let err = set_scan_capability_authorization_on(&c, "scan-a", 1, false, &["code_exec".into()], "op")
            .unwrap_err();
        assert!(err.contains("合法授权"), "{err}");
    }

    #[test]
    fn unknown_class_rejected() {
        let c = db();
        assert!(set_scan_capability_authorization_on(&c, "scan-a", 1, true, &["make_coffee".into()], "op").is_err());
    }

    #[test]
    fn empty_authorization_ok_without_attestation() {
        let c = db();
        assert!(set_scan_capability_authorization_on(&c, "scan-a", 1, false, &[], "op").is_ok());
    }

    #[test]
    fn authorization_is_immutable() {
        let c = db();
        assert!(set_scan_capability_authorization_on(&c, "scan-a", 1, true, &["code_exec".into()], "op").is_ok());
        let again = set_scan_capability_authorization_on(&c, "scan-a", 1, true, &["exfil".into()], "op")
            .unwrap_err();
        assert!(again.contains("不可更改"), "{again}");
    }

    #[test]
    fn unknown_scan_rejected() {
        let c = db();
        assert!(set_scan_capability_authorization_on(&c, "scan-missing", 1, true, &["code_exec".into()], "op").is_err());
    }
}
