//! Immutable publication of an independently delivered source review. Consumers
//! must audit the complete receipt-bound set, not trust a row or verdict alone.
use super::{
    lease::CoordinatorLease,
    source,
    source_reviewer::{self, ReviewAudit},
};
use crate::artifact_import::canonical::sha256_hex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::source_review_contract::{CandidateDecision, CandidateIdentity, SourceVerdict};

/// Serialization is for display/export only. Deserializing a record does not
/// confer qualification: only read_audited can construct the opaque set below.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublishedDecision {
    schema_version: u32,
    id: String,
    scan_id: String,
    attempt_number: i64,
    root_run_id: String,
    reviewer_run_id: String,
    assignment_id: String,
    message_id: String,
    model_request_hash: String,
    model_response_hash: String,
    material_digest: String,
    decision: CandidateDecision,
}

/// A transaction-local projection of a complete, receipt-audited V3 set.
/// No Deserialize, public fields, or JSON constructor: a caller cannot turn
/// imported history or a model's own assertion into qualified source findings.
#[derive(Debug, PartialEq)]
pub(crate) struct SourceDecisionSet {
    scan_id: String,
    attempt_number: i64,
    root_run_id: String,
    material_digest: String,
    records: Vec<PublishedDecision>,
    findings: Vec<Value>,
}

impl SourceDecisionSet {
    pub(crate) fn decisions(&self) -> impl Iterator<Item = &CandidateDecision> {
        self.records.iter().map(|record| &record.decision)
    }

    pub(crate) fn confirmed_count(&self) -> usize {
        self.findings.len()
    }

    pub(crate) fn as_json(&self) -> Value {
        json!({"schemaVersion":1,"surface":"source","scanId":self.scan_id,
            "attemptNumber":self.attempt_number,"rootRunId":self.root_run_id,
            "materialDigest":self.material_digest,"independentCandidateReviewCompleted":true,
            "independentReviewCompleted":false,
            "counts":{"decisions":self.records.len(),"confirmed":self.confirmed_count(),
                "rejected":self.records.iter().filter(|r|r.decision.verdict==SourceVerdict::Rejected).count(),
                "insufficient":self.records.iter().filter(|r|r.decision.verdict==SourceVerdict::InsufficientEvidence).count()},
            "decisions":self.records,"findings":self.findings})
    }
}

/// Caller must retain its transaction through consumption and re-audit after
/// writes. Historical reads are allowed, but never imply permission to execute
/// or project an old attempt into the current scan's gate.
pub(crate) fn read_audited(
    db: &Connection,
    lease: &CoordinatorLease,
) -> Result<SourceDecisionSet, String> {
    if db.is_autocommit() {
        return Err("source_decision_consumer_transaction_required".into());
    }
    if !enabled(db, lease)? {
        return Err("source_decision_version_unqualified".into());
    }
    // Includes exact published-set equality, independent author, all four
    // prerequisite phases, selected frozen bytes, actual model receipt and ACK.
    let audit = source_reviewer::audit_delivery(db, lease)?;
    let records = expected(db, lease, &audit)?
        .into_iter()
        .map(|r| {
            serde_json::from_str::<PublishedDecision>(&r.json)
                .map_err(|_| "source_decision_record_invalid".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let candidates = audit.payload["sourceTask"]["reviewMaterial"]["material"]["candidates"]
        .as_array()
        .ok_or("source_decision_candidates_missing")?;
    let mut findings = Vec::new();
    for record in &records {
        let candidate = candidates
            .iter()
            .find(|c| {
                serde_json::from_value::<CandidateIdentity>(c["identity"].clone())
                    .ok()
                    .as_ref()
                    == Some(&record.decision.identity)
            })
            .ok_or("source_decision_candidate_missing")?;
        if sha256_hex(candidate.to_string().as_bytes()) != record.decision.candidate_digest {
            return Err("source_decision_candidate_changed".into());
        }
        if record.decision.verdict != SourceVerdict::Confirmed {
            continue;
        }
        let payload = match &record.decision.identity {
            CandidateIdentity::AnalyzerRevision { .. } => &candidate["envelope"]["payload"],
            CandidateIdentity::GraphCandidate { .. } => &candidate["payload"],
        };
        let text = |keys: &[&str]| {
            keys.iter()
                .find_map(|key| payload[*key].as_str())
                .unwrap_or("")
        };
        findings.push(json!({"id":record.id,"sourceDecisionId":record.id,"surface":"source",
            "scanId":record.scan_id,"attemptNumber":record.attempt_number,"rootRunId":record.root_run_id,
            "reviewerRunId":record.reviewer_run_id,"materialDigest":record.material_digest,
            "identity":record.decision.identity,"candidateDigest":record.decision.candidate_digest,
            "title":text(&["title","rule","rule_id"]),"path":text(&["path","repoPath"]),
            "line":payload["line"].as_u64().or_else(||payload.pointer("/region/startLine").and_then(Value::as_u64)),
            "cwe":text(&["cwe"]),"severity":record.decision.severity,"reviewState":"confirmed",
            "rationale":record.decision.rationale,"reasonCodes":record.decision.reason_codes,
            "evidenceRefs":record.decision.evidence_refs,"confidence":record.decision.confidence}));
    }
    Ok(SourceDecisionSet {
        scan_id: lease.scan_id.clone(),
        attempt_number: lease.attempt_number,
        root_run_id: lease.root_run_id.clone(),
        material_digest: audit.payload["result"]["materialDigest"]
            .as_str()
            .ok_or("source_decision_material_missing")?
            .into(),
        records,
        findings,
    })
}

#[derive(Debug, PartialEq, Eq)]
struct Record {
    id: String,
    scan_id: String,
    attempt: i64,
    identity: String,
    material: String,
    json: String,
    digest: String,
}

fn enabled(db: &Connection, lease: &CoordinatorLease) -> Result<bool, String> {
    let text: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let plan: Value = serde_json::from_str(&text).map_err(|_| "source_decision_plan_invalid")?;
    Ok(source::SourcePhaseContract::from_plan(&plan)?.candidate_decisions)
}

fn expected(
    db: &Connection,
    lease: &CoordinatorLease,
    audit: &ReviewAudit,
) -> Result<Vec<Record>, String> {
    if !enabled(db, lease)? {
        return Ok(Vec::new());
    }
    let (request, response): (String, String) = db
        .query_row(
            "SELECT request_hash,response_hash FROM agent_specialist_calls WHERE assignment_id=?1
         AND child_run_id=?2 AND root_run_id=?3 AND role='evidence_reviewer' AND state='received'",
            params![
                audit.child.assignment_id,
                audit.child.run_id,
                lease.root_run_id
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "source_decision_receipt_missing")?;
    let material = audit.payload["result"]["materialDigest"]
        .as_str()
        .ok_or("source_decision_material_missing")?;
    let decisions = audit.payload["result"]["decisions"]
        .as_array()
        .ok_or("source_decision_set_missing")?;
    if decisions.is_empty() {
        return Err("source_decision_set_empty".into());
    }
    let mut records = decisions.iter().map(|decision| {
        let identity = decision["identity"].to_string();
        let id = format!("source-decision-{}", sha256_hex(json!([
            lease.scan_id,lease.attempt_number,lease.root_run_id,material,decision["identity"]
        ]).to_string().as_bytes()));
        let record = json!({"schemaVersion":1,"id":id,"scanId":lease.scan_id,"attemptNumber":lease.attempt_number,
            "rootRunId":lease.root_run_id,"reviewerRunId":audit.child.run_id,"assignmentId":audit.child.assignment_id,
            "messageId":audit.message_id,"modelRequestHash":request,"modelResponseHash":response,
            "materialDigest":material,"decision":decision});
        let text = record.to_string();
        Record { id, scan_id:lease.scan_id.clone(), attempt:lease.attempt_number, identity,
            material:material.into(), digest:sha256_hex(text.as_bytes()), json:text }
    }).collect::<Vec<_>>();
    records.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(records)
}

fn stored(db: &Connection, lease: &CoordinatorLease) -> Result<Vec<Record>, String> {
    db.prepare("SELECT id,scan_id,attempt_number,candidate_identity_json,material_digest,record_json,record_digest
        FROM agent_source_review_decisions WHERE root_run_id=?1 ORDER BY id")
        .map_err(|e|e.to_string())?.query_map([&lease.root_run_id], |r| Ok(Record {
            id:r.get(0)?,scan_id:r.get(1)?,attempt:r.get(2)?,identity:r.get(3)?,material:r.get(4)?,json:r.get(5)?,digest:r.get(6)?,
        })).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())
}

/// This is intentionally only called by the receipt audit. Its input has been
/// rebuilt from the journal, ACK, frozen material and terminal child, not JSON
/// submitted by an API client. Historical v1/v2 must have no canonical rows.
pub(super) fn audit_records(
    db: &Connection,
    lease: &CoordinatorLease,
    audit: &ReviewAudit,
) -> Result<Vec<String>, String> {
    let records = expected(db, lease, audit)?;
    if stored(db, lease)? != records {
        return Err("source_decision_publication_mismatch".into());
    }
    Ok(records.into_iter().map(|record| record.id).collect())
}

/// Publish all candidates in the caller's delivery transaction. Never repair a
/// partial/damaged existing set, and never publish from a caller-forged audit.
pub(crate) fn publish(db: &Connection, lease: &CoordinatorLease) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("source_decision_transaction_required".into());
    }
    let audit = source_reviewer::audit_delivery_receipt(db, lease)?;
    let records = expected(db, lease, &audit)?;
    if !records.is_empty() && stored(db, lease)?.is_empty() {
        super::lease::require_active_attempt(db, &lease.scan_id, lease.attempt_number)?;
        super::lease::validate_coordinator_lease(db, lease)?;
        let running: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1
            AND status='running' AND cancel_requested_at='' AND terminal_state='' AND finished_at='')",
            [&lease.root_run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
        if !running {
            return Err("source_decision_publication_requires_running_root".into());
        }
        for record in &records {
            let changed = db.execute("INSERT INTO agent_source_review_decisions
                (id,root_run_id,scan_id,attempt_number,candidate_identity_json,material_digest,record_json,record_digest)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![record.id,lease.root_run_id,record.scan_id,record.attempt,
                    record.identity,record.material,record.json,record.digest]).map_err(|_|"source_decision_insert_failed")?;
            if changed != 1 {
                return Err("source_decision_insert_unconfirmed".into());
            }
        }
    }
    // Rebuild proof after all writes: late triggers cannot invalidate the ACK,
    // material, receipt or rowset while leaving publication apparently valid.
    let after = source_reviewer::audit_delivery_receipt(db, lease)?;
    if after != audit {
        return Err("source_decision_receipt_changed".into());
    }
    audit_records(db, lease, &after)?;
    Ok(())
}
