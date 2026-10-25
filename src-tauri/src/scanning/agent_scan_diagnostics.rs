//! Safe, durable operation outcomes. A rejected admission is different from an
//! unknown worker result; neither authorizes replay or a fresh allowance.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ExecutionSnapshot {
    attempt: i64,
    status: String,
    attempts: i64,
    roots: i64,
    dispatches: i64,
    requests: i64,
    tokens: i64,
}

pub(super) fn execution_snapshot(c: &rusqlite::Connection, project: i64, task: &str) -> Result<ExecutionSnapshot, String> {
    owned_scope(c, project, task)?;
    c.query_row("SELECT attempt_count,status,llm_requests,total_tokens,
        (SELECT COUNT(*) FROM sentinel_scan_attempts WHERE scan_id=s.id),
        (SELECT COUNT(*) FROM agent_runs WHERE scan_id=s.id),
        (SELECT COUNT(*) FROM native_branch_dispatches WHERE scan_id=s.id AND claim_id<>'' AND claimed_at<>'')
        FROM sentinel_scans s WHERE id=?1 AND project_id=?2", params![task,project], |r| Ok(ExecutionSnapshot {
        attempt:r.get(0)?,status:r.get(1)?,requests:r.get(2)?,tokens:r.get(3)?,attempts:r.get(4)?,roots:r.get(5)?,dispatches:r.get(6)?,
    })).map_err(|_| "scan_execution_snapshot_unavailable".into())
}

pub(super) fn reason_code(reason: &str) -> String {
    let head = reason.split(':').next().unwrap_or_default();
    if (3..=100).contains(&head.len()) && head.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && ["scan_","web_","budget_","financial_","model_","native_","tool_","agent_","root_","worker_",
            "readonly_","authorization_","protection_","administrative_","followup_","legacy_","resume_","unsupported_","invalid_web_"].iter().any(|p|head.starts_with(p)) {
        return head.into();
    }
    if reason.starts_with("任务当前状态为 ") { return "scan_state_not_eligible".into(); }
    if reason.starts_with("任务不存在") { return "scan_task_unavailable".into(); }
    if reason.starts_with("任务没有可扫描目标") { return "scan_no_eligible_targets".into(); }
    "scan_engine_unconfirmed".into()
}

pub(super) fn failed_receipt(c:&rusqlite::Connection, project:i64, operation:&str, action:&str,
    args:&Value, before:Option<&ExecutionSnapshot>, reason:&str) -> (&'static str,Value) {
    let after = args.get("taskId").and_then(Value::as_str)
        .and_then(|id|execution_snapshot(c,project,id).ok());
    let code = reason_code(reason);
    // Ordinary start/resume atomically publishes a new attempt before worker
    // dispatch. An unchanged durable attempt set proves it never reached that
    // boundary. Ambiguous COMMIT and dispatch results always remain unknown.
    let not_started = matches!(action,"start"|"resume") && before.is_some() && before == after.as_ref()
        && code != "scan_engine_unconfirmed" && !reason.contains("commit_unconfirmed")
        && !reason.contains("outcome_unknown") && !reason.contains("unconfirmed_no_automatic_retry");
    let phase = if not_started {"not_started"} else if code.contains("persist") || code.contains("commit") {"persistence"} else {"engine_dispatch"};
    let state = if not_started {"completed"} else {"uncertain"};
    (state,json!({"projectId":project,"operationId":operation,"operationState":if not_started{"rejected"}else{"uncertain"},
        "action":action,"taskId":args.get("taskId"),"phase":phase,"reasonCode":code,"outcomeUnknown":!not_started,
        "executionStartedByThisOperation":if not_started{Some(false)}else{None},"retryable":false,
        "attemptNumber":after.as_ref().map(|v|v.attempt),"previousAttemptNumber":before.map(|v|v.attempt),
        "nextAction":if not_started{"scan_preflight"}else{"scan_status"},
        "reason":if not_started{"启动前校验被拒绝，没有创建新执行尝试或发送请求；稳定原因码已保留。修正原因后重新预检，不要重复创建任务。"}
            else{"本次操作未取得确定执行回执；稳定原因码与尝试编号已保留。读取当前任务和原账本核对，不自动重发。"},
        "receiptIsHistorical":true}))
}
