// Durable local submission journal. Reconciliation is read-only; no startup
// path creates a task, captures an identity, invokes a model, or starts a scan.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GapFollowupSubmission {
    input: GapFollowupDraftInput,
    scan: Option<SentinelScan>,
    created_scan_id: String,
}

fn gap_submission_input_hash(input: &GapFollowupDraftInput) -> Result<(String,String),String> {
    if Uuid::parse_str(&input.request_id).is_err() { return Err("followup_request_id_invalid".into()); }
    if input.max_budget_usd.is_some_and(|v| !v.is_finite()) { return Err("followup_budget_invalid".into()); }
    let text=serde_json::to_string(input).map_err(|e|e.to_string())?;
    if text.len()>262_144 { return Err("followup_submission_too_large".into()); }
    let hash=crate::agent_runtime::store::stable_hash(&text);
    Ok((text,hash))
}

fn read_gap_submission(tx: &rusqlite::Transaction<'_>, request: &str) -> Result<GapFollowupSubmission,String> {
    let (raw,hash,source,message,created):(String,String,String,String,String)=tx.query_row(
        "SELECT input_json,input_hash,source_scan_id,assessment_message_id,created_scan_id FROM agent_gap_followup_submissions WHERE request_id=?1",
        [request],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).map_err(|e|format!("followup_submission_unavailable:{e}"))?;
    let input:GapFollowupDraftInput=serde_json::from_str(&raw).map_err(|_|"followup_submission_corrupt")?;
    let (canonical,actual)=gap_submission_input_hash(&input)?;
    if canonical!=raw || actual!=hash || input.request_id!=request || input.source_scan_id!=source || input.assessment_message_id!=message {
        return Err("followup_submission_corrupt".into());
    }
    let receipt:Option<(String,String)>=tx.query_row(
        "SELECT scan_id,request_hash FROM agent_gap_followups WHERE request_id=?1",[request],|r|Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(|e|e.to_string())?;
    let (scan,created_scan_id)=match receipt {
        Some((scan,receipt_hash))=>{
            if receipt_hash!=hash || (!created.is_empty() && created!=scan) { return Err("followup_submission_receipt_mismatch".into()); }
            (Some(sentinel_scan_by_id(tx,&scan)?),scan)
        },
        None=>{
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)",[&created],|r|r.get(0)).map_err(|e|e.to_string())?;
            if exists { return Err("followup_submission_receipt_missing".into()); }
            (None,created)
        },
    };
    Ok(GapFollowupSubmission {input,scan,created_scan_id})
}

fn record_gap_submission_result(tx:&rusqlite::Transaction<'_>,request:&str,scan:&str)->Result<(),String> {
    let recorded:Option<String>=tx.query_row("SELECT created_scan_id FROM agent_gap_followup_submissions WHERE request_id=?1",[request],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let Some(recorded)=recorded else { return Ok(()) }; // Historical non-journaled callers.
    if !recorded.is_empty() && recorded!=scan { return Err("followup_submission_result_conflict".into()); }
    let changed=tx.execute("UPDATE agent_gap_followup_submissions SET created_scan_id=?2 WHERE request_id=?1",params![request,scan]).map_err(|e|e.to_string())?;
    let saved:String=tx.query_row("SELECT created_scan_id FROM agent_gap_followup_submissions WHERE request_id=?1",[request],|r|r.get(0)).map_err(|e|e.to_string())?;
    if changed!=1 || saved!=scan { return Err("followup_submission_result_not_saved".into()); }
    Ok(())
}

fn stage_gap_submission(connection:&rusqlite::Connection,input:&GapFollowupDraftInput)->Result<(),String> {
    let (raw,hash)=gap_submission_input_hash(input)?;
    let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    let state:Option<String>=tx.query_row("SELECT state FROM agent_gap_followup_submissions WHERE request_id=?1",
        [&input.request_id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if let Some(state)=state {
        let saved=read_gap_submission(&tx,&input.request_id)?;
        if gap_submission_input_hash(&saved.input)?.1!=hash { return Err("followup_request_id_conflict".into()); }
        if state=="released" && saved.created_scan_id.is_empty() { return Err("followup_submission_released".into()); }
        return Ok(());
    }
    let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_gap_followup_submissions WHERE source_scan_id=?1 AND assessment_message_id=?2 AND state='pending')",
        params![input.source_scan_id,input.assessment_message_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if pending { return Err("followup_submission_conflict_reconcile".into()); }
    let changed=tx.execute("INSERT INTO agent_gap_followup_submissions(request_id,source_scan_id,assessment_message_id,input_json,input_hash) VALUES(?1,?2,?3,?4,?5)",
        params![input.request_id,input.source_scan_id,input.assessment_message_id,raw,hash]).map_err(|e|format!("followup_submission_write:{e}"))?;
    if changed!=1 { return Err("followup_submission_not_saved".into()); }
    let saved=read_gap_submission(&tx,&input.request_id)?;
    let pending:bool=tx.query_row("SELECT state='pending' FROM agent_gap_followup_submissions WHERE request_id=?1",[&input.request_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !pending || gap_submission_input_hash(&saved.input)?.1!=hash { return Err("followup_submission_not_saved".into()); }
    if !saved.created_scan_id.is_empty() { record_gap_submission_result(&tx,&input.request_id,&saved.created_scan_id)?; }
    tx.commit().map_err(|e|format!("followup_submission_commit:{e}"))
}

fn gap_followup_submission_in(connection:&rusqlite::Connection,source:&str,message:&str)->Result<Option<GapFollowupSubmission>,String> {
    let tx=connection.unchecked_transaction().map_err(|e|e.to_string())?;
    let request:Option<String>=tx.query_row("SELECT request_id FROM agent_gap_followup_submissions WHERE source_scan_id=?1 AND assessment_message_id=?2 AND state='pending'",
        params![source,message],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    request.map(|id|read_gap_submission(&tx,&id)).transpose()
}

fn release_gap_submission_in(connection:&rusqlite::Connection,request:&str,expected_scan:Option<&str>)->Result<(),String> {
    let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    let saved=read_gap_submission(&tx,request)?;
    // If a racing creator committed after the UI read, require another explicit
    // decision with that task visible. Never turn an unknown result into retry.
    let created=(!saved.created_scan_id.is_empty()).then_some(saved.created_scan_id.as_str());
    if created!=expected_scan {
        return Err("followup_submission_changed_reconcile".into());
    }
    let changed=tx.execute("UPDATE agent_gap_followup_submissions SET state='released' WHERE request_id=?1",[request]).map_err(|e|e.to_string())?;
    let released:bool=tx.query_row("SELECT state='released' FROM agent_gap_followup_submissions WHERE request_id=?1",[request],|r|r.get(0)).map_err(|e|e.to_string())?;
    if changed!=1 || !released { return Err("followup_submission_release_failed".into()); }
    tx.commit().map_err(|e|e.to_string())
}

fn submit_agent_gap_followup_in(connection:&rusqlite::Connection,input:&GapFollowupDraftInput)->Result<SentinelScan,String> {
    gap_submission_input_hash(input).map_err(|error|format!("followup_rejected:{error}"))?;
    stage_gap_submission(connection,input)?;
    match create_agent_gap_followup_in(connection,input) {
        Ok(scan)=>Ok(scan),
        Err(error)=>{
            // Release only after a separate locked read proves no task exists.
            // An uncertain commit or reconciliation failure stays recoverable.
            release_gap_submission_in(connection,&input.request_id,None)
                .map_err(|release|format!("followup_submission_needs_reconciliation:{error};{release}"))?;
            Err(format!("followup_rejected:{error}"))
        },
    }
}

#[tauri::command]
pub fn get_agent_gap_followup_submission(state:State<AppState>,source_scan_id:String,assessment_message_id:String)->Result<Option<GapFollowupSubmission>,String> {
    gap_followup_submission_in(&db::open(&state.db_path)?,&source_scan_id,&assessment_message_id)
}

#[tauri::command]
pub fn release_agent_gap_followup_submission(state:State<AppState>,request_id:String,expected_scan_id:Option<String>)->Result<(),String> {
    release_gap_submission_in(&db::open(&state.db_path)?,&request_id,expected_scan_id.as_deref())
}
