// Per-action human approval for high-risk pentest capabilities (design §3.4 —
// the third gate, on top of B1 default-closed and B2 per-task opt-in). When a
// high-risk capability class is about to act, the per-call chokepoint does NOT
// execute it: it records a pending request (what action, on what target, with
// what reversibility) and the agent waits. The operator approves or rejects a
// SINGLE action — bound to the scan attempt and the current coordinator fencing
// token — and approval is single-use (consumed on the one execution). A target
// page can drive the agent, so the decision is the operator's, never the
// model's; the agent has no tool that writes this table.
//
// Wiring: `high_risk_action_gate_on` is the hook the per-call chokepoint
// (agent_authorize_tool_on) calls once a high-risk tool actually exists (Track
// C). It is intentionally not wired into that intricate path yet — there is no
// high-risk tool to gate — but the subsystem and its operator commands are
// complete and tested so the gate is ready to drop in.
const HIGH_RISK_APPROVAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS high_risk_action_approvals (
    request_id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    fencing_token TEXT NOT NULL,
    capability_class TEXT NOT NULL,
    action_fingerprint TEXT NOT NULL,
    action_summary TEXT NOT NULL DEFAULT '',
    target TEXT NOT NULL DEFAULT '',
    reversible INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','approved','rejected','consumed')),
    decided_by TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    decided_at TEXT NOT NULL DEFAULT '',
    UNIQUE(scan_id, attempt_number, fencing_token, action_fingerprint)
);
"#;

// `HIGH_RISK_CLASS_STRINGS` is defined once in scan_capability_authorization.rs
// and shared here (both are include!'d into the same `scanning` module).

#[derive(Clone, PartialEq, Eq, Debug)]
#[allow(dead_code)] // Consumed by the per-call chokepoint once a high-risk tool exists (Track C); exercised by tests now.
enum HighRiskGate {
    /// An approval existed and was just consumed (single-use); the action runs.
    Allow,
    /// A request is awaiting the operator's decision; the agent must wait.
    Pending(String),
    /// Refused — the operator rejected it, or an approval was already consumed.
    Denied(String),
}

fn high_risk_approval_storage_exists(connection: &rusqlite::Connection) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='high_risk_action_approvals')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "high_risk_approval_storage_unavailable".into())
}

#[allow(clippy::too_many_arguments)]
fn request_high_risk_action_on(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    fencing_token: &str,
    capability_class: &str,
    action_fingerprint: &str,
    action_summary: &str,
    target: &str,
    reversible: bool,
) -> Result<(String, String), String> {
    if !HIGH_RISK_CLASS_STRINGS.contains(&capability_class) {
        return Err("未知的高危能力类".into());
    }
    if action_fingerprint.trim().is_empty() {
        return Err("缺少动作指纹".into());
    }
    connection
        .execute_batch(HIGH_RISK_APPROVAL_SCHEMA)
        .map_err(|_| "high_risk_approval_storage_unavailable")?;
    // Idempotent per (scan, attempt, fencing, fingerprint): re-asking for the
    // exact same action returns the existing request and its current status,
    // never a second pending row.
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT request_id, status FROM high_risk_action_approvals \
             WHERE scan_id=?1 AND attempt_number=?2 AND fencing_token=?3 AND action_fingerprint=?4",
            params![scan_id, attempt_number, fencing_token, action_fingerprint],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(found) = existing {
        return Ok(found);
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    let changed = connection
        .execute(
            "INSERT INTO high_risk_action_approvals(request_id,scan_id,attempt_number,fencing_token,capability_class,action_fingerprint,action_summary,target,reversible) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![request_id, scan_id, attempt_number, fencing_token, capability_class, action_fingerprint, action_summary, target, i64::from(reversible)],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("待批准请求写入未确认".into());
    }
    Ok((request_id, "pending".to_string()))
}

/// Approve or reject a single pending request. Only a `pending` row can be
/// decided, so a decision cannot be flipped and a consumed approval cannot be
/// revived.
fn decide_high_risk_action_on(
    connection: &rusqlite::Connection,
    request_id: &str,
    approve: bool,
    decided_by: &str,
) -> Result<(), String> {
    if !high_risk_approval_storage_exists(connection)? {
        return Err("没有待批准记录".into());
    }
    let status = if approve { "approved" } else { "rejected" };
    let changed = connection
        .execute(
            "UPDATE high_risk_action_approvals SET status=?2, decided_by=?3, decided_at=datetime('now','localtime') \
             WHERE request_id=?1 AND status='pending'",
            params![request_id, status, decided_by],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("该请求不存在或已不是待批准状态".into());
    }
    Ok(())
}

/// Single-use consumption: if an `approved` row matches this exact action, mark
/// it `consumed` and return true (the action may run once). A second attempt
/// finds `consumed`, not `approved`, and returns false.
fn consume_high_risk_approval_on(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    fencing_token: &str,
    action_fingerprint: &str,
) -> Result<bool, String> {
    if !high_risk_approval_storage_exists(connection)? {
        return Ok(false);
    }
    let changed = connection
        .execute(
            "UPDATE high_risk_action_approvals SET status='consumed', decided_at=datetime('now','localtime') \
             WHERE scan_id=?1 AND attempt_number=?2 AND fencing_token=?3 AND action_fingerprint=?4 AND status='approved'",
            params![scan_id, attempt_number, fencing_token, action_fingerprint],
        )
        .map_err(|e| e.to_string())?;
    Ok(changed == 1)
}

/// The gate the per-call chokepoint calls for a high-risk action: consume an
/// existing approval (→ Allow, once), else surface the pending/ rejected state.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)] // Called by agent_authorize_tool_on once a high-risk tool exists (Track C).
fn high_risk_action_gate_on(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    fencing_token: &str,
    capability_class: &str,
    action_fingerprint: &str,
    action_summary: &str,
    target: &str,
    reversible: bool,
    enforcing_tool: Option<&str>,
) -> Result<HighRiskGate, String> {
    if consume_high_risk_approval_on(connection, scan_id, attempt_number, fencing_token, action_fingerprint)? {
        return Ok(HighRiskGate::Allow);
    }
    // Operator-authorized auto-allow. The deliberate per-task high-risk opt-in
    // (the capability class authorized for this attempt + the legal-authorization
    // attestation that INSERT enforces) IS the operator's decision. When it is in
    // force AND the action is reversible / non-destructive — the read-only
    // SQL-injection differential and the no-network analysis sandbox pass
    // reversible=true — the class-level decision stands in for a second per-action
    // click, so active testing actually runs once the operator has authorized it.
    // Irreversible actions (reversible=false: the target-scoped network PoC and
    // non-harmless shell commands) still require the single-use operator approval
    // below. The decision is recorded as auto-approved+consumed for the audit
    // trail; unlike a manual approval it is not single-use, so the executor may
    // repeat a reversible probe across injection points without re-asking.
    // Axis 2 stays in front of the picker: a tier never authorizes a class the
    // operator has not attested. `ask` always waits. `auto` (the default, and
    // what tests hit when no tier is stored) auto-allows only reversible work.
    // `full` auto-allows irreversible work on an already-authorized class.
    // SSRF and scope checks are not in this function and are not affected.
    let tier = operator_permission_tier(connection);
    let class_authorized = matches!(
        scan_capability_class_authorized(connection, scan_id, attempt_number, capability_class),
        Ok(true)
    );
    let mut tier_allows = match tier.as_str() {
        "ask" => false,
        "full" => true,
        _ => reversible,
    };
    if let Some(tool) = enforcing_tool.filter(|name| !name.is_empty()) {
        match tool_permission_decision(connection, tool) {
            ToolPermissionDecision::Deny => {
                return Ok(HighRiskGate::Denied("工具已被权限规则拒绝".into()));
            }
            ToolPermissionDecision::Ask => tier_allows = false,
            ToolPermissionDecision::Allow | ToolPermissionDecision::Default => {}
        }
    }
    if class_authorized && tier_allows {
        // An explicit operator rejection of this exact action is more specific
        // than the class-level authorization and must win over the auto-allow, so
        // a "no" the operator already gave is never silently overridden.
        if high_risk_approval_storage_exists(connection)? {
            let prior: Option<String> = connection
                .query_row(
                    "SELECT status FROM high_risk_action_approvals \
                     WHERE scan_id=?1 AND attempt_number=?2 AND fencing_token=?3 AND action_fingerprint=?4",
                    params![scan_id, attempt_number, fencing_token, action_fingerprint],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if prior.as_deref() == Some("rejected") {
                return Ok(HighRiskGate::Denied("operator 已拒绝该动作".into()));
            }
        }
        connection
            .execute_batch(HIGH_RISK_APPROVAL_SCHEMA)
            .map_err(|_| "high_risk_approval_storage_unavailable")?;
        connection
            .execute(
                "INSERT OR IGNORE INTO high_risk_action_approvals(request_id,scan_id,attempt_number,fencing_token,capability_class,action_fingerprint,action_summary,target,reversible,status,decided_by,decided_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,'consumed','auto:class-authorized',datetime('now','localtime'))",
                params![uuid::Uuid::new_v4().to_string(), scan_id, attempt_number, fencing_token, capability_class, action_fingerprint, action_summary, target],
            )
            .map_err(|e| e.to_string())?;
        return Ok(HighRiskGate::Allow);
    }
    let (request_id, status) = request_high_risk_action_on(
        connection, scan_id, attempt_number, fencing_token, capability_class, action_fingerprint, action_summary, target, reversible,
    )?;
    match status.as_str() {
        "rejected" => Ok(HighRiskGate::Denied("operator 已拒绝该动作".into())),
        "consumed" => Ok(HighRiskGate::Denied("该动作的批准已被使用，需重新批准".into())),
        _ => Ok(HighRiskGate::Pending(request_id)),
    }
}

fn operator_permission_tier(connection: &rusqlite::Connection) -> String {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='agent_permission_tier'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .filter(|value| matches!(value.as_str(), "ask" | "auto" | "full"))
        .unwrap_or_else(|| "auto".to_string())
}

#[tauri::command]
pub fn get_operator_permission_tier(state: State<AppState>) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    Ok(operator_permission_tier(&connection))
}

#[tauri::command]
pub fn set_operator_permission_tier(state: State<AppState>, tier: String) -> Result<(), String> {
    if !matches!(tier.as_str(), "ask" | "auto" | "full") {
        return Err("permission_tier_invalid".into());
    }
    let connection = db::open(&state.db_path)?;
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('agent_permission_tier',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [&tier],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn list_pending_high_risk_actions_in(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Vec<JsonValue>, String> {
    if !high_risk_approval_storage_exists(connection)? {
        return Ok(vec![]);
    }
    let mut statement = connection
        .prepare(
            "SELECT request_id, attempt_number, capability_class, action_summary, target, reversible, created_at \
             FROM high_risk_action_approvals WHERE scan_id=?1 AND status='pending' ORDER BY created_at",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([scan_id], |r| {
            Ok(serde_json::json!({
                "requestId": r.get::<_, String>(0)?,
                "attemptNumber": r.get::<_, i64>(1)?,
                "capabilityClass": r.get::<_, String>(2)?,
                "actionSummary": r.get::<_, String>(3)?,
                "target": r.get::<_, String>(4)?,
                "reversible": r.get::<_, i64>(5)? != 0,
                "createdAt": r.get::<_, String>(6)?,
            }))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

fn decide_high_risk_action_inner(
    db_path: &Path,
    request_id: &str,
    approve: bool,
    decided_by: &str,
) -> Result<(), String> {
    let mut connection = db::open(db_path)?;
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    decide_high_risk_action_on(&tx, request_id, approve, decided_by)?;
    tx.commit().map_err(|e| e.to_string())
}

/// Operator: list the high-risk actions this scan has pending approval.
#[tauri::command]
pub fn list_pending_high_risk_actions(
    state: State<AppState>,
    scan_id: String,
) -> Result<Vec<JsonValue>, String> {
    let connection = db::open(&state.db_path)?;
    list_pending_high_risk_actions_in(&connection, &scan_id)
}

/// Operator: approve one pending high-risk action (single-use).
#[tauri::command]
pub fn approve_high_risk_action(
    state: State<AppState>,
    request_id: String,
    decided_by: Option<String>,
) -> Result<(), String> {
    decide_high_risk_action_inner(&state.db_path, &request_id, true, decided_by.as_deref().unwrap_or(""))
}

/// Operator: reject one pending high-risk action.
#[tauri::command]
pub fn reject_high_risk_action(
    state: State<AppState>,
    request_id: String,
    decided_by: Option<String>,
) -> Result<(), String> {
    decide_high_risk_action_inner(&state.db_path, &request_id, false, decided_by.as_deref().unwrap_or(""))
}

#[cfg(test)]
mod high_risk_approval_tests {
    use super::{
        consume_high_risk_approval_on, decide_high_risk_action_on, high_risk_action_gate_on,
        list_pending_high_risk_actions_in, request_high_risk_action_on, HighRiskGate,
    };
    use rusqlite::Connection;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY);
             INSERT INTO sentinel_scans(id) VALUES('s');",
        )
        .unwrap();
        c
    }

    #[test]
    fn unknown_class_rejected() {
        let c = db();
        assert!(request_high_risk_action_on(&c, "s", 1, "ftok", "make_coffee", "fp", "sum", "t", false).is_err());
    }

    #[test]
    fn request_is_idempotent_per_action() {
        let c = db();
        let (id1, s1) = request_high_risk_action_on(&c, "s", 1, "ftok", "code_exec", "fp1", "id", "t", true).unwrap();
        let (id2, s2) = request_high_risk_action_on(&c, "s", 1, "ftok", "code_exec", "fp1", "id", "t", true).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(s1, "pending");
        assert_eq!(s2, "pending");
        assert_eq!(list_pending_high_risk_actions_in(&c, "s").unwrap().len(), 1);
    }

    #[test]
    fn gate_pending_then_allow_once_after_approval() {
        let c = db();
        // First encounter → pending (agent waits), one pending row listed.
        let gate = high_risk_action_gate_on(&c, "s", 1, "ftok", "code_exec", "fp", "run id", "host", false, None).unwrap();
        let request_id = match gate {
            HighRiskGate::Pending(id) => id,
            other => panic!("expected pending, got {other:?}"),
        };
        assert_eq!(list_pending_high_risk_actions_in(&c, "s").unwrap().len(), 1);
        // Operator approves → next gate consumes it exactly once.
        decide_high_risk_action_on(&c, &request_id, true, "op").unwrap();
        assert_eq!(high_risk_action_gate_on(&c, "s", 1, "ftok", "code_exec", "fp", "run id", "host", false, None).unwrap(), HighRiskGate::Allow);
        // Single-use: a second attempt at the same action is denied.
        assert!(matches!(
            high_risk_action_gate_on(&c, "s", 1, "ftok", "code_exec", "fp", "run id", "host", false, None).unwrap(),
            HighRiskGate::Denied(_)
        ));
    }

    #[test]
    fn rejected_action_stays_denied() {
        let c = db();
        let (request_id, _) = request_high_risk_action_on(&c, "s", 1, "ftok", "exfil", "fp", "sum", "t", false).unwrap();
        decide_high_risk_action_on(&c, &request_id, false, "op").unwrap();
        assert!(matches!(
            high_risk_action_gate_on(&c, "s", 1, "ftok", "exfil", "fp", "sum", "t", false, None).unwrap(),
            HighRiskGate::Denied(_)
        ));
        // A decided request can't be re-decided.
        assert!(decide_high_risk_action_on(&c, &request_id, true, "op").is_err());
    }

    #[test]
    fn consume_requires_prior_approval() {
        let c = db();
        request_high_risk_action_on(&c, "s", 1, "ftok", "code_exec", "fp", "sum", "t", false).unwrap();
        // Pending, not approved → cannot consume.
        assert!(!consume_high_risk_approval_on(&c, "s", 1, "ftok", "fp").unwrap());
        // Different fencing token → no match.
        let (rid, _) = request_high_risk_action_on(&c, "s", 1, "ftok", "code_exec", "fp", "sum", "t", false).unwrap();
        decide_high_risk_action_on(&c, &rid, true, "op").unwrap();
        assert!(!consume_high_risk_approval_on(&c, "s", 1, "other-ftok", "fp").unwrap());
        assert!(consume_high_risk_approval_on(&c, "s", 1, "ftok", "fp").unwrap());
    }

    /// Minimal authorization row the capability reader accepts: attested, with the
    /// class in the JSON list (mirrors scan_capability_authorizations).
    fn authorize(c: &Connection, class: &str) {
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS scan_capability_authorizations(\
                 scan_id TEXT, attempt_number INTEGER, legal_attestation INTEGER, authorized_classes TEXT);",
        )
        .unwrap();
        c.execute(
            "INSERT INTO scan_capability_authorizations(scan_id,attempt_number,legal_attestation,authorized_classes) \
             VALUES('s',1,1,?1)",
            [format!("[\"{class}\"]")],
        )
        .unwrap();
    }

    #[test]
    fn reversible_action_auto_allows_when_class_authorized() {
        let c = db();
        authorize(&c, "target_write");
        // Reversible + class authorized → runs with no per-point operator approval.
        assert_eq!(
            high_risk_action_gate_on(&c, "s", 1, "ftok", "target_write", "fp", "sum", "t", true, None).unwrap(),
            HighRiskGate::Allow
        );
        // Nothing is left waiting in the operator's inbox.
        assert_eq!(list_pending_high_risk_actions_in(&c, "s").unwrap().len(), 0);
        // Not single-use: another injection point under the same class also runs.
        assert_eq!(
            high_risk_action_gate_on(&c, "s", 1, "ftok", "target_write", "fp2", "sum", "t", true, None).unwrap(),
            HighRiskGate::Allow
        );
    }

    #[test]
    fn irreversible_action_still_pends_when_class_authorized() {
        let c = db();
        authorize(&c, "egress");
        // Irreversible (reversible=false) is never auto-allowed by class authorization
        // alone; the operator still decides it per action.
        assert!(matches!(
            high_risk_action_gate_on(&c, "s", 1, "ftok", "egress", "fp", "sum", "t", false, None).unwrap(),
            HighRiskGate::Pending(_)
        ));
        assert_eq!(list_pending_high_risk_actions_in(&c, "s").unwrap().len(), 1);
    }

    #[test]
    fn operator_rejection_overrides_reversible_auto_allow() {
        let c = db();
        authorize(&c, "target_write");
        // The operator explicitly rejected this exact action earlier.
        let (request_id, _) =
            request_high_risk_action_on(&c, "s", 1, "ftok", "target_write", "fp", "sum", "t", true).unwrap();
        decide_high_risk_action_on(&c, &request_id, false, "op").unwrap();
        // A per-point "no" wins over the class-level auto-allow.
        assert!(matches!(
            high_risk_action_gate_on(&c, "s", 1, "ftok", "target_write", "fp", "sum", "t", true, None).unwrap(),
            HighRiskGate::Denied(_)
        ));
    }

    #[test]
    fn reversible_action_pends_without_authorization() {
        let c = db();
        // No authorization row at all → reversible action is not auto-allowed.
        assert!(matches!(
            high_risk_action_gate_on(&c, "s", 1, "ftok", "target_write", "fp", "sum", "t", true, None).unwrap(),
            HighRiskGate::Pending(_)
        ));
    }
}
