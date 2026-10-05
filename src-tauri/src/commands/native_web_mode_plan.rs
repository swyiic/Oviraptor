include!("native_web_root_budget.rs");

// Ordinary Web entry only. Source's old plan publisher never calls this helper.
fn persist_frozen_web_execution_plan(
    db_path: &Path,
    scan: &str,
    attempt: i64,
    target: &str,
    plan: &AgentExecutionPlan,
) -> Result<(), String> {
    use crate::agent_runtime::{store, web_mode::root};
    let db = db::open(db_path)?;
    let proof = private_web_mode_on(&db, scan, attempt)?;
    if !proof.fact().targets.iter().any(|v| v == target) {
        return Err("web_mode_target_not_frozen".into());
    }
    let existing: Vec<String> = {
        let mut statement=db.prepare("SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role='coordinator' ORDER BY id")
            .map_err(|e|e.to_string())?;
        let ids = statement
            .query_map(params![scan, attempt, target], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())?;
        ids
    };
    if existing.is_empty() {
        let declaration =
            root::NewRootModeDeclaration::from_verified(&proof, target, &plan.hash())?.with_local_deliberation()?;
        let budget = fresh_web_root_budget_declaration(&proof, target, plan)?;
        return store::record_attempt_plan_with_declarations(
            &db,
            scan,
            attempt,
            target,
            plan.backend,
            &plan.hash(),
            &plan.as_json(),
            Some(&budget),
            Some(&declaration),
        );
    }
    if existing.len() != 1 {
        return Err("web_mode_existing_root_ambiguous".into());
    }
    let declaration =
        root::read(&db, &existing[0])?.ok_or("web_mode_existing_root_missing_declaration")?;
    if declaration.fact() != proof.fact() {
        return Err("web_mode_existing_root_original_fact_changed".into());
    }
    // Reentry compares its original sidecar first; it never supplies Some(mode)
    // or silently fills missing mode/budget authority on an existing Root.
    persist_agent_execution_plan(db_path, scan, attempt, target, plan)
}

fn native_frozen_web_root_mode(
    context: &AgentRunContext,
) -> Result<crate::agent_runtime::web_mode::WebMode, String> {
    let db = db::open(&context.db_path)?;
    native_frozen_web_root_mode_on(&db, context)
}

fn native_frozen_web_root_mode_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<crate::agent_runtime::web_mode::WebMode, String> {
    let run = context.run.as_ref().ok_or("web_mode_root_missing")?;
    let declaration = crate::agent_runtime::web_mode::root::read(db, &run.run_id)?
        .ok_or("web_mode_root_declaration_missing")?;
    let fact = private_web_mode_on(db, &context.scan_id, context.attempt_number)?;
    let text:String=db.query_row("SELECT plan_json FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3
        AND target_url=?4 AND role='coordinator' AND backend='native' AND parent_run_id IS NULL AND assignment_id=''",
        params![run.run_id,context.scan_id,context.attempt_number,context.target_url],|r|r.get(0)).map_err(|_|"web_mode_root_scope_changed")?;
    let plan: JsonValue = serde_json::from_str(&text).map_err(|_| "web_mode_root_plan_invalid")?;
    if declaration.fact() != fact.fact() || plan != context.execution_plan.as_json() {
        return Err("web_mode_original_root_fact_changed".into());
    }
    Ok(declaration.mode())
}

// This narrow test/read gate is not mode selection. A Multi worker keeps the
// existing inbox flow; a Single run never acquires a Coordinator for chatting.
fn native_single_directives_disabled(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<bool, String> {
    let Some(run) = context.run.as_ref() else {
        return Ok(false);
    };
    let (policy, role): (String, String) = db
        .query_row(
            "SELECT orchestration_policy,role FROM agent_runs WHERE id=?1
        AND scan_id=?2 AND attempt_number=?3 AND target_url=?4",
            params![
                run.run_id,
                context.scan_id,
                context.attempt_number,
                context.target_url
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "web_mode_directive_scope_missing")?;
    if policy == "single" && role == "coordinator" {
        // Do not mint a collaboration fence for legacy Single diagnostic runs.
        return Ok(true);
    }
    if policy != "multi" {
        return Err("web_mode_directive_policy_invalid".into());
    }
    Ok(false)
}

// Pure original owner load; no current C or zero-use history issues authority.
fn native_web_original_finance_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let root = &context
        .run
        .as_ref()
        .ok_or("web_finance_root_missing")?
        .run_id;
    let present: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !present {
        return Err("web_root_original_finance_missing".into());
    }
    RootOwner::load_original(db, root)?.require_executable(db)
}
