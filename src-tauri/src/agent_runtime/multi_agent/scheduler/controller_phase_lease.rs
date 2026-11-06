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
// boundary.
//
// B3-2 wiring: the coordinator now enters the zero-capability Planner phase on
// its own run (flag-gated, default OFF — see coordinator_enter_planner_phase),
// so the issue/release path below is live. Leasing a capability-bearing phase
// whose lease a live controller TOOL CALL consumes stays deferred to B3-3 (it
// needs the controller execution window + the per-role surface preconditions),
// so controller_phase_lease_active remains reserved for that step.

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

fn controller_phase_str(profile: ControllerProfile) -> &'static str {
    match profile {
        ControllerProfile::Planner => "planner",
        ControllerProfile::Recon => "recon",
        ControllerProfile::Web => "web",
        ControllerProfile::Api => "api",
        ControllerProfile::Validator => "validator",
    }
}

fn controller_phase_lease_storage_exists(connection: &Connection) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='agent_controller_phase_leases')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "controller_phase_lease_storage_unavailable".into())
}

#[allow(clippy::too_many_arguments)]
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
/// Reserved for B3-3: the execution-time active-lease check a live controller
/// tool call will use (today the chokepoint uses `controller_phase_authorized`,
/// which additionally proves the live coordinator binding), so it is exercised
/// by tests but not yet on a non-test path.
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
pub(crate) fn revoke_controller_phase(
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
/// current epoch + fencing). Issues nothing if validation fails. Called in
/// production for the Planner phase (zero capabilities) via
/// coordinator_enter_planner_phase, flag-gated and default OFF; the capability-
/// bearing Web/Api phases are leased here via coordinator_enter_execution_phase.
///
/// B3-4 (independent validation档, hard invariants enforced here): the single
/// controller run can NEVER become its own validator — it cannot grade its own
/// homework. (1) The Validator phase is refused outright: validation stays a
/// SEPARATE EvidenceReviewer run on the exclusive Review lane, with its own
/// identity + evidence. (2) No controller phase (deliberation OR execution) may
/// ever be leased a `review.*` capability, so the execution brain can never
/// author a review verdict even if a caller mis-passes the capability.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lease_controller_phase(
    connection: &Connection,
    lease: &CoordinatorLease,
    profile: ControllerProfile,
    task_slice: &JsonValue,
    evidence_revision: i64,
    capabilities: &[String],
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<(), String> {
    if profile == ControllerProfile::Validator {
        return Err("controller_phase_validator_must_be_separate_run".into());
    }
    if capabilities.iter().any(|c| c.starts_with("review.")) {
        return Err("controller_phase_must_not_hold_review_capability".into());
    }
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

/// B3-2 production entry: the coordinator enters the Planner phase on its OWN
/// run. Planner is pure deliberation — it leases NO capability and reserves NO
/// budget (reserved_tokens/requests = 0), so it touches neither the child-
/// executor authorization path nor the budget root; it is the safest possible
/// live exercise of the enter path, bound to the live coordinator lease epoch +
/// fencing. Flag-gated by the caller (scan_controller_phase_leasing_on, default
/// OFF), so production is byte-identical until an operator froze it ON.
pub(crate) fn coordinator_enter_planner_phase(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<(), String> {
    lease_controller_phase(
        connection,
        lease,
        ControllerProfile::Planner,
        &JsonValue::Null,
        0,
        &[],
        0,
        0,
    )
}

/// Release the controller's deliberation phase(s) for the current epoch +
/// fencing. Symmetric to coordinator_enter_planner_phase; safe to call even when
/// nothing was leased (it revokes zero rows). A lease roll (new epoch/fencing)
/// already invalidates every prior phase lease, so this is belt-and-suspenders.
pub(crate) fn coordinator_release_controller_phases(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<(), String> {
    revoke_controller_phase(
        connection,
        &lease.root_run_id,
        lease.lease_epoch,
        &lease.fencing_token,
        controller_phase_str(ControllerProfile::Planner),
    )
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

    // ---- B3-2: coordinator enter/release-phase (fail-closed) ----
    use super::{
        coordinator_enter_planner_phase, coordinator_release_controller_phases,
        lease_controller_phase, ControllerProfile, CoordinatorLease,
    };

    fn lease_for(epoch: i64, fencing: &str) -> CoordinatorLease {
        CoordinatorLease {
            scan_id: "s".into(),
            attempt_number: 1,
            target_key: "t".into(),
            root_run_id: "ctrl".into(),
            lease_epoch: epoch,
            fencing_token: fencing.into(),
            lease_expires_at: FUTURE.into(),
        }
    }

    fn phase_row_count(c: &Connection) -> i64 {
        c.query_row("SELECT COUNT(*) FROM agent_controller_phase_leases", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn enter_planner_phase_leases_nothing_and_is_budget_neutral() {
        // Planner is zero-capability deliberation: entering it creates the store
        // but issues NO lease row (no capability, no budget).
        let c = db();
        coordinator_enter_planner_phase(&c, &lease_for(7, "ftok")).unwrap();
        assert_eq!(phase_row_count(&c), 0, "Planner must lease no capability");
    }

    #[test]
    fn planner_phase_refuses_any_capability() {
        // The Planner phase can never be used to smuggle a capability in: a
        // non-empty capability set is rejected by validate_controller_phase and
        // nothing is issued. (Fail-closed: the deliberation phase grants none.)
        let c = db();
        let err = lease_controller_phase(
            &c,
            &lease_for(7, "ftok"),
            ControllerProfile::Planner,
            &serde_json::Value::Null,
            0,
            &["evidence.read".into()],
            0,
            0,
        )
        .unwrap_err();
        assert_eq!(err, "controller_planner_phase_grants_no_capabilities");
        // The store may or may not have been created, but no lease row exists.
        if super::controller_phase_lease_storage_exists(&c).unwrap() {
            assert_eq!(phase_row_count(&c), 0);
        }
    }

    #[test]
    fn release_revokes_only_the_matching_epoch_and_fencing() {
        // Seed a (capability-bearing) planner-phase lease, then prove release
        // withdraws it only for the matching epoch + fencing — a stale epoch or
        // a forged fencing token leaves it untouched (fail-closed boundary).
        let c = db();
        insert_controller_phase_lease(&c, "ctrl", "s", 1, "t", "planner", "evidence.read", 7, "ftok", FUTURE).unwrap();
        assert!(controller_phase_lease_active(&c, "ctrl", 7, "ftok", "evidence.read").unwrap());
        // Wrong epoch / wrong fencing: nothing is revoked.
        coordinator_release_controller_phases(&c, &lease_for(8, "ftok")).unwrap();
        coordinator_release_controller_phases(&c, &lease_for(7, "other")).unwrap();
        assert!(controller_phase_lease_active(&c, "ctrl", 7, "ftok", "evidence.read").unwrap());
        // Matching epoch + fencing: revoked.
        coordinator_release_controller_phases(&c, &lease_for(7, "ftok")).unwrap();
        assert!(!controller_phase_lease_active(&c, "ctrl", 7, "ftok", "evidence.read").unwrap());
    }

    #[test]
    fn release_is_safe_when_store_absent_or_nothing_leased() {
        // Release before any enter (no store) is a no-op, not an error.
        let bare = Connection::open_in_memory().unwrap();
        bare.execute_batch("CREATE TABLE agent_runs(id TEXT PRIMARY KEY); INSERT INTO agent_runs(id) VALUES('ctrl');").unwrap();
        coordinator_release_controller_phases(&bare, &lease_for(7, "ftok")).unwrap();
        // Enter (nothing leased) then release round-trips cleanly.
        let c = db();
        coordinator_enter_planner_phase(&c, &lease_for(7, "ftok")).unwrap();
        coordinator_release_controller_phases(&c, &lease_for(7, "ftok")).unwrap();
        assert_eq!(phase_row_count(&c), 0);
    }

    // ---- B3-4: the single controller run can never self-validate ----

    #[test]
    fn validator_phase_is_refused_on_the_controller_run() {
        // The Validator must stay a SEPARATE EvidenceReviewer run (Review lane)
        // with its own identity + evidence; leasing it to the controller's own
        // run is refused outright, so the single brain cannot grade its own work.
        let c = db();
        let err = lease_controller_phase(
            &c,
            &lease_for(7, "ftok"),
            ControllerProfile::Validator,
            &serde_json::Value::Null,
            0,
            &["review.write".into()],
            0,
            0,
        )
        .unwrap_err();
        assert_eq!(err, "controller_phase_validator_must_be_separate_run");
    }

    #[test]
    fn no_controller_phase_may_hold_a_review_capability() {
        // Even a (non-Validator) execution/deliberation phase cannot be leased a
        // review.* capability — review authorship belongs only to the separate
        // validator run. Refused before any store write.
        let c = db();
        for profile in [ControllerProfile::Planner, ControllerProfile::Web, ControllerProfile::Api] {
            let err = lease_controller_phase(
                &c,
                &lease_for(7, "ftok"),
                profile,
                &serde_json::Value::Null,
                0,
                &["review.write".into()],
                0,
                0,
            )
            .unwrap_err();
            assert_eq!(err, "controller_phase_must_not_hold_review_capability");
        }
    }

    // ---- B3-3: a capability-bearing execution phase lease is load-bearing ----

    // A DB with a live coordinator run row rich enough for validate_surface_role
    // (non-source web plan) plus the coordinator lease + phase-lease store, so a
    // real Web-phase lease validates exactly like the WebExecutor child contract.
    fn web_phase_db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE agent_runs(id TEXT PRIMARY KEY, root_run_id TEXT, role TEXT, status TEXT, cancel_requested_at TEXT, scan_id TEXT, attempt_number INTEGER, target_url TEXT, plan_json TEXT, hard_request_budget INTEGER, orchestration_policy TEXT, assignment_id TEXT, parent_run_id TEXT);
             INSERT INTO agent_runs(id,root_run_id,role,status,cancel_requested_at,scan_id,attempt_number,target_url,plan_json,hard_request_budget,orchestration_policy,assignment_id,parent_run_id) \
               VALUES('ctrl','ctrl','coordinator','running','','s',1,'t','{\"surface\":\"web\"}',100,'multi','',NULL);
             CREATE TABLE agent_coordinator_leases(root_run_id TEXT, scan_id TEXT, attempt_number INTEGER, target_key TEXT, lease_epoch INTEGER, fencing_token TEXT, lease_expires_at TEXT);
             INSERT INTO agent_coordinator_leases(root_run_id,scan_id,attempt_number,target_key,lease_epoch,fencing_token,lease_expires_at) VALUES('ctrl','s',1,'t',7,'ftok','2999-01-01 00:00:00');",
        )
        .unwrap();
        c.execute_batch(CONTROLLER_PHASE_LEASE_SCHEMA).unwrap();
        c
    }

    #[test]
    fn web_execution_phase_authorizes_only_its_leased_capability() {
        // Enter the Web execution phase (WebExecutor / target-touching) on the
        // controller's OWN run: it validates via the SAME child contract, leases
        // the executor tool capability to the controller run bound to the live
        // coordinator epoch + fencing, and the per-call chokepoint predicate then
        // authorizes exactly that capability on the controller run — and nothing
        // else (phase boundary == equivalent child allow-list, fail-closed).
        let c = web_phase_db();
        lease_controller_phase(
            &c,
            &lease_for(7, "ftok"),
            ControllerProfile::Web,
            &serde_json::Value::Null,
            0,
            &["replay_http".into()],
            0,
            0,
        )
        .unwrap();
        assert!(controller_phase_authorized(&c, "ctrl", "replay_http").unwrap());
        // A capability outside the lease is never authorized...
        assert!(!controller_phase_authorized(&c, "ctrl", "finish_target").unwrap());
        // ...and in particular review authorship is never reachable this way.
        assert!(!controller_phase_authorized(&c, "ctrl", "review.write").unwrap());
        // Release withdraws it; the chokepoint then fails closed.
        revoke_controller_phase(&c, "ctrl", 7, "ftok", "web").unwrap();
        assert!(!controller_phase_authorized(&c, "ctrl", "replay_http").unwrap());
    }

    #[test]
    fn web_phase_lease_is_bound_to_the_live_coordinator_epoch_and_fencing() {
        // A lease roll (new epoch/fencing on the coordinator lease) instantly
        // invalidates a prior Web-phase lease: the controller is no longer
        // authorized, even though the phase-lease row still exists.
        let c = web_phase_db();
        lease_controller_phase(
            &c,
            &lease_for(7, "ftok"),
            ControllerProfile::Web,
            &serde_json::Value::Null,
            0,
            &["replay_http".into()],
            0,
            0,
        )
        .unwrap();
        assert!(controller_phase_authorized(&c, "ctrl", "replay_http").unwrap());
        c.execute("UPDATE agent_coordinator_leases SET lease_epoch=8, fencing_token='rolled'", [])
            .unwrap();
        assert!(!controller_phase_authorized(&c, "ctrl", "replay_http").unwrap());
    }
}
