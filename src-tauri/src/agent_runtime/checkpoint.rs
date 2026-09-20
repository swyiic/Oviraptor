//! Snapshot + event replay (§11).
//!
//! Recovery is deterministic: read the snapshot, replay only the events after
//! it, mark unfinished tool calls as interrupted and never re-charge budget for
//! work the event log already accounts for.
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
            "targetRequests": self.target_requests,
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
                if event
                    .payload
                    .get("progressAdvanced")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
                {
                    state.no_progress_streak = 0;
                } else {
                    state.no_progress_streak += 1;
                }
            }
            super::contract::AgentEventKind::ToolInvocationCompleted => {
                state.target_requests += 1;
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
    // §11 step 4: an interrupted call has no determined result, so its contract
    // goes back to the queue; the budget it already consumed stays charged.
    let without_result = store::contract_without_result(connection, run_id)?;
    if interrupted > 0 {
        for contract in without_result {
            if state.completed_contracts.contains(&contract) {
                state.completed_contracts.retain(|value| *value != contract);
                if !state.pending_contracts.contains(&contract) {
                    state.pending_contracts.push(contract);
                }
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
