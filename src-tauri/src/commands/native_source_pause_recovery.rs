// Explicit consumption of a closed original Native failure. No execution,
// live C, renewal, new attempt, refund or missing receipt can be manufactured.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePauseResultReceipt {
    schema_version: u8,
    scan_id: String,
    attempt_number: i64,
    root_run_id: String,
    scan_status: &'static str,
    branch_status: &'static str,
    changed: bool,
    execution_replayed: bool,
}

#[tauri::command]
pub fn recover_native_source_pause_result(
    state: State<AppState>,
    scan_id: String,
    attempt_number: i64,
) -> Result<SourcePauseResultReceipt, String> {
    recover_native_source_pause_result_inner(&state.db_path, &scan_id, attempt_number)
}

fn recover_native_source_pause_result_inner(
    path: &Path,
    scan: &str,
    attempt: i64,
) -> Result<SourcePauseResultReceipt, String> {
    use crate::agent_runtime::multi_agent::{source_rounds, specialist};
    if attempt <= 0 || scan.is_empty() {
        return Err("source_pause_recovery_scope_invalid".into());
    }
    let _control = claim_scan_control(path, scan)?;
    source_pause_recovery_write(path, |tx| {
        let eligible:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN sentinel_scan_attempts a
            ON a.scan_id=s.id AND a.attempt_number=s.attempt_count WHERE s.id=?1 AND s.attempt_count=?2
            AND s.status='paused' AND a.status='paused' AND s.source_path<>'')
            AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",params![scan,attempt],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !eligible {
            return Err("source_pause_recovery_original_paused_attempt_required".into());
        }
        let _parents = claim_scan_quiescence_in(tx, path, scan)?;
        let roots = source_branch_original_roots(tx, scan, attempt)?;
        let [root] = roots.as_slice() else {
            return Err("source_pause_recovery_original_root_required".into());
        };
        let actor = native_source_fresh_finance::original_for_financial_exit(tx, root)?;
        if actor.scan_id != scan || actor.attempt_number != attempt {
            return Err("source_pause_recovery_original_scope_changed".into());
        }
        let original = AgentTargetOutcome::incomplete(
            "源码多智能体执行未完成，子任务回执和未决预算保留待核对",
        );
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND status='terminal'
            AND terminal_state=?2 AND terminal_code=?3 AND terminal_reason=?4 AND cancel_requested_at='' AND finished_at<>'')",
            params![root,original.terminal_status(),original.terminal_code(),crate::agent_runtime::secrets::redact_text_with(&original.detail(),None)],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !exact {
            return Err("source_pause_recovery_original_closed_failure_required".into());
        }
        // All original local SDK/parent guards are retained to COMMIT. This
        // pure read accepts expired TTL only; original epoch/fence still match.
        let _sdk = source_rounds::require_idle_for_root(tx, &actor)?;
        let _specialists = specialist::require_idle_for_root(tx, &actor)?;
        known_source_pause_branch_outcome_in(tx, &actor)?;
        let (status,checkpoint,report):(String,String,String)=tx.query_row("SELECT status,checkpoint,report_json
            FROM native_scan_branches b WHERE scan_id=?1 AND attempt_number=?2 AND branch='source'
            AND EXISTS(SELECT 1 FROM native_branch_dispatches d WHERE d.scan_id=b.scan_id
                AND d.attempt_number=b.attempt_number AND d.branch=b.branch AND d.claim_id<>'' AND d.claimed_at<>'')",
            params![scan,attempt],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"source_pause_recovery_original_branch_required")?;
        let changed = match status.as_str() {
            "pending" if checkpoint.is_empty() && report == "{}" => {
                publish_known_source_pause_branch_in(tx, &actor)?;
                true
            }
            "partial" => false,
            _ => return Err("source_pause_recovery_original_result_conflict".into()),
        };
        let plan: String = tx
            .query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        // The complete original runtime/CI/material/worker/cost/Exit/target
        // audit holds the same write lock. Any failure rolls back the six cells.
        verify_exhausted_source_for_deletion(tx, &actor, &plan)?;
        Ok(SourcePauseResultReceipt {
            schema_version: 1,
            scan_id: scan.into(),
            attempt_number: attempt,
            root_run_id: root.clone(),
            scan_status: "paused",
            branch_status: "partial",
            changed,
            execution_replayed: false,
        })
    })
}

include!("native_source_pause_recovery_writer.rs");
