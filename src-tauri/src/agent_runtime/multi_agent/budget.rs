//! Fenced append-only budget transitions. All writes require the caller's
//! transaction, so admission, assignment, lane and accounting commit together.
use super::lease::{validate_coordinator_lease, CoordinatorLease};
use rusqlite::{params, Transaction};

pub(crate) mod admission;
pub(crate) mod clock;
pub(crate) mod continuation;
pub(crate) mod diagnostics;
mod entries;
mod execution_slots;
pub(crate) use entries::balance_for_attempt;
mod historical;
pub(crate) mod limits;
pub(crate) mod model;
pub(crate) mod model_facts;
pub(crate) mod receipts;
pub(crate) mod root;
pub(crate) mod root_definition;
mod scope;
pub(crate) mod supervision_capacity;
pub(crate) mod target;
pub(crate) mod web_lifetime;

// SQL aliases d/t denote original dispatch/terminal receipt. A call id alone
// cannot close an obligation restored under a different worker or request.
pub(crate) const WEB_RECEIPT_BINDING: &str = "t.root_run_id=d.root_run_id
    AND t.assignment_id=d.assignment_id AND t.child_run_id=d.child_run_id
    AND t.lease_epoch=d.lease_epoch AND t.fencing_token=d.fencing_token
    AND t.round=d.round AND t.request_hash=d.request_hash";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Authority {
    Live,
    ClockSample,
    TerminalReceipt,
    BoundReceipt,
}

pub const DIMENSIONS: [&str; 10] = [
    "model_input_tokens",
    "model_cached_tokens",
    "model_output_tokens",
    "model_requests",
    "target_requests",
    "browser_actions",
    "controlled_writes",
    "upload_bytes",
    "concurrency_batches",
    "wall_time_ms",
];

/// No live append writer yet; diagnostics coverage stays `denied_by_contract`.
pub const DENIED_BY_CONTRACT_DIMENSIONS: &[&str] = &[
    "browser_actions",
    "controlled_writes",
    "upload_bytes",
];


#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BudgetVector {
    pub model_input_tokens: i64,
    pub model_cached_tokens: i64,
    pub model_output_tokens: i64,
    pub model_requests: i64,
    pub target_requests: i64,
    pub browser_actions: i64,
    pub controlled_writes: i64,
    pub upload_bytes: i64,
    pub concurrency_batches: i64,
    pub wall_time_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Reserve,
    Consume,
    Release,
    Forfeit,
    Reconcile,
}
impl Kind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "reserve" => Ok(Self::Reserve),
            "consume" => Ok(Self::Consume),
            "release" => Ok(Self::Release),
            "forfeit" => Ok(Self::Forfeit),
            "reconcile" => Ok(Self::Reconcile),
            _ => Err("budget_kind_invalid".into()),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Reserve => "reserve",
            Self::Consume => "consume",
            Self::Release => "release",
            Self::Forfeit => "forfeit",
            Self::Reconcile => "reconcile",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Balance {
    pub reserved: i64,
    pub consumed: i64,
    pub indeterminate: i64,
}

pub fn balance(
    db: &rusqlite::Connection,
    root: &str,
    assignment: Option<&str>,
    dimension: &str,
) -> Result<Balance, String> {
    // A misspelled or unsupported name is not evidence of an unspent budget.
    // Configured zero-use dimensions still read normally; caller ownership
    // and snapshot proofs remain separate requirements.
    if !DIMENSIONS.contains(&dimension) {
        return Err("budget_dimension_unknown".into());
    }
    let mut statement=db.prepare("SELECT kind,amount FROM agent_budget_entries WHERE root_run_id=?1 AND (?2 IS NULL OR assignment_id=?2) AND dimension=?3 ORDER BY rowid").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map(params![root, assignment, dimension], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut b = Balance::default();
    for row in rows {
        let (kind, amount) = row.map_err(|e| e.to_string())?;
        apply(&mut b, &kind, amount)?;
    }
    Ok(b)
}

fn add(value: i64, amount: i64) -> Result<i64, String> {
    value
        .checked_add(amount)
        .ok_or("budget_amount_overflow".into())
}
fn subtract(value: i64, amount: i64) -> Result<i64, String> {
    value
        .checked_sub(amount)
        .filter(|v| *v >= 0)
        .ok_or("budget_transition_exceeds_balance".into())
}
fn apply(b: &mut Balance, kind: &str, amount: i64) -> Result<(), String> {
    match Kind::parse(kind)? {
        Kind::Reserve => b.reserved = add(b.reserved, amount)?,
        Kind::Consume => {
            b.reserved = subtract(b.reserved, amount)?;
            b.consumed = add(b.consumed, amount)?;
        }
        Kind::Release => b.reserved = subtract(b.reserved, amount)?,
        Kind::Forfeit => {
            b.reserved = subtract(b.reserved, amount)?;
            b.indeterminate = add(b.indeterminate, amount)?;
        }
        Kind::Reconcile => {
            b.indeterminate = subtract(b.indeterminate, amount)?;
            b.consumed = add(b.consumed, amount)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod settlement_characterization {
    // Characterization harness for the append-only budget fold (a B3.3
    // prerequisite). These pin the settlement invariants a controller+validator
    // budget rework must preserve: a reservation nets to zero once consumed or
    // released (no leak); a forfeited reservation becomes indeterminate until a
    // reconcile consumes it (crash spend is never silently refunded); an
    // impossible transition is refused rather than going negative; an unknown
    // dimension is never read as spare budget.
    use super::{balance, Balance, DIMENSIONS};
    use rusqlite::{params, Connection};

    const DIM: &str = DIMENSIONS[0]; // "model_input_tokens"

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE agent_budget_entries(root_run_id TEXT, assignment_id TEXT, dimension TEXT, kind TEXT, amount INTEGER);",
        )
        .unwrap();
        c
    }
    fn entry(c: &Connection, kind: &str, amount: i64) {
        c.execute(
            "INSERT INTO agent_budget_entries(root_run_id,assignment_id,dimension,kind,amount) VALUES('root','a',?1,?2,?3)",
            params![DIM, kind, amount],
        )
        .unwrap();
    }
    fn bal(c: &Connection) -> Balance {
        balance(c, "root", None, DIM).unwrap()
    }

    #[test]
    fn reserve_then_consume_settles_reserved_to_zero() {
        let c = db();
        entry(&c, "reserve", 100);
        assert_eq!(bal(&c).reserved, 100);
        entry(&c, "consume", 100);
        let b = bal(&c);
        assert_eq!(b.reserved, 0); // settled — no leak
        assert_eq!(b.consumed, 100);
    }

    #[test]
    fn release_returns_the_reservation_without_spending() {
        let c = db();
        entry(&c, "reserve", 40);
        entry(&c, "release", 40);
        let b = bal(&c);
        assert_eq!(b.reserved, 0);
        assert_eq!(b.consumed, 0);
    }

    #[test]
    fn forfeit_becomes_indeterminate_then_reconcile_consumes_it() {
        let c = db();
        entry(&c, "reserve", 30);
        entry(&c, "forfeit", 30);
        let b = bal(&c);
        assert_eq!(b.reserved, 0);
        assert_eq!(b.indeterminate, 30); // crash-orphaned: unknown, never auto-refunded
        entry(&c, "reconcile", 30);
        let b = bal(&c);
        assert_eq!(b.indeterminate, 0);
        assert_eq!(b.consumed, 30);
    }

    #[test]
    fn an_impossible_transition_is_refused_not_negative() {
        let c = db();
        entry(&c, "reserve", 10);
        entry(&c, "release", 11); // over-release
        assert!(balance(&c, "root", None, DIM).is_err());
    }

    #[test]
    fn unknown_dimension_is_rejected() {
        let c = db();
        assert!(balance(&c, "root", None, "not_a_dimension").is_err());
    }

    // Option b co-resides TWO assignment scopes on ONE root: the controller's
    // model scope (assignment '') and a separate execution assignment. They must
    // fold INDEPENDENTLY so execution usage is never double-counted into the model
    // scope, and the root aggregate (assignment=None) is exactly their sum — the
    // guard against the double-count failure mode a shared-root settlement risks.
    #[test]
    fn per_assignment_scopes_fold_independently_on_one_root() {
        let c = db();
        let scoped = |asg: &str, kind: &str, amount: i64| {
            c.execute(
                "INSERT INTO agent_budget_entries(root_run_id,assignment_id,dimension,kind,amount) VALUES('root',?1,?2,?3,?4)",
                params![asg, DIM, kind, amount],
            )
            .unwrap();
        };
        // model '' : reserve 100 → consume 60 → release 40  (reserved 0, consumed 60)
        scoped("", "reserve", 100);
        scoped("", "consume", 60);
        scoped("", "release", 40);
        // exec : reserve 30 → consume 30  (reserved 0, consumed 30)
        scoped("exec", "reserve", 30);
        scoped("exec", "consume", 30);
        let model = balance(&c, "root", Some(""), DIM).unwrap();
        let exec = balance(&c, "root", Some("exec"), DIM).unwrap();
        assert_eq!((model.reserved, model.consumed), (0, 60), "model scope folds alone");
        assert_eq!((exec.reserved, exec.consumed), (0, 30), "exec scope folds alone");
        // The root aggregate is exactly their sum — no cross-scope leak either way.
        let all = balance(&c, "root", None, DIM).unwrap();
        assert_eq!((all.reserved, all.consumed), (0, 90));
    }
}

/// A reconciliation conservatively consumes a previously indeterminate charge.
/// It never refunds sent work. The receipt-verifying caller owns that authority.
#[allow(clippy::too_many_arguments)]
pub fn append(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
) -> Result<(), String> {
    append_inner(
        tx,
        lease,
        assignment,
        dimension,
        kind,
        amount,
        key,
        source,
        Authority::Live,
    )
}

/// Only a separately verified saved receipt may use an expired, unchanged
/// terminal root owner. This never grants capability or changes run status.
#[allow(clippy::too_many_arguments)]
pub(crate) fn append_terminal_receipt(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
) -> Result<(), String> {
    append_inner(
        tx,
        lease,
        assignment,
        dimension,
        kind,
        amount,
        key,
        source,
        Authority::TerminalReceipt,
    )
}

/// A verified failed dispatch can record its unknown debt after expiry. This
/// moves existing reservations only and grants no execution or refund.
pub(crate) fn append_unknown_cost(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    amount: i64,
    source: &str,
) -> Result<(), String> {
    append_inner(
        tx,
        lease,
        assignment,
        dimension,
        Kind::Forfeit,
        amount,
        &format!("unknown:{source}:{dimension}"),
        source,
        Authority::BoundReceipt,
    )
}

// The caller has verified a durable model receipt in this same transaction.
// Recording its cost after expiry never authorizes another dispatch or refund.
#[allow(clippy::too_many_arguments)]
fn append_received_cost(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
) -> Result<(), String> {
    validate_received_transition(tx, lease, dimension, kind)?;
    append_inner(
        tx,
        lease,
        assignment,
        dimension,
        kind,
        amount,
        key,
        source,
        Authority::BoundReceipt,
    )
}

fn validate_received_transition(
    _tx: &Transaction<'_>,
    _lease: &CoordinatorLease,
    dimension: &str,
    kind: Kind,
) -> Result<(), String> {
    // A receipt may extend a token-class reservation to the actual reported cost
    // even when a hard limit is configured: the real ceiling is enforced by
    // persist_inner (`budget_hard_limit_exceeded`) when the reserve is appended,
    // so a known overrun settles within the Root budget instead of forfeiting to
    // an indeterminate balance. Only non-token dimensions (a class cannot be
    // reserved outside input/cached/output) and non-settlement kinds are invalid
    // at receipt time.
    if kind == Kind::Reserve {
        if !DIMENSIONS[..3].contains(&dimension) {
            return Err("budget_receipt_transition_invalid".into());
        }
    } else if !matches!(kind, Kind::Consume | Kind::Forfeit) {
        return Err("budget_receipt_transition_invalid".into());
    }
    Ok(())
}

fn validate_budget_owner(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    authority: Authority,
) -> Result<(), String> {
    if matches!(authority, Authority::Live | Authority::ClockSample) {
        validate_coordinator_lease(tx, lease)?;
    }
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_coordinator_leases c ON c.root_run_id=r.id
        WHERE r.id=?1 AND r.backend='native' AND r.role='coordinator' AND r.status<>'legacy_backend_removed'
        AND r.scan_id=?2 AND r.attempt_number=?3 AND r.target_url=?4
        AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url
        AND c.lease_epoch=?5 AND c.fencing_token=?6 AND (?7=0 OR r.status='terminal'))",
        params![lease.root_run_id,lease.scan_id,lease.attempt_number,lease.target_key,lease.lease_epoch,lease.fencing_token,authority==Authority::TerminalReceipt],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {
        return Err("budget_root_authority_conflict".into());
    }
    Ok(())
}

fn validate_budget_assignment(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    kind: Kind,
) -> Result<(), String> {
    let bound:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND target_key=?3
        AND ((lease_epoch=?4 AND fencing_token=?5) OR (?6='reserve' AND state='prepared' AND lease_epoch=0 AND fencing_token='')))",
        params![assignment,lease.root_run_id,lease.target_key,lease.lease_epoch,lease.fencing_token,kind.as_str()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound {
        return Err("budget_assignment_scope_conflict".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_inner(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
    authority: Authority,
) -> Result<(), String> {
    validate_budget_owner(tx, lease, authority)?;
    if !DIMENSIONS.contains(&dimension)
        || amount <= 0
        || key.trim().is_empty()
        || source.trim().is_empty()
    {
        return Err("budget_entry_invalid".into());
    }
    if DENIED_BY_CONTRACT_DIMENSIONS.contains(&dimension) {
        return Err("budget_dimension_denied_by_contract".into());
    }
    validate_budget_assignment(tx, lease, assignment, kind)?;
    if authority == Authority::Live && kind == Kind::Reserve {
        super::attempts::require_reservation(tx, lease, assignment)?;
    }
    let attempt = super::attempts::current(tx, lease, assignment)?.id;
    let write = if matches!(
        authority,
        Authority::BoundReceipt | Authority::TerminalReceipt
    ) && kind == Kind::Reserve
    {
        entries::persist_known_cost
    } else {
        entries::persist
    };
    write(
        tx,
        &lease.root_run_id,
        assignment,
        attempt.clone(),
        dimension,
        kind,
        amount,
        key,
        source,
    )?;
    validate_budget_owner(tx, lease, authority)?;
    validate_budget_assignment(tx, lease, assignment, kind)?;
    if super::attempts::current(tx, lease, assignment)?.id != attempt {
        return Err("budget_attempt_scope_conflict".into());
    }
    if authority == Authority::Live && kind == Kind::Reserve {
        super::attempts::require_reservation(tx, lease, assignment)?;
    }
    Ok(())
}
