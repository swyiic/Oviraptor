// Controller-phase capability leases (design Option A / B3.2). In the single-
// controller model the one controller run holds, for a bounded PHASE, the
// capabilities of the equivalent former role — leased to its OWN run, bound to
// the current coordinator lease epoch + fencing token, released when the phase
// ends. This is a SEPARATE store from agent_capability_leases (whose assignment_id
// is a NOT NULL FK to a child assignment the controller run has no part in):
// keeping it separate means the existing child-executor authorization path is
// untouched. Nothing in production issues these yet, so behavior is unchanged
// until the per-call chokepoint learns to honor them (B3.2b) and the coordinator
// learns to enter phases (later). Every grant is validated exactly like the
// equivalent child contract via validate_controller_phase — same lane, same
// allow-list, same B1/B2 high-risk gate — so one controller relaxes no per-role
// boundary. Each item is #[allow(dead_code)] until the chokepoint/coordinator
// wiring consumes it.

#[allow(dead_code)]
const CONTROLLER_PHASE_LEASE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_controller_phase_leases (
    id TEXT PRIMARY KEY,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    target_key TEXT NOT NULL,
    phase TEXT NOT NULL,
    capability TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    fencing_token TEXT NOT NULL,
    lease_expires_at TEXT NOT NULL,
    revoked_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(root_run_id, lease_epoch, fencing_token, phase, capability)
);
"#;

#[allow(dead_code)]
fn controller_phase_str(profile: ControllerProfile) -> &'static str {
    match profile {
        ControllerProfile::Planner => "planner",
        ControllerProfile::Recon => "recon",
        ControllerProfile::Web => "web",
        ControllerProfile::Api => "api",
        ControllerProfile::Validator => "validator",
    }
}

#[allow(dead_code)]
fn controller_phase_lease_storage_exists(connection: &Connection) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='agent_controller_phase_leases')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "controller_phase_lease_storage_unavailable".into())
}

#[allow(dead_code, clippy::too_many_arguments)]
fn insert_controller_phase_lease(
    connection: &Connection,
    root_run_id: &str,
    scan_id: &str,
    attempt_number: i64,
    target_key: &str,
    phase: &str,
    capability: &str,
    lease_epoch: i64,
    fencing_token: &str,
    lease_expires_at: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO agent_controller_phase_leases(id,root_run_id,scan_id,attempt_number,target_key,phase,capability,lease_epoch,fencing_token,lease_expires_at) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) \
             ON CONFLICT(root_run_id,lease_epoch,fencing_token,phase,capability) DO NOTHING",
            params![
                uuid::Uuid::new_v4().to_string(), root_run_id, scan_id, attempt_number, target_key,
                phase, capability, lease_epoch, fencing_token, lease_expires_at
            ],
        )
        .map_err(|e| format!("无法签发控制器相位租约：{e}"))?;
    Ok(())
}

/// True iff a valid controller-phase lease (unrevoked, unexpired, matching the
/// current lease epoch + fencing token) exists for this capability on the
/// controller run. This is the predicate the per-call chokepoint will add as a
/// branch PARALLEL to the existing child-executor path (B3.2b); it never widens
/// that path. Returns false (fail closed) when the store does not exist yet.
#[allow(dead_code)]
fn controller_phase_lease_active(
    connection: &Connection,
    root_run_id: &str,
    lease_epoch: i64,
    fencing_token: &str,
    capability: &str,
) -> Result<bool, String> {
    if !controller_phase_lease_storage_exists(connection)? {
        return Ok(false);
    }
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_controller_phase_leases \
             WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3 AND capability=?4 \
               AND revoked_at='' AND lease_expires_at>datetime('now','localtime'))",
            params![root_run_id, lease_epoch, fencing_token, capability],
            |r| r.get(0),
        )
        .map_err(|_| "controller_phase_lease_lookup_failed".into())
}

/// Authorize a tool call made by the CONTROLLER run itself (single-controller
/// model, B3.2b). Grants IFF `run_id` is a LIVE coordinator root run for its scan
/// attempt AND holds a valid controller-phase lease for `capability` bound to
/// that coordinator lease's CURRENT epoch + fencing token. The per-call
/// chokepoint ORs this alongside the child-executor path: it can only grant,
/// never widen that path. Fail-closed on every miss — no store, not the live
/// coordinator, cancelled, stale epoch, wrong fencing, revoked, or expired
/// (either the phase lease or the coordinator lease). A lease roll changes the
/// epoch/fencing and instantly invalidates every prior controller-phase lease.
pub(crate) fn controller_phase_authorized(
    connection: &Connection,
    run_id: &str,
    capability: &str,
) -> Result<bool, String> {
    if !controller_phase_lease_storage_exists(connection)? {
        return Ok(false);
    }
    connection
        .query_row(
            "SELECT EXISTS(\
             SELECT 1 FROM agent_runs r \
             JOIN agent_coordinator_leases c \
               ON c.root_run_id=r.id AND c.scan_id=r.scan_id \
               AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url \
             JOIN agent_controller_phase_leases p \
               ON p.root_run_id=r.id AND p.capability=?2 \
               AND p.lease_epoch=c.lease_epoch AND p.fencing_token=c.fencing_token \
               AND p.revoked_at='' AND p.lease_expires_at>datetime('now','localtime') \
             WHERE r.id=?1 AND r.root_run_id=r.id AND r.role='coordinator' \
               AND r.status IN ('prepared','running') AND r.cancel_requested_at='' \
               AND c.lease_expires_at>datetime('now','localtime'))",
            params![run_id, capability],
            |r| r.get(0),
        )
        .map_err(|_| "controller_phase_authorization_unavailable".into())
}

/// Release every capability of a phase for this controller run + epoch + fencing.
#[allow(dead_code)]
fn revoke_controller_phase(
    connection: &Connection,
    root_run_id: &str,
    lease_epoch: i64,
    fencing_token: &str,
    phase: &str,
) -> Result<(), String> {
    if !controller_phase_lease_storage_exists(connection)? {
        return Ok(());
    }
    connection
        .execute(
            "UPDATE agent_controller_phase_leases SET revoked_at=datetime('now','localtime') \
             WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3 AND phase=?4 AND revoked_at=''",
            params![root_run_id, lease_epoch, fencing_token, phase],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Enter a phase: validate it exactly like the equivalent child contract, then
/// lease the phase's capabilities to the controller's own run (bound to the
/// current epoch + fencing). Issues nothing if validation fails. Not called in
/// production yet (the coordinator does not enter phases until a later step), so
/// the system is unchanged until then.
#[allow(dead_code, clippy::too_many_arguments)]
fn lease_controller_phase(
    connection: &Connection,
    lease: &CoordinatorLease,
    profile: ControllerProfile,
    task_slice: &JsonValue,
    evidence_revision: i64,
    capabilities: &[String],
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<(), String> {
    validate_controller_phase(
        connection, lease, profile, task_slice, evidence_revision, capabilities, reserved_tokens,
        reserved_requests,
    )?;
    connection
        .execute_batch(CONTROLLER_PHASE_LEASE_SCHEMA)
        .map_err(|_| "controller_phase_lease_storage_unavailable")?;
    let phase = controller_phase_str(profile);
    for capability in capabilities {
        insert_controller_phase_lease(
            connection,
            &lease.root_run_id,
            &lease.scan_id,
            lease.attempt_number,
            &lease.target_key,
            phase,
            capability,
            lease.lease_epoch,
            &lease.fencing_token,
            &lease.lease_expires_at,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod controller_phase_lease_tests {
    use super::{
        controller_phase_authorized, controller_phase_lease_active, insert_controller_phase_lease,
        revoke_controller_phase, CONTROLLER_PHASE_LEASE_SCHEMA,
    };
    use rusqlite::{params, Connection};

    const FUTURE: &str = "2999-01-01 00:00:00";
    const PAST: &str = "2000-01-01 00:00:00";

    // A DB with the three tables the authorization chain joins. `run`/`coord`/
    // `phase` insert one row each; each test seeds a valid baseline and mutates
    // exactly one field to prove that field is load-bearing (fail-closed).
    fn authz_db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE agent_runs(id TEXT PRIMARY KEY, root_run_id TEXT, role TEXT, status TEXT, cancel_requested_at TEXT, scan_id TEXT, attempt_number INTEGER, target_url TEXT);
             CREATE TABLE agent_coordinator_leases(root_run_id TEXT, scan_id TEXT, attempt_number INTEGER, target_key TEXT, lease_epoch INTEGER, fencing_token TEXT, lease_expires_at TEXT);",
        )
        .unwrap();
        c.execute_batch(CONTROLLER_PHASE_LEASE_SCHEMA).unwrap();
        c
    }
    fn run(c: &Connection, role: &str, status: &str, cancel: &str) {
        c.execute(
            "INSERT INTO agent_runs(id,root_run_id,role,status,cancel_requested_at,scan_id,attempt_number,target_url) VALUES('ctrl','ctrl',?1,?2,?3,'s',1,'t')",
            params![role, status, cancel],
        ).unwrap();
    }
    fn coord(c: &Connection, epoch: i64, fencing: &str, expires: &str) {
        c.execute(
            "INSERT INTO agent_coordinator_leases(root_run_id,scan_id,attempt_number,target_key,lease_epoch,fencing_token,lease_expires_at) VALUES('ctrl','s',1,'t',?1,?2,?3)",
            params![epoch, fencing, expires],
        ).unwrap();
    }
    fn phase(c: &Connection, epoch: i64, fencing: &str, expires: &str, revoked: &str) {
        c.execute(
            "INSERT INTO agent_controller_phase_leases(id,root_run_id,scan_id,attempt_number,target_key,phase,capability,lease_epoch,fencing_token,lease_expires_at,revoked_at) VALUES('p','ctrl','s',1,'t','web','replay_http',?1,?2,?3,?4)",
            params![epoch, fencing, expires, revoked],
        ).unwrap();
    }
    fn authorized(c: &Connection) -> bool {
        controller_phase_authorized(c, "ctrl", "replay_http").unwrap()
    }

    #[test]
    fn authorizes_a_live_controller_phase() {
        let c = authz_db();
        run(&c, "coordinator", "running", "");
        coord(&c, 7, "ftok", FUTURE);
        phase(&c, 7, "ftok", FUTURE, "");
        assert!(authorized(&c));
        // Wrong capability is never granted.
        assert!(!controller_phase_authorized(&c, "ctrl", "finish_target").unwrap());
    }

    #[test]
    fn denied_on_stale_epoch_or_wrong_fencing() {
        let stale = authz_db();
        run(&stale, "coordinator", "running", "");
        coord(&stale, 8, "ftok", FUTURE); // lease rolled to epoch 8
        phase(&stale, 7, "ftok", FUTURE, ""); // phase lease still epoch 7
        assert!(!authorized(&stale));

        let forged = authz_db();
        run(&forged, "coordinator", "running", "");
        coord(&forged, 7, "ftok", FUTURE);
        phase(&forged, 7, "other", FUTURE, ""); // fencing mismatch
        assert!(!authorized(&forged));
    }

    #[test]
    fn denied_on_revoked_or_expired() {
        let revoked = authz_db();
        run(&revoked, "coordinator", "running", "");
        coord(&revoked, 7, "ftok", FUTURE);
        phase(&revoked, 7, "ftok", FUTURE, "2020-01-01 00:00:00");
        assert!(!authorized(&revoked));

        let phase_expired = authz_db();
        run(&phase_expired, "coordinator", "running", "");
        coord(&phase_expired, 7, "ftok", FUTURE);
        phase(&phase_expired, 7, "ftok", PAST, "");
        assert!(!authorized(&phase_expired));

        let coord_expired = authz_db();
        run(&coord_expired, "coordinator", "running", "");
        coord(&coord_expired, 7, "ftok", PAST);
        phase(&coord_expired, 7, "ftok", FUTURE, "");
        assert!(!authorized(&coord_expired));
    }

    #[test]
    fn denied_when_not_the_live_coordinator() {
        // Not a coordinator run (e.g. a child web_executor): never via this branch.
        let not_coord = authz_db();
        run(&not_coord, "web_executor", "running", "");
        coord(&not_coord, 7, "ftok", FUTURE);
        phase(&not_coord, 7, "ftok", FUTURE, "");
        assert!(!authorized(&not_coord));

        // Cancellation requested → denied.
        let cancelled = authz_db();
        run(&cancelled, "coordinator", "running", "2026-01-01 00:00:00");
        coord(&cancelled, 7, "ftok", FUTURE);
        phase(&cancelled, 7, "ftok", FUTURE, "");
        assert!(!authorized(&cancelled));

        // No coordinator lease at all → denied.
        let no_lease = authz_db();
        run(&no_lease, "coordinator", "running", "");
        phase(&no_lease, 7, "ftok", FUTURE, "");
        assert!(!authorized(&no_lease));
    }

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        // The lease table's root_run_id FK references agent_runs; production has
        // it, so the test stubs the parent + the controller run row.
        c.execute_batch(
            "CREATE TABLE agent_runs(id TEXT PRIMARY KEY);
             INSERT INTO agent_runs(id) VALUES('ctrl');",
        )
        .unwrap();
        c.execute_batch(CONTROLLER_PHASE_LEASE_SCHEMA).unwrap();
        c
    }

    #[test]
    fn active_only_for_matching_unrevoked_unexpired_lease() {
        let c = db();
        insert_controller_phase_lease(&c, "ctrl", "s", 1, "t", "web", "replay_http", 7, "ftok", "2999-01-01 00:00:00").unwrap();
        assert!(controller_phase_lease_active(&c, "ctrl", 7, "ftok", "replay_http").unwrap());
        // Fencing isolation: wrong epoch / fencing / capability never matches.
        assert!(!controller_phase_lease_active(&c, "ctrl", 8, "ftok", "replay_http").unwrap());
        assert!(!controller_phase_lease_active(&c, "ctrl", 7, "other", "replay_http").unwrap());
        assert!(!controller_phase_lease_active(&c, "ctrl", 7, "ftok", "finish_target").unwrap());
    }

    #[test]
    fn expired_lease_is_not_active() {
        let c = db();
        insert_controller_phase_lease(&c, "ctrl", "s", 1, "t", "web", "replay_http", 7, "ftok", "2000-01-01 00:00:00").unwrap();
        assert!(!controller_phase_lease_active(&c, "ctrl", 7, "ftok", "replay_http").unwrap());
    }

    #[test]
    fn revoke_deactivates_the_phase() {
        let c = db();
        insert_controller_phase_lease(&c, "ctrl", "s", 1, "t", "web", "replay_http", 7, "ftok", "2999-01-01 00:00:00").unwrap();
        assert!(controller_phase_lease_active(&c, "ctrl", 7, "ftok", "replay_http").unwrap());
        revoke_controller_phase(&c, "ctrl", 7, "ftok", "web").unwrap();
        assert!(!controller_phase_lease_active(&c, "ctrl", 7, "ftok", "replay_http").unwrap());
    }

    #[test]
    fn active_false_when_store_absent() {
        let c = Connection::open_in_memory().unwrap();
        assert!(!controller_phase_lease_active(&c, "ctrl", 7, "ftok", "replay_http").unwrap());
    }
}
