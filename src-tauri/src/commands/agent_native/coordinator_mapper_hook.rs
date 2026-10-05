fn native_coordinator_mapper_decision(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    mapper: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<(NativeCoordinatorFrame, NativeCoordinatorTickReceipt), String> {
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    let frame = NativeCoordinatorFrame::mapper(&tx, actor, mapper)?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    tx.commit().map_err(|e| e.to_string())?;
    let receipt = native_coordinator_tick_for_frame(context, actor, &frame)?;
    Ok((frame, receipt))
}
