// Every actual Single executor return settles its captured original owner.
// A historical or missing owner is never initialized by this exit path.
fn native_run_with_single_finally(context: &AgentRunContext) -> AgentTargetOutcome {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let original: Result<Option<RootOwner>, String> = (|| {
        let connection = db::open(&context.db_path)?;
        if !native_single_policy(&connection, context)? {
            return Ok(None);
        }
        agent_require_frozen_web_plan(&connection, context).map_err(str::to_string)?;
        let run = context.run.as_ref().ok_or("tool_run_not_found")?;
        let owner = RootOwner::load_original(&connection, &run.run_id)?;
        Ok(Some(owner))
    })();
    let original =
        match original {
            Ok(owner) => owner,
            Err(error) => return AgentTargetOutcome::Incomplete(AgentStop {
                code:
                    crate::agent_runtime::contract::terminal_code::REQUEST_RECONCILIATION_REQUIRED,
                reason: format!("原 Single 财务身份未获证明：{error}"),
            }),
        };
    let outcome = run_native_agent(context);
    let Some(owner) = original else {
        return outcome;
    };
    let settled =
        db::open(&context.db_path).and_then(|connection| owner.close_single_finance(&connection));
    match settled {
        Ok(fact) if fact.exhausted && outcome.completion().is_some() => {
            AgentTargetOutcome::Limited(AgentStop {
                code: crate::agent_runtime::contract::terminal_code::HARD_WALL_TIME_BUDGET,
                reason: format!(
                    "原 Root elapsed={}ms，hard 已耗尽；原退出：{}",
                    fact.elapsed_ms,
                    outcome.detail()
                ),
            })
        }
        Ok(_) => outcome,
        Err(error) => AgentTargetOutcome::Failed(AgentStop {
            code: AGENT_STOP_PERSISTENCE,
            reason: format!(
                "Single 最后费用保存失败：{error}；原退出 {}：{}",
                outcome.terminal_code(),
                outcome.detail()
            ),
        }),
    }
}

// An explicit pause of this exact original attempt is a yield, not a stale
// callback, unknown permission, or a continuation grant. Fees are already saved.
fn native_original_single_pause_requested(context: &AgentRunContext) -> bool {
    let Some(run) = &context.run else {
        return false;
    };
    if run.db_path != context.db_path {
        return false;
    }
    db::open(&context.db_path).is_ok_and(|db| {
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, &run.run_id).is_ok()
            && db.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN agent_runs r ON r.scan_id=s.id
                WHERE s.id=?1 AND s.attempt_count=?2 AND s.status='pausing' AND r.id=?3 AND r.attempt_number=?2
                AND r.target_url=?4 AND r.backend='native' AND r.orchestration_policy='single' AND r.status IN ('prepared','running'))",
                params![context.scan_id,context.attempt_number,run.run_id,context.target_url], |r| r.get::<_,bool>(0)).unwrap_or(false)
    })
}
