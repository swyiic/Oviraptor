use super::super::lease::{
    require_executable_coordinator, validate_coordinator_lease, CoordinatorLease,
};
use super::{
    draft_store::{draft_integrity_valid, load_draft, validate_thread_key},
    fact_refs_current, UserDirective,
};
use rusqlite::{params, OptionalExtension};
#[cfg(test)]
use rusqlite::{Connection, TransactionBehavior};

// Storage-only test wrapper. Live collection uses the original-parent transaction.
#[cfg(test)]
pub fn claim_pending_directives(
    connection: &Connection,
    lease: &CoordinatorLease,
    limit: i64,
) -> Result<Vec<UserDirective>, String> {
    validate_coordinator_lease(connection, lease)?;
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定用户指令：{error}"))?;
    let directives = claim_pending_in_transaction(&transaction, lease, limit)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交用户指令 claim：{error}"))?;
    Ok(directives)
}

pub(super) fn claim_pending_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    limit: i64,
) -> Result<Vec<UserDirective>, String> {
    validate_coordinator_lease(transaction, lease)?;
    require_executable_coordinator(transaction, lease)?;
    // Upgrade safety: rows created by the pre-draft API, hand-written SQL, or a
    // partially migrated client must never become executable. Keep them for
    // audit, but fail them closed before selecting work for the Coordinator.
    transaction
        .execute(
            "UPDATE agent_user_directives AS directive SET status='rejected',\
             rejection_code='unconfirmed_directive_blocked',finished_at=datetime('now','localtime'),\
             updated_at=datetime('now','localtime') \
             WHERE directive.scan_id=?1 AND directive.attempt_number=?2 AND directive.status='pending' \
             AND (directive.target_key='' OR directive.target_key=?4) \
             AND NOT EXISTS(\
               SELECT 1 FROM agent_directive_drafts AS draft \
               WHERE draft.id=directive.source_draft_id AND draft.scan_id=directive.scan_id \
                 AND draft.attempt_number=directive.attempt_number AND draft.status='confirmed' \
                 AND draft.confirmed_directive_id=directive.id AND draft.confirmed_at<>'' \
                 AND draft.revision=directive.confirmed_revision AND draft.draft_hash=directive.confirmed_hash \
                 AND draft.validation_result<>'rejected' AND draft.confirmation_required=1 \
                 AND draft.safe_execution_text=directive.text_redacted AND directive.confirmation_at<>'' \
                 AND draft.thread_key=directive.thread_key \
                 AND directive.root_run_id=draft.root_run_id AND directive.target_key=draft.target_key \
                 AND draft.root_run_id=?3 AND draft.target_key=?4 AND draft.bound_lease_epoch=?5 \
                 AND draft.bound_fencing_token=?6\
             )",
            params![
                lease.scan_id,
                lease.attempt_number,
                lease.root_run_id,
                lease.target_key,
                lease.lease_epoch,
                lease.fencing_token
            ],
        )
        .map_err(|error| format!("无法隔离未确认用户指令：{error}"))?;
    let ids: Vec<String> = {
        let mut statement = transaction
            .prepare(
                "SELECT directive.id FROM agent_user_directives AS directive \
                 JOIN agent_directive_drafts AS draft ON draft.id=directive.source_draft_id \
                 WHERE directive.scan_id=?1 AND directive.attempt_number=?2 \
                   AND directive.status='pending' AND directive.root_run_id=?3 \
                   AND directive.target_key=?4 \
                   AND draft.scan_id=directive.scan_id AND draft.attempt_number=directive.attempt_number \
                   AND draft.status='confirmed' AND draft.confirmed_directive_id=directive.id AND draft.confirmed_at<>'' \
                   AND draft.revision=directive.confirmed_revision AND draft.draft_hash=directive.confirmed_hash \
                   AND draft.validation_result<>'rejected' AND draft.confirmation_required=1 \
                   AND draft.safe_execution_text=directive.text_redacted AND directive.confirmation_at<>'' \
                   AND draft.thread_key=directive.thread_key \
                   AND draft.root_run_id=?3 AND draft.target_key=?4 AND draft.bound_lease_epoch=?5 \
                   AND draft.bound_fencing_token=?6 \
                 ORDER BY directive.rowid LIMIT ?7",
            )
            .map_err(|error| format!("无法读取用户指令：{error}"))?;
        let rows = statement
            .query_map(
                params![
                    lease.scan_id,
                    lease.attempt_number,
                    lease.root_run_id,
                    lease.target_key,
                    lease.lease_epoch,
                    lease.fencing_token,
                    limit.clamp(1, 50)
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法遍历用户指令：{error}"))?
            .collect::<Result<_, _>>()
            .map_err(|error| format!("无法解码用户指令：{error}"))?;
        rows
    };
    let mut verified_ids = Vec::new();
    for id in &ids {
        let source: String = transaction
            .query_row(
                "SELECT source_draft_id FROM agent_user_directives WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法校验用户指令来源：{error}"))?;
        let draft = load_draft(transaction, &source)?;
        let valid = draft.as_ref().is_some_and(|draft| {
            draft_integrity_valid(draft)
                && validate_thread_key(
                    transaction,
                    &lease.root_run_id,
                    &lease.target_key,
                    &draft.thread_key,
                )
                .is_ok()
        });
        let fact_refs_valid = match (draft.as_ref(), valid) {
            (Some(draft), true) => {
                fact_refs_current(transaction, &lease.root_run_id, &draft.referenced_fact_ids)?
            }
            _ => false,
        };
        if !valid || !fact_refs_valid {
            let rejection_code = if !valid {
                "directive_draft_integrity_failed"
            } else {
                "directive_fact_reference_not_current"
            };
            transaction.execute(
                "UPDATE agent_user_directives SET status='rejected',rejection_code=?2,\
                 finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1 AND status='pending'",
                params![id, rejection_code],
            ).map_err(|error| format!("无法隔离失效用户指令：{error}"))?;
            continue;
        }
        transaction
            .execute(
                "UPDATE agent_user_directives SET status='claimed',root_run_id=?1,target_key=?2,claim_run_id=?1,\
                 claim_lease_epoch=?3,claim_fencing_token=?4,claimed_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
                 WHERE id=?5 AND status='pending'",
                params![
                    lease.root_run_id,
                    lease.target_key,
                    lease.lease_epoch,
                    lease.fencing_token,
                    id
                ],
            )
            .map_err(|error| format!("无法 claim 用户指令：{error}"))?;
        verified_ids.push(id);
    }
    let mut directives = Vec::new();
    for id in verified_ids {
        if let Some(row) = transaction
            .query_row(
                "SELECT id,text_redacted,status FROM agent_user_directives WHERE id=?1 AND claim_run_id=?2 AND claim_fencing_token=?3",
                params![id, lease.root_run_id, lease.fencing_token],
                |row| {
                    Ok(UserDirective {
                        id: row.get(0)?,
                        text: row.get(1)?,
                        status: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("无法确认用户指令 claim：{error}"))?
        {
            directives.push(row);
        }
    }
    Ok(directives)
}

#[cfg(test)]
pub fn transition_directive(
    connection: &Connection,
    lease: &CoordinatorLease,
    id: &str,
    from: &str,
    to: &str,
) -> Result<(), String> {
    // Validate under the same write lock as the transition: a lease can be
    // replaced while this connection waits for another SQLite writer.
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定用户指令状态：{error}"))?;
    transition_directive_in_transaction(&transaction, lease, id, from, to)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交用户指令状态：{error}"))
}

pub(super) fn transition_directive_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    id: &str,
    from: &str,
    to: &str,
) -> Result<(), String> {
    validate_coordinator_lease(transaction, lease)?;
    let allowed = matches!(
        (from, to),
        ("claimed", "accepted")
            | ("claimed", "rejected")
            | ("claimed", "deferred")
            | ("accepted", "assigned")
            | ("accepted", "applied")
            | ("accepted", "deferred")
            | ("assigned", "applied")
            | ("assigned", "failed")
            | ("applied", "completed")
            | ("applied", "failed")
    );
    if !allowed {
        return Err(format!("directive_transition_invalid:{from}:{to}"));
    }
    if matches!(to, "accepted" | "assigned" | "applied") {
        require_executable_coordinator(transaction, lease)?;
    }
    let stamp = match to {
        "accepted" => ",accepted_at=datetime('now','localtime')",
        "assigned" => ",assigned_at=datetime('now','localtime')",
        "applied" => ",applied_at=datetime('now','localtime')",
        "completed" | "failed" | "rejected" => ",finished_at=datetime('now','localtime')",
        _ => "",
    };
    let changed = transaction
        .execute(
            &format!(
                "UPDATE agent_user_directives SET status=?1{stamp},updated_at=datetime('now','localtime') \
                 WHERE id=?2 AND status=?3 AND claim_run_id=?4 AND claim_lease_epoch=?5 AND claim_fencing_token=?6 \
                 AND scan_id=?7 AND attempt_number=?8 AND target_key=?9"
            ),
            params![
                to,
                id,
                from,
                lease.root_run_id,
                lease.lease_epoch,
                lease.fencing_token,
                lease.scan_id,
                lease.attempt_number,
                lease.target_key
            ],
        )
        .map_err(|error| format!("无法更新用户指令：{error}"))?;
    if changed == 1 {
        Ok(())
    } else {
        Err("directive_fencing_or_state_conflict".into())
    }
}
