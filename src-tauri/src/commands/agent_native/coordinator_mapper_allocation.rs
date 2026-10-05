// New creation contract only: paid Root + allocation + issuance share one lock.
fn native_coordinator_prepare_mapper_allocated(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    decision: &NativeCoordinatorTickReceipt,
    task: &JsonValue,
    original: &JsonValue,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{budget, scheduler},
        store,
    };
    let floor = json!({"modelTokens":original["localDeliberation"]["reviewerTokenFloor"], "modelRequests":original["localDeliberation"]["reviewerRequestFloor"]});
    // The original local contract already pins these values; no caller floor.
    if floor["modelTokens"] != 15_000 || floor["modelRequests"] != 1 {
        return Err("mapper_allocation_reviewer_floor_conflict".into());
    }
    let verify = |db: &rusqlite::Connection| -> Result<(), String> {
        crate::collaboration_events::dispatch_schema::verify(db)?;
        native_coordinator_tick_authority(db, context, actor)?;
        native_coordinator_verify_original_basis(db, context, actor, original)?;
        decision.tick.require_executable(db)?;
        budget::admission::require_determinate(db, &actor.root_run_id)?;
        if decision.tick.published(db, &decision.saved)? != Some(decision.event_sequence) {
            return Err("root_decision_publication_changed".into());
        }
        Ok(())
    };
    db.authorizer(Some(native_coordinator_bootstrap_authorize))
        .map_err(|e| e.to_string())?;
    let result =
        (|| -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
            let tx =
                rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
                    .map_err(|e| format!("mapper_allocation_lock:{e}"))?;
            verify(&tx)?;
            let dedup = format!(
                "spa_api_mapper:frontend_evidence_ready:{}:1",
                actor.target_key
            );
            let id = format!(
                "asg-{}",
                &store::stable_hash(&format!("{}:{dedup}", actor.root_run_id))[..24]
            );
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1)",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let tokens = if exists {
                budget::model::original_mapper_grant(&tx, actor, &id)?
            } else {
                let (tokens, requests) =
                    budget::root::remaining_child_capacity(&tx, &actor.root_run_id)?;
                if requests.is_some_and(|n| n < 2) {
                    return Err("mapper_allocation_reviewer_floor_exhausted".into());
                }
                tokens.map_or(8_000, |n| n.saturating_sub(15_000).min(8_000))
            };
            if tokens <= 0 {
                return Err("mapper_allocation_reviewer_floor_exhausted".into());
            }
            let mut task = task.clone();
            let object = task.as_object_mut().ok_or("root_decision_task_invalid")?;
            if object.contains_key("rootDecision") {
                return Err("root_decision_task_shadow".into());
            }
            object.insert("rootDecision".into(), json!({"step":"dispatch:spa_api_mapper","eventSequence":decision.event_sequence,
            "summaryHash":store::stable_hash(&decision.summary.as_json().to_string()),"basisHash":store::stable_hash(&original.to_string()),
            "rustPolicy":{"schemaVersion":2,"trigger":"initial_frozen_evidence","selection":"accepted",
                "reasonCode":"frozen_frontend_inventory_requires_independent_mapper","targetRequestsGranted":0,
                "reservedModelTokens":tokens,"reservedModelRequests":1,"targetEvidenceProven":false,
                "allocationRule":"current_balance_after_reviewer_floor","reviewerFloor":floor}}));
            let (child, fresh) = scheduler::prepare_readonly_child_in_transaction(
                &tx,
                actor,
                AgentRole::SpaApiMapper,
                "frontend_evidence_ready",
                &task,
                tokens,
                verify,
            )?;
            if fresh == exists
                || budget::model::original_mapper_grant(&tx, actor, &child.assignment_id)? != tokens
            {
                return Err("mapper_allocation_original_grant_conflict".into());
            }
            let running: bool = tx
                .query_row(
                    "SELECT state='running' FROM agent_assignments WHERE id=?1",
                    [&child.assignment_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if running {
                scheduler::verify_running_mapper_in_transaction(&tx, actor, &child, &task, tokens)?;
            }
            if fresh {
                budget::root::require_child_capacity(&tx, &actor.root_run_id, 15_000, 1)?;
            }
            verify(&tx)?;
            if fresh {
                tx.commit()
                    .map_err(|e| format!("mapper_allocation_commit:{e}"))?;
            }
            Ok(child)
        })();
    use rusqlite::hooks::{AuthContext, Authorization};
    let clear = db
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let child = result?;
    clear?;
    Ok(child)
}
