//! Local original SDK lifetime only. No remote completion or financial grant.
use super::*;

pub(super) fn require_idle(
    db: &Connection, job: &ActionJob,
) -> Result<Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>, String> {
    let worker = attempts::current(db, &job.scope, &job.child.assignment_id)?;
    if worker.child_run_id != job.child.run_id {
        return Err("ordered_transport_original_worker_changed".into());
    }
    let path = db.path().filter(|path| !path.is_empty())
        .ok_or("ordered_transport_database_missing")?;
    let owner = crate::agent_runtime::execution_owner::probe_native_invocation(
        std::path::Path::new(path), &job.scope.scan_id, job.scope.attempt_number,
        "ordered-assessment-sdk", &job.child.run_id,
    ).map_err(|error| format!("ordered_transport_not_idle:{error}"))?;
    let dispatched: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 OR child_run_id=?2)",
        params![job.child.assignment_id, job.child.run_id], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if dispatched && owner.is_none() {
        return Err("ordered_transport_original_exit_proof_missing".into());
    }
    Ok(owner)
}
