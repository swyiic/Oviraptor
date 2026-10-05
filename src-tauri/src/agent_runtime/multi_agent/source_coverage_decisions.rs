//! Immutable coverage publication, derived exclusively from a delivered receipt.
use super::{
    lease::CoordinatorLease,
    source_coverage_reviewer::{self, ReviewAudit},
};
use crate::artifact_import::canonical::sha256_hex;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

#[derive(Debug, PartialEq, Eq)]
struct Record {
    id: String,
    scan_id: String,
    attempt: i64,
    material: String,
    json: String,
    digest: String,
}

fn expected(
    db: &Connection,
    lease: &CoordinatorLease,
    audit: &ReviewAudit,
) -> Result<Record, String> {
    if !source_coverage_reviewer::enabled(db, lease)? {
        return Err("source_coverage_decision_version_unqualified".into());
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
        .map_err(|_| "source_coverage_decision_receipt_missing")?;
    let material = audit.payload["result"]["materialDigest"]
        .as_str()
        .ok_or("source_coverage_decision_material_missing")?;
    let id = format!(
        "source-coverage-decision-{}",
        sha256_hex(
            json!([
                lease.scan_id,
                lease.attempt_number,
                lease.root_run_id,
                material
            ])
            .to_string()
            .as_bytes()
        )
    );
    let record = json!({"schemaVersion":1,"id":id,"scanId":lease.scan_id,
        "attemptNumber":lease.attempt_number,"rootRunId":lease.root_run_id,
        "reviewerRunId":audit.child.run_id,"assignmentId":audit.child.assignment_id,
        "messageId":audit.message_id,"modelRequestHash":request,"modelResponseHash":response,
        "materialDigest":material,"decision":audit.payload["result"]});
    let text = record.to_string();
    Ok(Record {
        id,
        scan_id: lease.scan_id.clone(),
        attempt: lease.attempt_number,
        material: material.into(),
        digest: sha256_hex(text.as_bytes()),
        json: text,
    })
}

fn stored(db: &Connection, lease: &CoordinatorLease) -> Result<Vec<Record>, String> {
    db.prepare(
        "SELECT id,scan_id,attempt_number,material_digest,record_json,record_digest
        FROM agent_source_coverage_decisions WHERE root_run_id=?1 ORDER BY id",
    )
    .map_err(|e| e.to_string())?
    .query_map([&lease.root_run_id], |r| {
        Ok(Record {
            id: r.get(0)?,
            scan_id: r.get(1)?,
            attempt: r.get(2)?,
            material: r.get(3)?,
            json: r.get(4)?,
            digest: r.get(5)?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

pub(super) fn audit_records(
    db: &Connection,
    lease: &CoordinatorLease,
    audit: &ReviewAudit,
) -> Result<Vec<String>, String> {
    let record = expected(db, lease, audit)?;
    let records = stored(db, lease)?;
    if records.len() != 1 || records[0] != record {
        return Err("source_coverage_decision_publication_mismatch".into());
    }
    Ok(vec![record.id])
}

/// Must share the transaction that delivered/ACKed the response and settled usage.
/// Never accept a caller-supplied JSON verdict or repair an existing publication.
pub(crate) fn publish(db: &Connection, lease: &CoordinatorLease) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("source_coverage_decision_transaction_required".into());
    }
    let audit = source_coverage_reviewer::audit_delivery_receipt(db, lease)?;
    let record = expected(db, lease, &audit)?;
    if stored(db, lease)?.is_empty() {
        super::lease::require_active_attempt(db, &lease.scan_id, lease.attempt_number)?;
        super::lease::validate_coordinator_lease(db, lease)?;
        let running: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1
            AND status='running' AND cancel_requested_at='' AND terminal_state='' AND finished_at='')",
            [&lease.root_run_id], |r| r.get(0)).map_err(|e|e.to_string())?;
        if !running {
            return Err("source_coverage_decision_publication_requires_running_root".into());
        }
        let changed = db
            .execute(
                "INSERT INTO agent_source_coverage_decisions
            (id,root_run_id,scan_id,attempt_number,material_digest,record_json,record_digest)
            VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    record.id,
                    lease.root_run_id,
                    record.scan_id,
                    record.attempt,
                    record.material,
                    record.json,
                    record.digest
                ],
            )
            .map_err(|_| "source_coverage_decision_insert_failed")?;
        if changed != 1 {
            return Err("source_coverage_decision_insert_unconfirmed".into());
        }
    }
    let after = source_coverage_reviewer::audit_delivery_receipt(db, lease)?;
    if after != audit {
        return Err("source_coverage_decision_receipt_changed".into());
    }
    audit_records(db, lease, &after)?;
    Ok(())
}

/// Opaque, audited consumer value: no Deserialize or import constructor.
#[derive(Debug, PartialEq)]
pub(crate) struct CoverageDecision {
    value: Value,
}

impl CoverageDecision {
    pub(crate) fn as_json(&self) -> Value {
        self.value.clone()
    }
}

pub(crate) fn read_audited(
    db: &Connection,
    lease: &CoordinatorLease,
) -> Result<CoverageDecision, String> {
    if db.is_autocommit() {
        return Err("source_coverage_decision_consumer_transaction_required".into());
    }
    let audit = source_coverage_reviewer::audit_delivery(db, lease)?;
    let record = expected(db, lease, &audit)?;
    Ok(CoverageDecision {
        value: serde_json::from_str(&record.json)
            .map_err(|_| "source_coverage_decision_record_invalid")?,
    })
}
