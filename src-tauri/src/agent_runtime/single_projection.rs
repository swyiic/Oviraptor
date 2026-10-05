//! Financial closure and original Single publication. No grant or history fork.
use super::multi_agent::budget::root::RootOwner;
use super::{
    reducer::{self, backend_exit::BackendExit, Reduction},
    runtime_adapter::{snapshot, BackendReport},
    store,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::path::Path;
pub(crate) mod deletion;
mod proof;
mod writer;
pub(crate) struct SingleReport<'a> {
    pub report: &'a BackendReport,
    pub exit: BackendExit,
    pub code: &'a str,
    pub reason: &'a str,
    pub source: &'a str,
}
pub(crate) fn source_hash(db: &Connection, report: &BackendReport) -> Result<String, String> {
    proof::hash(db,"SELECT rowid,* FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage='native_agent_state' ORDER BY rowid",params![report.scan_id,report.target_url])
}
pub(crate) fn close(
    path: &Path,
    root: &str,
    input: &SingleReport<'_>,
) -> Result<Option<Reduction>, String> {
    // Do not replace a caller's authorizer or transaction. Own this connection.
    let db = crate::db::open(path)?;
    require_scope(&db, root, input.report)?;
    let owner = RootOwner::load_single(&db, root)?;
    let financial = owner.close_single_finance(&db)?;
    let (finance_id, control, cutoff) = financial.reference();
    writer::write(&db, |tx| {
        let report = input.report;
        let run = require_scope(tx, root, report)?;
        let saved:Option<(String,String,String,String)>=tx.query_row("SELECT receipt_id,control_id,financial_receipt_id,proof_json FROM agent_single_projection_receipts WHERE root_run_id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
        if let Some((id, old_control, old_finance, text)) = saved {
            let saved: proof::Proof =
                serde_json::from_str(&text).map_err(|_| "single_projection_saved_proof_invalid")?;
            if uuid::Uuid::parse_str(&id).is_err()
                || old_control != control
                || old_finance != finance_id
                || serde_json::to_string(&saved).map_err(|e| e.to_string())? != text
            {
                return Err("single_projection_saved_proof_conflict".into());
            }
            saved.verify(tx, root, control, finance_id, cutoff)?;
            if saved.source != source_hash(tx, report)? {
                return Err("single_projection_source_conflict".into());
            }
            return if saved.paused {
                Ok(None)
            } else {
                Ok(Some(Reduction {
                    state: run
                        .terminal_state
                        .ok_or("single_projection_terminal_missing")?,
                    code: saved.code,
                    reason: saved.reason,
                }))
            };
        }
        if run.is_terminal() {
            return Err("single_projection_terminal_requires_original_receipt".into());
        }
        if input.source != source_hash(tx, report)? {
            return Err("single_projection_source_conflict".into());
        }
        let (active, status): (i64, String) = tx
            .query_row(
                "SELECT attempt_count,status FROM sentinel_scans WHERE id=?1",
                [&report.scan_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if active != report.attempt_number {
            return Err("single_projection_attempt_replaced_finance_only".into());
        }
        let paused = report.cancelled && report.resumable && status == "pausing";
        let unsettled = if matches!(input.exit, BackendExit::Completed | BackendExit::Bounded) {
            super::multi_agent::budget::admission::require_settled_for_completion(tx, root).err()
        } else {
            None
        };
        let reduction = if paused {
            None
        } else if let Some(error) = unsettled {
            Some(reducer::backend_exit::reduce_backend_exit(
                &report.signals(),
                BackendExit::Incomplete,
                super::contract::terminal_code::REQUEST_RECONCILIATION_REQUIRED,
                &format!("原费用尚未结清：{error}；原退出：{}", input.reason),
            ))
        } else if financial.exhausted
            && matches!(input.exit, BackendExit::Completed | BackendExit::Bounded)
        {
            Some(reducer::backend_exit::reduce_backend_exit(
                &report.signals(),
                BackendExit::Limited,
                super::contract::terminal_code::HARD_WALL_TIME_BUDGET,
                &format!(
                    "原 Root elapsed={}ms，hard 已耗尽；原退出：{}",
                    financial.elapsed_ms, input.reason
                ),
            ))
        } else {
            Some(reducer::backend_exit::reduce_backend_exit(
                &report.signals(),
                input.exit,
                input.code,
                input.reason,
            ))
        };
        let terminal = reduction.as_ref().map(|r| r.state.as_str()).unwrap_or("");
        let code = reduction.as_ref().map(|r| r.code.as_str()).unwrap_or("");
        let reason = reduction.as_ref().map(|r| r.reason.as_str()).unwrap_or("");
        let status = if paused { "paused" } else { "terminal" };
        let finished = if paused { "" } else { cutoff };
        let before = proof::Before::capture(tx, root)?;
        let changed=tx.execute("UPDATE agent_runs SET status=?2,terminal_state=?3,terminal_code=?4,terminal_reason=?5,finished_at=?6,updated_at=?7 WHERE id=?1 AND status IN ('prepared','running','paused') AND terminal_state='' AND finished_at=''",
            params![root,status,terminal,code,reason,finished,cutoff]).map_err(|e|e.to_string())?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND status=?2 AND terminal_state=?3 AND terminal_code=?4 AND terminal_reason=?5 AND finished_at=?6 AND updated_at=?7)",params![root,status,terminal,code,reason,finished,cutoff],|r|r.get(0)).map_err(|e|e.to_string())?;
        if changed != 1 || !exact {
            return Err("single_projection_root_write_conflict".into());
        }
        let kind = if paused {
            super::contract::AgentEventKind::SnapshotWritten
        } else {
            super::contract::AgentEventKind::TerminalReduced
        };
        let payload = super::secrets::redact_json(&match &reduction {
            Some(r) => r.as_json(),
            None => {
                json!({"status":"paused","reason":input.reason,"financialReceiptId":finance_id,"cutoff":cutoff,"resumeAuthorized":false})
            }
        });
        let sequence = store::append_event(tx, root, kind, &payload, &[])?;
        let mut paid_report = report.clone();
        paid_report.usage = original_usage(tx, root)?;
        let mut state = snapshot(root, &paid_report);
        state.last_sequence = sequence;
        if let Some(r) = &reduction {
            state.terminal = Some(r.state);
            state.terminal_code = r.code.clone();
        }
        let expected = super::secrets::redact_json(&state.as_json());
        store::write_snapshot(tx, root, sequence, &expected)?;
        let snapshot:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_snapshots WHERE run_id=?1 AND last_sequence=?2 AND schema_version=?3 AND snapshot_json=?4)",params![root,sequence,super::contract::RUNTIME_SCHEMA_VERSION,expected.to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !snapshot {
            return Err("single_projection_snapshot_write_conflict".into());
        }
        let (event_id, collaboration_sequence) = before.verify(
            tx,
            root,
            report,
            (kind.as_str(), &payload),
            paused,
            terminal,
        )?;
        let proof = proof::Proof {
            version: 1,
            root: root.into(),
            control: control.into(),
            financial: finance_id.into(),
            cutoff: cutoff.into(),
            source: input.source.into(),
            paused,
            terminal: terminal.into(),
            code: code.into(),
            reason: reason.into(),
            row: proof::hash(tx, "SELECT rowid,* FROM agent_runs WHERE id=?1", [root])?,
            snapshot: proof::hash(
                tx,
                "SELECT rowid,* FROM agent_snapshots WHERE run_id=?1",
                [root],
            )?,
            event_id,
            event: proof::hash(
                tx,
                "SELECT rowid,* FROM agent_events WHERE id=?1",
                [event_id],
            )?,
            collaboration_sequence,
            collaboration: proof::hash(
                tx,
                "SELECT rowid,* FROM agent_collaboration_events WHERE sequence=?1",
                [collaboration_sequence],
            )?,
        };
        proof.verify(tx, root, control, finance_id, cutoff)?;
        let text = serde_json::to_string(&proof).map_err(|e| e.to_string())?;
        let id = uuid::Uuid::new_v4().to_string();
        let changed=tx.execute("INSERT INTO agent_single_projection_receipts(receipt_id,root_run_id,control_id,financial_receipt_id,proof_json) VALUES(?1,?2,?3,?4,?5)",params![id,root,control,finance_id,text]).map_err(|e|e.to_string())?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_single_projection_receipts WHERE receipt_id=?1 AND root_run_id=?2 AND control_id=?3 AND financial_receipt_id=?4 AND proof_json=?5)",params![id,root,control,finance_id,text],|r|r.get(0)).map_err(|e|e.to_string())?;
        if changed != 1 || !exact {
            return Err("single_projection_receipt_write_conflict".into());
        }
        // Re-load the original immutable finance after every publication write.
        // Read-only verification avoids installing another authorizer here.
        financial.verify_original(tx, &RootOwner::load_single(tx, root)?)?;
        proof.verify(tx, root, control, finance_id, cutoff)?;
        Ok(reduction)
    })
}

fn require_scope(
    db: &Connection,
    root: &str,
    report: &BackendReport,
) -> Result<store::AgentRunRow, String> {
    let count:i64=db.query_row("SELECT count(*) FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role='coordinator'",params![report.scan_id,report.attempt_number,report.target_url],|r|r.get(0)).map_err(|e|e.to_string())?;
    let run = store::load_run(db, root)?.ok_or("single_projection_root_missing")?;
    if count != 1
        || run.scan_id != report.scan_id
        || run.attempt_number != report.attempt_number
        || run.target_url != report.target_url
        || run.backend != report.backend
        || run.plan_hash != report.plan_hash
    {
        return Err("single_projection_original_scope_conflict".into());
    }
    Ok(run)
}

// Reporting cannot charge or synthesize usage. Read the original committed
// classes, including cached input as a subset; late fees remain separate facts.
fn original_usage(db: &Connection, root: &str) -> Result<store::UsageDelta, String> {
    let consumed = |dimension| {
        super::multi_agent::budget::balance(db, root, None, dimension).map(|b| b.consumed)
    };
    let input_tokens = consumed("model_input_tokens")?;
    let cached_input_tokens = consumed("model_cached_tokens")?;
    let output_tokens = consumed("model_output_tokens")?;
    let total_tokens = input_tokens
        .checked_add(output_tokens)
        .ok_or("single_projection_token_sum_invalid")?;
    if cached_input_tokens > input_tokens {
        return Err("single_projection_cached_usage_invalid".into());
    }
    Ok(store::UsageDelta {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        total_tokens,
        model_requests: consumed("model_requests")?,
    })
}
