// This operation only creates a separately reviewable Web draft. Unknown
// source effects, requests, accounting and capabilities are never transferred.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClosureHandoffInput {
    request_id: String,
    source_scan_id: String,
    closure_id: String,
    source_hash: String,
    task_name: String,
    urls: Vec<String>,
    scan_mode: String,
    max_budget_usd: f64,
    auth_session_ids: Vec<String>,
    auth_session_scope_id: String,
    skill_ids: Vec<i64>,
    instruction: String,
    closure: String,
    operator_confirmed: bool,
}

fn closure_handoff_source(connection: &rusqlite::Connection, scan_id: &str) -> Result<JsonValue,String> {
    let receipt = verified_administrative_closure(connection,scan_id)?
        .ok_or("closure_handoff_requires_administrative_closure")?;
    let project: i64 = connection.query_row(
        "SELECT project_id FROM sentinel_scans WHERE id=?1 AND scan_type='web' AND source_path=''",
        [scan_id],|r|r.get(0),
    ).map_err(|_|"closure_handoff_source_unavailable")?;
    let targets = connection.prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1 ORDER BY url")
        .map_err(|e|e.to_string())?.query_map([scan_id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?
        .collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    if targets.is_empty() { return Err("closure_handoff_targets_missing".into()); }
    let mut source = json!({"sourceScanId":scan_id,"closureId":receipt["closureId"],
        "attemptNumber":receipt["attemptNumber"],"snapshotHash":receipt["snapshotHash"],
        "projectId":project,"targetUrls":targets,"executionSettled":false,"targetRequestsGranted":0});
    source["sourceHash"] = json!(agent_stable_hash(&source));
    Ok(source)
}

fn closure_handoff_saved(connection: &rusqlite::Connection, source_scan_id: &str) -> Result<Option<JsonValue>,String> {
    let row: Option<(String,String,String,String,String,String,String)> = connection.query_row(
        "SELECT scan_id,request_id,closure_id,input_json,input_hash,preview_json,created_at
         FROM native_web_closure_handoffs WHERE source_scan_id=?1",[source_scan_id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).optional().map_err(|e|e.to_string())?;
    let Some((scan_id,request_id,closure_id,raw,hash,preview_raw,created_at)) = row else {
        let orphan: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_collaboration_events WHERE scan_id=?1 AND event_type='closure_handoff')",
            [source_scan_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        return if orphan {Err("closure_handoff_event_orphan".into())} else {Ok(None)};
    };
    let input: ClosureHandoffInput = serde_json::from_str(&raw).map_err(|_|"closure_handoff_input_corrupt")?;
    let input_value = serde_json::to_value(&input).map_err(|e|e.to_string())?;
    let mut preview: JsonValue = serde_json::from_str(&preview_raw).map_err(|_|"closure_handoff_preview_corrupt")?;
    let source_hash = preview.as_object_mut().and_then(|p|p.remove("sourceHash")).ok_or("closure_handoff_preview_corrupt")?;
    let receipt = verified_administrative_closure(connection,source_scan_id)?.ok_or("closure_handoff_source_unavailable")?;
    let scan = sentinel_scan_by_id(connection,&scan_id)?;
    if agent_stable_hash(&input_value)!=hash || input.request_id!=request_id || Uuid::parse_str(&request_id).is_err()
        || input.source_scan_id!=source_scan_id || input.closure_id!=closure_id || !input.operator_confirmed
        || receipt["closureId"]!=closure_id || preview["sourceScanId"]!=source_scan_id
        || preview["closureId"]!=closure_id || preview["snapshotHash"]!=receipt["snapshotHash"]
        || preview["attemptNumber"]!=receipt["attemptNumber"] || preview["projectId"].as_i64()!=scan.project_id
        || source_hash!=agent_stable_hash(&preview) || source_hash!=input.source_hash
        || preview["executionSettled"]!=false || preview["targetRequestsGranted"]!=0
        || scan.id==source_scan_id || scan.scan_type!="web" || !scan.source_path.is_empty()
        || chrono::DateTime::parse_from_rfc3339(&created_at).is_err() {
        return Err("closure_handoff_receipt_integrity".into());
    }
    let events = connection.prepare("SELECT scan_id,attempt_number,entity_type,event_type,payload_json FROM agent_collaboration_events
        WHERE entity_id=?1 OR (scan_id=?2 AND event_type='closure_handoff')")
        .map_err(|e|e.to_string())?.query_map(params![request_id,source_scan_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))
        .map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let payload = json!({"scanId":scan_id,"closureId":closure_id,"executionGranted":false,"sourceExecutionSettled":false});
    if events.len()!=1 || events[0].0!=source_scan_id || Some(events[0].1)!=receipt["attemptNumber"].as_i64()
        || events[0].2!="closure_handoff" || events[0].3!="closure_handoff"
        || serde_json::from_str::<JsonValue>(&events[0].4).ok()!=Some(payload) {
        return Err("closure_handoff_event_integrity".into());
    }
    Ok(Some(json!({"requestId":request_id,"sourceScanId":source_scan_id,"closureId":closure_id,
        "sourceHash":input.source_hash,"scanId":scan_id,"createdAt":created_at,
        "executionGranted":false,"sourceExecutionSettled":false,"scan":scan})))
}

fn closure_handoff_preview_in(connection: &rusqlite::Connection, scan_id: &str) -> Result<JsonValue,String> {
    let mut source = closure_handoff_source(connection,scan_id)?;
    source["savedHandoff"] = closure_handoff_saved(connection,scan_id)?.unwrap_or(JsonValue::Null);
    Ok(source)
}

fn closure_handoff_relations(connection: &rusqlite::Connection, scan_id: &str) -> Result<JsonValue,String> {
    let predecessor: Option<String> = connection.query_row("SELECT source_scan_id FROM native_web_closure_handoffs WHERE scan_id=?1",
        [scan_id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let source = predecessor.map(|id|closure_handoff_saved(connection,&id)).transpose()?.flatten();
    Ok(json!({"source":source,"successor":closure_handoff_saved(connection,scan_id)?}))
}

fn create_closure_handoff_in(connection: &rusqlite::Connection, input: &ClosureHandoffInput) -> Result<JsonValue,String> {
    if !input.operator_confirmed {return Err("closure_handoff_confirmation_required".into());}
    if Uuid::parse_str(&input.request_id).is_err() || Uuid::parse_str(&input.closure_id).is_err()
        || input.auth_session_scope_id.trim().is_empty() || input.task_name.trim().is_empty()
        || !input.max_budget_usd.is_finite() || input.max_budget_usd<=0.0 || input.max_budget_usd>10_000.0
        || !["quick","standard","deep"].contains(&input.scan_mode.as_str())
        || !["breadth","proof"].contains(&input.closure.as_str()) {
        return Err("closure_handoff_input_invalid".into());
    }
    let value = serde_json::to_value(input).map_err(|e|e.to_string())?;
    let hash = agent_stable_hash(&value);
    let tx = rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate)
        .map_err(|e|format!("closure_handoff_lock:{e}"))?;
    if let Some(saved) = closure_handoff_saved(&tx,&input.source_scan_id)? {
        let saved_hash: String = tx.query_row("SELECT input_hash FROM native_web_closure_handoffs WHERE source_scan_id=?1",
            [&input.source_scan_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if saved["requestId"]!=input.request_id || saved_hash!=hash {
            return Err("closure_handoff_exists_refresh_original_submission".into());
        }
        // Return the current task even after later manual startup; never start
        // it again or try to consume its identity handles a second time.
        return Ok(saved);
    }
    let preview = closure_handoff_source(&tx,&input.source_scan_id)?;
    if preview["closureId"]!=input.closure_id || preview["sourceHash"]!=input.source_hash {
        return Err("closure_handoff_source_changed_preview_again".into());
    }
    let targets: Vec<String> = serde_json::from_value(preview["targetUrls"].clone()).map_err(|e|e.to_string())?;
    let mut selected = input.urls.clone(); selected.sort(); selected.dedup();
    if selected.is_empty() || selected.len()!=input.urls.len() || selected.iter().any(|url|!targets.contains(url)) {
        return Err("closure_handoff_exact_source_targets_required".into());
    }
    let project = preview["projectId"].as_i64().ok_or("closure_handoff_project_invalid")?;
    let source_records = administrative_closure_evidence(&tx,&input.source_scan_id)?;
    let mode_window=crate::agent_runtime::web_mode::draft::DraftCreationWindow::begin(&tx)?;
    let scan = create_sentinel_url_draft_rows(&tx,project,input.task_name.clone(),selected.clone(),
        Some(input.scan_mode.clone()),Some(input.max_budget_usd),None,Some(input.auth_session_ids.clone()),
        Some(input.auth_session_scope_id.clone()),Some(input.skill_ids.clone()),Some(input.instruction.clone()),
        Some(input.closure.clone()),None)?;
    // The generic draft builder normalizes URLs and can skip fused targets.
    // This handoff must never silently change the exact confirmed resource set.
    let actual = tx.prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1 ORDER BY url")
        .map_err(|e|e.to_string())?.query_map([&scan.id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?
        .collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let normalized = selected.iter().map(|url|url.trim_end_matches('/').to_string()).collect::<Vec<_>>();
    if actual!=normalized {return Err("closure_handoff_target_set_changed".into());}
    for (old,exact) in actual.iter().zip(&selected) {
        if old!=exact && tx.execute("UPDATE sentinel_targets SET url=?1 WHERE scan_id=?2 AND url=?3",
            params![exact,scan.id,old]).map_err(|e|e.to_string())?!=1 {
            return Err("closure_handoff_exact_target_write_failed".into());
        }
    }
    let final_targets = tx.prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1 ORDER BY url")
        .map_err(|e|e.to_string())?.query_map([&scan.id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?
        .collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    if final_targets!=selected {return Err("closure_handoff_exact_target_write_failed".into());}
    let created_at = chrono::Utc::now().to_rfc3339();
    let changed = tx.execute("INSERT INTO native_web_closure_handoffs
        (source_scan_id,closure_id,scan_id,request_id,input_json,input_hash,preview_json,created_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![input.source_scan_id,input.closure_id,scan.id,input.request_id,value.to_string(),hash,preview.to_string(),created_at])
        .map_err(|e|format!("closure_handoff_provenance_write:{e}"))?;
    if changed!=1 {return Err("closure_handoff_provenance_missing".into());}
    let receipt = closure_handoff_saved(&tx,&input.source_scan_id)?.ok_or("closure_handoff_receipt_missing")?;
    if receipt["scanId"]!=scan.id || receipt["scan"]["status"]!="draft" || receipt["scan"]["attemptCount"]!=0 {
        return Err("closure_handoff_draft_integrity".into());
    }
    if administrative_closure_evidence_except_event(&tx,&input.source_scan_id,Some(&input.request_id))?!=source_records {
        return Err("closure_handoff_source_preservation_failed".into());
    }
    let mode=crate::agent_runtime::web_mode::draft::register_new(mode_window,&scan.id,crate::agent_runtime::web_mode::WebMode::Multi)?;
    crate::agent_runtime::web_mode::action::freeze_new(mode,&crate::agent_runtime::web_mode::action::NewWebAction {
        schema_version:1,kind:"closure_handoff".into(),request_id:input.request_id.clone(),source_scan_id:input.source_scan_id.clone(),
        source_hash:input.source_hash.clone(),request_hash:hash.clone(),targets:selected.clone(),
    })?;
    if administrative_closure_evidence_except_event(&tx,&input.source_scan_id,Some(&input.request_id))?!=source_records {
        return Err("web_mode_handoff_source_changed_during_creation".into());
    }
    tx.commit().map_err(|e|format!("closure_handoff_commit_unconfirmed:{e}"))?;
    Ok(receipt)
}

#[tauri::command]
pub async fn preview_web_closure_handoff(state: State<'_,AppState>, scan_id: String) -> Result<JsonValue,String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = db::open(&path)?;
        let tx = connection.unchecked_transaction().map_err(|e|e.to_string())?;
        closure_handoff_preview_in(&tx,&scan_id)
    }).await.map_err(|e|e.to_string())?
}

#[tauri::command]
pub async fn create_web_closure_handoff(state: State<'_,AppState>, input: ClosureHandoffInput) -> Result<JsonValue,String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = db::open(&path)?;
        connection.pragma_update(None,"synchronous","FULL").map_err(|e|e.to_string())?;
        create_closure_handoff_in(&connection,&input)
    }).await.map_err(|e|e.to_string())?
}
