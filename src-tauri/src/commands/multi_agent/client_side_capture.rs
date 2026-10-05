// Production producer: only previously captured, actual Broker HTTP graph facts.
fn client_side_capture_source(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    id: &str,
    target_dir: &Path,
) -> Result<crate::agent_runtime::multi_agent::client_side::Observation, String> {
    use crate::agent_runtime::{
        evidence_graph::{
            contract::{EvidenceNodeKind, EvidenceProvenance},
            store::load_evidence_node,
        },
        multi_agent::client_side::{Artifact, Observation},
    };
    let node = load_evidence_node(db, id)?.ok_or("client_side_source_fact_missing")?;
    if node.root_run_id != lease.root_run_id
        || node.kind != EvidenceNodeKind::RequestRecord
        || node.provenance != EvidenceProvenance::Observed
        || node.revision <= 0
        || !verified_web_http_review_fact(target_dir, &node.artifact_refs, &node.payload)
    {
        return Err("client_side_original_http_fact_invalid".into());
    }
    let [path] = node.artifact_refs.as_slice() else {
        return Err("client_side_original_http_fact_invalid".into());
    };
    let name = path
        .strip_prefix("agent-http/")
        .ok_or("client_side_original_http_path_invalid")?;
    let directory = open_agent_artifact_directory(&target_dir.join(AGENT_HTTP_DIRECTORY))
        .ok_or("client_side_original_artifact_root_invalid")?;
    let (bytes, _) = read_agent_artifact_file(&directory, name, 1_048_576)
        .ok_or("client_side_original_record_missing")?;
    let record: JsonValue =
        serde_json::from_slice(&bytes).map_err(|_| "client_side_original_record_invalid")?;
    // Native HTTP complete names can prove omission; its version-header
    // allowlist cannot. CSP present but not captured remains an explicit gap.
    let (kind, value) = match client_side_local_value("http_csp_absent", &bytes) {
        Ok(value) => ("http_csp_absent", value),
        Err(_) => match client_side_local_value("csp_header", &bytes) {
            Ok(value) => ("csp_header", value),
            Err(_) => return Err("client_side_configuration_value_not_captured".into()),
        },
    };
    let identity = record
        .pointer("/response/identity")
        .and_then(JsonValue::as_str)
        .ok_or("client_side_original_identity_missing")?;
    let status = record
        .pointer("/response/status")
        .and_then(JsonValue::as_i64)
        .ok_or("client_side_original_status_missing")?;
    let source = client_side_source_proof(db, lease, &node, identity, status)?;
    Ok(Observation {
        id: node.id,
        kind: kind.into(),
        classification: "source-derived".into(),
        artifact: Artifact {
            path: path.clone(),
            content_hash: crate::agent_runtime::store::artifact_id(&bytes),
        },
        value,
        source_proof: Some(source),
    })
}
fn client_side_frozen_http_task(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    target_dir: &Path,
) -> Result<Option<JsonValue>, String> {
    use crate::agent_runtime::multi_agent::client_side::Task;
    let ids: Vec<String> = db
        .prepare(
            "SELECT id FROM agent_evidence_nodes WHERE root_run_id=?1 AND kind='request_record'
        AND provenance='observed' ORDER BY revision DESC,id LIMIT 4",
        )
        .map_err(|e| e.to_string())?
        .query_map([&lease.root_run_id], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut observations = Vec::new();
    for id in ids {
        match client_side_capture_source(db, lease, &id, target_dir) {
            Ok(o) => observations.push(o),
            Err(code) if code == "client_side_configuration_value_not_captured" => continue,
            Err(code) => return Err(code),
        }
    }
    if observations.is_empty() {
        return Ok(None);
    }
    let task = Task {
        schema_version: 1,
        phase: "client_side_readonly".into(),
        root_run_id: lease.root_run_id.clone(),
        scan_id: lease.scan_id.clone(),
        attempt_number: lease.attempt_number,
        target: lease.target_key.clone(),
        coordinator_epoch: lease.lease_epoch,
        coordinator_fence: lease.fencing_token.clone(),
        evidence_revision: 1,
        target_requests_granted: 0,
        browser_actions_granted: 0,
        tools_granted: Vec::new(),
        observations,
    };
    let value = serde_json::to_value(task).map_err(|e| e.to_string())?;
    Task::from_value(&value, lease, 1)?;
    Ok(Some(value))
}
