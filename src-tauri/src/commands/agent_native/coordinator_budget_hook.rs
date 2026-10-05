// Production entry: current Native Web4 only, before every executor SDK/target request.
fn native_coordinator_budget_before_executor(context: &AgentRunContext) -> Result<(), String> {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    let Some(run) = context.run.as_ref() else {
        return Ok(());
    };
    let db = db::open(&context.db_path)?;
    let child_run = crate::agent_runtime::store::load_run(&db, &run.run_id)?
        .ok_or("root_budget_executor_missing")?;
    if child_run.role != AgentRole::WebExecutor
        || child_run.orchestration_policy != crate::agent_runtime::contract::MultiAgentPolicy::Multi
    {
        return Ok(());
    }
    let mode = crate::agent_runtime::web_mode::root::read(&db, &child_run.root_run_id)?
        .ok_or("web_mode_root_missing")?;
    if mode
        .bootstrap_dispatch()
        .unwrap_or(JsonValue::Null)
        .get("rootSupervisionTokenFloor")
        .is_none()
    {
        return Ok(());
    }
    let actor = context
        .supervision
        .as_ref()
        .ok_or("worker_supervisor_stopped")?
        .original_actor_for_run(
            &db,
            &context.db_path,
            &context.scan_id,
            context.attempt_number,
            &context.target_url,
            &run.run_id,
        )?;
    let child = scheduler::ScheduledChild {
        assignment_id: child_run.assignment_id,
        run_id: run.run_id.clone(),
        role: AgentRole::WebExecutor,
    };
    crate::agent_runtime::multi_agent::budget::admission::require_determinate(
        &db,
        &actor.root_run_id,
    )?;
    let raw: String = db
        .query_row(
            "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let task: JsonValue =
        serde_json::from_str(&raw).map_err(|_| "root_budget_original_task_invalid")?;
    let id = task["mapperAssignmentId"]
        .as_str()
        .ok_or("root_budget_original_mapper_missing")?;
    let mapper_run: String = db.query_row("SELECT child_run_id FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND role='spa_api_mapper'",params![id,actor.root_run_id],|r|r.get(0)).map_err(|_|"root_budget_original_mapper_missing")?;
    let mapper = scheduler::ScheduledChild {
        assignment_id: id.into(),
        run_id: mapper_run,
        role: AgentRole::SpaApiMapper,
    };
    let root = native_coordinator_root_context(context, &actor);
    // Require the existing publication/request before any cached tick lookup.
    // A caller cannot point at another closed Mapper and manufacture a new paid origin.
    let seq = task["rootDecision"]["eventSequence"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("root_budget_original_dispatch_missing")?;
    let (request,publication): (String,String) = db.query_row("SELECT q.fact_json,p.fact_json FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q ON q.root_run_id=p.root_run_id AND q.round=p.round AND q.phase='request' WHERE p.root_run_id=?1 AND p.phase='publication' AND json_extract(p.fact_json,'$.eventSequence')=?2",params![actor.root_run_id,seq],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"root_budget_original_dispatch_missing")?;
    let request: JsonValue =
        serde_json::from_str(&request).map_err(|_| "root_budget_original_dispatch_invalid")?;
    let publication: JsonValue =
        serde_json::from_str(&publication).map_err(|_| "root_budget_original_dispatch_invalid")?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, &actor)?;
    let original = NativeCoordinatorFrame::mapper(&tx, &actor, &mapper)?;
    if request["request"]["basis"]["phase"] != "mapper-output"
        || request["request"]["basis"]["changedFact"] != original.fact()
        || task["rootDecision"]["frameHash"]
            != crate::agent_runtime::store::stable_hash(&original.fact().to_string())
        || task["rootDecision"]["summaryHash"] != publication["decisionHash"]
    {
        return Err("root_budget_original_dispatch_conflict".into());
    }
    tx.commit().map_err(|e| e.to_string())?;
    // Original paid Mapper decision is looked up; no new decision may repair it.
    let paid = native_coordinator_tick_for_frame(&root, &actor, &original)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, &actor)?;
    let frame =
        NativeCoordinatorFrame::budget_allocation(&tx, &root, &actor, &child, original, &paid)?;
    frame.verify(&tx, &root, &actor)?;
    tx.commit().map_err(|e| e.to_string())?;
    let decision = native_coordinator_tick_for_frame(&root, &actor, &frame)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, &actor)?;
    frame.verify(&tx, &root, &actor)?;
    decision.tick.require_executable(&tx)?;
    crate::agent_runtime::multi_agent::budget::admission::require_determinate(
        &tx,
        &actor.root_run_id,
    )?;
    if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
        return Err("root_budget_publication_changed".into());
    }
    let summary = decision.summary.as_json();
    let suggestions = summary["suggestions"]
        .as_array()
        .ok_or("root_decision_schema_invalid")?;
    if !suggestions
        .iter()
        .all(|item| item.as_str() == Some(frame.step()))
    {
        return Err("root_budget_step_not_bounded".into());
    }
    if suggestions.is_empty() {
        return Err("root_budget_allocation_deferred".into());
    }
    // Publication permits only this original allocation, no reserve/lease/scope mutation.
    tx.commit().map_err(|e| e.to_string())
}
