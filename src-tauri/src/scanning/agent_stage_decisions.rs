//! Agent stage-adjudication timeline (§9.3). Append-only, read-mostly.
//!
//! This is the DATA + SURFACING half of §9 ("Agent 接管阶段编排"). It records
//! *that* the agent adjudicated a pipeline stage and *what* it decided, so the
//! 执行概览 UI can show the operator "the agent stepped in here and decided X",
//! and so the decision trail is auditable.
//!
//! It deliberately carries NO authority. Writing a row here never continues,
//! retries, degrades or unblocks a scan — the reducer and the sealed budget
//! contracts remain the only things that move control flow. Keeping the record
//! inert is exactly what makes this (§9.5 step 1) safe to ship ahead of the
//! §9.1 adjudicator: the enums and `record_stage_decision` give that later step
//! a ready place to log into, without this file ever changing what a scan does.
#![allow(dead_code)]

use crate::{db, AppState};
use rusqlite::{params, Connection};
use tauri::State;

/// The whole-schema text for this table, mirrored into the DB bootstrap the
/// same way `mapper_loop::SCHEMA` is (see `db.rs::initialize`). Additive only.
pub(crate) const SCHEMA: &str = include_str!("agent_stage_decisions_schema.sql");

/// What a pipeline stage produced, beyond plain success/failure (§9.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageOutcome {
    Ok,
    Degraded,
    Blocked,
    Gap,
}

impl StageOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "degraded",
            Self::Blocked => "blocked",
            Self::Gap => "gap",
        }
    }
}

/// The agent's adjudication decision for a stage (§9.1). `NeedsUser` is the only
/// decision that stops the pipeline for a human, and it is reserved for tier-3
/// (真·安全) gates per §9.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageDecision {
    Continue,
    RetryAdjusted,
    Degrade,
    Skip,
    NeedsUser,
}

impl StageDecision {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::RetryAdjusted => "retry_adjusted",
            Self::Degrade => "degrade",
            Self::Skip => "skip",
            Self::NeedsUser => "needs_user",
        }
    }

    /// The gate tier a decision implies when the caller does not pin one (§9.0):
    /// tier 1 keeps moving (continue/retry), tier 2 is a resource/budget
    /// degrade-or-skip, tier 3 is the safety stop that escalates to the user.
    pub(crate) fn default_tier(self) -> i64 {
        match self {
            Self::NeedsUser => 3,
            Self::Degrade | Self::Skip => 2,
            Self::Continue | Self::RetryAdjusted => 1,
        }
    }
}

/// Append one adjudication record. Returns the new row id. This NEVER moves
/// control flow — see the module docs. `gate_tier` outside 1..=3 is replaced by
/// the decision's default tier; a non-JSON `detail_json` is replaced by `{}` so
/// the column's `json_valid` CHECK can never reject a caller's free-form detail.
pub(crate) fn record_stage_decision(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    stage: &str,
    outcome: StageOutcome,
    decision: StageDecision,
    gate_tier: i64,
    rationale: &str,
    detail_json: &str,
) -> Result<i64, String> {
    let tier = if (1..=3).contains(&gate_tier) {
        gate_tier
    } else {
        decision.default_tier()
    };
    let detail = if serde_json::from_str::<serde_json::Value>(detail_json).is_ok() {
        detail_json
    } else {
        "{}"
    };
    connection
        .execute(
            "INSERT INTO agent_stage_decisions\
             (scan_id,attempt_number,stage,outcome,decision,gate_tier,rationale,detail_json) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                scan_id,
                attempt_number,
                stage,
                outcome.as_str(),
                decision.as_str(),
                tier,
                rationale,
                detail
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(connection.last_insert_rowid())
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStageDecisionView {
    id: i64,
    scan_id: String,
    attempt_number: i64,
    stage: String,
    outcome: String,
    decision: String,
    gate_tier: i64,
    rationale: String,
    detail_json: String,
    created_at: String,
}

fn list_agent_stage_decisions_in(
    connection: &Connection,
    scan_id: &str,
) -> Result<Vec<AgentStageDecisionView>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id,scan_id,attempt_number,stage,outcome,decision,gate_tier,rationale,detail_json,created_at \
             FROM agent_stage_decisions WHERE scan_id=?1 ORDER BY attempt_number,id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([scan_id], |row| {
            Ok(AgentStageDecisionView {
                id: row.get(0)?,
                scan_id: row.get(1)?,
                attempt_number: row.get(2)?,
                stage: row.get(3)?,
                outcome: row.get(4)?,
                decision: row.get(5)?,
                gate_tier: row.get(6)?,
                rationale: row.get(7)?,
                detail_json: row.get(8)?,
                created_at: row.get(9)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// The agent-intervention timeline for one scan, for the 执行概览 UI (§9.3).
/// Read-only; rows come back oldest-first so the UI renders them as a timeline.
#[tauri::command]
pub async fn list_agent_stage_decisions(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<Vec<AgentStageDecisionView>, String> {
    let database = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = db::open(&database)?;
        list_agent_stage_decisions_in(&connection, &scan_id)
    })
    .await
    .map_err(|_| "agent_stage_decisions_reader_failed".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        // Minimal parent so the real FK shape (ON DELETE CASCADE to
        // sentinel_scans) is exercised; the production schema has the full table.
        connection
            .execute_batch(
                "CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY);\n\
                 INSERT INTO sentinel_scans(id) VALUES('scan-1'),('scan-2');",
            )
            .unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        connection
    }

    #[test]
    fn records_and_lists_in_timeline_order() {
        let connection = memory_db();
        record_stage_decision(
            &connection,
            "scan-1",
            1,
            "root_frame_mapper",
            StageOutcome::Degraded,
            StageDecision::RetryAdjusted,
            1,
            "映射降级,已吸收重试",
            "{}",
        )
        .unwrap();
        record_stage_decision(
            &connection,
            "scan-1",
            1,
            "budget",
            StageOutcome::Blocked,
            StageDecision::Degrade,
            2,
            "预算到顶,转 partial",
            "{\"spentUsd\":5.0}",
        )
        .unwrap();
        // A different scan's row must not leak into scan-1's timeline.
        record_stage_decision(
            &connection,
            "scan-2",
            1,
            "fetch",
            StageOutcome::Blocked,
            StageDecision::NeedsUser,
            3,
            "联网拉源需用户授权",
            "{}",
        )
        .unwrap();

        let timeline = list_agent_stage_decisions_in(&connection, "scan-1").unwrap();
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].stage, "root_frame_mapper");
        assert_eq!(timeline[0].outcome, "degraded");
        assert_eq!(timeline[0].decision, "retry_adjusted");
        assert_eq!(timeline[0].gate_tier, 1);
        assert_eq!(timeline[1].stage, "budget");
        assert_eq!(timeline[1].decision, "degrade");
        assert_eq!(timeline[1].gate_tier, 2);
    }

    #[test]
    fn out_of_range_tier_falls_back_to_decision_default() {
        let connection = memory_db();
        record_stage_decision(
            &connection,
            "scan-1",
            0,
            "fetch",
            StageOutcome::Blocked,
            StageDecision::NeedsUser,
            99,
            "坏 tier 应回落到 needs_user 的默认档(3)",
            "{}",
        )
        .unwrap();
        let timeline = list_agent_stage_decisions_in(&connection, "scan-1").unwrap();
        assert_eq!(timeline[0].gate_tier, 3);
    }

    #[test]
    fn non_json_detail_is_replaced_so_check_never_rejects() {
        let connection = memory_db();
        // A caller passing non-JSON must not blow up the CHECK(json_valid).
        record_stage_decision(
            &connection,
            "scan-1",
            1,
            "gap_review",
            StageOutcome::Gap,
            StageDecision::Skip,
            2,
            "未覆盖族,记 gap 继续",
            "not json at all",
        )
        .unwrap();
        let timeline = list_agent_stage_decisions_in(&connection, "scan-1").unwrap();
        assert_eq!(timeline[0].detail_json, "{}");
        assert_eq!(timeline[0].outcome, "gap");
    }

    #[test]
    fn invalid_outcome_is_rejected_by_check() {
        let connection = memory_db();
        let result = connection.execute(
            "INSERT INTO agent_stage_decisions(scan_id,outcome) VALUES('scan-1','bogus')",
            [],
        );
        assert!(result.is_err());
    }
}
