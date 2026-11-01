// New creation-only executor rule; current ledger read is inside paid issuance TX.
fn native_coordinator_executor_allocation(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    trigger: &str,
    revision: i64,
) -> Result<Option<(i64, i64, bool)>, String> {
    use crate::agent_runtime::{multi_agent::budget, store};
    let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("web_mode_root_missing")?;
    let bootstrap = mode.bootstrap_dispatch().unwrap_or(JsonValue::Null);
    if bootstrap["executorAllocationRule"]
        != "current_balance_after_reviewer_floor_and_ordered_proposals"
    {
        return Ok(None);
    }
    let local = mode
        .local_deliberation()
        .ok_or("executor_allocation_original_floor_missing")?;
    if local["reviewerTokenFloor"] != 15_000 || local["reviewerRequestFloor"] != 1 {
        return Err("executor_allocation_original_floor_conflict".into());
    }
    budget::admission::require_determinate(db, &actor.root_run_id)?;
    let dedup = format!("web_executor:{trigger}:{}:{revision}", actor.target_key);
    let id = format!(
        "asg-{}",
        &store::stable_hash(&format!("{}:{dedup}", actor.root_run_id))[..24]
    );
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1)",
            [&id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        let (tokens, requests) = budget::model::original_web_grant(db, actor, &id)?;
        return Ok(Some((tokens, requests, false)));
    }
    let (token_floor, request_floor) = native_coordinator_executor_retained_floor(db, actor)?;
    // The retained reviewer floor alone is smaller than a single coordinator
    // supervision tick, so granting the executor everything above the floor let
    // it hold nearly the whole gross ceiling (observed: 789k/900k reserved, 0
    // consumed). The coordinator's continuing ticks and the executor's own
    // sub-reservations then had no room and tripped
    // child_budget_reservation_exceeded_or_stale, deferring the web branch before
    // it ran. Reserve bounded continuation headroom above the floor so the
    // coordinator can keep supervising and the executor can actually start. This
    // only sizes THIS grant smaller; it changes no hard cap, no floor, and no
    // other reservation. The deeper two-ledger reservation-timing fix stays
    // tracked in docs/SESSION_HARDENING_STATUS.md.
    const COORDINATOR_CONTINUATION_HEADROOM_TOKENS: i64 = 120_000;
    let token_reserve = token_floor + COORDINATOR_CONTINUATION_HEADROOM_TOKENS;
    let (available_tokens, available_requests) =
        match budget::root::remaining_child_capacity(db, &actor.root_run_id) {
            Ok(capacity) => capacity,
            // A root that is transiently fully occupied — a large tick reservation
            // still held while this grant is being sized — makes remaining capacity
            // momentarily negative, which `available` surfaces as
            // budget_hard_limit_exceeded. For sizing the web-executor grant that is
            // just "no budget window right now," the same as the too-little-capacity
            // case below; defer the web branch gracefully (the map/surface children
            // are already recorded) instead of hard-failing the whole scan and
            // discarding their completed work. This changes no reservation and no
            // cap — only how a read-time capacity shortfall is classified. The
            // deeper reservation-timing fix is tracked in docs/SESSION_HARDENING_STATUS.md.
            Err(error) if error == "budget_hard_limit_exceeded" => {
                return Err("multi_agent_executor_budget_unavailable".into());
            }
            Err(error) => return Err(error),
        };
    if available_tokens.is_some_and(|n| n <= token_reserve)
        || available_requests.is_some_and(|n| n <= request_floor)
    {
        return Err("multi_agent_executor_budget_unavailable".into());
    }
    let mut tokens = available_tokens.map_or(0, |n| n - token_reserve);
    let mut requests = available_requests.map_or(0, |n| n - request_floor);
    if budget::supervision_capacity::enabled(&bootstrap) {
        if let Some(cap) = budget::supervision_capacity::executor_requests_on(db, actor, available_requests)? {
            requests = requests.min(cap);
        }
    }
    let slots = if (available_tokens.is_none() || tokens >= 12_000)
        && (available_requests.is_none() || requests >= 3)
    {
        2
    } else if (available_tokens.is_none() || tokens >= 8_000)
        && (available_requests.is_none() || requests >= 2)
    {
        1
    } else {
        0
    };
    if available_tokens.is_some() {
        tokens -= slots * crate::agent_runtime::multi_agent::directive::proposals::PROPOSAL_TOKENS;
    }
    if available_requests.is_some() {
        requests -= slots;
    }
    Ok(Some((tokens, requests, true)))
}

// This is retained capacity, not a fabricated model Reserve or a future bill.
// Existing Native contracts keep their original floor and original grant.
fn native_coordinator_executor_retained_floor(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(i64, i64), String> {
    let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("web_mode_root_missing")?;
    let original = mode.bootstrap_dispatch().unwrap_or(JsonValue::Null);
    match (
        original.get("rootSupervisionTokenFloor"),
        original.get("rootSupervisionRequestFloor"),
    ) {
        (None, None) => Ok((15_000, 1)),
        (Some(t), Some(r)) if *t == json!(16_000) && *r == json!(1) => Ok((31_000, 2)),
        _ => Err("executor_allocation_original_supervision_floor_conflict".into()),
    }
}
