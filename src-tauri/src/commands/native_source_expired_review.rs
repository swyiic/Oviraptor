// At an expired deadline only a fully delivered source review can proceed to
// local closure. This audit must never prepare or dispatch another child.
fn source_expired_completed_reviews(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Option<Vec<JsonValue>>, String> {
    use crate::agent_runtime::multi_agent::{source_coverage_reviewer, source_phases, source_reviewer};
    let candidate_ready = if !source_reviewer::enabled(connection, lease)? {
        true
    } else {
        match source_reviewer::progress(connection, lease)? {
            source_reviewer::ReviewProgress::Delivered(_) => true,
            source_reviewer::ReviewProgress::NotStarted => source_reviewer::task_slice(connection, lease)?.is_none(),
            _ => false,
        }
    };
    if !candidate_ready { return Ok(None); }
    if source_coverage_reviewer::enabled(connection, lease)?
        && !matches!(source_coverage_reviewer::progress(connection, lease)?, source_coverage_reviewer::ReviewProgress::Delivered(_)) {
        return Ok(None);
    }
    let phases = source_phases::audit(connection, lease)?;
    source_assessment_completion(connection, lease, &phases.bases)?;
    Ok(Some(phases.assessments))
}
