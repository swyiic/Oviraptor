//! Receipt-bound source review material. Preparing material is NOT a review,
//! permission grant, or CI verdict. Analyzer hashes and graph revision numbers
//! deliberately occupy different identity namespaces.
use super::{lease::CoordinatorLease, source, source_rounds::SourceRoundAudit};
use crate::{
    agent_runtime::{
        evidence_graph::{
            contract::{EvidenceNodeKind, EvidenceProvenance},
            store::{evidence_natural_key, list_effective_evidence_nodes_at_revision},
        },
        secrets::redact_json,
        store::stable_hash,
    },
    artifact_import::canonical::sha256_hex,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

/// Called inside the same transaction as the completed phase/mailbox/usage
/// proof. The caller must establish those completion invariants as well; the
/// opaque audits additionally prove the actual tool outputs used here.
pub(crate) fn capture(
    db: &Connection,
    lease: &CoordinatorLease,
    audits: &[SourceRoundAudit],
) -> Result<Value, String> {
    if db.is_autocommit() {
        return Err("source_review_material_transaction_required".into());
    }
    source::validate_surface_role(
        db,
        lease,
        crate::agent_runtime::contract::AgentRole::SourceAnalyst,
    )?;
    let (view, results, plan_hash) =
        source::historical_materials(db, &lease.scan_id, lease.attempt_number)?;
    let bound: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND root_run_id=id
         AND parent_run_id IS NULL AND assignment_id='' AND role='coordinator'
         AND backend='native' AND orchestration_policy='multi' AND scan_id=?2
         AND attempt_number=?3 AND target_url=?4 AND plan_hash=?5 AND evidence_hash=?6)",
            params![
                lease.root_run_id,
                lease.scan_id,
                lease.attempt_number,
                source::target_key(&view),
                plan_hash,
                results.digest()
            ],
            |r| r.get(0),
        )
        .map_err(|_| "source_review_material_root_unavailable")?;
    if !bound || lease.target_key != source::target_key(&view) {
        return Err("source_review_material_root_mismatch".into());
    }
    let mut candidates = Vec::new();
    for candidate in results.candidates(db)? {
        let receipt = &candidate.receipt.record;
        candidates.push(json!({
            "identity":{"kind":"analyzer_revision","logicalKey":receipt.logical_key,
                "revisionHash":receipt.revision_hash,"revisionId":receipt.revision_id},
            "envelopeSha256":receipt.envelope_sha256,"envelope":candidate.envelope,
            "acceptedSources":candidate.sources,
        }));
    }
    let revision: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(revision),0) FROM agent_evidence_revisions WHERE root_run_id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .map_err(|_| "source_review_material_revision_unavailable")?;
    let mut tools = audits
        .iter()
        .flat_map(|a| a.tool_evidence().iter().cloned())
        .collect::<Vec<_>>();
    tools.sort_by_key(Value::to_string);
    let mut tool_keys = std::collections::BTreeSet::new();
    for tool in &tools {
        if tool["rootRunId"] != lease.root_run_id
            || !tool_keys.insert((
                tool["assignmentId"].to_string(),
                tool["roundNumber"].to_string(),
                tool["callIndex"].to_string(),
            ))
        {
            return Err("source_review_material_tool_binding_invalid".into());
        }
    }
    let nodes = list_effective_evidence_nodes_at_revision(db, &lease.root_run_id, revision)?;
    // A successfully committed submit may not disappear from review merely
    // because its graph row or revision registration was subsequently lost.
    // SourceBroker v1 does not produce supersessions; supporting them requires
    // explicit, receipt-bound lineage rather than silently dropping a submit.
    for tool in tools
        .iter()
        .filter(|tool| tool["call"]["name"] == "evidence.submit_candidate")
    {
        if !nodes.iter().any(|node| {
            node.kind == EvidenceNodeKind::CandidateFinding
                && tool["output"]["id"] == node.id
                && tool["authorRunId"] == node.created_by_run_id
        }) {
            return Err("source_review_material_submitted_candidate_missing".into());
        }
    }
    for node in nodes
        .iter()
        .filter(|n| n.kind == EvidenceNodeKind::CandidateFinding)
    {
        if node.provenance != EvidenceProvenance::SourceDerived
            || node.payload["reviewState"] != "candidate"
        {
            return Err("source_review_material_candidate_provenance_invalid".into());
        }
        let receipts = tools
            .iter()
            .filter(|tool| {
                tool["call"]["name"] == "evidence.submit_candidate"
                    && tool["output"]["id"] == node.id
                    && tool["authorRunId"] == node.created_by_run_id
            })
            .collect::<Vec<_>>();
        if receipts.is_empty() {
            return Err("source_review_material_candidate_receipt_missing".into());
        }
        for receipt in &receipts {
            let args = &receipt["call"]["arguments"];
            let path = args["path"]
                .as_str()
                .ok_or("source_review_material_location_invalid")?;
            let file = view
                .manifest
                .files
                .iter()
                .find(|file| file.path == path)
                .ok_or("source_review_material_location_outside_view")?;
            let line = args["line"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or("source_review_material_location_invalid")?;
            let expected = redact_json(&json!({
                "title":args["title"].as_str().unwrap_or("").trim(),
                "rationale":args["rationale"].as_str().unwrap_or("").trim(),
                "severity":args["severity"].as_str().unwrap_or("informational"),
                "rule":args["rule"].as_str().unwrap_or("model-suspicion"),
                "cwe":args["cwe"].as_str().unwrap_or(""),"path":path,"line":line,
                "contentHash":file.content_hash,"analysisManifestDigest":view.manifest.digest(),
                "scanId":lease.scan_id,"attemptNumber":lease.attempt_number,
                "sourceAnalyzer":args["sourceAnalyzer"].as_str().unwrap_or("model"),
                "treeHash":view.manifest.source_tree_hash,"reviewState":"candidate",
            }));
            let identity = crate::native_pipeline::tools::source_candidate_identity(
                &lease.root_run_id,
                &lease.scan_id,
                lease.attempt_number,
                &view.manifest.digest(),
                args.as_object()
                    .ok_or("source_review_material_arguments_invalid")?,
            );
            let identity_hash = stable_hash(&identity);
            let author: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
                 WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND a.role='source_analyst'
                 AND a.state='completed' AND a.evidence_revision=?4 AND a.target_key=?5
                 AND json_extract(a.task_slice_json,'$.phase')='source_tools'
                 AND r.assignment_id=a.id AND r.root_run_id=?3 AND r.parent_run_id=?3
                 AND r.scan_id=?6 AND r.attempt_number=?7 AND r.role=a.role
                 AND r.status='terminal' AND r.terminal_state='completed')",
                params![receipt["assignmentId"].as_str(), node.created_by_run_id, lease.root_run_id,
                    node.revision, lease.target_key, lease.scan_id, lease.attempt_number], |r| r.get(0),
            ).map_err(|_| "source_review_material_author_unavailable")?;
            if !author
                || node.payload != expected
                || !node.supersedes_id.is_empty()
                || node.id != format!("cand-{}", &identity_hash[..16])
                || node.natural_key_hash
                    != evidence_natural_key(
                        &lease.root_run_id,
                        EvidenceNodeKind::CandidateFinding,
                        &identity,
                    )
                || node.artifact_refs != vec![view.manifest.source_tree_hash.clone()]
                || receipt["output"]["reviewState"] != "candidate"
                || receipt["output"]["confirmable"] != false
            {
                return Err("source_review_material_candidate_binding_invalid".into());
            }
        }
        candidates.push(json!({
            "identity":{"kind":"graph_candidate","nodeId":node.id,"revision":node.revision},
            "authorRunId":node.created_by_run_id,"payload":node.payload,
            "toolEvidenceHashes":receipts.iter().map(|r|sha256_hex(r.to_string().as_bytes())).collect::<Vec<_>>(),
        }));
    }
    candidates.sort_by_key(|c| c["identity"].to_string());
    let material = redact_json(&json!({
        "schemaVersion":1,"surface":"source","rootRunId":lease.root_run_id,
        "scanId":lease.scan_id,"attemptNumber":lease.attempt_number,"target":lease.target_key,
        "analysisDigest":view.manifest.digest(),"analysisResultsDigest":results.digest(),
        "sourcePlanHash":plan_hash,"graphRevision":revision,"selectedFiles":view.manifest.files,
        "analyzerRuns":results.runs,"gaps":results.gaps,"candidates":candidates,"toolEvidence":tools,
        "independentReviewCompleted":false,"targetRequestsGranted":0,"hostActionsGranted":0,
    }));
    let decision_contract = super::source_review_contract::SourceReviewContract::from_material(
        &material,
        &lease.root_run_id,
    )?
    .map(|contract| contract.as_json());
    Ok(
        json!({"digest":sha256_hex(material.to_string().as_bytes()),"material":material,
        "decisionContract":decision_contract}),
    )
}
