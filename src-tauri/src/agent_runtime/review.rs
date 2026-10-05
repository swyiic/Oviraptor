//! Evidence Reviewer verdict contract and storage (§4.6, §5.5).
//!
//! The reviewer is the only role allowed to call something a finding. This module
//! stores the immutable decision; the fenced publication gate projects a confirmed
//! decision separately. Replays are idempotent and conflicting answers are refused.

// The live review gate consumes this repository. Some query helpers remain
// acceptance-test-only, so the reachability warning is expected.
#![allow(dead_code)]
use crate::agent_runtime::contract::AgentRole;
use crate::agent_runtime::secrets::redact_json;
use crate::agent_runtime::store::decode_json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

const COLUMNS: &str = "id,root_run_id,candidate_id,candidate_revision,reviewer_run_id,verdict,\
     reason_codes_json,evidence_refs_json,counter_evidence_refs_json,missing_evidence_json,\
     confidence,severity,created_at";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Confirmed,
    Rejected,
    #[default]
    InsufficientEvidence,
}

impl ReviewVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
            Self::InsufficientEvidence => "insufficient_evidence",
        }
    }

    /// Anything unreadable is treated as "not enough evidence", never as confirmed.
    pub fn parse(value: &str) -> Self {
        Self::try_parse(value).unwrap_or(Self::InsufficientEvidence)
    }

    /// A stored verdict outside the three words is corruption. Reading it as
    /// `insufficient_evidence` would be an answer the reviewer never gave (§3.2).
    pub fn try_parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "confirmed" => Some(Self::Confirmed),
            "rejected" => Some(Self::Rejected),
            "insufficient_evidence" => Some(Self::InsufficientEvidence),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReviewDecision {
    pub id: i64,
    pub root_run_id: String,
    pub candidate_id: String,
    pub candidate_revision: i64,
    pub reviewer_run_id: String,
    pub verdict: ReviewVerdict,
    pub reason_codes: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub counter_evidence_refs: Vec<String>,
    pub missing_evidence: Vec<String>,
    pub confidence: f64,
    pub severity: String,
    pub created_at: String,
}

impl ReviewDecision {
    pub fn new(
        root_run_id: impl Into<String>,
        candidate_id: impl Into<String>,
        candidate_revision: i64,
        reviewer_run_id: impl Into<String>,
        verdict: ReviewVerdict,
    ) -> Self {
        Self {
            id: 0,
            root_run_id: root_run_id.into(),
            candidate_id: candidate_id.into(),
            candidate_revision,
            reviewer_run_id: reviewer_run_id.into(),
            verdict,
            reason_codes: Vec::new(),
            evidence_refs: Vec::new(),
            counter_evidence_refs: Vec::new(),
            missing_evidence: Vec::new(),
            confidence: 0.0,
            severity: String::new(),
            created_at: String::new(),
        }
    }

    fn content_key(&self) -> JsonValue {
        redact_json(&serde_json::json!({
            "verdict": self.verdict.as_str(),
            "reasonCodes": self.reason_codes,
            "evidenceRefs": self.evidence_refs,
            "counterEvidenceRefs": self.counter_evidence_refs,
            "missingEvidence": self.missing_evidence,
            "confidence": self.confidence,
            "severity": self.severity,
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewInsert {
    Inserted(i64),
    /// The identical decision was already stored.
    Existing(i64),
}

type StoredDecision = (ReviewDecision, String, String, String, String, String);

fn decision_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredDecision> {
    // Verdict word and the four reference lists come back as text; `decode_decision`
    // is the only place that turns them into values (§3.2, §3.3).
    let decision = ReviewDecision {
        id: row.get(0)?,
        root_run_id: row.get(1)?,
        candidate_id: row.get(2)?,
        candidate_revision: row.get(3)?,
        reviewer_run_id: row.get(4)?,
        verdict: ReviewVerdict::InsufficientEvidence,
        reason_codes: Vec::new(),
        evidence_refs: Vec::new(),
        counter_evidence_refs: Vec::new(),
        missing_evidence: Vec::new(),
        confidence: row.get(10)?,
        severity: row.get(11)?,
        created_at: row.get(12)?,
    };
    Ok((
        decision,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}

fn decode_decision(stored: StoredDecision) -> Result<ReviewDecision, String> {
    let (mut decision, verdict, reasons, refs, counter_refs, missing) = stored;
    let id = decision.id.to_string();
    decision.verdict = ReviewVerdict::try_parse(&verdict).ok_or_else(|| {
        format!("表 agent_review_decisions 的 verdict 不是已知结论（记录 {id}）：{verdict}")
    })?;
    decision.reason_codes =
        decode_json("reason_codes_json", "agent_review_decisions", &id, &reasons)?;
    decision.evidence_refs =
        decode_json("evidence_refs_json", "agent_review_decisions", &id, &refs)?;
    decision.counter_evidence_refs = decode_json(
        "counter_evidence_refs_json",
        "agent_review_decisions",
        &id,
        &counter_refs,
    )?;
    decision.missing_evidence = decode_json(
        "missing_evidence_json",
        "agent_review_decisions",
        &id,
        &missing,
    )?;
    Ok(decision)
}

/// §3.7: only an evidence reviewer may answer for a candidate, and only inside the
/// root it belongs to. Evidence judgment remains the independent reviewer's job.
fn require_reviewer(connection: &Connection, decision: &ReviewDecision) -> Result<(), String> {
    let reviewer: Option<(String, String)> = connection
        .query_row(
            "SELECT role,root_run_id FROM agent_runs WHERE id=?1",
            [&decision.reviewer_run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("无法检查 reviewer run：{error}"))?;
    let (role, reviewer_root) = reviewer.ok_or_else(|| {
        format!(
            "review 的 reviewer run 不存在：{}",
            decision.reviewer_run_id
        )
    })?;
    if decision.reviewer_run_id != decision.root_run_id && reviewer_root != decision.root_run_id {
        return Err(format!(
            "review 的 reviewer run 不属于 root {}：{}",
            decision.root_run_id, decision.reviewer_run_id
        ));
    }
    if AgentRole::parse(&role) != AgentRole::EvidenceReviewer {
        return Err(format!(
            "只有 evidence reviewer 能写 review 决定，当前 run 角色不是 evidence_reviewer：{}",
            decision.reviewer_run_id
        ));
    }
    let root_exists: Option<String> = connection
        .query_row(
            "SELECT id FROM agent_runs WHERE id=?1",
            [&decision.root_run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法检查 review root：{error}"))?;
    if root_exists.is_none() {
        return Err(format!(
            "review 的 root run 不存在：{}",
            decision.root_run_id
        ));
    }
    Ok(())
}

pub fn insert_review_decision(
    connection: &Connection,
    decision: &ReviewDecision,
) -> Result<ReviewInsert, String> {
    for (field, value) in [
        ("root_run_id", decision.root_run_id.as_str()),
        ("candidate_id", decision.candidate_id.as_str()),
        ("reviewer_run_id", decision.reviewer_run_id.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("review 决定缺少 {field}"));
        }
    }
    if !decision.confidence.is_finite() || !(0.0..=1.0).contains(&decision.confidence) {
        return Err(format!(
            "review 决定的 confidence 必须是 0.0..=1.0 的有限数：{}",
            decision.confidence
        ));
    }
    require_reviewer(connection, decision)?;
    let stored: Option<StoredDecision> = connection
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM agent_review_decisions WHERE candidate_id=?1 AND candidate_revision=?2 AND reviewer_run_id=?3"
            ),
            params![decision.candidate_id, decision.candidate_revision, decision.reviewer_run_id],
            decision_from_row,
        )
        .optional()
        .map_err(|error| format!("无法读取已有 review：{error}"))?;
    if let Some(found) = stored.map(decode_decision).transpose()? {
        // The same answer replays; a different one is a conflict, never an overwrite.
        if found.content_key() == decision.content_key() {
            return Ok(ReviewInsert::Existing(found.id));
        }
        return Err(format!(
            "同一 candidate/revision/reviewer 已存在不同的 review 决定，拒绝覆盖：{}",
            decision.candidate_id
        ));
    }
    let lists = |values: &Vec<String>| redact_json(&serde_json::json!(values)).to_string();
    connection
        .execute(
            "INSERT INTO agent_review_decisions(root_run_id,candidate_id,candidate_revision,reviewer_run_id,\
             verdict,reason_codes_json,evidence_refs_json,counter_evidence_refs_json,missing_evidence_json,\
             confidence,severity,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,datetime('now','localtime'))",
            params![
                decision.root_run_id,
                decision.candidate_id,
                decision.candidate_revision,
                decision.reviewer_run_id,
                decision.verdict.as_str(),
                lists(&decision.reason_codes),
                lists(&decision.evidence_refs),
                lists(&decision.counter_evidence_refs),
                lists(&decision.missing_evidence),
                decision.confidence,
                decision.severity
            ],
        )
        .map_err(|error| format!("无法写入 review 决定：{error}"))?;
    Ok(ReviewInsert::Inserted(connection.last_insert_rowid()))
}

pub fn list_review_decisions_for_candidate(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Vec<ReviewDecision>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {COLUMNS} FROM agent_review_decisions WHERE candidate_id=?1 ORDER BY candidate_revision,id"
        ))
        .map_err(|error| format!("无法准备 review 查询：{error}"))?;
    let rows = statement
        .query_map([candidate_id], decision_from_row)
        .map_err(|error| format!("无法读取 review 列表：{error}"))?;
    rows.map(|item| {
        item.map_err(|error| format!("无法解析 review 列表：{error}"))
            .and_then(decode_decision)
    })
    .collect()
}

/// Every decision stored for one root run, oldest first. The CI gate reads this and
/// nothing else: a candidate nobody reviewed stays a candidate (§10.4).
pub fn list_review_decisions(
    connection: &Connection,
    root_run_id: &str,
) -> Result<Vec<ReviewDecision>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {COLUMNS} FROM agent_review_decisions WHERE root_run_id=?1 ORDER BY candidate_revision,id"
        ))
        .map_err(|error| format!("无法准备 review 查询：{error}"))?;
    let rows = statement
        .query_map([root_run_id], decision_from_row)
        .map_err(|error| format!("无法读取 review 列表：{error}"))?;
    rows.map(|item| {
        item.map_err(|error| format!("无法解析 review 列表：{error}"))
            .and_then(decode_decision)
    })
    .collect()
}
