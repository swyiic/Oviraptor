// A pause is not a receipt that target effects were rolled back. These locks
// only prove the locally owned callers have returned. Never infer this from a
// PID, a released Coordinator lease, or a request-review attestation.
fn claim_scan_quiescence_in(
    connection: &rusqlite::Connection, db_path: &Path, scan_id: &str,
) -> Result<Vec<NativeInvocationOwner>, String> {
    claim_scan_quiescence_with_source_parents(connection,db_path,scan_id,None)
}
fn claim_scan_quiescence_with_source_parents(
    connection:&rusqlite::Connection,db_path:&Path,scan_id:&str,
    mut parents:Option<&mut crate::agent_runtime::deleted_scan_audit::OriginalSourceParents>,
)->Result<Vec<NativeInvocationOwner>,String> {
    if connection.is_autocommit() { return Err("scan_quiescence_requires_transaction".into()); }
    let unresolved: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_processes WHERE scan_id=?1)
         OR EXISTS(SELECT 1 FROM analyzer_container_receipts WHERE scan_id=?1 AND cleanup_status<>'confirmed')",
        [scan_id], |r|r.get(0),
    ).map_err(|_| "scan_quiescence_cleanup_lookup_failed")?;
    if unresolved { return Err("scan_quiescence_cleanup_unconfirmed".into()); }
    let mut attempts = connection.prepare(
        "SELECT attempt_count FROM sentinel_scans WHERE id=?1
         UNION SELECT attempt_number FROM native_scan_branches WHERE scan_id=?1
         UNION SELECT attempt_number FROM agent_runs WHERE scan_id=?1
         UNION SELECT attempt_number FROM sentinel_scan_attempts WHERE scan_id=?1
         UNION SELECT json_extract(contract_json,'$.root.attempt') FROM agent_root_budget_attempts
         WHERE json_extract(contract_json,'$.root.scan')=?1 ORDER BY 1",
    ).map_err(|_| "scan_quiescence_attempt_lookup_failed")?;
    let attempts = attempts.query_map([scan_id], |r|r.get::<_,i64>(0))
        .map_err(|_| "scan_quiescence_attempt_lookup_failed")?.collect::<Result<Vec<_>,_>>()
        .map_err(|_| "scan_quiescence_attempt_lookup_failed")?;
    let mut targets = connection.prepare(
        "SELECT url FROM sentinel_targets WHERE scan_id=?1
         UNION SELECT target_url FROM agent_runs WHERE scan_id=?1
         UNION SELECT json_extract(contract_json,'$.root.target') FROM agent_root_budget_attempts
         WHERE json_extract(contract_json,'$.root.scan')=?1 ORDER BY 1",
    ).map_err(|_| "scan_quiescence_target_lookup_failed")?;
    let targets = targets.query_map([scan_id], |r|r.get::<_,String>(0))
        .map_err(|_| "scan_quiescence_target_lookup_failed")?.collect::<Result<Vec<_>,_>>()
        .map_err(|_| "scan_quiescence_target_lookup_failed")?;
    let mut owners = Vec::new();
    for attempt in attempts {
        for (kind,target) in [("branch","web"),("branch","source"),("frontend_producer","web")] {
            owners.push(claim_native_invocation(db_path,scan_id,attempt,kind,target)
                .map_err(|_| "scan_quiescence_worker_active_or_unverifiable")?);
        }
        let mut paid_source = false;
        for target in &targets {
            let (source, web) = scan_quiescence_original_surfaces_in(connection, scan_id, attempt, target)?;
            paid_source |= source;
            for kind in ["target","frontend_recon"] {
                // A born Source Root owns the source-model caller, never a Web
                // target caller. Do not create a Web inode to pretend it exited.
                if kind == "target" && source && !web { continue; }
                let owner = if kind == "target" && web {
                    crate::agent_runtime::execution_owner::probe_native_invocation(
                        db_path, scan_id, attempt, kind, target,
                    )
                    .map_err(|_| "scan_quiescence_worker_active_or_unverifiable")?
                    .ok_or("scan_quiescence_original_target_exit_missing")?
                } else {
                    claim_native_invocation(db_path, scan_id, attempt, kind, target)
                        .map_err(|_| "scan_quiescence_worker_active_or_unverifiable")?
                };
                owners.push(owner);
            }
        }
        if paid_source {
            if let Some(parents)=parents.as_mut() {
                parents.hold_existing(db_path,scan_id,attempt)
                    .map_err(|e|if e=="scan_quiescence_original_source_exit_missing" {e} else {"scan_quiescence_worker_active_or_unverifiable".into()})?;
                continue;
            }
        }
        let source_owner = if paid_source {
            crate::agent_runtime::execution_owner::probe_native_invocation(
                db_path, scan_id, attempt, "source-model", "source",
            )
            .map_err(|_| "scan_quiescence_worker_active_or_unverifiable")?
            .ok_or("scan_quiescence_original_source_exit_missing")?
        } else {
            // Also join a Source caller before its financial Root is born.
            claim_native_invocation(db_path, scan_id, attempt, "source-model", "source")
                .map_err(|_| "scan_quiescence_worker_active_or_unverifiable")?
        };
        owners.push(source_owner);
    }
    Ok(owners)
}

// Financial identity is read from the original immutable control, including
// attempts/targets removed from mutable projections. This read neither renews
// the Coordinator nor grants work. A changed plan/scope cannot reclassify Web
// as Source or escape the proof by changing the current run's target label.
fn scan_quiescence_original_surfaces_in(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt: i64,
    target: &str,
) -> Result<(bool, bool), String> {
    let mut query = connection
        .prepare(
            "SELECT b.root_run_id,r.plan_json FROM agent_root_budget_attempts b
         LEFT JOIN agent_runs r ON r.id=b.root_run_id
         WHERE json_extract(b.contract_json,'$.root.scan')=?1
         AND json_extract(b.contract_json,'$.root.attempt')=?2
         AND json_extract(b.contract_json,'$.root.target')=?3
         AND json_extract(b.contract_json,'$.root.policy')='multi' ORDER BY b.rowid",
        )
        .map_err(|_| "scan_quiescence_original_scope_unverifiable")?;
    let roots = query
        .query_map(params![scan_id, attempt, target], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| "scan_quiescence_original_scope_unverifiable")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "scan_quiescence_original_scope_unverifiable")?;
    let (mut source, mut web) = (false, false);
    for (root, text) in roots {
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
            connection, &root,
        )
        .map_err(|_| "scan_quiescence_original_scope_unverifiable")?;
        let plan: JsonValue = serde_json::from_str(&text)
            .map_err(|_| "scan_quiescence_original_scope_unverifiable")?;
        if plan["surface"] == "source" {
            crate::agent_runtime::multi_agent::source::SourcePhaseContract::from_plan(&plan)
                .map_err(|_| "scan_quiescence_original_scope_unverifiable")?;
            let mode: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_root_mode_definitions WHERE root_run_id=?1)",
                    [&root],
                    |r| r.get(0),
                )
                .map_err(|_| "scan_quiescence_original_scope_unverifiable")?;
            let canonical = serde_json::to_string(&plan)
                .map_err(|_| "scan_quiescence_original_scope_unverifiable")?;
            if mode
                || text != canonical
                || plan["targetRequestsGranted"] != 0
                || plan["hostActionsGranted"] != 0
            {
                return Err("scan_quiescence_original_scope_unverifiable".into());
            }
            source = true;
        } else {
            web = true;
        }
    }
    Ok((source, web))
}

const SCAN_PAUSED_CHECKPOINT: &str = "已暂停；本地登记的执行线程已退出，已有证据与请求占用保留。未知目标效果仍需核对，不自动恢复或重放。";

// Unlike the best-effort progress synchronizer, a pause must prove the entire
// attempt transition persisted. Capture expected values BEFORE the write so
// AFTER triggers cannot redefine our expected accounting or stop history.
fn sync_pause_attempt_in(connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
    status: &str, checkpoint: &str) -> Result<Vec<rusqlite::types::Value>, String> {
    if attempt == 0 { return Ok(Vec::new()); } // Legacy scans predating attempt records.
    if attempt < 0 { return Err("scan_pause_invalid_attempt".into()); }
    let terminal = status == "paused";
    let stage = sentinel_attempt_stage(status, checkpoint);
    let expected: Vec<rusqlite::types::Value> = connection.query_row(
        "SELECT ?3,?4,CASE WHEN trim(a.stop_reason)<>'' THEN a.checkpoint ELSE ?5 END,
         CASE WHEN ?6 AND trim(a.stop_reason)='' THEN ?5 ELSE a.stop_reason END,
         MAX(0,s.llm_requests-a.llm_requests_start),MAX(0,s.input_tokens-a.input_tokens_start),
         MAX(0,s.output_tokens-a.output_tokens_start),MAX(0,s.cached_tokens-a.cached_tokens_start),
         MAX(0,s.total_tokens-a.total_tokens_start),
         CASE WHEN ?6 AND a.finished_at='' THEN datetime('now','localtime') ELSE a.finished_at END,
         datetime('now','localtime')
         FROM sentinel_scan_attempts a JOIN sentinel_scans s ON s.id=a.scan_id
         WHERE a.scan_id=?1 AND a.attempt_number=?2 AND s.attempt_count=?2
         AND NOT EXISTS(SELECT 1 FROM native_web_attempt_closures WHERE scan_id=?1 AND attempt_number=?2)",
        params![scan_id,attempt,status,stage,checkpoint,terminal],
        |r|(0..11).map(|i|r.get(i)).collect(),
    ).map_err(|_| "scan_pause_attempt_unavailable")?;
    let mut values = expected.clone();
    values.push(scan_id.to_string().into());
    values.push(attempt.into());
    let changed = connection.execute(
        "UPDATE sentinel_scan_attempts SET status=?1,stage=?2,checkpoint=?3,stop_reason=?4,
         llm_requests_delta=?5,input_tokens_delta=?6,output_tokens_delta=?7,cached_tokens_delta=?8,total_tokens_delta=?9,
         finished_at=?10,updated_at=?11 WHERE scan_id=?12 AND attempt_number=?13",
        rusqlite::params_from_iter(values.iter()),
    ).map_err(|_| "scan_pause_attempt_write_failed")?;
    if changed!=1 { return Err("scan_pause_attempt_postcondition".into()); }
    verify_pause_attempt_in(connection,scan_id,attempt,&expected)?;
    Ok(expected)
}

fn verify_pause_attempt_in(connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
    expected: &[rusqlite::types::Value]) -> Result<(),String> {
    if attempt==0 { return Ok(()); }
    let actual: Vec<rusqlite::types::Value> = connection.query_row(
        "SELECT status,stage,checkpoint,stop_reason,llm_requests_delta,input_tokens_delta,output_tokens_delta,
         cached_tokens_delta,total_tokens_delta,finished_at,updated_at FROM sentinel_scan_attempts
         WHERE scan_id=?1 AND attempt_number=?2",params![scan_id,attempt],
        |r|(0..11).map(|i|r.get(i)).collect(),
    ).map_err(|_| "scan_pause_attempt_verify_failed")?;
    if actual!=expected { return Err("scan_pause_attempt_postcondition".into()); }
    Ok(())
}

fn request_sentinel_pause(db_path: &Path, scan_id: &str) -> Result<i64,String> {
    let _lifecycle = claim_scan_control(db_path,scan_id)?;
    let mut connection = db::open(db_path)?;
    connection.pragma_update(None,"synchronous","FULL").map_err(|e|e.to_string())?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    let (attempt,status):(i64,String) = tx.query_row(
        "SELECT attempt_count,status FROM sentinel_scans WHERE id=?1
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        [scan_id],|r|Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|_| "scan_pause_scope_unavailable")?;
    if !matches!(status.as_str(),"scanning"|"pausing") { return Err("scan_pause_state_ineligible".into()); }
    let before = scan_pause_preservation_hash(&tx,scan_id,attempt)?;
    let checkpoint = "暂停请求已接收；等待执行线程退出，未知目标效果与清理记录保留";
    let expected = sync_pause_attempt_in(&tx,scan_id,attempt,"pausing",checkpoint)?;
    let changed = tx.execute(
        "UPDATE sentinel_scans SET status='pausing',current_checkpoint='暂停请求已接收；等待执行线程退出，未知目标效果与清理记录保留',updated_at=datetime('now','localtime')
         WHERE id=?1 AND attempt_count=?2 AND status IN ('scanning','pausing')",params![scan_id,attempt],
    ).map_err(|e|e.to_string())?;
    verify_pause_attempt_in(&tx,scan_id,attempt,&expected)?;
    let valid:bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2 AND status='pausing' AND current_checkpoint=?3)",params![scan_id,attempt,checkpoint],|r|r.get(0)).map_err(|e|e.to_string())?;
    if changed!=1 || !valid || scan_pause_preservation_hash(&tx,scan_id,attempt)?!=before { return Err("scan_pause_request_not_persisted".into()); }
    tx.commit().map_err(|e|format!("scan_pause_request_commit_unconfirmed:{e}"))?;
    Ok(attempt)
}

fn scan_pause_preservation_hash(connection: &rusqlite::Connection, scan_id: &str, attempt: i64) -> Result<String,String> {
    let mut snapshot = Vec::new();
    for table in ["sentinel_scans","sentinel_scan_attempts","sentinel_targets","native_scan_branches",
        "native_branch_dispatches","native_web_dispatch_bindings","agent_runs","agent_budget_ledger",
        "agent_http_request_claims","agent_request_reviews","sentinel_processes","analyzer_container_receipts"] {
        let filter = match table {
            "sentinel_scans" => "id=?1",
            "agent_budget_ledger" => "root_run_id IN (SELECT id FROM agent_runs WHERE scan_id=?1)",
            _ => "scan_id=?1",
        };
        let excluded: &[&str] = match table {
            "sentinel_scans" => &["status","current_checkpoint","updated_at"],
            "sentinel_scan_attempts" => &["status","stage","checkpoint","stop_reason","finished_at","updated_at",
                "llm_requests_delta","input_tokens_delta","output_tokens_delta","cached_tokens_delta","total_tokens_delta"],
            _ => &[],
        };
        let mut statement = connection.prepare(&format!("SELECT * FROM {table} WHERE {filter} ORDER BY rowid")).map_err(|e|e.to_string())?;
        let columns = statement.column_names().iter().enumerate()
            .map(|(i,name)|(i,(*name).to_string())).collect::<Vec<_>>();
        let rows = statement.query_map([scan_id],|row| {
            let mutable = table!="sentinel_scan_attempts" || row.get::<_,i64>("attempt_number")?==attempt;
            columns.iter().filter(|(_,name)|!mutable || !excluded.contains(&name.as_str())).map(|(i,name)|
                row.get::<_,rusqlite::types::Value>(*i).map(|value|format!("{name}:{value:?}"))).collect::<Result<Vec<_>,_>>()
        })
            .map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        snapshot.push(json!([table,rows]));
    }
    Ok(agent_stable_hash(&json!(snapshot)))
}

fn finish_sentinel_pause(db_path: &Path, scan_id: &str, attempt: i64) -> Result<bool,String> {
    // Serialize finalizers through the database, not the UI lifecycle try-lock.
    // Otherwise the last worker can release its ownership while a pause command
    // holds that lock, lose its only completion callback, and remain pausing.
    // Exact-attempt checks and invocation locks stay held through commit.
    let mut connection = db::open(db_path)?;
    connection.pragma_update(None,"synchronous","FULL").map_err(|e|e.to_string())?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    let eligible: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2 AND status='pausing')
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        params![scan_id,attempt],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if !eligible { return Ok(false); }
    let _owners = claim_scan_quiescence_in(&tx,db_path,scan_id)?;
    if let Some(actor) = known_source_pause_root_in(&tx,scan_id,attempt)? {
        // Ownership stays held while the dedicated private writer rechecks all
        // originals and publishes Root + pause atomically. Never nest writers.
        tx.rollback().map_err(|e|e.to_string())?;
        return finish_known_source_pause(&connection,&actor);
    }
    publish_sentinel_pause_in(&tx,scan_id,attempt)?;
    tx.commit().map_err(|e|format!("scan_pause_commit_unconfirmed:{e}"))?;
    Ok(true)
}

fn publish_sentinel_pause_in(tx: &rusqlite::Transaction<'_>, scan_id: &str, attempt: i64) -> Result<(),String> {
    let before = scan_pause_preservation_hash(tx,scan_id,attempt)?;
    let expected = sync_pause_attempt_in(tx,scan_id,attempt,"paused",SCAN_PAUSED_CHECKPOINT)?;
    let changed = tx.execute(
        "UPDATE sentinel_scans SET status='paused',current_checkpoint=?3,updated_at=datetime('now','localtime')
         WHERE id=?1 AND attempt_count=?2 AND status='pausing'",
        params![scan_id,attempt,SCAN_PAUSED_CHECKPOINT],
    ).map_err(|e|e.to_string())?;
    verify_pause_attempt_in(tx,scan_id,attempt,&expected)?;
    let valid: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2
         AND status='paused' AND current_checkpoint=?3)",
        params![scan_id,attempt,SCAN_PAUSED_CHECKPOINT],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if changed!=1 || !valid || scan_pause_preservation_hash(tx,scan_id,attempt)?!=before {
        return Err("scan_pause_postcondition".into());
    }
    Ok(())
}

include!("native_source_pause_closure.rs");
include!("native_source_pause_recovery.rs");

// Held by the actual worker, including detached native-recon threads. Producer
// cancellation cannot drop this on their behalf. The last exiting owner can
// finalize pausing; a stale attempt callback cannot change a newer attempt.
struct ScanWorkerOwner {
    db_path: PathBuf,
    scan_id: String,
    attempt: i64,
    owner: Option<NativeInvocationOwner>,
}

impl ScanWorkerOwner {
    fn claim(db_path: &Path, scan_id: &str, attempt: i64, kind: &str, target: &str) -> Result<Self,String> {
        let owner = claim_native_invocation(db_path,scan_id,attempt,kind,target)?;
        if !native_web_attempt_active(db_path,scan_id,attempt) {
            return Err("scan_worker_attempt_inactive".into());
        }
        Ok(Self { db_path:db_path.into(),scan_id:scan_id.into(),attempt,owner:Some(owner) })
    }
}

impl Drop for ScanWorkerOwner {
    fn drop(&mut self) {
        drop(self.owner.take());
        if sentinel_scan_pause_requested(&self.db_path,&self.scan_id) {
            let _ = finish_sentinel_pause(&self.db_path,&self.scan_id,self.attempt);
        }
    }
}
