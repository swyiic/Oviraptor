// A received original identity envelope is assessed before target grant.
fn native_coordinator_identity_feedback(
    context:&AgentRunContext,actor:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    mapper:&crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    child:&crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
)->Result<NativeCoordinatorTickReceipt,String> {
    let root=native_coordinator_root_context(context,actor);
    let db=db::open(&context.db_path)?;
    let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    native_coordinator_tick_authority(&tx,&root,actor)?;
    let origin=NativeCoordinatorFrame::mapper(&tx,actor,mapper)?;
    let mut frame=NativeCoordinatorFrame::identity_session(&tx,actor,child)?;
    frame.rows.extend(origin.rows);
    frame.verify(&tx,&root,actor)?;
    native_coordinator_tick_authority(&tx,&root,actor)?;
    tx.commit().map_err(|e|e.to_string())?;
    let decision=native_coordinator_tick_for_frame(&root,actor,&frame)?;
    if !decision.summary.as_json()["suggestions"].as_array()
        .is_some_and(|items|items.iter().all(|item|item.as_str()==Some(frame.step()))) {
        return Err("root_identity_step_not_bounded".into());
    }
    let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    native_coordinator_tick_authority(&tx,&root,actor)?;frame.verify(&tx,&root,actor)?;
    decision.tick.require_executable(&tx)?;
    if decision.tick.published(&tx,&decision.saved)?!=Some(decision.event_sequence) {return Err("root_identity_publication_changed".into());}
    tx.commit().map_err(|e|e.to_string())?;
    Ok(decision)
}
