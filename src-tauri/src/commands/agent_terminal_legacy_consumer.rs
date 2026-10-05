fn record_agent_target_outcome_original(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
    original: &OriginalAgentTerminalIdentity,
    outcome: AgentTargetOutcome,
    tally: &mut AgentPipelineTally,
) -> Result<bool, String> {
    original.require_scope(db_path, scan_id, route)?;
    {
        let db = db::open(db_path)?;
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        if let Some(reduction) = original_multi_terminal_on(&tx, original, scan_id, route)? {
            require_original_multi_outcome(&reduction, &outcome)?;
        }
    }
    let publication =
        record_runtime_terminal_facts_original(db_path, scan_id, route, original, &outcome);
    let (outcome, published) = match publication {
        Ok(reduction) => (outcome, reduction),
        Err(error) => (
            AgentTargetOutcome::persistence_failure(format!(
                "原结果发布失败：{error}；原退出 {}：{}",
                outcome.terminal_code(),
                outcome.detail()
            )),
            None,
        ),
    };
    let outcome = agent_outcome_from_reduction(outcome, published.as_ref());
    #[cfg(test)]
    original_terminal_legacy_test_pause();
    let reason = outcome.detail();
    let mut next_tally = tally.clone();
    if published.is_none() && matches!(outcome, AgentTargetOutcome::Cancelled) {
        // A yielded Single has a saved financial boundary and no resume grant.
        // Do not publish a cancelled terminal checkpoint for this pause.
        write_terminal_legacy_projection(db_path, scan_id, original, route, "paused", None, None)?;
        return Ok(false);
    }
    // §4.2: the reducer decides the terminal state and `agent_runs` holds it; the
    // legacy columns are a projection of that decision. Only when the run stayed
    // open (a pause, a resumable stop, or no run row at all) does the outcome
    // itself supply the state, so the two can never disagree in the other direction.
    let outcome_status = published
        .as_ref()
        .map(|r| r.state.to_sentinel_status())
        .unwrap_or_else(|| outcome.terminal_status());
    // One traceable terminal record per target, whatever produced it.
    // Build all derived writes first. Count only after the checked transaction
    // commits; the already published original invoice is never rolled back.
    let terminal = serde_json::json!({
        "code": published.as_ref().map(|r| r.code.as_str()).unwrap_or_else(|| outcome.terminal_code()),
        "status": outcome_status,
        "detail": reason,
        "stop": outcome.stop().map(|stop| serde_json::json!({"code": stop.code, "reason": stop.reason})),
        "completion": outcome.completion().map(|completion| serde_json::json!({
            "coveredFamilies": completion.covered_families,
            "uncoveredFamilies": completion.uncovered_families,
            "confirmedFindings": completion.confirmed_findings,
            "ledgerReported": completion.ledger_reported,
        })),
    });
    let mut projection_route = route.clone();
    let mut projection_status = outcome_status;
    let mut fuse_reason = None;
    let mut continue_pipeline = true;
    match outcome {
        AgentTargetOutcome::Completed(completion) => {
            next_tally.completed += 1;
            let mut completed_route = route.clone();
            if completion.ledger_reported {
                completed_route
                    .reasons
                    .push(format!("覆盖账本：{}", completion.summary));
            }
            projection_route = completed_route;
        }
        AgentTargetOutcome::BoundedCompleted(_) => {
            next_tally.completed_with_gaps += 1;
            let mut completed_route = route.clone();
            completed_route
                .reasons
                .push(format!("有界调查已完成：{reason}"));
            projection_route = completed_route;
        }
        AgentTargetOutcome::Incomplete(stop) => {
            next_tally.partial += 1;
            let mut incomplete_route = route.clone();
            next_tally.push_detail(format!("{}：{}", route.url, stop.reason));
            incomplete_route.reasons.push(format!(
                "自动验证尚未取得目标请求/响应；前端证据已保留，可重试未完成阶段：{}",
                stop.reason
            ));
            projection_route = incomplete_route;
        }
        AgentTargetOutcome::Limited(stop) => {
            let mut stopped_route = route.clone();
            if stop.requires_fuse() {
                next_tally.limited += 1;
                stopped_route
                    .reasons
                    .push(format!("确认拦截并熔断：{}", stop.reason));
                projection_route = stopped_route;
                fuse_reason = Some(stop.reason.clone());
            } else {
                next_tally.partial += 1;
                next_tally.push_detail(format!("{}：{}", route.url, stop.reason));
                stopped_route.reasons.push(format!(
                    "本地模型资源策略需要调整；前端证据已保留，可重试未完成阶段：{}",
                    stop.reason
                ));
                // A limit that is not a protection stop (local model capacity) is
                // resumable work, so it takes the reducer's paused spelling.
                projection_route = stopped_route;
                projection_status =
                    crate::agent_runtime::contract::TerminalState::Incomplete.to_sentinel_status();
            }
        }
        AgentTargetOutcome::Failed(stop)
            if stop.code != AGENT_STOP_PERSISTENCE
                && (model_configuration_failure(&stop.reason)
                    || model_retryable_provider_failure(&stop.reason)) =>
        {
            next_tally.failed += 1;
            let mut failed_route = route.clone();
            next_tally.push_detail(format!("{}：{}", route.url, stop.reason));
            failed_route.reasons.push(format!(
                "模型服务不可用或配置错误，自动流程无法继续；已保留完整前端侦察结果：{}",
                stop.reason
            ));
            projection_route = failed_route;
        }
        AgentTargetOutcome::Failed(stop) => {
            next_tally.failed += 1;
            let mut failed_route = route.clone();
            let detail = format!("{}：{}", route.url, stop.reason);
            next_tally.push_detail(detail.clone());
            failed_route.reasons.push(detail);
            projection_route = failed_route;
        }
        // §11: a continuation that cannot inherit its parent's state is its own
        // terminal state. It is never reported as a model or tool failure, and the
        // target is excluded from automatic resume so only 重新执行 can pick it up.
        AgentTargetOutcome::ResumeIncompatible(stop) => {
            next_tally.manual_review += 1;
            let mut stopped_route = route.clone();
            next_tally.push_detail(format!("{}：{}", route.url, stop.reason));
            stopped_route
                .reasons
                .push(format!("续跑状态不兼容，需要重新执行：{}", stop.reason));
            projection_route = stopped_route;
        }
        AgentTargetOutcome::Cancelled => continue_pipeline = false,
    }
    write_terminal_legacy_projection(
        db_path,
        scan_id,
        original,
        &projection_route,
        projection_status,
        Some(&terminal),
        fuse_reason.as_deref(),
    )?;
    let root = original
        .root_run_id
        .as_ref()
        .expect("validated original Root");
    if next_tally.projected_roots.insert(root.clone()) && !tally.projected_roots.contains(root) {
        *tally = next_tally;
    }
    Ok(continue_pipeline)
}
