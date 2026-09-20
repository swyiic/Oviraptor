//! Typed repository for `agent_assignments` (Stage 1A §5.2, §6).
//!
//! Role, lane, state, budget and lifecycle live in their own columns; only the task
//! slice and the reference lists are JSON. Every write redacts first, and every
//! failure comes back as `Err` — a caller can never lose an assignment to a
//! swallowed database error.

// Stage 1A declares the contract and its storage only; the scheduler, the child
// runs and the review gate that consume them land in the next stages. Every item
// here is exercised by the Stage 1A tests, so the reachability warning is expected.
#![allow(dead_code)]
use super::super::contract::{can_transition, AgentLane, AgentRole, AssignmentState};
use super::super::secrets::redact_json;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

const COLUMNS: &str = "id,coordinator_run_id,child_run_id,role,lane,target_key,state,dedup_key,\
     trigger_code,task_slice_json,evidence_revision,contract_keys_json,identity_handles_json,\
     reserved_tokens,reserved_requests,capability_lease_json,deadline_at,failure_class,\
     created_at,leased_at,started_at,finished_at,updated_at";

#[derive(Clone, Debug, PartialEq)]
pub struct AgentAssignment {
    pub id: String,
    pub coordinator_run_id: String,
    pub child_run_id: String,
    pub role: AgentRole,
    pub lane: AgentLane,
    pub target_key: String,
    pub state: AssignmentState,
    pub dedup_key: String,
    pub trigger_code: String,
    pub task_slice: JsonValue,
    pub evidence_revision: i64,
    pub contract_keys: Vec<String>,
    pub identity_handles: Vec<String>,
    pub reserved_tokens: i64,
    pub reserved_requests: i64,
    pub capability_lease: Vec<String>,
    pub deadline_at: String,
    pub failure_class: String,
    pub created_at: String,
    pub leased_at: String,
    pub started_at: String,
    pub finished_at: String,
    pub updated_at: String,
}

impl AgentAssignment {
    /// A prepared slice for a role. The state machine starts at `prepared`; nothing
    /// in this type implies the assignment was ever leased or executed.
    pub fn new(
        id: impl Into<String>,
        coordinator_run_id: impl Into<String>,
        role: AgentRole,
        lane: AgentLane,
        target_key: impl Into<String>,
        dedup_key: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            coordinator_run_id: coordinator_run_id.into(),
            child_run_id: String::new(),
            role,
            lane,
            target_key: target_key.into(),
            state: AssignmentState::Prepared,
            dedup_key: dedup_key.into(),
            trigger_code: String::new(),
            task_slice: JsonValue::Object(serde_json::Map::new()),
            evidence_revision: 0,
            contract_keys: Vec::new(),
            identity_handles: Vec::new(),
            reserved_tokens: 0,
            reserved_requests: 0,
            capability_lease: Vec::new(),
            deadline_at: String::new(),
            failure_class: String::new(),
            created_at: String::new(),
            leased_at: String::new(),
            started_at: String::new(),
            finished_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn text_list(&self) -> Result<(String, String, String), String> {
        Ok((
            redact_json(&serde_json::json!(self.contract_keys)).to_string(),
            redact_json(&serde_json::json!(self.identity_handles)).to_string(),
            redact_json(&serde_json::json!(self.capability_lease)).to_string(),
        ))
    }
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentAssignment> {
    let strings = |index: usize| -> Vec<String> {
        let text = row.get::<_, String>(index).unwrap_or_default();
        serde_json::from_str::<Vec<String>>(&text).unwrap_or_default()
    };
    Ok(AgentAssignment {
        id: row.get(0)?,
        coordinator_run_id: row.get(1)?,
        child_run_id: row.get(2)?,
        role: AgentRole::parse(&row.get::<_, String>(3)?),
        lane: AgentLane::parse(&row.get::<_, String>(4)?),
        target_key: row.get(5)?,
        state: AssignmentState::parse(&row.get::<_, String>(6)?),
        dedup_key: row.get(7)?,
        trigger_code: row.get(8)?,
        task_slice: serde_json::from_str(&row.get::<_, String>(9).unwrap_or_default())
            .unwrap_or_else(|_| JsonValue::Object(serde_json::Map::new())),
        evidence_revision: row.get(10)?,
        contract_keys: strings(11),
        identity_handles: strings(12),
        reserved_tokens: row.get(13)?,
        reserved_requests: row.get(14)?,
        capability_lease: strings(15),
        deadline_at: row.get(16)?,
        failure_class: row.get(17)?,
        created_at: row.get(18)?,
        leased_at: row.get(19)?,
        started_at: row.get(20)?,
        finished_at: row.get(21)?,
        updated_at: row.get(22)?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssignmentInsert {
    /// Newly stored under this id.
    Inserted(String),
    /// The same dedup key already exists for this coordinator, so nothing was added.
    Existing(String),
}

/// Store one assignment slice. Replaying the same `(coordinator_run_id, dedup_key)`
/// returns the row that is already there instead of creating a second task for the
/// same work (§11.2); two coordinators may use the same dedup key.
pub fn insert_assignment(
    connection: &Connection,
    assignment: &AgentAssignment,
) -> Result<AssignmentInsert, String> {
    if assignment.id.trim().is_empty() {
        return Err("assignment 缺少 id".to_string());
    }
    if assignment.dedup_key.trim().is_empty() {
        return Err("assignment 缺少 dedup_key".to_string());
    }
    let slice = redact_json(&assignment.task_slice).to_string();
    let (contracts, identities, lease) = assignment.text_list()?;
    let existing: Option<String> = connection
        .query_row(
            "SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND dedup_key=?2",
            params![assignment.coordinator_run_id, assignment.dedup_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法读取已有 assignment：{error}"))?;
    if let Some(found) = existing {
        return Ok(AssignmentInsert::Existing(found));
    }
    match connection.execute(
        &format!(
            "INSERT INTO agent_assignments({}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,datetime('now','localtime'),'',?,?,datetime('now','localtime'))",
            COLUMNS
        ),
        params![
            assignment.id,
            assignment.coordinator_run_id,
            assignment.child_run_id,
            assignment.role.as_str(),
            assignment.lane.as_str(),
            assignment.target_key,
            assignment.state.as_str(),
            assignment.dedup_key,
            assignment.trigger_code,
            slice,
            assignment.evidence_revision,
            contracts,
            identities,
            assignment.reserved_tokens,
            assignment.reserved_requests,
            lease,
            assignment.deadline_at,
            assignment.failure_class,
            // created_at, leased_at and updated_at are stamped by the database; the
            // caller's copy is never trusted over the clock.
            assignment.started_at,
            assignment.finished_at,
        ],
    ) {
        Ok(_) => Ok(AssignmentInsert::Inserted(assignment.id.clone())),
        // A concurrent insert won the same unique key: the replay answer is the row
        // that exists, never a second one.
        Err(error) if error.to_string().contains("UNIQUE") => connection
            .query_row(
                "SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND dedup_key=?2",
                params![assignment.coordinator_run_id, assignment.dedup_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|found| match found {
                Some(id) => AssignmentInsert::Existing(id),
                None => AssignmentInsert::Inserted(assignment.id.clone()),
            })
            .map_err(|error| format!("无法确认重放的 assignment：{error}")),
        Err(error) => Err(format!("无法写入 assignment：{error}")),
    }
}

pub fn load_assignment(
    connection: &Connection,
    id: &str,
) -> Result<Option<AgentAssignment>, String> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM agent_assignments WHERE id=?1"),
            [id],
            read_row,
        )
        .optional()
        .map_err(|error| format!("无法读取 assignment：{error}"))
}

pub fn list_assignments_for_coordinator(
    connection: &Connection,
    coordinator_run_id: &str,
) -> Result<Vec<AgentAssignment>, String> {
    let mut statement = connection
        .prepare(
            &format!(
                "SELECT {COLUMNS} FROM agent_assignments WHERE coordinator_run_id=?1 ORDER BY created_at,id"
            ),
        )
        .map_err(|error| format!("无法准备 assignment 查询：{error}"))?;
    let rows = statement
        .query_map([coordinator_run_id], read_row)
        .map_err(|error| format!("无法读取 assignment 列表：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析 assignment 列表：{error}"))
}

/// Compare-and-set the lifecycle. The stored state must still be `from`, and the
/// move must be legal; anything else leaves the row untouched and answers `false`
/// rather than rewriting history.
pub fn transition_assignment(
    connection: &Connection,
    id: &str,
    from: AssignmentState,
    to: AssignmentState,
) -> Result<bool, String> {
    if !can_transition(from, to) {
        return Err(format!(
            "不允许的 assignment 状态转换：{} -> {}",
            from.as_str(),
            to.as_str()
        ));
    }
    let stamped = match to {
        AssignmentState::Leased => ",leased_at=datetime('now','localtime')",
        AssignmentState::Running => ",started_at=datetime('now','localtime')",
        state if state.is_terminal() => ",finished_at=datetime('now','localtime')",
        _ => "",
    };
    let changed = connection
        .execute(
            &format!(
                "UPDATE agent_assignments SET state=?1{stamped},updated_at=datetime('now','localtime') \
                 WHERE id=?2 AND state=?3"
            ),
            params![to.as_str(), id, from.as_str()],
        )
        .map_err(|error| format!("无法更新 assignment 状态：{error}"))?;
    Ok(changed > 0)
}
