use super::*;

// A successor Coordinator may stop an abandoned original worker. It does not
// adopt that worker's epoch, receipt or reservation under its own new fence.
pub(crate) fn pause_original(
    db: &Connection,
    scope: &CoordinatorLease,
    assignment: &str,
    run: &str,
    failure: &str,
) -> Result<(), String> {
    super::super::lease::validate_coordinator_lease(db, scope)?;
    let (epoch,fence,role):(i64,String,String)=db.query_row("SELECT lease_epoch,fencing_token,role
        FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND target_key=?4",
        params![assignment,run,scope.root_run_id,scope.target_key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    let role_parsed = crate::agent_runtime::contract::AgentRole::parse(&role);
    if role_parsed.as_str() != role {
        return Err("assignment_attempt_cleanup_role_invalid".into());
    }
    let mut original = scope.clone();
    original.lease_epoch = epoch;
    original.fencing_token = fence;
    pause(
        db,
        &original,
        &ScheduledChild {
            assignment_id: assignment.into(),
            run_id: run.into(),
            role: role_parsed,
        },
        failure,
    )
}
