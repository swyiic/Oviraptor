// Read the canonical decision ledger, never report JSON or importer projections.
// A historical read confers neither execution authority nor coverage completion.
#[tauri::command]
pub async fn get_native_source_findings(
    state: State<'_, AppState>, scan_id: String, attempt_number: i64,
    offset: Option<i64>, limit: Option<i64>,
) -> Result<JsonValue, String> {
    let database = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = db::open(&database)?;
        native_source_findings(&connection, &scan_id, attempt_number, offset.unwrap_or(0), limit.unwrap_or(50))
    }).await.map_err(|_| "source_findings_reader_failed".to_string())?
}

fn native_source_findings(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64, offset: i64, limit: i64,
) -> Result<JsonValue, String> {
    if attempt < 1 || offset < 0 || !(1..=100).contains(&limit) {
        return Err("invalid_source_findings_page".into());
    }
    let mut response = native_source_findings_snapshot(connection, scan_id, attempt)?;
    response["offset"] = json!(offset);
    if response["status"] == "audited" {
        let findings = response["findings"].as_array().ok_or("source_findings_projection_invalid")?;
        let start = usize::try_from(offset).map_err(|_| "invalid_source_findings_page")?.min(findings.len());
        let end = start.saturating_add(limit as usize).min(findings.len());
        let next = if end < findings.len() { json!(end) } else { JsonValue::Null };
        let page = json!(findings[start..end]);
        response["nextOffset"] = next;
        response["findings"] = page;
    }
    Ok(response)
}

enum SourceFindingsAudit {
    NotAvailable,
    Unverified(Option<String>),
    Audited {
        lease: Box<crate::agent_runtime::multi_agent::lease::CoordinatorLease>,
        set: crate::agent_runtime::multi_agent::source_review_projection::SourceReviewProjection,
    },
}

// Shared eligibility and whole-set audit for detail, export and overview. The
// opaque decision set cannot be reconstructed from imported/display JSON.
fn native_source_findings_audit(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
) -> Result<SourceFindingsAudit, String> {
    use crate::agent_runtime::multi_agent::{lease::CoordinatorLease, source_review_projection};
    if attempt < 1 { return Err("invalid_source_findings_page".into()); }
    if connection.is_autocommit() { return Err("source_findings_transaction_required".into()); }
    let tx = connection;
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts a
        JOIN sentinel_scans s ON s.id=a.scan_id WHERE a.scan_id=?1 AND a.attempt_number=?2
        AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id))",
        params![scan_id,attempt], |r| r.get(0)).map_err(|_| "source_findings_attempt_lookup_failed")?;
    if !exists { return Err("attempt_not_found".into()); }
    // Include ledger/assignment evidence even if the root's surface marker has
    // been damaged. Multiple roots are an ambiguity, never pick LIMIT 1.
    let mut statement = tx.prepare("SELECT r.id,r.target_url FROM agent_runs r
        WHERE r.scan_id=?1 AND r.attempt_number=?2 AND r.parent_run_id IS NULL
        AND (r.target_url LIKE 'source:%'
          OR EXISTS(SELECT 1 FROM agent_source_review_decisions d WHERE d.root_run_id=r.id)
          OR EXISTS(SELECT 1 FROM agent_source_coverage_decisions d WHERE d.root_run_id=r.id)
          OR EXISTS(SELECT 1 FROM agent_assignments a WHERE a.coordinator_run_id=r.id AND a.role='repo_mapper'))
        ORDER BY r.id LIMIT 2").map_err(|_| "source_findings_root_lookup_failed")?;
    let roots = statement.query_map(params![scan_id,attempt], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))
        .map_err(|_| "source_findings_root_lookup_failed")?.collect::<Result<Vec<_>,_>>()
        .map_err(|_| "source_findings_root_lookup_failed")?;
    if roots.is_empty() { return Ok(SourceFindingsAudit::NotAvailable); }
    if roots.len() != 1 { return Ok(SourceFindingsAudit::Unverified(None)); }
    let (root, target) = &roots[0];
    let lease = tx.query_row("SELECT lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases
        WHERE scan_id=?1 AND attempt_number=?2 AND root_run_id=?3 AND target_key=?4",
        params![scan_id,attempt,root,target], |r| Ok(CoordinatorLease {
            scan_id:scan_id.into(),attempt_number:attempt,target_key:target.clone(),root_run_id:root.clone(),
            lease_epoch:r.get(0)?,fencing_token:r.get(1)?,lease_expires_at:r.get(2)?,
        })).optional().map_err(|_| "source_findings_lease_lookup_failed")?;
    let Some(lease) = lease else { return Ok(SourceFindingsAudit::Unverified(Some(root.clone()))) };
    // Audit the whole set before paging. A corrupt decision outside this page
    // must invalidate it too. Never expose unverified titles or raw errors.
    let Ok(set) = source_review_projection::read_audited(tx, &lease) else {
        return Ok(SourceFindingsAudit::Unverified(Some(root.clone())));
    };
    Ok(SourceFindingsAudit::Audited { lease: Box::new(lease), set })
}

// Exporting must not loop through independently audited pages or trust a UI
// projection. Retain the read transaction through projection consumption.
fn native_source_findings_snapshot(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
) -> Result<JsonValue, String> {
    let snapshot = if connection.is_autocommit() {
        Some(rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Deferred)
            .map_err(|_| "source_findings_snapshot_failed")?)
    } else { None };
    let tx = snapshot.as_deref().unwrap_or(connection);
    let audit = native_source_findings_audit(tx, scan_id, attempt)?;
    native_source_findings_projection(tx, audit, scan_id, attempt)
}

fn native_source_findings_projection(
    connection: &rusqlite::Connection, audit: SourceFindingsAudit, scan_id: &str, attempt: i64,
) -> Result<JsonValue, String> {
    if connection.is_autocommit() {return Err("source_findings_transaction_required".into());}
    let mut response = json!({"schemaVersion":1,"scanId":scan_id,"attemptNumber":attempt,
        "status":"not_available","rootRunId":null,"materialDigest":null,"counts":null,
        "independentCandidateReviewCompleted":false,"independentReviewCompleted":false,
        "offset":0,"nextOffset":null,"coverage":null,"findings":[]});
    let set = match audit {
        SourceFindingsAudit::NotAvailable => return Ok(response),
        SourceFindingsAudit::Unverified(root) => {
            response["status"] = json!("unverified");
            response["rootRunId"] = json!(root);
            return Ok(response);
        }
        SourceFindingsAudit::Audited { lease, set } => {
            response["coverage"] = set.coverage_summary();
            response["rootRunId"] = json!(lease.root_run_id);
            set
        }
    };
    let projection = set.as_json();
    let findings = projection["findings"].as_array().ok_or("source_findings_projection_invalid")?;
    response["status"] = json!("audited");
    response["materialDigest"] = projection["materialDigest"].clone();
    response["counts"] = projection["counts"].clone();
    response["independentCandidateReviewCompleted"] = projection["independentCandidateReviewCompleted"].clone();
    response["independentReviewCompleted"] = projection["independentReviewCompleted"].clone();
    response["candidateReviewStatus"] = projection["candidateReviewStatus"].clone();
    response["materialSubject"] = projection["materialSubject"].clone();
    response["findings"] = json!(findings.iter().map(|finding| {
        let mut row = finding.clone();
        // Redact human-readable strings only; don't rewrite opaque provenance IDs.
        for key in ["title","path","cwe","rationale"] {
            if let Some(text) = row[key].as_str() {
                row[key] = json!(crate::agent_runtime::secrets::redact_text_with(text,None));
            }
        }
        row
    }).collect::<Vec<_>>());
    Ok(response)
}

#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeSourceExportFormat {
    #[default]
    Json,
    Sarif,
    Bundle,
}

#[tauri::command]
pub async fn export_native_source_findings(
    state: State<'_, AppState>, scan_id: String, attempt_number: i64,
    format: Option<NativeSourceExportFormat>,
) -> Result<String, String> {
    let database = state.db_path.clone();
    let directory = state.export_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = db::open(&database)?;
        match format.unwrap_or_default() {
            NativeSourceExportFormat::Json => write_native_source_findings_export(&connection, &directory, &scan_id, attempt_number),
            NativeSourceExportFormat::Sarif => write_native_source_findings_export_as(&connection, &directory, &scan_id, attempt_number, NativeSourceExportFormat::Sarif),
            NativeSourceExportFormat::Bundle => write_native_source_findings_export_as(&connection, &directory, &scan_id, attempt_number, NativeSourceExportFormat::Bundle),
        }
    }).await.map_err(|_| "source_findings_export_failed".to_string())?
}

fn write_native_source_findings_export(
    connection: &rusqlite::Connection, directory: &Path, scan_id: &str, attempt: i64,
) -> Result<String, String> {
    write_native_source_findings_export_as(connection, directory, scan_id, attempt, NativeSourceExportFormat::Json)
}

fn write_native_source_findings_export_as(
    connection: &rusqlite::Connection, directory: &Path, scan_id: &str, attempt: i64,
    format: NativeSourceExportFormat,
) -> Result<String, String> {
    let review = native_source_findings_export_snapshot(connection, scan_id, attempt)?;
    // This is a report, not a transferable execution/review grant. Importers
    // cannot recreate canonical decisions or Reviewer receipts from this JSON.
    let bundle = json!({"format":"oviraptor-source-review-v1", "schemaVersion":1,
        "exportedAt":chrono::Utc::now().to_rfc3339(),
        "qualification":"verified_at_export", "executionEligible":false,
        "coverageReviewCompleted":review["independentReviewCompleted"], "review":review});
    let (kind, report) = match format {
        NativeSourceExportFormat::Json => (crate::snapshot_export::Kind::SourceReview, bundle),
        NativeSourceExportFormat::Sarif => (crate::snapshot_export::Kind::SourceReviewSarif, native_source_review_sarif(&bundle)?),
        NativeSourceExportFormat::Bundle => {
            let sarif = native_source_review_sarif(&bundle)?;
            let container = crate::artifact_import::report_bundle::build(bundle, sarif)?;
            let bytes = serde_json::to_vec_pretty(&container).map_err(|_| "source_findings_export_encoding_failed")?;
            let limits = crate::artifact_import::Limits::default();
            if bytes.len() as u64 > limits.file_bytes.min(limits.bundle_bytes) || !limits.json_within_depth(&bytes) {
                return Err("source_findings_export_bundle_limit".into());
            }
            let count = container["documents"]["json"]["review"]["findings"].as_array().ok_or("source_findings_projection_invalid")?.len();
            if count.saturating_add(1).saturating_mul(2) > limits.records {
                return Err("source_findings_export_bundle_limit".into());
            }
            (crate::snapshot_export::Kind::SourceReviewBundle, container)
        },
    };
    crate::snapshot_export::write_json(directory, kind, &report)
        .map_err(|error| format!("source_findings_export_{}", error.code()))
}

// Freeze, policy, decisions and counts are read from one database snapshot.
// Historical export neither acquires an execution lease nor uses display JSON.
fn native_source_findings_export_snapshot(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
) -> Result<JsonValue, String> {
    let snapshot = if connection.is_autocommit() {
        Some(rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Deferred)
            .map_err(|_| "source_findings_snapshot_failed")?)
    } else { None };
    let tx = snapshot.as_deref().unwrap_or(connection);
    let audit = native_source_findings_audit(tx, scan_id, attempt)?;
    let SourceFindingsAudit::Audited { lease, set } = &audit else {
        return Err("source_findings_export_unverified".into());
    };
    let plan = NativeSourcePlan::load(tx, scan_id, attempt)?.ok_or("source_ci_plan_missing")?;
    let gate = if plan.scan_type == "cicd" {
        let policy = load_historical_source_ci_policy(tx, scan_id, attempt)?;
        let mut report = crate::native_pipeline::source_ci::evaluate(tx, lease, set, policy)?.as_json();
        // Findings already live in the redacted review projection. Keep only
        // CI context here, avoiding raw-text leakage and duplicate report rows.
        let object = report.as_object_mut().ok_or("source_ci_projection_invalid")?;
        object.remove("findings");
        object.remove("sourceDecisionProjection");
        crate::agent_runtime::secrets::redact_json(&report)
    } else { JsonValue::Null };
    let mut review = native_source_findings_projection(tx, audit, scan_id, attempt)?;
    review["ciGate"] = gate;
    Ok(review)
}

// Serialization only. The sole production caller supplies a freshly audited,
// fully redacted snapshot; importing this representation never grants authority.
fn native_source_review_sarif(bundle: &JsonValue) -> Result<JsonValue, String> {
    let findings = bundle["review"]["findings"].as_array().ok_or("source_findings_projection_invalid")?;
    let mut summary = bundle.clone();
    summary["review"].as_object_mut().ok_or("source_findings_projection_invalid")?.remove("findings");
    let results = findings.iter().map(|row| {
        let severity = row["severity"].as_str().unwrap_or("informational");
        let mut locations = Vec::new();
        if let Some(path) = row["path"].as_str().filter(|path| !path.is_empty()) {
            // Encode path bytes as a URI reference, including spaces, Unicode,
            // fragments and drive-letter colons. Never dereference this URI.
            let uri: String = path.replace('\\', "/").bytes().map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                    (byte as char).to_string()
                } else { format!("%{byte:02X}") }
            }).collect();
            let mut physical = json!({"artifactLocation":{"uri":uri}});
            if let Some(line) = row["line"].as_u64().filter(|line| *line > 0) {
                physical["region"] = json!({"startLine":line});
            }
            locations.push(json!({"physicalLocation":physical}));
        }
        json!({"ruleId":row["sourceDecisionId"], "kind":"fail",
            "level":if matches!(severity,"critical"|"high") {"error"} else {"warning"},
            "message":{"text":row["title"].as_str().filter(|text| !text.is_empty()).unwrap_or("Reviewed source finding")},
            "locations":locations,
            "partialFingerprints":{"oviraptorSourceDecisionId":row["sourceDecisionId"],"candidateDigest":row["candidateDigest"]},
            "properties":{"severity":severity,"cwe":row["cwe"],"qualification":"verified_at_export",
                "executionEligible":false,"oviraptorSourceFinding":row}})
    }).collect::<Vec<_>>();
    Ok(json!({"version":"2.1.0", "$schema":"https://json.schemastore.org/sarif-2.1.0.json",
        "runs":[{"tool":{"driver":{"name":"oviraptor-source-review","version":env!("CARGO_PKG_VERSION")}},
            "properties":{"oviraptorSourceReview":summary}, "results":results}]}))
}
