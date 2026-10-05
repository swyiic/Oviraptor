//! Backend-neutral runtime report adapter.
//!
//! This module translates a native executor report into append-only runtime
//! facts. It owns neither legacy scan state nor the terminal verdict; the
//! reducer remains the single writer for terminal state.
use super::contract::{
    AgentBackendKind, AgentEventKind, AgentRole, AgentRunStatus, TerminalSignals,
};
#[cfg(test)]
use super::reducer;
use super::store;
use rusqlite::Connection;

/// Everything a backend is allowed to report. Facts and artifact references only
/// — no verdict about the final state.
#[derive(Clone, Debug)]
pub struct BackendReport {
    pub scan_id: String,
    pub attempt_number: i64,
    pub target_url: String,
    pub backend: AgentBackendKind,
    pub plan_hash: String,
    pub plan_json: serde_json::Value,
    pub evidence_hash: String,
    pub soft_token_budget: i64,
    pub hard_token_budget: i64,
    pub soft_request_budget: i64,
    pub hard_request_budget: i64,
    pub pending_contracts: i64,
    pub covered_families: Vec<String>,
    pub required_families: Vec<String>,
    pub evidence_records: i64,
    pub confirmed_findings: i64,
    /// Requests the run actually sent to the target, kept separate from model
    /// calls so the run row shows both spends.
    pub target_requests: i64,
    pub request_accounting: Option<serde_json::Value>,
    pub completed_contracts: Vec<String>,
    pub ledger_closed: bool,
    pub cancelled: bool,
    /// The attempt is not over: the checkpoint is still resumable, so the run
    /// must stay open and be reused by the resume instead of forking a second
    /// row whose spend is counted separately (§11).
    pub resumable: bool,
    pub protection_signal: Option<String>,
    pub hard_limit_reason: Option<String>,
    pub soft_budget_stall_reason: Option<String>,
    pub configuration_error: Option<String>,
    /// Set when the attempt refused to continue because its parent checkpoint was
    /// not inheritable (§3.6).
    pub resume_incompatible: Option<String>,
    /// Set when a local write failed and the attempt stopped (§5.2).
    pub persistence_failure: Option<String>,
    pub request_reconciliation_required: Option<String>,
    pub execution_authorization_denied: Option<String>,
    pub unsupported_capability: Option<String>,
    pub usage: store::UsageDelta,
    pub detail: String,
}

impl BackendReport {
    pub fn new(
        scan_id: impl Into<String>,
        attempt_number: i64,
        target_url: impl Into<String>,
        backend: AgentBackendKind,
    ) -> Self {
        Self {
            scan_id: scan_id.into(),
            attempt_number,
            target_url: target_url.into(),
            backend,
            plan_hash: String::new(),
            plan_json: serde_json::Value::Null,
            evidence_hash: String::new(),
            soft_token_budget: 0,
            hard_token_budget: 0,
            soft_request_budget: 0,
            hard_request_budget: 0,
            pending_contracts: 0,
            covered_families: Vec::new(),
            required_families: super::contract::COVERAGE_FAMILIES
                .iter()
                .map(|value| value.to_string())
                .collect(),
            evidence_records: 0,
            confirmed_findings: 0,
            target_requests: 0,
            request_accounting: None,
            completed_contracts: Vec::new(),
            ledger_closed: false,
            cancelled: false,
            resumable: false,
            protection_signal: None,
            hard_limit_reason: None,
            soft_budget_stall_reason: None,
            configuration_error: None,
            resume_incompatible: None,
            persistence_failure: None,
            request_reconciliation_required: None,
            execution_authorization_denied: None,
            unsupported_capability: None,
            usage: store::UsageDelta::default(),
            detail: String::new(),
        }
    }

    pub fn signals(&self) -> TerminalSignals {
        TerminalSignals {
            cancelled: self.cancelled,
            configuration_error: self.configuration_error.clone(),
            protection_signal: self.protection_signal.clone(),
            hard_limit_reason: self.hard_limit_reason.clone(),
            soft_budget_stall_reason: self.soft_budget_stall_reason.clone(),
            unsupported_capability: self.unsupported_capability.clone(),
            resume_incompatible: self.resume_incompatible.clone(),
            persistence_failure: self.persistence_failure.clone(),
            request_reconciliation_required: self.request_reconciliation_required.clone(),
            execution_authorization_denied: self.execution_authorization_denied.clone(),
            ledger_closed: self.ledger_closed,
            pending_contracts: self.pending_contracts,
            evidence_records: self.evidence_records,
            confirmed_findings: self.confirmed_findings,
            covered_families: self.covered_families.clone(),
            required_families: self.required_families.clone(),
            detail: self.detail.clone(),
        }
    }
}

/// Register (or reuse) the run row for this attempt and target.
/// A sealed legacy attempt may never become an active Agent run again, so the
/// runtime adapter refuses before it looks for a row to reuse or creates one.
pub fn open_run_refusal(
    connection: &Connection,
    report: &BackendReport,
) -> Result<Option<String>, String> {
    if report.backend != AgentBackendKind::Native {
        return Ok(Some(
            "backend_not_executable: only native reports may open an active run".into(),
        ));
    }
    let sealed: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 \
             AND target_url=?3 AND (status='legacy_backend_removed' OR backend<>'native')",
            rusqlite::params![report.scan_id, report.attempt_number, report.target_url],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if sealed > 0 {
        return Ok(Some(
            "该次执行的后端已退役或不可识别，不能再注册为活动 run；请发起全新的 Native 执行"
                .to_string(),
        ));
    }
    Ok(None)
}

pub fn open_run(connection: &Connection, report: &BackendReport) -> Result<String, String> {
    if let Some(refusal) = open_run_refusal(connection, report)? {
        return Err(refusal);
    }
    if let Some(existing) = store::find_run(
        connection,
        &report.scan_id,
        report.attempt_number,
        &report.target_url,
        AgentRole::Coordinator,
    )? {
        if existing.is_legacy_sealed() {
            // The lineage lookup above is by scan/attempt/target; a sealed row cannot be
            // revived through any path, so this is checked again at the mutation point.
            return Err(
                "legacy_backend_removed 的历史 run 不能重新成为活动 run；请发起全新的 Native 执行"
                    .to_string(),
            );
        }
        if !existing.is_terminal() {
            // The row may predate this call (created when the attempt's plan was
            // frozen), so the plan's ceilings travel with it here too (§11).
            store::apply_run_plan(
                connection,
                &existing.id,
                &report.plan_hash,
                report.soft_token_budget,
                report.hard_token_budget,
                report.soft_request_budget,
                report.hard_request_budget,
            )?;
            store::set_run_status(connection, &existing.id, AgentRunStatus::Running)?;
            return Ok(existing.id);
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let row = store::AgentRunRow::new(
        id.clone(),
        report.scan_id.clone(),
        report.attempt_number,
        report.target_url.clone(),
        report.backend,
        AgentRole::Coordinator,
        report.plan_hash.clone(),
        report.evidence_hash.clone(),
    )
    .with_budget(
        report.soft_token_budget,
        report.hard_token_budget,
        report.soft_request_budget,
        report.hard_request_budget,
    );
    store::create_run(connection, &row)?;
    store::append_event(
        connection,
        &id,
        AgentEventKind::RunCreated,
        &serde_json::json!({"backend": report.backend.as_str()}),
        &[],
    )?;
    store::append_event(
        connection,
        &id,
        AgentEventKind::PlanFrozen,
        &serde_json::json!({
            "planHash": report.plan_hash,
            "evidenceHash": report.evidence_hash,
            "requiredFamilies": report.required_families,
            "plan": report.plan_json,
        }),
        &[],
    )?;
    store::set_run_status(connection, &id, AgentRunStatus::Running)?;
    Ok(id)
}

/// Append what happened, then let the reducer decide the terminal state. `None`
/// means the attempt is still open and the run row keeps waiting for a resume.
#[cfg(test)]
pub fn close_run(
    connection: &Connection,
    run_id: &str,
    report: &BackendReport,
) -> Result<Option<reducer::Reduction>, String> {
    if report.usage.total_tokens > 0 || report.usage.model_requests > 0 {
        store::settle_usage(connection, run_id, &report.usage)?;
    }
    if !report.covered_families.is_empty() {
        store::append_event(
            connection,
            run_id,
            AgentEventKind::CoverageUpdated,
            &serde_json::json!({"families": report.covered_families}),
            &[],
        )?;
    }
    if let Some(signal) = &report.protection_signal {
        store::append_event(
            connection,
            run_id,
            AgentEventKind::ProtectionDetected,
            &serde_json::json!({"signal": signal}),
            &[],
        )?;
    }
    let mut state = snapshot(run_id, report);
    if report.resumable
        && report.request_reconciliation_required.is_none()
        && report.execution_authorization_denied.is_none()
    {
        // Persist the boundary without a terminal state: recovery replays from
        // here on the next attempt of the same run.
        super::checkpoint::write_checkpoint(connection, &state)?;
        store::set_run_status(
            connection,
            run_id,
            if report.cancelled {
                super::contract::AgentRunStatus::Paused
            } else {
                super::contract::AgentRunStatus::Running
            },
        )?;
        return Ok(None);
    }
    let reduction = reducer::commit(connection, run_id, &report.signals())?;
    state.terminal = Some(reduction.state);
    state.terminal_code = reduction.code.clone();
    // Persist a snapshot boundary so recovery replays only newer events (§11).
    super::checkpoint::write_checkpoint(connection, &state)?;
    Ok(Some(reduction))
}

/// The deterministic snapshot of what the backend reported so far.
pub(super) fn snapshot(run_id: &str, report: &BackendReport) -> super::checkpoint::RunState {
    let mut state = super::checkpoint::RunState::new(run_id, Vec::new());
    state.turns = report.usage.model_requests;
    state.model_requests = report.usage.model_requests;
    state.target_requests = report.target_requests;
    state.request_accounting = report.request_accounting.clone();
    state.input_tokens = report.usage.input_tokens;
    state.output_tokens = report.usage.output_tokens;
    state.used_tokens = report.usage.total_tokens;
    state.cached_input_tokens = report.usage.cached_input_tokens;
    state.evidence_records = report.evidence_records;
    state.confirmed_findings = report.confirmed_findings;
    state.covered_families = report.covered_families.clone();
    state.completed_contracts = report.completed_contracts.clone();
    state.pending_contracts = (0..report.pending_contracts)
        .map(|index| format!("pending-{index}"))
        .collect();
    state.protection_signal = report.protection_signal.clone();
    state
}
