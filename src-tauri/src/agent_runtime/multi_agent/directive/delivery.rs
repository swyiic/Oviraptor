use super::super::lease::{
    require_executable_coordinator, validate_coordinator_lease, CoordinatorLease,
};
use super::{
    draft_store::{draft_integrity_valid, load_draft, validate_thread_key},
    fact_refs_current,
    inbox_claim::transition_directive_in_transaction,
    UserDirective,
};
use rusqlite::{params, Connection, TransactionBehavior};

/// Rehydrate the durable inbox on every model round, including after a crash
/// between claim and acceptance. Acceptance is not action application. A
/// replaced lease must never silently adopt a previous worker's instruction.
pub fn prepare_model_context(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<Vec<UserDirective>, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定用户指令上下文：{error}"))?;
    let result = prepare_model_context_in_transaction(&transaction, lease)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(result)
}

pub(super) fn prepare_model_context_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
) -> Result<Vec<UserDirective>, String> {
    validate_coordinator_lease(transaction, lease)?;
    require_executable_coordinator(transaction, lease)?;
    transaction.execute(
        "UPDATE agent_user_directives SET status='deferred',rejection_code='directive_reconfirmation_required',\
         updated_at=datetime('now','localtime') WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 \
         AND status IN ('claimed','accepted') \
         AND (claim_run_id<>?4 OR claim_lease_epoch<>?5 OR claim_fencing_token<>?6)",
        params![lease.scan_id, lease.attempt_number, lease.target_key, lease.root_run_id,
            lease.lease_epoch, lease.fencing_token],
    ).map_err(|error| format!("无法隔离旧租约用户指令：{error}"))?;
    let items = {
        let mut query = transaction.prepare(
            "SELECT id,text_redacted,status,source_draft_id,confirmed_revision,confirmed_hash,thread_key \
             FROM agent_user_directives WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 \
             AND root_run_id=?4 AND claim_run_id=?4 AND claim_lease_epoch=?5 AND claim_fencing_token=?6 \
             AND status IN ('claimed','accepted') ORDER BY rowid",
        ).map_err(|error| error.to_string())?;
        let rows = query
            .query_map(
                params![
                    lease.scan_id,
                    lease.attempt_number,
                    lease.target_key,
                    lease.root_run_id,
                    lease.lease_epoch,
                    lease.fencing_token
                ],
                |row| {
                    Ok((
                        UserDirective {
                            id: row.get(0)?,
                            text: row.get(1)?,
                            status: row.get(2)?,
                        },
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let mut result = Vec::new();
    for (mut item, source, revision, hash, thread) in items {
        let draft = load_draft(transaction, &source)?;
        let valid = match draft {
            Some(draft) => {
                draft_integrity_valid(&draft)
                    && draft.status == "confirmed"
                    && draft.confirmed_directive_id == item.id
                    && draft.scan_id == lease.scan_id
                    && draft.attempt_number == lease.attempt_number
                    && draft.revision == revision
                    && draft.draft_hash == hash
                    && draft.safe_execution_text == item.text
                    && draft.thread_key == thread
                    && draft.validation_result != "rejected"
                    && ((draft.root_run_id.is_empty()
                        && draft.target_key.is_empty()
                        && draft.bound_lease_epoch == 0
                        && draft.bound_fencing_token.is_empty())
                        || (draft.root_run_id == lease.root_run_id
                            && draft.target_key == lease.target_key
                            && draft.bound_lease_epoch == lease.lease_epoch
                            && draft.bound_fencing_token == lease.fencing_token))
                    && validate_thread_key(
                        transaction,
                        &lease.root_run_id,
                        &lease.target_key,
                        &thread,
                    )
                    .is_ok()
                    && fact_refs_current(
                        transaction,
                        &lease.root_run_id,
                        &draft.referenced_fact_ids,
                    )?
            }
            None => false,
        };
        if !valid {
            transaction.execute(
                "UPDATE agent_user_directives SET status='rejected',rejection_code='directive_context_invalid',\
                 finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1",
                [&item.id],
            ).map_err(|error| error.to_string())?;
            continue;
        }
        if item.status == "claimed" {
            transition_directive_in_transaction(
                transaction,
                lease,
                &item.id,
                "claimed",
                "accepted",
            )?;
            item.status = "accepted".into();
        }
        result.push(item);
    }
    Ok(result)
}

/// The caller owns the transaction containing ModelRoundCompleted. The receipt
/// proves transport delivery, never a priority change, assignment or result.
pub fn record_model_delivery(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    run_id: &str,
    sequence: i64,
    items: &[UserDirective],
) -> Result<(), String> {
    validate_coordinator_lease(transaction, lease)?;
    let matching_run: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 \
         AND target_url=?4 AND COALESCE(NULLIF(root_run_id,''),id)=?5)",
        params![run_id, lease.scan_id, lease.attempt_number, lease.target_key, lease.root_run_id],
        |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if !matching_run {
        return Err("directive_delivery_run_scope_mismatch".into());
    }
    for item in items {
        let changed = transaction.execute(
            "UPDATE agent_user_directives SET payload_json=json_set(payload_json,'$.modelDelivery',\
             json_object('state','model_received','runId',?1,'eventSequence',?2,'receivedAt',datetime('now','localtime'))),\
             updated_at=datetime('now','localtime') WHERE id=?3 AND status='accepted' \
             AND scan_id=?4 AND attempt_number=?5 AND target_key=?6 AND root_run_id=?7 \
             AND claim_run_id=?7 AND claim_lease_epoch=?8 AND claim_fencing_token=?9 AND text_redacted=?10",
            params![run_id, sequence, item.id, lease.scan_id, lease.attempt_number, lease.target_key,
                lease.root_run_id, lease.lease_epoch, lease.fencing_token, item.text],
        ).map_err(|error| format!("无法保存用户指令送达回执：{error}"))?;
        if changed != 1 {
            return Err("directive_delivery_fencing_or_state_conflict".into());
        }
    }
    Ok(())
}
