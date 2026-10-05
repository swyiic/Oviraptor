// A closed paid proposal produces a new Root-only advisory round. Its existing
// Rust assessment remains the authority: this hook cannot issue target work.
fn native_coordinator_gap_feedback(
    context: &AgentRunContext,
    session: &MultiAgentSession,
    candidate: &str,
    revision: i64,
    missing: &JsonValue,
) -> Result<(), String> {
    if context.target_url.starts_with("source:") {
        return Ok(());
    }
    let root = native_coordinator_root_context(context, &session.lease);
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, &session.lease)?;
    let frame = NativeCoordinatorFrame::investigator(
        &tx,
        &root,
        &session.lease,
        candidate,
        revision,
        missing,
    )?;
    frame.verify(&tx, &root, &session.lease)?;
    native_coordinator_tick_authority(&tx, &root, &session.lease)?;
    tx.commit().map_err(|e| e.to_string())?;
    let decision = native_coordinator_tick_for_frame(&root, &session.lease, &frame)?;
    if !decision.summary.as_json()["suggestions"]
        .as_array()
        .is_some_and(|items| items.iter().all(|item| item.as_str() == Some(frame.step())))
    {
        return Err("root_proposal_step_not_bounded".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let verify = || -> Result<(), String> {
        native_coordinator_tick_authority(&tx, &root, &session.lease)?;
        frame.verify(&tx, &root, &session.lease)?;
        decision.tick.require_executable(&tx)?;
        if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
            return Err("root_proposal_publication_changed".into());
        }
        Ok(())
    };
    verify()?; // No business/target/child write follows the advisory publication.
    tx.commit().map_err(|e| e.to_string())
}
