use super::*;
use crate::agent_runtime::multi_agent::attempts::{self, AssignmentAttempt};

pub(super) fn root(db: &Connection, lease: &CoordinatorLease) -> Result<(), String> {
    validate_coordinator_lease(db, lease)?;
    require_active_attempt(db, &lease.scan_id, lease.attempt_number)?;
    super::super::lease::require_executable_coordinator(db, lease)?;
    let native:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1
        AND backend='native' AND role='coordinator' AND orchestration_policy IN ('single','multi')
        AND (root_run_id=id OR (root_run_id='' AND orchestration_policy='single' AND assignment_id='' AND parent_run_id IS NULL)))",
        [&lease.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !native {
        return Err("mailbox_root_binding_conflict".into());
    }
    Ok(())
}

pub(super) fn receiver(
    db: &Connection,
    lease: &CoordinatorLease,
    run: &str,
) -> Result<Option<AssignmentAttempt>, String> {
    root(db, lease)?;
    if run == lease.root_run_id {
        return Ok(None);
    }
    let worker = attempts::require_live_for_run(db, run)?;
    if worker.root_run_id != lease.root_run_id {
        return Err("mailbox_receiver_scope_conflict".into());
    }
    Ok(Some(worker))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn route(
    db: &Connection,
    lease: &CoordinatorLease,
    from: &str,
    to: &str,
    from_role: &str,
    to_role: &str,
    assignment: &str,
    revision: i64,
) -> Result<AssignmentAttempt, String> {
    root(db, lease)?;
    let child:Option<(String,String)>=db.query_row("SELECT child.id,a.role FROM agent_assignments a
        JOIN agent_runs child ON child.id=a.child_run_id JOIN agent_runs root ON root.id=a.coordinator_run_id
        WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.target_key=?3 AND a.evidence_revision=?4
          AND a.lease_epoch=?7 AND a.fencing_token=?8 AND child.assignment_id=a.id AND child.backend='native'
          AND root.role='coordinator' AND root.scan_id=?5 AND root.attempt_number=?6 AND root.target_url=?3
          AND child.root_run_id=root.id AND child.role=a.role AND child.scan_id=root.scan_id
          AND child.attempt_number=root.attempt_number AND child.target_url=root.target_url",
        params![assignment,lease.root_run_id,lease.target_key,revision,lease.scan_id,lease.attempt_number,
            lease.lease_epoch,lease.fencing_token],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
    let bound = child.as_ref().is_some_and(|(id, role)| {
        from == lease.root_run_id && to == id && from_role == "coordinator" && to_role == role
            || to == lease.root_run_id
                && from == id
                && from_role == role
                && to_role == "coordinator"
    });
    if !bound {
        return Err("mailbox_route_not_assignment_bound".into());
    }
    let worker = attempts::current(db, lease, assignment)?;
    if child.as_ref().map(|(id, _)| id) != Some(&worker.child_run_id) {
        return Err("mailbox_worker_binding_conflict".into());
    }
    Ok(worker)
}
