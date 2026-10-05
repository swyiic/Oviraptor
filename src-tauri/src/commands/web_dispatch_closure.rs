const WEB_CLOSURE_REASON: &str = "operator_closed_before_dispatch";
const WEB_CLOSURE_CHECKPOINT: &str = "操作者已结束未派发尝试；未执行目标测试。旧记录保留，需另行启动新尝试。";

// Hash the rows this operation must leave unchanged, excluding only its exact
// terminal-state fields. No configuration file, secret or tool is opened. The
// hash is a transaction/reconciliation invariant, not an authorization token.
fn web_closure_preservation_hash(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<String,String> {
    let mut material = Vec::new();
    for (sql,excluded) in [
        ("SELECT * FROM sentinel_scans WHERE id=?1 AND attempt_count=?2", &["status","current_checkpoint","updated_at"][..]),
        ("SELECT * FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2", &["status","stage","checkpoint","stop_reason","finished_at","updated_at"][..]),
        ("SELECT * FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 ORDER BY branch", &["status","checkpoint","report_json","updated_at"][..]),
        ("SELECT * FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=?2 ORDER BY branch", &[][..]),
        ("SELECT * FROM native_web_dispatch_bindings WHERE scan_id=?1 AND attempt_number=?2", &[][..]),
        ("SELECT * FROM sentinel_targets WHERE scan_id=?1 AND ?2>0 ORDER BY id", &[][..]),
    ] {
        let mut statement = connection.prepare(sql).map_err(|e|e.to_string())?;
        let columns: Vec<(usize,String)> = statement.column_names().iter().enumerate()
            .filter(|(_,name)|!excluded.contains(name)).map(|(i,name)|(i,(*name).to_string())).collect();
        let rows = statement.query_map(params![scan_id,attempt],|row| {
            columns.iter().map(|(i,name)|row.get::<_,rusqlite::types::Value>(*i).map(|v|format!("{name}:{v:?}")))
                .collect::<Result<Vec<_>,_>>()
        }).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        material.push(json!({"query":sql,"rows":rows}));
    }
    Ok(agent_stable_hash(&json!(material)))
}

fn web_closure_no_effects_in(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<bool,String> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN sentinel_scan_attempts a
         ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
         JOIN projects p ON p.id=s.project_id AND p.status='active'
         WHERE s.id=?1 AND s.attempt_count=?2 AND s.scan_type='web' AND s.source_path=''
         AND s.llm_requests=a.llm_requests_start AND s.total_tokens=a.total_tokens_start
         AND s.input_tokens=a.input_tokens_start AND s.output_tokens=a.output_tokens_start
         AND s.cached_tokens=a.cached_tokens_start
         AND a.llm_requests_delta=0 AND a.total_tokens_delta=0 AND a.input_tokens_delta=0
         AND a.output_tokens_delta=0 AND a.cached_tokens_delta=0)
         AND (SELECT COUNT(*) FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2)=1
         AND EXISTS(SELECT 1 FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=?2
                    AND branch='web' AND claim_id='' AND claimed_at='')
         AND EXISTS(SELECT 1 FROM native_web_dispatch_bindings WHERE scan_id=?1 AND attempt_number=?2 AND schema_version=1)
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM environment_preparation_lease)
         AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2)
         AND NOT EXISTS(SELECT 1 FROM sentinel_processes WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM analyzer_container_receipts WHERE scan_id=?1 AND cleanup_status<>'confirmed')
         AND NOT EXISTS(SELECT 1 FROM agent_gap_followups WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2)
         AND NOT EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1 AND last_attempt_number=?2 AND status<>'queued')",
        params![scan_id,attempt],|r|r.get(0),
    ).map_err(|e|e.to_string())
}

fn web_closure_available_in(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> bool {
    web_recovery_available_in(connection,scan_id,attempt)
        && web_closure_no_effects_in(connection,scan_id,attempt).unwrap_or(false)
}

fn web_closure_report(id: &str, closed_at: &str) -> JsonValue {
    json!({"code":WEB_CLOSURE_REASON,"closureId":id,"closedAt":closed_at,
        "executionState":"closed_without_dispatch","automaticReplayAllowed":false})
}

// A retry of an ambiguous IPC only reads/validates the same terminal receipt.
// It never closes another attempt or automatically starts any work.
fn verified_web_closure_in(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<Option<JsonValue>,String> {
    let row: Option<(String,String,String,String)> = connection.query_row(
        "SELECT id,closed_at,preservation_hash,terminal_timestamp FROM native_web_attempt_closures
         WHERE scan_id=?1 AND attempt_number=?2 AND reason=?3",
        params![scan_id,attempt,WEB_CLOSURE_REASON],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(|e|e.to_string())?;
    let Some((id,closed_at,hash,stamp)) = row else {return Ok(None)};
    let valid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN sentinel_scan_attempts a
         ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
         JOIN native_scan_branches b ON b.scan_id=s.id AND b.attempt_number=a.attempt_number AND b.branch='web'
         WHERE s.id=?1 AND s.attempt_count=?2 AND s.status='cancelled' AND s.current_checkpoint=?3 AND s.updated_at=?4
         AND a.status='cancelled' AND a.stage=?5 AND a.stop_reason=?5 AND a.checkpoint=?3
         AND a.finished_at=?4 AND a.updated_at=?4
         AND b.status='failed' AND b.checkpoint=?3 AND b.report_json=?6 AND b.updated_at=?4)",
        params![scan_id,attempt,WEB_CLOSURE_CHECKPOINT,stamp,WEB_CLOSURE_REASON,web_closure_report(&id,&closed_at).to_string()],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if id.is_empty() || !valid || !web_closure_no_effects_in(connection,scan_id,attempt)?
        || web_closure_preservation_hash(connection,scan_id,attempt)? != hash {
        return Err("web_closure_receipt_unconfirmed".into());
    }
    Ok(Some(json!({"scanId":scan_id,"attemptNumber":attempt,"closureId":id,"closedAt":closed_at,
        "executionState":"closed_without_dispatch","automaticReplayAllowed":false})))
}

fn close_unclaimed_web_attempt(
    db_path: &Path, scan_id: &str, attempt: i64, operator_confirmed: bool,
) -> Result<JsonValue, String> {
    if !operator_confirmed { return Err("web_closure_explicit_confirmation_required".into()); }
    validate_web_start_id(scan_id)?;
    if attempt <= 0 || attempt > i64::from(u32::MAX) {return Err("web_closure_attempt_invalid".into());}
    let _lifecycle = claim_scan_control(db_path,scan_id)?;
    // Do not construct a branch failure guard: closing must never claim dispatch
    // or cause a Drop implementation to overwrite the explicit terminal receipt.
    let _branch = claim_native_invocation(db_path,scan_id,attempt,"branch","web")?;
    let mut connection = db::open(db_path)?;
    connection.pragma_update(None,"synchronous","FULL").map_err(|e|e.to_string())?;
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    if let Some(receipt) = verified_web_closure_in(&transaction,scan_id,attempt)? {
        transaction.commit().map_err(|e|format!("web_closure_commit_unconfirmed:{e}"))?;
        return Ok(receipt);
    }
    if !web_closure_available_in(&transaction,scan_id,attempt) {return Err("web_closure_requires_separate_review".into());}
    let pristine: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2 AND finished_at='' AND stop_reason='')",params![scan_id,attempt],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !pristine {return Err("web_closure_attempt_not_pristine".into());}
    let hash = web_closure_preservation_hash(&transaction,scan_id,attempt)?;
    let id = Uuid::new_v4().to_string();
    let closed_at = chrono::Utc::now().to_rfc3339();
    // Existing task/history ordering uses SQLite local timestamps. Keep that
    // format for common rows, and separately retain an unambiguous UTC receipt.
    // Store both so later reconciliation does not depend on the OS timezone.
    let stamp: String = transaction.query_row("SELECT datetime('now','localtime')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    let report = web_closure_report(&id,&closed_at).to_string();
    for (sql,values) in [
        ("INSERT INTO native_web_attempt_closures(scan_id,attempt_number,id,closed_at,preservation_hash,reason,terminal_timestamp) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            vec![rusqlite::types::Value::from(scan_id.to_string()),attempt.into(),id.clone().into(),closed_at.clone().into(),hash.clone().into(),WEB_CLOSURE_REASON.to_string().into(),stamp.clone().into()]),
        ("UPDATE native_scan_branches SET status='failed',checkpoint=?3,report_json=?4,updated_at=?5 WHERE scan_id=?1 AND attempt_number=?2 AND branch='web' AND status='pending'",
            vec![scan_id.to_string().into(),attempt.into(),WEB_CLOSURE_CHECKPOINT.to_string().into(),report.into(),stamp.clone().into()]),
        ("UPDATE sentinel_scan_attempts SET status='cancelled',stage=?3,stop_reason=?3,checkpoint=?4,finished_at=?5,updated_at=?5 WHERE scan_id=?1 AND attempt_number=?2 AND status='scanning'",
            vec![scan_id.to_string().into(),attempt.into(),WEB_CLOSURE_REASON.to_string().into(),WEB_CLOSURE_CHECKPOINT.to_string().into(),stamp.clone().into()]),
        ("UPDATE sentinel_scans SET status='cancelled',current_checkpoint=?3,updated_at=?4 WHERE id=?1 AND attempt_count=?2 AND status='scanning'",
            vec![scan_id.to_string().into(),attempt.into(),WEB_CLOSURE_CHECKPOINT.to_string().into(),stamp.clone().into()]),
    ] {
        let changed = transaction.execute(sql,rusqlite::params_from_iter(values)).map_err(|e|e.to_string())?;
        if changed != 1 {return Err("web_closure_write_not_persisted".into());}
    }
    let receipt = verified_web_closure_in(&transaction,scan_id,attempt)?.ok_or("web_closure_receipt_missing")?;
    // Compare with values held before any write, not only mutually-consistent
    // receipt/row values that an AFTER trigger could have replaced together.
    if receipt["closureId"] != id || receipt["closedAt"] != closed_at
        || web_closure_preservation_hash(&transaction,scan_id,attempt)? != hash {
        return Err("web_closure_postcondition".into());
    }
    transaction.commit().map_err(|e|format!("web_closure_commit_unconfirmed:{e}"))?;
    Ok(receipt)
}

#[tauri::command]
pub async fn close_never_dispatched_web_attempt(
    state: State<'_,AppState>, scan_id: String, attempt_number: i64, operator_confirmed: bool,
) -> Result<JsonValue,String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move ||close_unclaimed_web_attempt(&db_path,&scan_id,attempt_number,operator_confirmed))
        .await.map_err(|e|e.to_string())?
}
