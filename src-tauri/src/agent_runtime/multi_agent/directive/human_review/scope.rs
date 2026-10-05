use super::*;
use crate::agent_runtime::multi_agent::lease::{require_active_attempt, require_open_coordinator};

pub(super) fn current(connection: &Connection, draft: &HumanDirectiveDraft) -> Result<(), String> {
    require_active_attempt(connection, &draft.scan_id, draft.attempt_number)?;
    require_open_coordinator(
        connection,
        &draft.scan_id,
        draft.attempt_number,
        &draft.target_key,
        &draft.root_run_id,
    )?;
    if draft.bound_lease_epoch <= 0 || draft.bound_fencing_token.is_empty() {
        return Err("directive_draft_recipient_not_bound".into());
    }
    let bound: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases
        WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4
        AND lease_epoch=?5 AND fencing_token=?6 AND lease_expires_at>datetime('now','localtime'))",
            params![
                draft.scan_id,
                draft.attempt_number,
                draft.target_key,
                draft.root_run_id,
                draft.bound_lease_epoch,
                draft.bound_fencing_token
            ],
            |r| r.get(0),
        )
        .map_err(|_| "directive_human_review_binding_unavailable")?;
    if !bound {
        return Err("directive_draft_stale_fencing_token".into());
    }
    draft_store::validate_thread_key(
        connection,
        &draft.root_run_id,
        &draft.target_key,
        &draft.thread_key,
    )
}

/// Full rows, including complete JSON strings, of the original Root and Native
/// Source/attempt contracts. This catches trigger damage without reading CAS.
pub(super) fn protected(
    connection: &Connection,
    draft: &HumanDirectiveDraft,
) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for (query, parameter) in [
        (
            "SELECT * FROM agent_runs WHERE id=?1 ORDER BY rowid",
            draft.root_run_id.as_str(),
        ),
        (
            "SELECT * FROM agent_coordinator_leases WHERE root_run_id=?1 ORDER BY rowid",
            draft.root_run_id.as_str(),
        ),
        (
            "SELECT * FROM agent_budget_ledger WHERE root_run_id=?1 ORDER BY rowid",
            draft.root_run_id.as_str(),
        ),
        (
            "SELECT * FROM agent_budget_entries WHERE root_run_id=?1 ORDER BY rowid",
            draft.root_run_id.as_str(),
        ),
        (
            "SELECT * FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY rowid",
            draft.root_run_id.as_str(),
        ),
        (
            "SELECT * FROM native_branch_dispatches WHERE scan_id=?1 ORDER BY rowid",
            draft.scan_id.as_str(),
        ),
        (
            "SELECT * FROM sentinel_scans WHERE id=?1 ORDER BY rowid",
            draft.scan_id.as_str(),
        ),
        (
            "SELECT * FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY rowid",
            draft.scan_id.as_str(),
        ),
        (
            "SELECT * FROM sentinel_checkpoints WHERE scan_id=?1 ORDER BY rowid",
            draft.scan_id.as_str(),
        ),
        (
            "SELECT * FROM source_scope_contracts WHERE scan_id=?1 ORDER BY rowid",
            draft.scan_id.as_str(),
        ),
        (
            "SELECT * FROM source_runtime_contracts WHERE scan_id=?1 ORDER BY rowid",
            draft.scan_id.as_str(),
        ),
        (
            "SELECT * FROM source_analysis_views WHERE scan_id=?1 ORDER BY rowid",
            draft.scan_id.as_str(),
        ),
        (
            "SELECT * FROM source_analysis_results WHERE scan_id=?1 ORDER BY rowid",
            draft.scan_id.as_str(),
        ),
        (
            "SELECT * FROM source_snapshots WHERE scan_id=?1 ORDER BY rowid",
            draft.scan_id.as_str(),
        ),
    ] {
        let mut statement = connection
            .prepare(query)
            .map_err(|_| "directive_human_review_scope_unavailable")?;
        let columns = statement.column_count();
        let rows = statement
            .query_map([parameter], |row| {
                let values = (0..columns)
                    .map(|i| row.get::<_, rusqlite::types::Value>(i))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(crate::agent_runtime::store::stable_hash(&format!(
                    "{values:?}"
                )))
            })
            .map_err(|_| "directive_human_review_scope_unavailable")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "directive_human_review_scope_unavailable")?;
        result.push(query.into());
        result.extend(rows);
    }
    Ok(result)
}

pub(super) fn same_frozen(original: &HumanDirectiveDraft, after: &HumanDirectiveDraft) -> bool {
    let mut normalized = after.clone();
    normalized.status = original.status.clone();
    normalized.confirmed_directive_id = original.confirmed_directive_id.clone();
    normalized == *original && draft_store::draft_integrity_valid(after)
}
