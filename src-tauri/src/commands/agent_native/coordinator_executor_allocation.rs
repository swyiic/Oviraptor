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
    let slots =
        native_coordinator_executor_proposal_slots(available_tokens, available_requests, tokens, requests);
    if available_tokens.is_some() {
        tokens -= slots * crate::agent_runtime::multi_agent::directive::proposals::PROPOSAL_TOKENS;
    }
    if available_requests.is_some() {
        requests -= slots;
    }
    Ok(Some((tokens, requests, true)))
}

// Pure "ordered proposal slot" decision over the post-floor executor window
// (B3.3c step 1). The creation-only executor rule carves 0/1/2 proposal slots
// from the remaining pool: 2 once >=12000 tokens AND >=3 requests survive, else 1
// once >=8000 AND >=2 survive, else 0. `None` means that dimension is unbounded
// (unlimited) and never gates the slot count. Extracted so the slot formula — a
// quantity B3 preserves while reframing "N executor" to "controller+validator" —
// is pinned by a hermetic characterization. Byte-identical to the former inline
// expression. (`prepare.rs` keeps its own copy, which uses 0 for "unbounded";
// Shape B removes that duplicate when the child dispatch goes away.)
fn native_coordinator_executor_proposal_slots(
    available_tokens: Option<i64>,
    available_requests: Option<i64>,
    tokens: i64,
    requests: i64,
) -> i64 {
    if (available_tokens.is_none() || tokens >= 12_000)
        && (available_requests.is_none() || requests >= 3)
    {
        2
    } else if (available_tokens.is_none() || tokens >= 8_000)
        && (available_requests.is_none() || requests >= 2)
    {
        1
    } else {
        0
    }
}

// This is retained capacity, not a fabricated model Reserve or a future bill.
// Existing Native contracts keep their original floor and original grant.
fn native_coordinator_executor_retained_floor(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(i64, i64), String> {
    let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("web_mode_root_missing")?;
    native_coordinator_executor_retained_floor_for(
        &mode.bootstrap_dispatch().unwrap_or(JsonValue::Null),
    )
}

// Pure supervision-floor decision over the frozen bootstrap contract, extracted
// (B3.3c step 1 — same pattern as `native_coordinator_token_estimate`) so the
// "coordinator + N executor" root-supervision floor — the exact quantity B3
// reframes to "controller + validator" — is pinned by a hermetic characterization
// without standing up the web_mode root. Behavior is byte-identical to the former
// inline match: a Null/empty bootstrap retains (15000,1); the one recognised
// raised floor (16000,1) retains (31000,2); every other shape is a conflict.
fn native_coordinator_executor_retained_floor_for(
    bootstrap: &JsonValue,
) -> Result<(i64, i64), String> {
    match (
        bootstrap.get("rootSupervisionTokenFloor"),
        bootstrap.get("rootSupervisionRequestFloor"),
    ) {
        (None, None) => Ok((15_000, 1)),
        (Some(t), Some(r)) if *t == json!(16_000) && *r == json!(1) => Ok((31_000, 2)),
        _ => Err("executor_allocation_original_supervision_floor_conflict".into()),
    }
}

#[cfg(test)]
mod executor_retained_floor_characterization {
    use super::native_coordinator_executor_retained_floor_for;
    use serde_json::{json, Value};

    // Default "coordinator + N executor" supervision floor: no explicit floor in
    // the frozen bootstrap ⇒ retain 15000 tokens / 1 request for root supervision.
    // A Null bootstrap (no contract) behaves the same as an empty object.
    #[test]
    fn default_bootstrap_retains_15000_tokens_1_request() {
        assert_eq!(
            native_coordinator_executor_retained_floor_for(&json!({})).unwrap(),
            (15_000, 1)
        );
        assert_eq!(
            native_coordinator_executor_retained_floor_for(&Value::Null).unwrap(),
            (15_000, 1)
        );
    }

    // The one recognised raised floor: (16000,1) ⇒ retain (31000,2).
    #[test]
    fn recognised_raised_supervision_floor_retains_31000_tokens_2_requests() {
        assert_eq!(
            native_coordinator_executor_retained_floor_for(
                &json!({"rootSupervisionTokenFloor": 16_000, "rootSupervisionRequestFloor": 1})
            )
            .unwrap(),
            (31_000, 2)
        );
    }

    // Any other shape is a conflict, fail-closed: a partial floor, an
    // unrecognised value, or the wrong JSON type. This guards the reframing — a
    // controller+validator floor change must be a deliberate edit to the helper,
    // never a silent acceptance of an unverified bootstrap shape.
    #[test]
    fn partial_or_unrecognised_supervision_floor_is_a_conflict() {
        for bootstrap in [
            json!({"rootSupervisionTokenFloor": 16_000}),
            json!({"rootSupervisionRequestFloor": 1}),
            json!({"rootSupervisionTokenFloor": 16_000, "rootSupervisionRequestFloor": 2}),
            json!({"rootSupervisionTokenFloor": 8_000, "rootSupervisionRequestFloor": 1}),
            json!({"rootSupervisionTokenFloor": "16000", "rootSupervisionRequestFloor": 1}),
        ] {
            assert!(
                native_coordinator_executor_retained_floor_for(&bootstrap).is_err(),
                "bootstrap {bootstrap} must be a supervision-floor conflict"
            );
        }
    }
}

#[cfg(test)]
mod executor_proposal_slots_characterization {
    use super::native_coordinator_executor_proposal_slots;

    // Bounded pool: both thresholds gate the slot count.
    #[test]
    fn bounded_pool_thresholds_pick_2_then_1_then_0() {
        let s = native_coordinator_executor_proposal_slots;
        // 2 slots exactly at (12000 tokens, 3 requests) and above.
        assert_eq!(s(Some(12_000), Some(3), 12_000, 3), 2);
        assert_eq!(s(Some(99_999), Some(9), 99_999, 9), 2);
        // Just below either 2-slot threshold drops to 1 (still >=8000 & >=2).
        assert_eq!(s(Some(11_999), Some(3), 11_999, 3), 1);
        assert_eq!(s(Some(12_000), Some(2), 12_000, 2), 1);
        // 1 slot exactly at (8000, 2).
        assert_eq!(s(Some(8_000), Some(2), 8_000, 2), 1);
        // Below either 1-slot threshold is 0.
        assert_eq!(s(Some(7_999), Some(2), 7_999, 2), 0);
        assert_eq!(s(Some(8_000), Some(1), 8_000, 1), 0);
        assert_eq!(s(Some(0), Some(0), 0, 0), 0);
    }

    // `None` (unbounded) never gates that dimension; the other still does.
    #[test]
    fn unbounded_dimension_does_not_gate_slots() {
        let s = native_coordinator_executor_proposal_slots;
        // Both unbounded ⇒ always 2 regardless of the (ignored) value args.
        assert_eq!(s(None, None, 0, 0), 2);
        // Tokens unbounded, requests gate: 3 ⇒ 2, 2 ⇒ 1, 1 ⇒ 0.
        assert_eq!(s(None, Some(3), 0, 3), 2);
        assert_eq!(s(None, Some(2), 0, 2), 1);
        assert_eq!(s(None, Some(1), 0, 1), 0);
        // Requests unbounded, tokens gate: 12000 ⇒ 2, 8000 ⇒ 1, 7999 ⇒ 0.
        assert_eq!(s(Some(12_000), None, 12_000, 0), 2);
        assert_eq!(s(Some(8_000), None, 8_000, 0), 1);
        assert_eq!(s(Some(7_999), None, 7_999, 0), 0);
    }
}
