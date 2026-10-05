//! v3 ordered readonly assessments. Review plans remain immutable and advisory.
use super::*;
use crate::agent_runtime::{
    contract::{AgentLane, AgentRole},
    multi_agent::{attempts, budget, mailbox, scheduler, specialist},
    store::stable_hash,
};
use serde_json::Value;
mod binding;
mod closure;
pub(crate) use closure::{close_for_terminal, ClosureKind};
mod financial;
mod lifecycle;
mod receipts;
mod scheduling;
mod writer;
pub(crate) use binding::{load, source, ActionJob};
pub(crate) use financial::{authorize, metadata, received, recover_received, result_for_mailbox};
pub(crate) use lifecycle::{consume_request, finish_in_transaction, start};
pub(crate) use receipts::{project, verified_context};
pub(crate) use scheduling::prepare_next;
pub(crate) use writer::{private_connection, Mode, Writer};

pub(crate) fn has_requests(db: &Connection, scope: &CoordinatorLease) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM agent_user_directives WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3
        AND json_extract(payload_json,'$.readonlyAssessmentPlan.schemaVersion')=3)",
        params![scope.root_run_id,scope.scan_id,scope.attempt_number],|r|r.get(0)).map_err(|e|e.to_string())
}

fn current(db: &Connection, scope: &CoordinatorLease) -> Result<(), String> {
    validate_coordinator_lease(db, scope)?;
    require_executable_coordinator(db, scope)?;
    let owner = budget::root::RootOwner::load_original(db, &scope.root_run_id)?;
    owner.require_original_coordinator(db, scope)?;
    owner.require_live(db)
}

fn event_exact(db: &Connection, job: &ActionJob, key: &str, value: &Value) -> Result<(), String> {
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2
      AND entity_type='user_directive' AND entity_id=?3 AND event_type='user_directive'
      AND json_extract(payload_json,'$.orderedActionId')=?4 AND json_extract(payload_json,?5)=?6",
            params![
                job.scope.scan_id,
                job.scope.attempt_number,
                job.directive_id,
                job.action.action_id,
                format!("$.{key}"),
                value.as_str().ok_or("ordered_event_value_invalid")?
            ],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count != 1 {
        return Err("ordered_event_not_persisted".into());
    }
    Ok(())
}
