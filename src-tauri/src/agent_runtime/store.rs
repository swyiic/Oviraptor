//! Event, run, message and artifact persistence (§11, §12).
//!
//! Model rounds and tool executions may run concurrently, but they only ever
//! append here. The terminal state is written by `reducer` alone.
use super::contract::{
    AgentBackendKind, AgentEventKind, AgentMessageKind, AgentRole, AgentRunStatus, TerminalState,
};
use super::secrets::redact_json;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub const RUNTIME_EVENT_LIMIT: i64 = 20_000;

pub fn stable_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageDelta {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub model_requests: i64,
}

impl UsageDelta {
    pub fn as_json(&self) -> JsonValue {
        serde_json::json!({
            "inputTokens": self.input_tokens,
            "cachedInputTokens": self.cached_input_tokens,
            "outputTokens": self.output_tokens,
            "totalTokens": self.total_tokens,
            "modelRequests": self.model_requests,
        })
    }
}

#[derive(Clone, Debug)]
pub struct AgentRunRow {
    pub id: String,
    pub scan_id: String,
    pub attempt_number: i64,
    pub target_url: String,
    pub backend: AgentBackendKind,
    pub role: AgentRole,
    pub parent_run_id: Option<String>,
    pub status: AgentRunStatus,
    pub plan_hash: String,
    pub evidence_hash: String,
    pub soft_token_budget: i64,
    pub hard_token_budget: i64,
    // Read by the Phase 3 status surface; the reducer writes them in Phase 0.
    #[allow(dead_code)]
    pub used_tokens: i64,
    #[allow(dead_code)]
    pub used_cached_tokens: i64,
    pub soft_request_budget: i64,
    pub hard_request_budget: i64,
    #[allow(dead_code)]
    pub used_requests: i64,
    pub lease_expires_at: String,
    pub terminal_reason: String,
    pub terminal_code: String,
    pub terminal_state: Option<TerminalState>,
}

impl AgentRunRow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        scan_id: impl Into<String>,
        attempt_number: i64,
        target_url: impl Into<String>,
        backend: AgentBackendKind,
        role: AgentRole,
        plan_hash: impl Into<String>,
        evidence_hash: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            scan_id: scan_id.into(),
            attempt_number,
            target_url: target_url.into(),
            backend,
            role,
            parent_run_id: None,
            status: AgentRunStatus::Prepared,
            plan_hash: plan_hash.into(),
            evidence_hash: evidence_hash.into(),
            soft_token_budget: 0,
            hard_token_budget: 0,
            used_tokens: 0,
            used_cached_tokens: 0,
            soft_request_budget: 0,
            hard_request_budget: 0,
            used_requests: 0,
            lease_expires_at: String::new(),
            terminal_reason: String::new(),
            terminal_code: String::new(),
            terminal_state: None,
        }
    }

    pub fn with_budget(
        mut self,
        soft: i64,
        hard: i64,
        soft_requests: i64,
        hard_requests: i64,
    ) -> Self {
        self.soft_token_budget = soft;
        self.hard_token_budget = hard;
        self.soft_request_budget = soft_requests;
        self.hard_request_budget = hard_requests;
        self
    }

    pub fn is_terminal(&self) -> bool {
        self.status == AgentRunStatus::Terminal
    }
}

pub fn create_run(connection: &Connection, row: &AgentRunRow) -> Result<(), String> {
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,parent_run_id,status,plan_hash,evidence_hash,soft_token_budget,hard_token_budget,soft_request_budget,hard_request_budget,lease_expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15) ON CONFLICT(id) DO UPDATE SET status=excluded.status,plan_hash=excluded.plan_hash,evidence_hash=excluded.evidence_hash,backend=excluded.backend,role=excluded.role,updated_at=datetime('now','localtime')",
        params![
            row.id,
            row.scan_id,
            row.attempt_number,
            row.target_url,
            row.backend.as_str(),
            row.role.as_str(),
            row.parent_run_id,
            row.status.as_str(),
            row.plan_hash,
            row.evidence_hash,
            row.soft_token_budget,
            row.hard_token_budget,
            row.soft_request_budget,
            row.hard_request_budget,
            row.lease_expires_at,
        ],
    )
    .map_err(|error| format!("无法创建 agent run：{error}"))?;
    Ok(())
}

/// Phase 2 §2.2: the frozen execution plan belongs to the attempt that ran it, not
/// to the URL. `agent_runs` already carries the attempt dimension, so the plan is
/// recorded there; the per-URL checkpoint row stays a compatibility projection and
/// nothing is ever deleted.
pub fn record_attempt_plan(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_url: &str,
    backend: AgentBackendKind,
    plan_hash: &str,
    plan: &JsonValue,
) -> Result<(), String> {
    let payload = plan.to_string();
    let updated = connection
        .execute(
            "UPDATE agent_runs SET plan_json=?1,plan_hash=?2,backend=?3,updated_at=datetime('now','localtime') WHERE scan_id=?4 AND attempt_number=?5 AND target_url=?6 AND role=?7",
            params![
                payload,
                plan_hash,
                backend.as_str(),
                scan_id,
                attempt_number,
                target_url,
                AgentRole::Coordinator.as_str()
            ],
        )
        .map_err(|error| format!("无法记录 attempt 计划：{error}"))?;
    if updated > 0 {
        return Ok(());
    }
    let row = AgentRunRow::new(
        format!(
            "plan-{scan_id}-{attempt_number}-{}",
            &stable_hash(target_url)[..12]
        ),
        scan_id,
        attempt_number,
        target_url,
        backend,
        AgentRole::Coordinator,
        plan_hash,
        "",
    );
    create_run(connection, &row)?;
    connection
        .execute(
            "UPDATE agent_runs SET plan_json=?1 WHERE id=?2",
            params![payload, row.id],
        )
        .map_err(|error| format!("无法写入 attempt 计划：{error}"))?;
    Ok(())
}

/// The plan one attempt ran on. `None` means that attempt never recorded one: either
/// it predates this column, or it never started.
pub fn attempt_plan(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_url: &str,
) -> Option<JsonValue> {
    connection
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role=?4 AND plan_json <> '{}' ORDER BY created_at DESC LIMIT 1",
            params![scan_id, attempt_number, target_url, AgentRole::Coordinator.as_str()],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

/// Bring an existing run row up to the frozen plan it is about to execute: the four
/// ceilings and the plan hash must describe the same plan the loop reads, even when
/// the row was first created by plan recording rather than by `open_run`.
pub fn apply_run_plan(
    connection: &Connection,
    run_id: &str,
    plan_hash: &str,
    soft_token_budget: i64,
    hard_token_budget: i64,
    soft_request_budget: i64,
    hard_request_budget: i64,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE agent_runs SET plan_hash=?2,soft_token_budget=?3,hard_token_budget=?4,soft_request_budget=?5,hard_request_budget=?6,updated_at=datetime('now','localtime') WHERE id=?1",
            params![run_id, plan_hash, soft_token_budget, hard_token_budget, soft_request_budget, hard_request_budget],
        )
        .map_err(|error| format!("无法写入运行预算：{error}"))?;
    Ok(())
}

pub fn set_run_status(
    connection: &Connection,
    run_id: &str,
    status: AgentRunStatus,
) -> Result<(), String> {
    connection.execute(
        "UPDATE agent_runs SET status=?1,updated_at=datetime('now','localtime'),started_at=CASE WHEN ?1='running' AND started_at='' THEN datetime('now','localtime') ELSE started_at END WHERE id=?2",
        params![status.as_str(), run_id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn load_run(connection: &Connection, run_id: &str) -> Result<Option<AgentRunRow>, String> {
    connection
        .query_row(
            "SELECT id,scan_id,attempt_number,target_url,backend,role,parent_run_id,status,plan_hash,evidence_hash,soft_token_budget,hard_token_budget,used_tokens,used_cached_tokens,soft_request_budget,hard_request_budget,used_requests,lease_expires_at,terminal_reason,terminal_code,terminal_state FROM agent_runs WHERE id=?1",
            [run_id],
            |row| {
                Ok(AgentRunRow {
                    id: row.get(0)?,
                    scan_id: row.get(1)?,
                    attempt_number: row.get(2)?,
                    target_url: row.get(3)?,
                    backend: AgentBackendKind::parse(&row.get::<_, String>(4)?).unwrap_or(AgentBackendKind::Strix),
                    role: AgentRole::parse(&row.get::<_, String>(5)?),
                    parent_run_id: row.get::<_, Option<String>>(6)?,
                    status: AgentRunStatus::parse(&row.get::<_, String>(7)?),
                    plan_hash: row.get(8)?,
                    evidence_hash: row.get(9)?,
                    soft_token_budget: row.get(10)?,
                    hard_token_budget: row.get(11)?,
                    used_tokens: row.get(12)?,
                    used_cached_tokens: row.get(13)?,
                    soft_request_budget: row.get(14)?,
                    hard_request_budget: row.get(15)?,
                    used_requests: row.get(16)?,
                    lease_expires_at: row.get(17)?,
                    terminal_reason: row.get(18)?,
                    terminal_code: row.get(19)?,
                    terminal_state: TerminalState::parse(
                        &row.get::<_, String>(20).unwrap_or_default()
                    ),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

pub fn find_run(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_url: &str,
    role: AgentRole,
) -> Result<Option<AgentRunRow>, String> {
    connection
        .query_row(
            "SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role=?4 ORDER BY created_at DESC LIMIT 1",
            params![scan_id, attempt_number, target_url, role.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())
        .and_then(|id| match id {
            Some(id) => load_run(connection, &id),
            None => Ok(None),
        })
}

/// Additive only: budgets and usage are never overwritten by a backend.
/// Write the cumulative spend of a run. The close-out can be replayed after a
/// resume, so the row converges on the reported total instead of adding the same
/// spend a second time.
pub fn settle_usage(
    connection: &Connection,
    run_id: &str,
    usage: &UsageDelta,
) -> Result<(), String> {
    connection.execute(
        "UPDATE agent_runs SET used_tokens=?1,used_cached_tokens=?2,used_requests=?3,updated_at=datetime('now','localtime') WHERE id=?4",
        params![usage.total_tokens, usage.cached_input_tokens, usage.model_requests, run_id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[allow(dead_code)]
pub fn renew_lease(connection: &Connection, run_id: &str, minutes: i64) -> Result<(), String> {
    connection.execute(
        "UPDATE agent_runs SET lease_expires_at=datetime('now','localtime',?2),updated_at=datetime('now','localtime') WHERE id=?1",
        params![run_id, format!("+{minutes} minutes")],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// The full row is kept because the Phase 2 broker reads the refs and the
/// debug view shows the timestamps; replay itself only needs the payload.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct AgentEventRow {
    pub id: i64,
    pub run_id: String,
    pub sequence: i64,
    pub event_type: AgentEventKind,
    pub payload: JsonValue,
    pub artifact_refs: Vec<String>,
    pub created_at: String,
}

/// Append one event and return its sequence. `UNIQUE(run_id,sequence)` rejects
/// a concurrent writer, so the caller retries a few times instead of racing.
pub fn append_event(
    connection: &Connection,
    run_id: &str,
    kind: AgentEventKind,
    payload: &JsonValue,
    artifact_refs: &[String],
) -> Result<i64, String> {
    let payload = redact_json(payload);
    let refs = redact_json(&serde_json::json!(artifact_refs));
    let refs_text = refs.to_string();
    for attempt in 0..5i64 {
        let next: i64 = connection
            .query_row(
                "SELECT COALESCE(MAX(sequence),0)+1 FROM agent_events WHERE run_id=?1",
                [run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let result = connection.execute(
            "INSERT INTO agent_events(run_id,sequence,event_type,payload_json,artifact_refs_json) VALUES(?1,?2,?3,?4,?5)",
            params![run_id, next, kind.as_str(), payload.to_string(), refs_text],
        );
        match result {
            Ok(_) => return Ok(next),
            Err(error) if error.to_string().contains("UNIQUE") && attempt < 4 => {
                continue;
            }
            Err(error) => return Err(format!("无法写入 agent event：{error}")),
        }
    }
    Err("agent event 序号竞争失败".to_string())
}

pub fn read_events_after(
    connection: &Connection,
    run_id: &str,
    sequence: i64,
) -> Result<Vec<AgentEventRow>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id,run_id,sequence,event_type,payload_json,artifact_refs_json,created_at FROM agent_events WHERE run_id=?1 AND sequence> ?2 ORDER BY sequence ASC LIMIT ?3",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id, sequence, RUNTIME_EVENT_LIMIT], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut events = Vec::new();
    for row in rows {
        let (id, run_id, sequence, kind, payload_text, refs_text, created_at) =
            row.map_err(|error| error.to_string())?;
        events.push(AgentEventRow {
            id,
            run_id,
            sequence,
            event_type: AgentEventKind::parse(&kind),
            payload: parse_json(&payload_text),
            artifact_refs: parse_json(&refs_text)
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            created_at,
        });
    }
    Ok(events)
}

pub fn latest_sequence(connection: &Connection, run_id: &str) -> i64 {
    connection
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_events WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

fn parse_json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap_or(JsonValue::Null)
}

pub fn write_snapshot(
    connection: &Connection,
    run_id: &str,
    last_sequence: i64,
    snapshot: &JsonValue,
) -> Result<(), String> {
    let snapshot = redact_json(snapshot);
    connection.execute(
        "INSERT INTO agent_snapshots(run_id,last_sequence,schema_version,snapshot_json) VALUES(?1,?2,?3,?4) ON CONFLICT(run_id) DO UPDATE SET last_sequence=excluded.last_sequence,schema_version=excluded.schema_version,snapshot_json=excluded.snapshot_json,updated_at=datetime('now','localtime')",
        params![run_id, last_sequence, super::contract::RUNTIME_SCHEMA_VERSION, snapshot.to_string()],
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[derive(Clone, Debug)]
pub struct SnapshotRow {
    #[allow(dead_code)]
    pub run_id: String,
    pub last_sequence: i64,
    pub schema_version: i64,
    pub snapshot: JsonValue,
}

pub fn read_snapshot(connection: &Connection, run_id: &str) -> Result<Option<SnapshotRow>, String> {
    connection
        .query_row(
            "SELECT run_id,last_sequence,schema_version,snapshot_json FROM agent_snapshots WHERE run_id=?1",
            [run_id],
            |row| {
                Ok(SnapshotRow {
                    run_id: row.get(0)?,
                    last_sequence: row.get(1)?,
                    schema_version: row.get(2)?,
                    snapshot: parse_json(&row.get::<_, String>(3)?),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

/// Messages are deduplicated by `(run_id, dedup_key)` so a replay after a crash
/// cannot process the same contract twice (§9). Consumed in Phase 5.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub fn append_message(
    connection: &Connection,
    run_id: &str,
    from_agent: &str,
    to_agent: &str,
    kind: AgentMessageKind,
    correlation_id: &str,
    dedup_key: &str,
    payload: &JsonValue,
    artifact_refs: &[String],
) -> Result<bool, String> {
    let payload = redact_json(payload);
    let inserted = connection
        .execute(
            "INSERT INTO agent_messages(id,run_id,from_agent,to_agent,kind,correlation_id,dedup_key,payload_json,artifact_refs_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(run_id,dedup_key) DO NOTHING",
            params![
                uuid::Uuid::new_v4().to_string(),
                run_id,
                from_agent,
                to_agent,
                kind.as_str(),
                correlation_id,
                dedup_key,
                payload.to_string(),
                serde_json::json!(artifact_refs).to_string()
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(inserted > 0)
}

#[allow(dead_code)]
pub fn read_undelivered_messages(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<(String, JsonValue, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT kind,payload_json,dedup_key FROM agent_messages WHERE run_id=?1 AND delivered_at='' ORDER BY created_at ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    Ok(rows
        .flatten()
        .map(|(kind, payload, key)| (kind, parse_json(&payload), key))
        .collect())
}

#[allow(dead_code)]
pub fn mark_message_delivered(connection: &Connection, dedup_key: &str) -> Result<(), String> {
    connection
        .execute(
            "UPDATE agent_messages SET delivered_at=datetime('now','localtime') WHERE dedup_key=?1 AND delivered_at=''",
            [dedup_key],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
/// Phase 2 §4.2 step 2: the row exists in `running` state before any side effect,
/// so a crash mid-call leaves something recoverable instead of a silent gap.
/// `invocation_id` is the id the runtime hands out and the coverage ledger cites,
/// which keeps the audit row and the ledger on the same key (§9.2).
pub fn begin_tool_invocation(
    connection: &Connection,
    run_id: &str,
    invocation_id: &str,
    tool_name: &str,
    tool_version: i64,
    contract_key: &str,
    identity_handle: &str,
    input: &JsonValue,
    policy_decision: &str,
) -> Result<i64, String> {
    let summary = redact_json(input);
    let input_hash = stable_hash(&summary.to_string());
    connection
        .execute(
            "INSERT INTO tool_invocations(run_id,invocation_id,tool_name,tool_version,contract_key,identity_handle,input_hash,input_summary_json,policy_decision,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'running')",
            params![run_id, invocation_id, tool_name, tool_version, contract_key, identity_handle, input_hash, summary.to_string(), policy_decision],
        )
        .map_err(|error| error.to_string())?;
    Ok(connection.last_insert_rowid())
}

/// Phase 2 §4.2 step 5: the deterministic end state of one call, with the response
/// artifact it produced. Only after this may the result travel back to the model.
pub fn finish_tool_invocation(
    connection: &Connection,
    invocation_id: i64,
    status: &str,
    progress_signature: &str,
    response_artifact_id: &str,
    error_class: &str,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE tool_invocations SET status=?1,progress_signature=?2,response_artifact_id=?3,error_class=?4,finished_at=datetime('now','localtime') WHERE id=?5",
            params![status, progress_signature, response_artifact_id, error_class, invocation_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// §11 step 3: a call still marked `running` after a crash can never be trusted.
pub fn mark_interrupted_tool_invocations(
    connection: &Connection,
    run_id: &str,
) -> Result<usize, String> {
    connection
        .execute(
            "UPDATE tool_invocations SET status='interrupted',error_class='process_interrupted',finished_at=datetime('now','localtime') WHERE run_id=?1 AND status='running'",
            [run_id],
        )
        .map_err(|error| error.to_string())
}

pub fn contract_without_result(
    connection: &Connection,
    run_id: &str,
) -> Result<BTreeSet<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT contract_key FROM tool_invocations WHERE run_id=?1 AND contract_key<>'' AND status NOT IN ('confirmed','rejected','exhausted','insufficient_evidence')",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([run_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    Ok(rows.flatten().collect())
}

pub fn mark_run_terminal(
    connection: &Connection,
    run_id: &str,
    state: TerminalState,
    code: &str,
    reason: &str,
) -> Result<bool, String> {
    let changed = connection
        .execute(
            "UPDATE agent_runs SET status='terminal',terminal_reason=?2,terminal_code=?3,terminal_state=?4,finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1 AND status<>'terminal'",
            params![run_id, reason, code, state.as_str()],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Ok(false);
    }
    let _ = append_event(
        connection,
        run_id,
        AgentEventKind::TerminalReduced,
        &serde_json::json!({"terminal": state.as_str(), "code": code, "reason": reason}),
        &[],
    );
    Ok(true)
}

/// Content-addressed artifact store (§12). Only the id is ever persisted in a
/// message or event; the bytes stay on disk. The Phase 2 broker writes the first
/// rows; the layout and permissions are fixed here and covered by tests.
#[allow(dead_code)]
pub fn artifact_id(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn artifact_path(root: &Path, id: &str) -> PathBuf {
    root.join("artifacts")
        .join(id.chars().take(2).collect::<String>())
        .join(id)
}

#[allow(dead_code)]
pub fn store_artifact(root: &Path, bytes: &[u8]) -> Result<String, String> {
    let id = artifact_id(bytes);
    let path = artifact_path(root, &id);
    if path.is_file() {
        return Ok(id);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    crate::db::write_private_file(&path, bytes)?;
    Ok(id)
}

#[allow(dead_code)]
pub fn read_artifact(root: &Path, id: &str) -> Option<Vec<u8>> {
    fs::read(artifact_path(root, id)).ok()
}
