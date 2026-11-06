//! Snapshot + event replay (§11).
//!
//! Recovery is deterministic: read the snapshot, replay only the events after
//! it, mark unfinished tool calls as interrupted and never re-charge budget for
//! work the event log already accounts for.
//!
//! Convergence口径 (deep-exploit): `no_progress_streak` is OUTCOME-based. Exploration
//! advancing (new endpoints/params/coverage) is NOT progress; only new evidence or a
//! newly confirmed finding resets the streak. A model round that produced neither
//! advances it. See `replay`.
use super::contract::TerminalState;
use super::store::{self, AgentEventRow};
use rusqlite::Connection;
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunState {
    pub run_id: String,
    pub turns: i64,
    pub model_requests: i64,
    pub target_requests: i64,
    pub request_accounting: Option<JsonValue>,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub used_tokens: i64,
    pub covered_families: Vec<String>,
    pub completed_contracts: Vec<String>,
    pub pending_contracts: Vec<String>,
    pub evidence_records: i64,
    pub confirmed_findings: i64,
    pub progress_signature: String,
    pub no_progress_streak: i64,
    pub protection_signal: Option<String>,
    pub terminal: Option<TerminalState>,
    pub terminal_code: String,
    pub last_sequence: i64,
}

impl RunState {
    pub fn new(run_id: impl Into<String>, pending: Vec<String>) -> Self {
        Self {
            run_id: run_id.into(),
            pending_contracts: pending,
            ..Self::default()
        }
    }

    #[allow(dead_code)]
    pub fn uncached_tokens(&self) -> i64 {
        (self.input_tokens - self.cached_input_tokens).max(0) + self.output_tokens
    }

    pub fn as_json(&self) -> JsonValue {
        serde_json::json!({
            "runId": self.run_id,
            "turns": self.turns,
            "modelRequests": self.model_requests,
            "targetRequests": if self.request_accounting.as_ref().is_some_and(|v|v["available"] == false) {
                JsonValue::Null
            } else { JsonValue::from(self.target_requests) },
            "requestAccounting": self.request_accounting,
            "inputTokens": self.input_tokens,
            "cachedInputTokens": self.cached_input_tokens,
            "outputTokens": self.output_tokens,
            "usedTokens": self.used_tokens,
            "coveredFamilies": self.covered_families,
            "completedContracts": self.completed_contracts,
            "pendingContracts": self.pending_contracts,
            "evidenceRecords": self.evidence_records,
            "confirmedFindings": self.confirmed_findings,
            "progressSignature": self.progress_signature,
            "noProgressStreak": self.no_progress_streak,
            "protectionSignal": self.protection_signal,
            "terminal": self.terminal.map(|value| value.as_str()),
            "terminalCode": self.terminal_code,
            "lastSequence": self.last_sequence,
        })
    }

    pub fn from_json(value: &JsonValue) -> Self {
        let strings = |key: &str| -> Vec<String> {
            value
                .get(key)
                .and_then(JsonValue::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        let number = |key: &str| -> i64 { value.get(key).and_then(JsonValue::as_i64).unwrap_or(0) };
        Self {
            run_id: value
                .get("runId")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            turns: number("turns"),
            model_requests: number("modelRequests"),
            target_requests: number("targetRequests"),
            request_accounting: value
                .get("requestAccounting")
                .filter(|v| !v.is_null())
                .cloned(),
            input_tokens: number("inputTokens"),
            cached_input_tokens: number("cachedInputTokens"),
            output_tokens: number("outputTokens"),
            used_tokens: number("usedTokens"),
            covered_families: strings("coveredFamilies"),
            completed_contracts: strings("completedContracts"),
            pending_contracts: strings("pendingContracts"),
            evidence_records: number("evidenceRecords"),
            confirmed_findings: number("confirmedFindings"),
            progress_signature: value
                .get("progressSignature")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            no_progress_streak: number("noProgressStreak"),
            protection_signal: value
                .get("protectionSignal")
                .and_then(JsonValue::as_str)
                .map(str::to_string),
            terminal: value
                .get("terminal")
                .and_then(JsonValue::as_str)
                .and_then(TerminalState::parse),
            terminal_code: value
                .get("terminalCode")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            last_sequence: number("lastSequence"),
        }
    }
}

/// Persist the state as of the newest event so recovery never replays the whole
/// history.
pub fn write_checkpoint(connection: &Connection, state: &RunState) -> Result<(), String> {
    let head = store::latest_sequence(connection, &state.run_id);
    let mut snapshot = state.clone();
    snapshot.last_sequence = head;
    store::write_snapshot(connection, &state.run_id, head, &snapshot.as_json())
}

/// Fold events into a base state. Pure and order-dependent, so replaying the
/// same log twice from the same base yields the same result.
pub fn replay(base: &RunState, events: &[AgentEventRow]) -> RunState {
    let mut state = base.clone();
    for event in events {
        state.last_sequence = event.sequence;
        match event.event_type {
            super::contract::AgentEventKind::ModelRoundCompleted => {
                state.turns += 1;
                state.model_requests += 1;
                state.input_tokens += number(&event.payload, "/usage/inputTokens");
                state.cached_input_tokens += number(&event.payload, "/usage/cachedInputTokens");
                state.output_tokens += number(&event.payload, "/usage/outputTokens");
                state.used_tokens += number(&event.payload, "/usage/totalTokens");
                // Progress口径 (deep-exploit): a model round, by itself, is NOT progress.
                // The old criterion reset the streak on `progressAdvanced` — i.e. on mere
                // exploration/coverage advancing — so a run that kept discovering new
                // endpoints/parameters but never produced an exploit would never look
                // stalled (a root cause of the 10M-token spin). Progress is now defined by
                // OUTPUT: only new evidence (EvidenceProduced) or a newly CONFIRMED finding
                // (HypothesisUpdated=confirmed) resets the no-progress streak, below. Every
                // model round that produced neither advances the streak toward convergence.
                state.no_progress_streak += 1;
            }
            super::contract::AgentEventKind::ToolInvocationCompleted => {
                // A local inspection sends zero requests, a paired probe can
                // send several, and a failed send can still consume budget.
                // Legacy events without a delta cannot supply an exact total.
                match event
                    .payload
                    .get("targetRequestsDelta")
                    .and_then(JsonValue::as_i64)
                    .filter(|n| *n >= 0)
                    .and_then(|n| state.target_requests.checked_add(n))
                {
                    Some(total) => {
                        if total != state.target_requests
                            && state
                                .request_accounting
                                .as_ref()
                                .is_some_and(|v| v["available"] == true)
                        {
                            // A snapshot aggregate has a different boundary
                            // from newly replayed per-run events. Refresh it
                            // from the source ledgers, never retain stale sums.
                            state.request_accounting = Some(serde_json::json!({
                                "available":false,"scope":"run_replay","recordedRequests":null,
                                "reasonCode":"request_accounting_requires_refresh","automaticReplayAllowed":false,
                            }));
                        }
                        state.target_requests = total;
                    }
                    None => {
                        state.request_accounting = Some(serde_json::json!({
                            "available":false,"scope":"run_replay","recordedRequests":null,
                            "reasonCode":"tool_request_delta_unavailable","automaticReplayAllowed":false,
                        }))
                    }
                }
                if let Some(signature) = event
                    .payload
                    .get("progressSignature")
                    .and_then(JsonValue::as_str)
                {
                    if !signature.is_empty() {
                        state.progress_signature = signature.to_string();
                    }
                }
                if let Some(contract) = event.payload.get("contractKey").and_then(JsonValue::as_str)
                {
                    if !contract.is_empty() {
                        state.pending_contracts.retain(|value| value != contract);
                        if !state
                            .completed_contracts
                            .iter()
                            .any(|value| value == contract)
                        {
                            state.completed_contracts.push(contract.to_string());
                        }
                    }
                }
            }
            super::contract::AgentEventKind::EvidenceProduced => {
                state.evidence_records += 1;
                // New PoC / impact evidence is real progress (the outcome口径).
                state.no_progress_streak = 0;
            }
            super::contract::AgentEventKind::CoverageUpdated => {
                for family in string_array(&event.payload, "families") {
                    if !state.covered_families.iter().any(|value| value == &family) {
                        state.covered_families.push(family);
                    }
                }
            }
            super::contract::AgentEventKind::HypothesisUpdated => {
                if event.payload.get("status").and_then(JsonValue::as_str) == Some("confirmed") {
                    state.confirmed_findings += 1;
                    // A newly confirmed finding is real progress (the outcome口径).
                    state.no_progress_streak = 0;
                }
            }
            super::contract::AgentEventKind::ProtectionDetected => {
                state.protection_signal = event
                    .payload
                    .get("signal")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string);
            }
            super::contract::AgentEventKind::TerminalReduced => {
                state.terminal = event
                    .payload
                    .get("terminal")
                    .and_then(JsonValue::as_str)
                    .and_then(TerminalState::parse);
                state.terminal_code = event
                    .payload
                    .get("code")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_string();
            }
            _ => {}
        }
    }
    state
}

#[allow(dead_code)]
pub struct RecoveredRun {
    pub state: RunState,
    pub events_replayed: usize,
    pub interrupted_invocations: usize,
}

/// §11 / Phase 2 §4.2 recovery order: snapshot → replay → interrupt unfinished
/// calls → requeue only contracts that never produced a determined result. The
/// native loop calls this when an attempt continues a run that already exists.
pub fn recover(connection: &Connection, run_id: &str) -> Result<RecoveredRun, String> {
    let base = match store::read_snapshot(connection, run_id)? {
        Some(snapshot) if snapshot.schema_version == super::contract::RUNTIME_SCHEMA_VERSION => {
            let mut state = RunState::from_json(&snapshot.snapshot);
            // The row is the authority on the replay boundary.
            state.last_sequence = snapshot.last_sequence.max(state.last_sequence);
            state
        }
        _ => RunState {
            run_id: run_id.to_string(),
            ..RunState::default()
        },
    };
    let head = base.last_sequence;
    let events = store::read_events_after(connection, run_id, head)?;
    let events_replayed = events.len();
    let mut state = replay(&base, &events);
    let interrupted = store::mark_interrupted_tool_invocations(connection, run_id)?;
    // A target request (or an unknown tool) can have reached the server before
    // the process died. Its absence from the local result ledger is not proof
    // that it did not run. This includes invocations interrupted by an earlier
    // failed recovery: do not replay one after the status is no longer running.
    let ambiguous: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1 \
             AND status='interrupted' AND tool_name<>'inspect_evidence')",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法核对中断的目标调用：{error}"))?;
    if ambiguous {
        return Err("interrupted_tool_result_unknown_manual_reconciliation_required".into());
    }
    // Only a local, read-only evidence inspection is safe to requeue. The
    // budget it already consumed stays charged.
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT contract_key FROM tool_invocations WHERE run_id=?1 \
             AND status='interrupted' AND tool_name='inspect_evidence' AND contract_key<>''",
        )
        .map_err(|error| format!("无法读取可安全恢复的只读调用：{error}"))?;
    let contracts = statement
        .query_map([run_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("无法读取只读合同：{error}"))?;
    for contract in contracts {
        let contract = contract.map_err(|error| format!("只读合同损坏：{error}"))?;
        if state.completed_contracts.contains(&contract) {
            state.completed_contracts.retain(|value| *value != contract);
            if !state.pending_contracts.contains(&contract) {
                state.pending_contracts.push(contract);
            }
        }
    }
    Ok(RecoveredRun {
        state,
        events_replayed,
        interrupted_invocations: interrupted,
    })
}

fn number(value: &JsonValue, pointer: &str) -> i64 {
    value
        .pointer(pointer)
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
}

fn string_array(value: &JsonValue, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(JsonValue::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod convergence_tests {
    use super::*;
    use crate::agent_runtime::contract::AgentEventKind;
    use crate::agent_runtime::store::AgentEventRow;

    fn ev(sequence: i64, kind: AgentEventKind, payload: JsonValue) -> AgentEventRow {
        AgentEventRow {
            id: sequence,
            run_id: "r".into(),
            sequence,
            event_type: kind,
            payload,
            artifact_refs: Vec::new(),
            created_at: String::new(),
        }
    }

    #[test]
    fn model_rounds_alone_grow_the_streak() {
        // A model round that produces no output is not progress — the streak climbs
        // toward convergence (the opposite of the old exploration-counts-as-progress口径).
        let events: Vec<_> = (1..=3)
            .map(|s| ev(s, AgentEventKind::ModelRoundCompleted, serde_json::json!({})))
            .collect();
        let state = replay(&RunState::default(), &events);
        assert_eq!(state.no_progress_streak, 3);
        assert_eq!(state.turns, 3);
    }

    #[test]
    fn new_evidence_resets_the_streak() {
        let events = vec![
            ev(1, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
            ev(2, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
            ev(3, AgentEventKind::EvidenceProduced, serde_json::json!({})),
            ev(4, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
        ];
        let state = replay(&RunState::default(), &events);
        // streak: 1, 2, (evidence → 0), 1
        assert_eq!(state.no_progress_streak, 1);
        assert_eq!(state.evidence_records, 1);
    }

    #[test]
    fn coverage_advance_is_not_progress() {
        // Exploration (new covered family) must NOT reset the streak under the new口径.
        let events = vec![
            ev(1, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
            ev(2, AgentEventKind::CoverageUpdated, serde_json::json!({"families": ["sqli"]})),
            ev(3, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
        ];
        let state = replay(&RunState::default(), &events);
        assert_eq!(state.no_progress_streak, 2, "coverage advancing must not reset the streak");
        assert!(state.covered_families.contains(&"sqli".to_string()));
    }

    #[test]
    fn confirmed_finding_resets_but_unconfirmed_does_not() {
        let confirmed = vec![
            ev(1, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
            ev(2, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
            ev(3, AgentEventKind::HypothesisUpdated, serde_json::json!({"status": "confirmed"})),
            ev(4, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
        ];
        let state = replay(&RunState::default(), &confirmed);
        assert_eq!(state.no_progress_streak, 1); // reset at seq 3, then +1
        assert_eq!(state.confirmed_findings, 1);

        let unconfirmed = vec![
            ev(1, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
            ev(2, AgentEventKind::HypothesisUpdated, serde_json::json!({"status": "rejected"})),
            ev(3, AgentEventKind::ModelRoundCompleted, serde_json::json!({})),
        ];
        let state = replay(&RunState::default(), &unconfirmed);
        assert_eq!(state.no_progress_streak, 2, "a non-confirmed hypothesis is not progress");
        assert_eq!(state.confirmed_findings, 0);
    }
}
