const ADMINISTRATIVE_CLOSURE_CHECKPOINT: &str = "已人工结案；不是执行成功或费用结清。原始请求、预算占用和证据保留，旧任务禁止恢复或重放。";

fn administrative_closure_available(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<bool,String> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN sentinel_scan_attempts a
           ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
           JOIN projects p ON p.id=s.project_id AND p.status='active'
           WHERE s.id=?1 AND s.attempt_count=?2 AND s.scan_type='web' AND s.source_path=''
           AND s.status IN ('paused','partial','failed','cancelled','limited','protected_stop',
               'manual_review','persistence_failure','resume_incompatible')
           AND EXISTS(SELECT 1 FROM native_branch_dispatches d WHERE d.scan_id=s.id
               AND d.attempt_number=s.attempt_count AND d.branch='web' AND d.claim_id<>'' AND d.claimed_at<>''))
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM native_web_attempt_closures WHERE scan_id=?1 AND attempt_number=?2)",
        params![scan_id,attempt], |r|r.get(0),
    ).map_err(|e|format!("administrative_closure_admission_unavailable:{e}"))
}

// Snapshot task-owned native records without exposing credentials/content to
// the UI. Enumerating ownership columns also includes new scan/run tables.
// This is a stale-preview and transaction preservation check, not a signature
// or an assertion that remote effects have been reconciled.
fn administrative_closure_evidence(connection: &rusqlite::Connection, scan_id: &str) -> Result<JsonValue,String> {
    administrative_closure_evidence_except_event(connection,scan_id,None)
}

fn administrative_closure_evidence_except_event(connection: &rusqlite::Connection, scan_id: &str, new_handoff_event: Option<&str>) -> Result<JsonValue,String> {
    let tables = connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|e|e.to_string())?.query_map([],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?
        .collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let mut result = Vec::new();
    for table in tables {
        // The receipt and its event are the only new rows this action creates.
        if table == "native_web_administrative_closures" { continue; }
        let quoted = format!("\"{}\"",table.replace('"',"\"\""));
        let mut metadata = connection.prepare(&format!("SELECT * FROM {quoted} LIMIT 0")).map_err(|e|e.to_string())?;
        let columns = metadata.column_names().iter().map(|name|(*name).to_string()).collect::<Vec<_>>();
        let mut filters = Vec::new();
        if table=="sentinel_scans" { filters.push("id=?1".to_string()); }
        if columns.iter().any(|name|name=="scan_id") { filters.push("scan_id=?1".to_string()); }
        if columns.iter().any(|name|name=="owner_scan_id") { filters.push("owner_scan_id=?1".to_string()); }
        for name in ["run_id","root_run_id","coordinator_run_id","child_run_id"] {
            if columns.iter().any(|column|column==name) {
                filters.push(format!("{name} IN (SELECT id FROM agent_runs WHERE scan_id=?1)"));
            }
        }
        if columns.iter().any(|name|name=="assignment_id") {
            filters.push("assignment_id IN (SELECT id FROM agent_assignments WHERE coordinator_run_id IN (SELECT id FROM agent_runs WHERE scan_id=?1))".into());
        }
        if columns.iter().any(|name|name=="directive_id") {
            filters.push("directive_id IN (SELECT id FROM agent_user_directives WHERE scan_id=?1)".into());
        }
        if filters.is_empty() { continue; }
        let mut bindings=vec![rusqlite::types::Value::Text(scan_id.into())];
        let event_filter = if table=="agent_collaboration_events" {
            if let Some(id)=new_handoff_event {
                bindings.push(rusqlite::types::Value::Text(id.into()));
                " AND event_type<>'administrative_closure' AND NOT(event_type='closure_handoff' AND entity_id=?2)"
            } else { " AND event_type<>'administrative_closure'" }
        } else { "" };
        let sql = format!("SELECT * FROM {quoted} WHERE ({}){event_filter}",filters.join(" OR "));
        metadata = connection.prepare(&sql).map_err(|e|e.to_string())?;
        let mut rows = metadata.query_map(rusqlite::params_from_iter(bindings),|row| {
            columns.iter().enumerate().filter(|(_,name)|table!="sentinel_scans"
                || !["status","current_checkpoint","updated_at"].contains(&name.as_str()))
                .map(|(i,name)|row.get::<_,rusqlite::types::Value>(i).map(|value|format!("{name}:{value:?}")))
                .collect::<Result<Vec<_>,_>>()
        }).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        rows.sort();
        result.push(json!({"table":table,"rows":rows.len(),"digest":agent_stable_hash(&json!(rows))}));
    }
    Ok(json!(result))
}

fn administrative_closure_snapshot(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<JsonValue,String> {
    if !administrative_closure_available(connection,scan_id,attempt)? { return Err("administrative_closure_requires_stopped_dispatched_web_task".into()); }
    let status: String = connection.query_row("SELECT status FROM sentinel_scans WHERE id=?1",[scan_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    Ok(json!({"schemaVersion":1,"scanId":scan_id,"attemptNumber":attempt,"previousStatus":status,
        "records":administrative_closure_evidence(connection,scan_id)?}))
}

fn verified_administrative_closure(connection: &rusqlite::Connection, scan_id: &str) -> Result<Option<JsonValue>,String> {
    let stored: Option<(i64,String,String,String,String,String,String)> = connection.query_row(
        "SELECT attempt_number,id,closed_at,previous_status,snapshot_json,snapshot_hash,actor
         FROM native_web_administrative_closures WHERE scan_id=?1",[scan_id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).optional().map_err(|e|e.to_string())?;
    let Some((attempt,id,closed_at,previous_status,raw,hash,actor)) = stored else {
        let orphan: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_collaboration_events WHERE scan_id=?1 AND event_type='administrative_closure')",[scan_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        return if orphan {Err("administrative_closure_event_orphan".into())} else {Ok(None)};
    };
    let snapshot: JsonValue = serde_json::from_str(&raw).map_err(|_|"administrative_closure_snapshot_corrupt")?;
    let valid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2 AND status='cancelled'
           AND current_checkpoint=?3) AND EXISTS(SELECT 1 FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2)",
        params![scan_id,attempt,ADMINISTRATIVE_CLOSURE_CHECKPOINT],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if !valid || Uuid::parse_str(&id).is_err() || chrono::DateTime::parse_from_rfc3339(&closed_at).is_err()
        || actor!="local_operator" || snapshot["schemaVersion"]!=1 || snapshot["scanId"]!=scan_id
        || snapshot["attemptNumber"]!=attempt || snapshot["previousStatus"]!=previous_status
        || !snapshot["records"].is_array() || agent_stable_hash(&snapshot)!=hash {
        return Err("administrative_closure_receipt_integrity".into());
    }
    let events = connection.prepare("SELECT scan_id,attempt_number,entity_type,event_type,payload_json FROM agent_collaboration_events WHERE entity_id=?1 OR (scan_id=?2 AND event_type='administrative_closure')")
        .map_err(|e|e.to_string())?.query_map(params![id,scan_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))
        .map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let payload = json!({"executionSettled":false,"automaticReplayAllowed":false});
    if events.len()!=1 || events[0].0!=scan_id || events[0].1!=attempt || events[0].2!="administrative_closure"
        || events[0].3!="administrative_closure" || serde_json::from_str::<JsonValue>(&events[0].4).ok()!=Some(payload) {
        return Err("administrative_closure_event_integrity".into());
    }
    Ok(Some(json!({"scanId":scan_id,"attemptNumber":attempt,"closureId":id,"closedAt":closed_at,
        "previousStatus":previous_status,"snapshotHash":hash,"actor":actor,
        "executionState":"administratively_closed_unsettled","executionSettled":false,
        "automaticReplayAllowed":false,"requiresIndependentTask":true})))
}

fn preview_administrative_closure(db_path: &Path, scan_id: &str, attempt: i64) -> Result<JsonValue,String> {
    validate_web_start_id(scan_id)?;
    let _lifecycle = claim_scan_control(db_path,scan_id)?;
    let mut connection = db::open(db_path)?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    if let Some(receipt) = verified_administrative_closure(&tx,scan_id)? {
        if receipt["attemptNumber"]!=attempt { return Err("administrative_closure_attempt_mismatch".into()); }
        return Ok(json!({"receipt":receipt}));
    }
    let _owners = claim_scan_quiescence_in(&tx,db_path,scan_id)?;
    let snapshot = administrative_closure_snapshot(&tx,scan_id,attempt)?;
    Ok(json!({"snapshotHash":agent_stable_hash(&snapshot),"snapshot":snapshot,"receipt":null}))
}

fn close_administrative_web_task(db_path: &Path, scan_id: &str, attempt: i64, operation_id: &str,
    snapshot_hash: &str, operator_confirmed: bool) -> Result<JsonValue,String> {
    if !operator_confirmed { return Err("administrative_closure_explicit_confirmation_required".into()); }
    validate_web_start_id(scan_id)?;
    if Uuid::parse_str(operation_id).is_err() || snapshot_hash.len()!=64 {
        return Err("administrative_closure_input_invalid".into());
    }
    let _lifecycle = claim_scan_control(db_path,scan_id)?;
    let mut connection = db::open(db_path)?;
    connection.pragma_update(None,"synchronous","FULL").map_err(|e|e.to_string())?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    if let Some(receipt) = verified_administrative_closure(&tx,scan_id)? {
        if receipt["attemptNumber"]!=attempt || receipt["closureId"]!=operation_id || receipt["snapshotHash"]!=snapshot_hash {
            return Err("administrative_closure_already_recorded_refresh_required".into());
        }
        return Ok(receipt);
    }
    let _owners = claim_scan_quiescence_in(&tx,db_path,scan_id)?;
    let snapshot = administrative_closure_snapshot(&tx,scan_id,attempt)?;
    if agent_stable_hash(&snapshot)!=snapshot_hash { return Err("administrative_closure_preview_stale".into()); }
    let closed_at = chrono::Utc::now().to_rfc3339();
    let previous = snapshot["previousStatus"].as_str().ok_or("administrative_closure_status_missing")?;
    if tx.execute("INSERT INTO native_web_administrative_closures(scan_id,attempt_number,id,closed_at,previous_status,snapshot_json,snapshot_hash,actor) VALUES(?1,?2,?3,?4,?5,?6,?7,'local_operator')",
        params![scan_id,attempt,operation_id,closed_at,previous,snapshot.to_string(),snapshot_hash]).map_err(|e|e.to_string())?!=1 {
        return Err("administrative_closure_receipt_not_persisted".into());
    }
    if tx.execute("UPDATE sentinel_scans SET status='cancelled',current_checkpoint=?3,updated_at=datetime('now','localtime') WHERE id=?1 AND attempt_count=?2 AND status=?4",
        params![scan_id,attempt,ADMINISTRATIVE_CLOSURE_CHECKPOINT,previous]).map_err(|e|e.to_string())?!=1 {
        return Err("administrative_closure_state_not_persisted".into());
    }
    let receipt = verified_administrative_closure(&tx,scan_id)?.ok_or("administrative_closure_receipt_missing")?;
    if receipt["closureId"]!=operation_id || receipt["closedAt"]!=closed_at || receipt["snapshotHash"]!=snapshot_hash
        || administrative_closure_evidence(&tx,scan_id)?!=snapshot["records"] {
        return Err("administrative_closure_preservation_failed".into());
    }
    tx.commit().map_err(|e|format!("administrative_closure_commit_unconfirmed:{e}"))?;
    Ok(receipt)
}

#[tauri::command]
pub async fn preview_web_administrative_closure(state: State<'_,AppState>, scan_id: String, attempt_number: i64) -> Result<JsonValue,String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move ||preview_administrative_closure(&path,&scan_id,attempt_number)).await.map_err(|e|e.to_string())?
}

#[tauri::command]
pub async fn close_web_task_administratively(state: State<'_,AppState>, scan_id: String, attempt_number: i64,
    operation_id: String, snapshot_hash: String, operator_confirmed: bool) -> Result<JsonValue,String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move ||close_administrative_web_task(&path,&scan_id,attempt_number,&operation_id,&snapshot_hash,operator_confirmed)).await.map_err(|e|e.to_string())?
}
