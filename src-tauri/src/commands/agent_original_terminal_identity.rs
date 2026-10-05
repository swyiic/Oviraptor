// Captured by the admitted invocation before model execution. Never deserialize
// or infer this identity from a current attempt, Native output, or model result.
#[derive(Clone, Debug)]
struct OriginalAgentTerminalIdentity {
    db_path: PathBuf,
    scan_id: String,
    attempt_number: i64,
    target_url: String,
    root_run_id: Option<String>,
}
impl OriginalAgentTerminalIdentity {
    fn pending(path: &Path, scan: &str, attempt: i64, target: &str) -> Self {
        Self {
            db_path: path.into(),
            scan_id: scan.into(),
            attempt_number: attempt,
            target_url: target.into(),
            root_run_id: None,
        }
    }
    #[cfg(test)]
    fn capture(context: &AgentRunContext) -> Self {
        let mut original = Self::pending(
            &context.db_path,
            &context.scan_id,
            context.attempt_number,
            &context.target_url,
        );
        original.bind_root(context).unwrap();
        original
    }
    fn bind_root(&mut self, context: &AgentRunContext) -> Result<(), String> {
        if self.db_path != context.db_path
            || self.scan_id != context.scan_id
            || self.attempt_number != context.attempt_number
            || self.target_url != context.target_url
            || context
                .run
                .as_ref()
                .is_some_and(|run| run.db_path != self.db_path)
            || self.root_run_id.is_some()
        {
            return Err("runtime_terminal_captured_scope_conflict".into());
        }
        self.root_run_id = context.run.as_ref().map(|run| run.run_id.clone());
        Ok(())
    }
    fn require_scope(&self, path: &Path, scan: &str, route: &FrontendRoute) -> Result<(), String> {
        if self.db_path != path
            || self.scan_id != scan
            || self.target_url != route.url
            || self.attempt_number < 1
        {
            return Err("runtime_terminal_captured_scope_conflict".into());
        }
        let db = db::open(path)?;
        self.require_on(&db, scan, route)
    }
    fn require_on(
        &self,
        db: &rusqlite::Connection,
        scan: &str,
        route: &FrontendRoute,
    ) -> Result<(), String> {
        if self.scan_id != scan || self.target_url != route.url || self.attempt_number < 1 {
            return Err("runtime_terminal_captured_scope_conflict".into());
        }
        let root = self
            .root_run_id
            .as_deref()
            .ok_or("runtime_terminal_original_root_missing_or_ambiguous")?;
        let (current,exact):(bool,bool)=db.query_row("SELECT
            EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?2 AND attempt_count=?3)
            AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?2),
            EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4
              AND role='coordinator' AND backend='native' AND parent_run_id IS NULL AND assignment_id='')
            AND (SELECT count(*) FROM agent_runs WHERE scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND role='coordinator')=1",
            params![root,scan,self.attempt_number,route.url],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        if !current {
            return Err("runtime_terminal_original_attempt_replaced".into());
        }
        if !exact {
            return Err("runtime_terminal_captured_root_conflict".into());
        }
        Ok(())
    }
}
fn record_owned_agent_target_outcome(
    path: &Path,
    scan: &str,
    route: &FrontendRoute,
    owned: &OwnedAgentTargetOutcome,
    tally: &mut AgentPipelineTally,
) -> bool {
    match record_agent_target_outcome_original(
        path,
        scan,
        route,
        &owned.original_terminal,
        owned.outcome.clone(),
        tally,
    ) {
        Ok(continued) => continued,
        Err(error) => {
            append_runner_log(
                &owned.log_path,
                &format!(
                    "原目标结果投影未提交，已停止流水线；原执行记录保留：{}",
                    crate::agent_runtime::secrets::redact_text_with(&error, None)
                ),
            );
            false
        }
    }
}
fn runtime_report_for_attempt(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
    attempt_number: i64,
) -> crate::agent_runtime::runtime_adapter::BackendReport {
    use crate::agent_runtime::runtime_adapter::BackendReport;
    use crate::agent_runtime::store::stable_hash;
    let plan_json = frozen_plan_of(db_path, scan_id, attempt_number, &route.url);
    let backend = persisted_attempt_backend(db_path, scan_id, attempt_number, &route.url)
        .ok()
        .flatten()
        .filter(|backend| {
            AgentExecutionPlan::from_json(&plan_json).is_some_and(|plan| plan.backend == *backend)
        })
        .unwrap_or(AgentBackendKind::LegacyRemoved);
    let mut report = BackendReport::new(scan_id, attempt_number, route.url.clone(), backend);
    // The attempt row stores the hash of the immutable execution contract,
    // excluding the attempt identity. Reopening a run must not replace that
    // hash with one of the full JSON projection (which includes attemptNumber),
    // or the next record_attempt_plan call falsely reports a frozen conflict.
    report.plan_hash = AgentExecutionPlan::from_json(&plan_json)
        .map(|plan| plan.hash())
        .unwrap_or_else(|| stable_hash(&plan_json.to_string()));
    report.plan_json = plan_json.clone();
    let budgets = plan_json.get("budgets").cloned().unwrap_or(JsonValue::Null);
    let budget = |key: &str| -> i64 { budgets.get(key).and_then(JsonValue::as_i64).unwrap_or(0) };
    report.soft_token_budget = budget("softUncachedTokens");
    report.hard_token_budget = budget("hardTotalTokens");
    report.soft_request_budget = budget("softModelRequests");
    report.hard_request_budget = budget("hardModelRequests");
    let native_state = NativeAgentState::read(db_path, scan_id, &route.url);
    report.evidence_hash = native_state
        .as_ref()
        .map(|value| value.evidence_hash.clone())
        .unwrap_or_default();
    report.required_families = AGENT_COVERAGE_FAMILIES
        .iter()
        .map(|value| value.to_string())
        .collect();
    report
}

// Historical production-entry test callers choose a scope explicitly here only.
// The live pipeline cannot call these current-attempt lookup wrappers.
#[cfg(test)]
fn current_terminal_identity_for_test(
    path: &Path,
    scan: &str,
    route: &FrontendRoute,
) -> OriginalAgentTerminalIdentity {
    let report = runtime_report(path, scan, route);
    let root=db::open(path).ok().and_then(|db| {
        let mut q=db.prepare("SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role='coordinator'").ok()?;
        let rows=q.query_map(params![scan,report.attempt_number,route.url],|r|r.get::<_,String>(0)).ok()?.collect::<rusqlite::Result<Vec<_>>>().ok()?;
        match rows.as_slice() {[id]=>Some(id.clone()),_=>None}
    });
    let mut original =
        OriginalAgentTerminalIdentity::pending(path, scan, report.attempt_number, &route.url);
    original.root_run_id = root;
    original
}
#[cfg(test)]
fn runtime_open_run(path: &Path, scan: &str, route: &FrontendRoute) -> Option<AgentRunLedger> {
    runtime_open_run_for_attempt(
        path,
        scan,
        route,
        runtime_report(path, scan, route).attempt_number,
    )
}
#[cfg(test)]
fn record_runtime_terminal_facts_checked(
    path: &Path,
    scan: &str,
    route: &FrontendRoute,
    outcome: &AgentTargetOutcome,
) -> Result<Option<crate::agent_runtime::reducer::Reduction>, String> {
    record_runtime_terminal_facts_original(
        path,
        scan,
        route,
        &current_terminal_identity_for_test(path, scan, route),
        outcome,
    )
}
#[cfg(test)]
fn record_agent_target_outcome(
    path: &Path,
    scan: &str,
    route: &FrontendRoute,
    outcome: AgentTargetOutcome,
    tally: &mut AgentPipelineTally,
) -> bool {
    record_agent_target_outcome_original(
        path,
        scan,
        route,
        &current_terminal_identity_for_test(path, scan, route),
        outcome,
        tally,
    )
    .unwrap_or(false)
}

// Test barrier at the actual production post-publication boundary; no behavior shim.
#[cfg(test)]
thread_local! {
    static ORIGINAL_TERMINAL_LEGACY_PAUSE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}
#[cfg(test)]
fn original_terminal_legacy_test_pause() {
    let pause = ORIGINAL_TERMINAL_LEGACY_PAUSE.with(|slot| slot.borrow_mut().take());
    if let Some(pause) = pause {
        pause();
    }
}
