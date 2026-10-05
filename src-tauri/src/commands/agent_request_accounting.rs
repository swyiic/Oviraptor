#[derive(Debug)]
struct AgentRequestAccounting {
    attempts: Vec<i64>,
    executor_recorded: i64,
    executor_unresolved: i64,
    external: crate::agent_runtime::target_requests::ExternalRequestUsage,
    authorization: crate::agent_runtime::target_requests::ExternalRequestUsage,
    recorded: i64,
    budget_committed: i64,
}

impl AgentRequestAccounting {
    fn as_json(&self) -> JsonValue {
        serde_json::json!({"available":true,"scope":"agent_budget_lineage",
            "attemptNumber":self.attempts[0],"includedAttempts":self.attempts,
            "executorRecordedRequests":self.executor_recorded,
            "executorUnresolvedClaims":self.executor_unresolved,
            "externalSurfaceReceivedRequests":self.external.received,
            "externalSurfaceUnresolvedClaims":self.external.unresolved,
            "authorizationReceivedRequests":self.authorization.received,
            "authorizationUnresolvedClaims":self.authorization.unresolved,
            "recordedRequests":self.recorded,"budgetCommittedRequests":self.budget_committed,
            "includesDeterministicRecon":false,"automaticReplayAllowed":false})
    }
}

fn agent_request_accounting(
    connection: &rusqlite::Connection, scan: &str, attempt: i64, target: &str,
) -> Result<AgentRequestAccounting,String> {
    use crate::agent_runtime::target_requests::{budget_attempts,external_usage,authorization_usage};
    let attempts = budget_attempts(connection,scan,attempt)?;
    let raw: Option<String> = connection.query_row(
        "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3",
        params![scan,target,NATIVE_AGENT_STATE_STAGE], |r|r.get(0),
    ).optional().map_err(|_| "request_accounting_checkpoint_unavailable")?;
    let executor_recorded = match raw {
        None => 0,
        Some(raw) => {
            let value: JsonValue = serde_json::from_str(&raw).map_err(|_| "request_accounting_checkpoint_invalid")?;
            let state = NativeAgentState::from_json(&value).ok_or("request_accounting_checkpoint_invalid")?;
            // Compatibility parsing may default absent historical fields to
            // zero. A truthful accounting projection must not treat defaults
            // as proof that no traffic occurred.
            if value.get("attemptNumber").and_then(JsonValue::as_i64)
                .is_none_or(|n|n < 1 || n != state.attempt_number) {
                return Err("request_accounting_checkpoint_attempt_unknown".into());
            }
            // A newer checkpoint must not erase access to an older immutable
            // journal. Without that journal, the old counter remains unknown.
            if state.attempt_number > attempt
                && agent_http_journal_usage(connection,scan,target,&attempts)?.is_none() {
                return Err("request_accounting_checkpoint_newer_attempt".into());
            }
            // A fresh retry must not display or charge the old attempt's native
            // counter. Continuations inherit it once, never sum snapshots.
            if attempts.contains(&state.attempt_number) {
                value.get("targetRequests").and_then(JsonValue::as_i64)
                    .filter(|n|*n >= 0).ok_or("request_accounting_checkpoint_counter_unknown")?
            } else { 0 }
        }
    };
    // Once installed, the immutable baseline + request journal replaces the
    // checkpoint counter. Never sum a new journal with its own projection.
    let (executor_recorded, executor_unresolved) = match agent_http_journal_usage(connection,scan,target,&attempts)? {
        Some((total,unresolved)) if total >= executor_recorded => (total,unresolved),
        Some(_) => return Err("http_journal_checkpoint_exceeds_ledger".into()),
        None => (executor_recorded,0),
    };
    let external = external_usage(connection,scan,&attempts,target)?;
    let authorization = authorization_usage(connection,scan,&attempts,target)?;
    let recorded = executor_recorded.checked_add(external.received)
        .and_then(|n|n.checked_add(authorization.received)).ok_or("request_accounting_overflow")?;
    let budget_committed = recorded.checked_add(external.unresolved)
        .and_then(|n|n.checked_add(authorization.unresolved)).ok_or("request_accounting_overflow")?;
    Ok(AgentRequestAccounting {attempts,executor_recorded,executor_unresolved,external,authorization,recorded,budget_committed})
}

fn agent_request_accounting_view(
    connection: &rusqlite::Connection, scan: &str, attempt: i64, target: &str,
) -> JsonValue {
    match agent_request_accounting(connection,scan,attempt,target) {
        Ok(usage) => usage.as_json(),
        Err(code) => serde_json::json!({"available":false,"scope":"agent_budget_lineage",
            "attemptNumber":attempt,"reasonCode":code,"recordedRequests":null,
            "budgetCommittedRequests":null,"automaticReplayAllowed":false}),
    }
}
