// Desktop-only operator attestations. Deliberately absent from model tool
// dispatch. This module performs no target IO, receipt settlement or execution.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentRequestReviewInput {
    scan_id: String,
    attempt_number: i64,
    target_url: String,
    request_key: String,
    snapshot_hash: String,
    previous_review_id: String,
    operation_id: String,
    disposition: String,
    note: String,
    operator_confirmed: bool,
}

fn request_review_sources(
    connection: &rusqlite::Connection, scan: &str, attempt: i64, target: &str,
) -> Result<(AgentRequestAccounting, Vec<JsonValue>), String> {
    let bound: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3)
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        params![scan,attempt,target], |r|r.get(0),
    ).map_err(|_| "request_review_scope_unavailable")?;
    if !bound { return Err("request_review_scope_not_found".into()); }
    // Validate lineage and every source binding before exposing editable facts.
    let accounting = agent_request_accounting(connection,scan,attempt,target)?;
    let mut sources = Vec::new();
    for source_attempt in &accounting.attempts {
        for (source,sql) in [
            ("native_http", "SELECT json_object('sourceAttempt',c.attempt_number,'runId',c.run_id,
                'invocationId',c.invocation_id,'requestIndex',c.request_index,'ordinal',c.ordinal,
                'budgetAttempt',c.budget_attempt,'requestHash',c.request_hash,'tool',c.tool_name,
                'responseStatus',c.response_status,'receivedAt',c.received_at,'createdAt',c.created_at,
                'toolStatus',i.status,'toolError',i.error_class,'responseArtifactId',i.response_artifact_id)
                FROM agent_http_request_claims c JOIN tool_invocations i ON i.run_id=c.run_id AND i.invocation_id=c.invocation_id
                WHERE c.scan_id=?1 AND c.attempt_number=?2 AND c.target_url=?3 ORDER BY c.ordinal"),
            ("external_surface", "SELECT json_object('sourceAttempt',attempt_number,'runId',child_run_id,
                'assignmentId',assignment_id,'state',state,'artifactId',artifact_id,
                'responseHash',response_hash,'createdAt',created_at)
                FROM agent_external_surface_captures WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3"),
            ("authorization_probe", "SELECT json_object('sourceAttempt',attempt_number,'runId',child_run_id,
                'assignmentId',assignment_id,'contractKey',contract_key,'side',side,'artifactId',artifact_id,'createdAt',created_at)
                FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 ORDER BY contract_key,side"),
        ] {
            let mut statement = connection.prepare(sql).map_err(|_| "request_review_source_unavailable")?;
            let rows = statement.query_map(params![scan,source_attempt,target], |r|r.get::<_,String>(0))
                .map_err(|_| "request_review_source_unavailable")?;
            for row in rows {
                let mut snapshot: JsonValue = serde_json::from_str(&row.map_err(|_| "request_review_source_invalid")?)
                    .map_err(|_| "request_review_source_invalid")?;
                snapshot["source"] = json!(source);
                if source == "native_http" && snapshot["budgetAttempt"].as_i64() != accounting.attempts.last().copied() {
                    return Err("request_review_source_budget_mismatch".into());
                }
                // Hash identity separately from mutable receipt/tool state.
                let key = agent_stable_hash(&json!([scan,target,source,source_attempt,
                    snapshot["runId"],snapshot["invocationId"],snapshot["requestIndex"],
                    snapshot["contractKey"],snapshot["side"]]));
                let receipt_state = match source {
                    "native_http" if snapshot["responseStatus"] == 0 => "no_headers_recorded",
                    "native_http" => "headers_recorded_body_not_asserted",
                    "external_surface" if snapshot["state"] == "received" => "response_recorded",
                    "authorization_probe" if snapshot["artifactId"].as_str().is_some_and(|s|!s.is_empty()) => "response_reference_recorded",
                    _ => "no_response_recorded",
                };
                sources.push(json!({"requestKey":key,"snapshotHash":agent_stable_hash(&snapshot),
                    "snapshot":snapshot,"receiptState":receipt_state}));
            }
        }
    }
    Ok((accounting,sources))
}

// A durable operator receipt includes its real timeline event. Validate on
// reads and idempotent retries as well as first commit; never synthesize a
// missing event or let a partial SQL JSON match hide contradictory fields.
fn verify_request_review_event(
    connection: &rusqlite::Connection, scan: &str, attempt: i64, id: &str, disposition: &str,
) -> Result<(), String> {
    let mut statement = connection.prepare(
        "SELECT scan_id,attempt_number,entity_type,event_type,payload_json
         FROM agent_collaboration_events WHERE entity_id=?1 ORDER BY sequence",
    ).map_err(|_| "request_review_event_unavailable")?;
    let mut rows = statement.query([id]).map_err(|_| "request_review_event_unavailable")?;
    let row = rows.next().map_err(|_| "request_review_event_unavailable")?
        .ok_or("request_review_event_integrity")?;
    let actual: (String,i64,String,String,String) = (|| -> rusqlite::Result<_> {
        Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))
    })().map_err(|_| "request_review_event_integrity")?;
    let payload: JsonValue = serde_json::from_str(&actual.4).map_err(|_| "request_review_event_integrity")?;
    if actual.0 != scan || actual.1 != attempt || actual.2 != "request_review" || actual.3 != "request_review"
        || payload != json!({"disposition":disposition,"executionUnlocked":false})
        || rows.next().map_err(|_| "request_review_event_unavailable")?.is_some() {
        return Err("request_review_event_integrity".into());
    }
    Ok(())
}

fn request_review_history(
    connection: &rusqlite::Connection, scan: &str, target: &str, key: &str,
) -> Result<Vec<JsonValue>, String> {
    let mut statement = connection.prepare(
        "SELECT id,previous_review_id,source_snapshot_json,source_snapshot_hash,disposition,note,actor,created_at,payload_hash,attempt_number
         FROM agent_request_reviews WHERE scan_id=?1 AND target_url=?2 AND request_key=?3 ORDER BY rowid"
    ).map_err(|_| "request_review_history_unavailable")?;
    let rows = statement.query_map(params![scan,target,key], |r| Ok((
        r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,
        r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?,r.get::<_,i64>(9)?,
    ))).map_err(|_| "request_review_history_unavailable")?;
    let mut history = Vec::new();
    let mut previous = String::new();
    for row in rows {
        let (id,parent,raw,hash,disposition,note,actor,created,payload_hash,attempt) = row.map_err(|_| "request_review_history_invalid")?;
        let snapshot: JsonValue = serde_json::from_str(&raw).map_err(|_| "request_review_history_invalid")?;
        let receipt = json!({"id":id,"previousReviewId":parent,"snapshot":snapshot,"snapshotHash":hash,
            "disposition":disposition,"note":note,"actor":actor,"createdAt":created,"reviewedAttempt":attempt});
        if parent != previous || actor != "local_operator" || agent_stable_hash(&snapshot) != hash
            || agent_stable_hash(&json!([scan,attempt,target,key,receipt])) != payload_hash {
            return Err("request_review_history_integrity".into());
        }
        verify_request_review_event(connection,scan,attempt,&id,&disposition)?;
        previous = id;
        history.push(receipt);
    }
    Ok(history)
}

fn verify_request_review_timeline(
    connection: &rusqlite::Connection, scan: &str, attempt: i64,
) -> Result<(), String> {
    // Validate before applying an incremental cursor. Otherwise a missing
    // event falls out of the delta query and leaves an old "persisted" chat
    // message displayed forever without its receipt being checked again.
    let orphan: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_collaboration_events ev
         WHERE ev.scan_id=?1 AND ev.attempt_number=?2
         AND (ev.event_type='request_review' OR ev.entity_type='request_review')
         AND NOT EXISTS(SELECT 1 FROM agent_request_reviews r WHERE r.id=ev.entity_id
             AND r.scan_id=ev.scan_id AND r.attempt_number=ev.attempt_number))",
        params![scan,attempt], |r|r.get(0),
    ).map_err(|_| "request_review_event_unavailable")?;
    if orphan { return Err("request_review_event_integrity".into()); }
    let mut statement = connection.prepare(
        "SELECT DISTINCT target_url,request_key FROM agent_request_reviews
         WHERE scan_id=?1 AND attempt_number=?2 ORDER BY target_url,request_key",
    ).map_err(|_| "request_review_history_unavailable")?;
    let rows = statement.query_map(params![scan,attempt], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))
        .map_err(|_| "request_review_history_unavailable")?;
    for row in rows {
        let (target,key) = row.map_err(|_| "request_review_history_invalid")?;
        // Verify the entire revision chain, not just the newest note. This
        // also verifies the scope and exact payload of each timeline event.
        request_review_history(connection,scan,&target,&key)?;
    }
    Ok(())
}

fn request_review_view_in(
    connection: &rusqlite::Connection, scan: &str, attempt: i64, target: &str,
) -> Result<JsonValue,String> {
    let (accounting,mut items) = request_review_sources(connection,scan,attempt,target)?;
    for item in &mut items {
        let history = request_review_history(connection,scan,target,item["requestKey"].as_str().unwrap_or_default())?;
        item["sourceChangedSinceReview"] = json!(history.last().is_some_and(|r|r["snapshotHash"] != item["snapshotHash"]));
        item["reviews"] = json!(history);
    }
    // Legacy counters are intentionally not expanded into invented requests.
    let native_items = items.iter().filter(|i|i["snapshot"]["source"] == "native_http").count() as i64;
    Ok(json!({"scanId":scan,"attemptNumber":attempt,"targetUrl":target,"items":items,
        "unitemizedExecutorRequests":accounting.executor_recorded.saturating_sub(native_items),
        "accounting":accounting.as_json(),"automaticReplayAllowed":false,"executionUnlocked":false}))
}

fn record_request_review_in(connection: &mut rusqlite::Connection, input: &AgentRequestReviewInput) -> Result<JsonValue,String> {
    if !input.operator_confirmed || Uuid::parse_str(&input.operation_id).is_err()
        || !matches!(input.disposition.as_str(),"effect_observed"|"not_sent_attested"|"still_unknown")
        || input.note.trim().is_empty() || input.note.len()>4000 || input.note.chars().any(|c|c.is_control() && c!='\n' && c!='\t') {
        return Err("request_review_confirmation_invalid".into());
    }
    let note = crate::agent_runtime::secrets::redact_text_with(input.note.trim(),None);
    if note != input.note.trim() { return Err("request_review_remove_secrets".into()); }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "request_review_transaction_unavailable")?;
    let (accounting,sources) = request_review_sources(&tx,&input.scan_id,input.attempt_number,&input.target_url)?;
    let source = sources.iter().find(|s|s["requestKey"]==input.request_key).ok_or("request_review_source_not_found")?;
    let history = request_review_history(&tx,&input.scan_id,&input.target_url,&input.request_key)?;
    // A lost IPC response is retried with the same operation ID. A later wire
    // receipt does not invalidate the historical, already committed attestation.
    if let Some(receipt) = history.iter().find(|r|r["id"]==input.operation_id) {
        if receipt["reviewedAttempt"]!=input.attempt_number || receipt["previousReviewId"]!=input.previous_review_id || receipt["snapshotHash"]!=input.snapshot_hash
            || receipt["disposition"]!=input.disposition || receipt["note"]!=note {
            return Err("request_review_operation_conflict".into());
        }
        return Ok(receipt.clone());
    }
    if source["snapshotHash"] != input.snapshot_hash { return Err("request_review_source_changed".into()); }
    if history.last().and_then(|r|r["id"].as_str()).unwrap_or_default()!=input.previous_review_id {
        return Err("request_review_revision_changed".into());
    }
    let receipt = json!({"id":input.operation_id,"previousReviewId":input.previous_review_id,
        "snapshot":source["snapshot"],"snapshotHash":input.snapshot_hash,"disposition":input.disposition,
        "note":note,"actor":"local_operator","createdAt":chrono::Utc::now().to_rfc3339(),"reviewedAttempt":input.attempt_number});
    let payload_hash = agent_stable_hash(&json!([input.scan_id,input.attempt_number,input.target_url,input.request_key,receipt]));
    let changed = tx.execute(
        "INSERT INTO agent_request_reviews(id,scan_id,attempt_number,target_url,request_key,previous_review_id,
         source_snapshot_json,source_snapshot_hash,disposition,note,actor,created_at,payload_hash)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'local_operator',?11,?12)",
        params![input.operation_id,input.scan_id,input.attempt_number,input.target_url,input.request_key,input.previous_review_id,
            source["snapshot"].to_string(),input.snapshot_hash,input.disposition,note,receipt["createdAt"].as_str(),payload_hash],
    ).map_err(|_| "request_review_write_failed")?;
    let saved = request_review_history(&tx,&input.scan_id,&input.target_url,&input.request_key)?;
    let (after_accounting,after_sources) = request_review_sources(&tx,&input.scan_id,input.attempt_number,&input.target_url)?;
    // request_review_history checked every receipt's exact event in this same
    // transaction, including duplicates outside the requested scan/attempt.
    if changed!=1 || saved.last()!=Some(&receipt)
        || after_sources != sources || after_accounting.as_json()!=accounting.as_json() {
        return Err("request_review_postcondition".into());
    }
    tx.commit().map_err(|_| "request_review_commit_unconfirmed")?;
    Ok(receipt)
}

#[tauri::command]
pub async fn get_agent_request_reviews(
    state: State<'_,AppState>, scan_id: String, attempt_number: i64, target_url: String,
) -> Result<JsonValue,String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut connection = db::open(&path)?;
        let tx = connection.transaction().map_err(|_| "request_review_transaction_unavailable")?;
        request_review_view_in(&tx,&scan_id,attempt_number,&target_url)
    }).await.map_err(|_| "request_review_worker_failed")?
}

#[tauri::command]
pub async fn record_agent_request_review(state: State<'_,AppState>, input: AgentRequestReviewInput) -> Result<JsonValue,String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut connection = db::open(&path)?;
        connection.pragma_update(None,"synchronous","FULL").map_err(|_| "request_review_durability_unavailable")?;
        record_request_review_in(&mut connection,&input)
    }).await.map_err(|_| "request_review_worker_failed")?
}
