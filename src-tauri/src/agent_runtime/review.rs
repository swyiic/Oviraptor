//! Evidence Reviewer verdict contract and storage (Stage 1A §4.6, §5.5).
//!
//! The reviewer is the only role allowed to call something a finding, and Stage 1A
//! only stores its decision: saving a `confirmed` review writes no Sentinel finding
//! and no `finding` node. A replay of the same decision is idempotent, while a
//! different answer for the same candidate and revision is refused rather than
//! silently overwritten.

// Stage 1A declares the contract and its storage only; the scheduler, the child
// runs and the review gate that consume them land in the next stages. Every item
// here is exercised by the Stage 1A tests, so the reachability warning is expected.
#![allow(dead_code)]
use crate::agent_runtime::secrets::redact_json;
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
        match value.trim().to_ascii_lowercase().as_str() {
            "confirmed" => Self::Confirmed,
            "rejected" => Self::Rejected,
            _ => Self::InsufficientEvidence,
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

fn decision_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewDecision> {
    let list = |index: usize| -> Vec<String> {
        serde_json::from_str(&row.get::<_, String>(index).unwrap_or_default()).unwrap_or_default()
    };
    Ok(ReviewDecision {
        id: row.get(0)?,
        root_run_id: row.get(1)?,
        candidate_id: row.get(2)?,
        candidate_revision: row.get(3)?,
        reviewer_run_id: row.get(4)?,
        verdict: ReviewVerdict::parse(&row.get::<_, String>(5)?),
        reason_codes: list(6),
        evidence_refs: list(7),
        counter_evidence_refs: list(8),
        missing_evidence: list(9),
        confidence: row.get(10)?,
        severity: row.get(11)?,
        created_at: row.get(12)?,
    })
}

pub fn insert_review_decision(
    connection: &Connection,
    decision: &ReviewDecision,
) -> Result<ReviewInsert, String> {
    if decision.candidate_id.trim().is_empty() || decision.reviewer_run_id.trim().is_empty() {
        return Err("review 决定缺少 candidate 或 reviewer".to_string());
    }
    let stored: Option<ReviewDecision> = connection
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM agent_review_decisions WHERE candidate_id=?1 AND candidate_revision=?2 AND reviewer_run_id=?3"
            ),
            params![decision.candidate_id, decision.candidate_revision, decision.reviewer_run_id],
            decision_from_row,
        )
        .optional()
        .map_err(|error| format!("无法读取已有 review：{error}"))?;
    if let Some(found) = stored {
        let incoming = decision.content_key();
        if found.content_key() == incoming {
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
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析 review 列表：{error}"))
}
