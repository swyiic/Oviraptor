// Controller phase-execution budget (design Option A / B3-3 / D2). In the single-
// controller model the ONE controller (coordinator root) run both decides AND
// executes target-touching. Its execution spend must be a SEGREGATED, per-phase
// budget — never laundered under the coordinator's own model-cost identity, and
// never written to `agent_runs.used_*` (the root birth invariant `used_*=0` must
// hold). It must ALSO stay off the hash-chained model-tick core.
//
// The main ledger cannot host it cleanly: a non-root (`assignment<>''`) scope in
// `agent_budget_entries` is keyed through `scope::stored_key`, which demands an
// `agent_assignment_attempts` row — i.e. the distinct-child-run lifecycle the
// single controller run does not have (that path IS the child executor, Option a).
// So the controller phase-execution budget is its OWN isolated append-only ledger
// here, keyed by the live coordinator lease epoch + fencing + phase. It mirrors
// the main ledger's reserve → consume/release (settle-to-zero) / forfeit
// semantics, is bound to the fencing chain (a lease roll strands a prior window,
// exactly like the phase lease), and NEVER touches `agent_runs` or the tick core.
// The eventual cutover reserves this window FROM the root pool before executing
// and settles it from the measured execution span; this module is that window's
// accounting, proven in isolation.

const CONTROLLER_PHASE_BUDGET_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_controller_phase_budget_entries (
    id TEXT PRIMARY KEY,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    lease_epoch INTEGER NOT NULL,
    fencing_token TEXT NOT NULL,
    phase TEXT NOT NULL,
    dimension TEXT NOT NULL CHECK(dimension IN ('tokens','requests')),
    kind TEXT NOT NULL CHECK(kind IN ('reserve','consume','release','forfeit')),
    amount INTEGER NOT NULL CHECK(amount >= 0),
    idempotency_key TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(root_run_id, lease_epoch, fencing_token, phase, dimension, idempotency_key)
);
"#;

const PHASE_BUDGET_DIMENSIONS: [&str; 2] = ["tokens", "requests"];

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct PhaseBudgetBalance {
    reserved: i64,
    consumed: i64,
    released: i64,
    forfeited: i64,
}

impl PhaseBudgetBalance {
    /// Still-held reservation = reserved − consumed − released − forfeited.
    fn outstanding(self) -> i64 {
        self.reserved - self.consumed - self.released - self.forfeited
    }
}

fn controller_phase_budget_storage_exists(connection: &Connection) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='agent_controller_phase_budget_entries')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "controller_phase_budget_storage_unavailable".into())
}

/// Append-only fold for one (root, epoch, fencing, phase, dimension) window.
/// Idempotent on `idempotency_key`: the same key re-applied is a no-op (so a
/// retried settlement never double-counts); a different amount under an existing
/// key is a replay conflict, fail-closed.
#[allow(clippy::too_many_arguments)]
fn append_phase_budget_entry(
    connection: &Connection,
    lease: &CoordinatorLease,
    phase: &str,
    dimension: &str,
    kind: &str,
    amount: i64,
    idempotency_key: &str,
) -> Result<(), String> {
    if amount < 0 {
        return Err("controller_phase_budget_amount_invalid".into());
    }
    let existing: Option<(String, i64)> = connection
        .query_row(
            "SELECT kind,amount FROM agent_controller_phase_budget_entries \
             WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3 AND phase=?4 AND dimension=?5 AND idempotency_key=?6",
            params![lease.root_run_id, lease.lease_epoch, lease.fencing_token, phase, dimension, idempotency_key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((existing_kind, existing_amount)) = existing {
        return if existing_kind == kind && existing_amount == amount {
            Ok(())
        } else {
            Err("controller_phase_budget_replay_conflict".into())
        };
    }
    connection
        .execute(
            "INSERT INTO agent_controller_phase_budget_entries(id,root_run_id,lease_epoch,fencing_token,phase,dimension,kind,amount,idempotency_key) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                uuid::Uuid::new_v4().to_string(), lease.root_run_id, lease.lease_epoch,
                lease.fencing_token, phase, dimension, kind, amount, idempotency_key
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn phase_budget_balance(
    connection: &Connection,
    root_run_id: &str,
    lease_epoch: i64,
    fencing_token: &str,
    phase: &str,
    dimension: &str,
) -> Result<PhaseBudgetBalance, String> {
    if !controller_phase_budget_storage_exists(connection)? {
        return Ok(PhaseBudgetBalance::default());
    }
    let mut statement = connection
        .prepare(
            "SELECT kind,amount FROM agent_controller_phase_budget_entries \
             WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3 AND phase=?4 AND dimension=?5",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(
            params![root_run_id, lease_epoch, fencing_token, phase, dimension],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let mut balance = PhaseBudgetBalance::default();
    for row in rows {
        let (kind, amount) = row.map_err(|e| e.to_string())?;
        match kind.as_str() {
            "reserve" => balance.reserved += amount,
            "consume" => balance.consumed += amount,
            "release" => balance.released += amount,
            "forfeit" => balance.forfeited += amount,
            _ => return Err("controller_phase_budget_kind_invalid".into()),
        }
    }
    Ok(balance)
}

/// Reserve the controller's per-phase execution window (tokens + requests),
/// bound to the live coordinator lease epoch + fencing. Idempotent per
/// (epoch, fencing, phase); a second reserve for the same window is refused so a
/// phase cannot silently grow its grant. Never writes `agent_runs.used_*`.
#[allow(dead_code)] // consumed by the controller-execution cutover (D2, flag-gated); proven in isolation here
fn reserve_phase_execution_window(
    connection: &Connection,
    lease: &CoordinatorLease,
    phase: &str,
    tokens: i64,
    requests: i64,
) -> Result<(), String> {
    if tokens < 0 || requests < 0 {
        return Err("controller_phase_budget_amount_invalid".into());
    }
    connection
        .execute_batch(CONTROLLER_PHASE_BUDGET_SCHEMA)
        .map_err(|_| "controller_phase_budget_storage_unavailable")?;
    for (dimension, amount) in PHASE_BUDGET_DIMENSIONS.iter().zip([tokens, requests]) {
        // A pre-existing reservation for this exact window is a conflict (not a
        // silent grow), unless it is a byte-identical retry of this same reserve.
        let balance = phase_budget_balance(
            connection, &lease.root_run_id, lease.lease_epoch, &lease.fencing_token, phase, dimension,
        )?;
        if balance.reserved != 0 && balance.reserved != amount {
            return Err("controller_phase_budget_already_reserved".into());
        }
        if amount > 0 {
            append_phase_budget_entry(connection, lease, phase, dimension, "reserve", amount, "reserve")?;
        }
    }
    Ok(())
}

/// Settle the per-phase execution window from the MEASURED execution span:
/// consume the actual spend, release the remainder (settle-to-zero). An actual
/// spend above the reservation is refused (`..._exceeds_reservation`) — a
/// controller phase can never overspend its frozen grant. Idempotent: re-settling
/// the same window with the same usage is a no-op. Never writes `agent_runs.used_*`.
#[allow(dead_code)] // consumed by the controller-execution cutover (D2, flag-gated); proven in isolation here
fn settle_phase_execution_window(
    connection: &Connection,
    lease: &CoordinatorLease,
    phase: &str,
    used_tokens: i64,
    used_requests: i64,
) -> Result<(), String> {
    if used_tokens < 0 || used_requests < 0 {
        return Err("controller_phase_budget_amount_invalid".into());
    }
    if !controller_phase_budget_storage_exists(connection)? {
        return Err("controller_phase_budget_not_reserved".into());
    }
    for (dimension, used) in PHASE_BUDGET_DIMENSIONS.iter().zip([used_tokens, used_requests]) {
        let balance = phase_budget_balance(
            connection, &lease.root_run_id, lease.lease_epoch, &lease.fencing_token, phase, dimension,
        )?;
        // Settle against the STABLE grant (reserved − forfeited), not the
        // post-settle outstanding: this keeps the check and the consume/release
        // amounts identical on a retry, so the idempotent appends below make a
        // re-settle with the same usage a no-op (and a re-settle with DIFFERENT
        // usage a replay conflict, fail-closed). Overspend is always refused.
        let settleable = balance.reserved - balance.forfeited;
        if used > settleable {
            return Err("controller_phase_budget_exceeds_reservation".into());
        }
        if used > 0 {
            append_phase_budget_entry(connection, lease, phase, dimension, "consume", used, "consume")?;
        }
        let remainder = settleable - used;
        if remainder > 0 {
            append_phase_budget_entry(connection, lease, phase, dimension, "release", remainder, "release")?;
        }
    }
    Ok(())
}

/// Forfeit the whole outstanding window when the execution span's usage is
/// UNKNOWN (an interrupted/indeterminate finish): the reservation is burned, not
/// released back, so an unknown-outcome execution can never reclaim budget.
/// Idempotent. Never writes `agent_runs.used_*`.
#[allow(dead_code)] // consumed by the controller-execution cutover (D2, flag-gated); proven in isolation here
fn forfeit_phase_execution_window(
    connection: &Connection,
    lease: &CoordinatorLease,
    phase: &str,
) -> Result<(), String> {
    if !controller_phase_budget_storage_exists(connection)? {
        return Ok(());
    }
    for dimension in PHASE_BUDGET_DIMENSIONS {
        let balance = phase_budget_balance(
            connection, &lease.root_run_id, lease.lease_epoch, &lease.fencing_token, phase, dimension,
        )?;
        let outstanding = balance.outstanding();
        if outstanding > 0 {
            append_phase_budget_entry(connection, lease, phase, dimension, "forfeit", outstanding, "forfeit")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod controller_phase_budget_tests {
    use super::{
        forfeit_phase_execution_window, phase_budget_balance, reserve_phase_execution_window,
        settle_phase_execution_window, CONTROLLER_PHASE_BUDGET_SCHEMA,
    };
    use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
    use rusqlite::Connection;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE agent_runs(id TEXT PRIMARY KEY);
             INSERT INTO agent_runs(id) VALUES('ctrl');",
        )
        .unwrap();
        c.execute_batch(CONTROLLER_PHASE_BUDGET_SCHEMA).unwrap();
        c
    }

    fn lease(epoch: i64, fencing: &str) -> CoordinatorLease {
        CoordinatorLease {
            scan_id: "s".into(),
            attempt_number: 1,
            target_key: "t".into(),
            root_run_id: "ctrl".into(),
            lease_epoch: epoch,
            fencing_token: fencing.into(),
            lease_expires_at: "2999-01-01 00:00:00".into(),
        }
    }

    fn outstanding(c: &Connection, epoch: i64, fencing: &str, dim: &str) -> i64 {
        phase_budget_balance(c, "ctrl", epoch, fencing, "web", dim).unwrap().outstanding()
    }

    #[test]
    fn reserve_then_settle_to_zero_consumes_actual_and_releases_remainder() {
        let c = db();
        reserve_phase_execution_window(&c, &lease(7, "ftok"), "web", 1000, 5).unwrap();
        assert_eq!(outstanding(&c, 7, "ftok", "tokens"), 1000);
        assert_eq!(outstanding(&c, 7, "ftok", "requests"), 5);
        // Spend 600 tokens / 2 requests; the rest is released. Window settles to 0.
        settle_phase_execution_window(&c, &lease(7, "ftok"), "web", 600, 2).unwrap();
        let tok = phase_budget_balance(&c, "ctrl", 7, "ftok", "web", "tokens").unwrap();
        assert_eq!(tok.consumed, 600);
        assert_eq!(tok.released, 400);
        assert_eq!(tok.outstanding(), 0);
        let req = phase_budget_balance(&c, "ctrl", 7, "ftok", "web", "requests").unwrap();
        assert_eq!(req.consumed, 2);
        assert_eq!(req.released, 3);
        assert_eq!(req.outstanding(), 0);
    }

    #[test]
    fn settle_is_idempotent() {
        let c = db();
        reserve_phase_execution_window(&c, &lease(7, "ftok"), "web", 1000, 5).unwrap();
        settle_phase_execution_window(&c, &lease(7, "ftok"), "web", 600, 2).unwrap();
        // Re-settling the same window with the same usage is a no-op (retry-safe).
        settle_phase_execution_window(&c, &lease(7, "ftok"), "web", 600, 2).unwrap();
        assert_eq!(outstanding(&c, 7, "ftok", "tokens"), 0);
    }

    #[test]
    fn overspend_is_refused() {
        let c = db();
        reserve_phase_execution_window(&c, &lease(7, "ftok"), "web", 1000, 5).unwrap();
        let err = settle_phase_execution_window(&c, &lease(7, "ftok"), "web", 1001, 0).unwrap_err();
        assert_eq!(err, "controller_phase_budget_exceeds_reservation");
    }

    #[test]
    fn unknown_outcome_forfeits_the_whole_window() {
        let c = db();
        reserve_phase_execution_window(&c, &lease(7, "ftok"), "web", 1000, 5).unwrap();
        forfeit_phase_execution_window(&c, &lease(7, "ftok"), "web").unwrap();
        assert_eq!(outstanding(&c, 7, "ftok", "tokens"), 0);
        let tok = phase_budget_balance(&c, "ctrl", 7, "ftok", "web", "tokens").unwrap();
        assert_eq!(tok.forfeited, 1000);
        assert_eq!(tok.released, 0, "a forfeited window never releases budget back");
    }

    #[test]
    fn reserve_is_bound_to_epoch_and_fencing() {
        // A lease roll (new epoch/fencing) strands the prior window: the new
        // window is a DISTINCT (epoch,fencing) key, so the old reservation is
        // neither visible nor reusable under the rolled lease.
        let c = db();
        reserve_phase_execution_window(&c, &lease(7, "ftok"), "web", 1000, 5).unwrap();
        assert_eq!(outstanding(&c, 7, "ftok", "tokens"), 1000);
        assert_eq!(outstanding(&c, 8, "rolled", "tokens"), 0);
        // Settling under the rolled lease finds nothing outstanding; any spend is refused.
        let err = settle_phase_execution_window(&c, &lease(8, "rolled"), "web", 1, 0).unwrap_err();
        assert_eq!(err, "controller_phase_budget_exceeds_reservation");
    }

    #[test]
    fn double_reserve_of_a_window_is_refused() {
        let c = db();
        reserve_phase_execution_window(&c, &lease(7, "ftok"), "web", 1000, 5).unwrap();
        // A second reserve with a different grant is refused (no silent grow).
        let err = reserve_phase_execution_window(&c, &lease(7, "ftok"), "web", 2000, 9).unwrap_err();
        assert_eq!(err, "controller_phase_budget_already_reserved");
        // A byte-identical re-reserve is an idempotent no-op.
        reserve_phase_execution_window(&c, &lease(7, "ftok"), "web", 1000, 5).unwrap();
        assert_eq!(outstanding(&c, 7, "ftok", "tokens"), 1000);
    }

    #[test]
    fn never_writes_agent_runs_used_columns() {
        // The isolated ledger has no agent_runs.used_* columns at all; prove the
        // root row is untouched by a full reserve+settle cycle (used_*=0 holds).
        let c = db();
        c.execute_batch("ALTER TABLE agent_runs ADD COLUMN used_tokens INTEGER NOT NULL DEFAULT 0; ALTER TABLE agent_runs ADD COLUMN used_requests INTEGER NOT NULL DEFAULT 0;").unwrap();
        reserve_phase_execution_window(&c, &lease(7, "ftok"), "web", 1000, 5).unwrap();
        settle_phase_execution_window(&c, &lease(7, "ftok"), "web", 600, 2).unwrap();
        let (ut, ur): (i64, i64) = c
            .query_row("SELECT used_tokens,used_requests FROM agent_runs WHERE id='ctrl'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((ut, ur), (0, 0), "phase-exec budget must never touch agent_runs.used_*");
    }
}
