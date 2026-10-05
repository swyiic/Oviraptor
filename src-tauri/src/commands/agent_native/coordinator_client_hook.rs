// One original closed Client envelope becomes a Root advisory fact; no grant.
fn native_coordinator_client_feedback(
    context: &AgentRunContext, session: &MultiAgentSession,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<(), String> {
    let root = native_coordinator_root_context(context, &session.lease);
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e|e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, &session.lease)?;
    // Bind the actual prepared pipeline's independent paid Mapper as well.
    let origin = NativeCoordinatorFrame::mapper(&tx, &session.lease, &session.mapper)?;
    let mut frame = NativeCoordinatorFrame::client_side(&tx, &root, &session.lease, child)?;
    frame.rows.extend(origin.rows);
    frame.verify(&tx, &root, &session.lease)?;
    native_coordinator_tick_authority(&tx, &root, &session.lease)?;
    tx.commit().map_err(|e|e.to_string())?;
    let decision = native_coordinator_tick_for_frame(&root, &session.lease, &frame)?;
    if !decision.summary.as_json()["suggestions"].as_array()
        .is_some_and(|items|items.iter().all(|item|item.as_str()==Some(frame.step()))) {
        return Err("root_client_step_not_bounded".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e|e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, &session.lease)?;
    frame.verify(&tx, &root, &session.lease)?;
    decision.tick.require_executable(&tx)?;
    if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
        return Err("root_client_publication_changed".into());
    }
    // No business/child/capability/tool/network effect follows publication.
    tx.commit().map_err(|e|e.to_string())
}
