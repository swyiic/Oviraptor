//! Verify a saved original publication, without closing, repairing or granting.
use super::*;
pub(crate) fn verify_closed(db: &Connection, root: &str) -> Result<(), String> {
    let owner = RootOwner::load_single(db, root)?;
    let fact = owner.read_single_exit(db)?;
    if fact.unsettled_ms != 0 {
        return Err("single_deletion_elapsed_unsettled".into());
    }
    super::super::multi_agent::budget::admission::require_settled_for_completion(db, root)?;
    super::super::multi_agent::budget::root::remaining_child_capacity(db, root)?;
    let (id,control,financial,text):(String,String,String,String)=db.query_row(
        "SELECT receipt_id,control_id,financial_receipt_id,proof_json FROM agent_single_projection_receipts WHERE root_run_id=?1",
        [root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"single_deletion_original_publication_missing")?;
    let saved: proof::Proof =
        serde_json::from_str(&text).map_err(|_| "single_projection_saved_proof_invalid")?;
    let (original, original_control, cutoff) = fact.reference();
    if uuid::Uuid::parse_str(&id).is_err()
        || control != original_control
        || financial != original
        || serde_json::to_string(&saved).map_err(|e| e.to_string())? != text
        || saved.paused
        || matches!(
            saved.code.as_str(),
            "request_reconciliation_required" | "evidence_integrity"
        )
    {
        return Err("single_deletion_original_publication_conflict".into());
    }
    saved.verify(db, root, original_control, original, cutoff)?;
    let run = store::load_run(db, root)?.ok_or("single_deletion_root_missing")?;
    let count:i64=db.query_row("SELECT count(*) FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role='coordinator'",params![run.scan_id,run.attempt_number,run.target_url],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count!=1 || saved.source!=proof::hash(db,"SELECT rowid,* FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage='native_agent_state' ORDER BY rowid",params![run.scan_id,run.target_url])? {
        return Err("single_projection_source_conflict".into());
    }
    Ok(())
}
