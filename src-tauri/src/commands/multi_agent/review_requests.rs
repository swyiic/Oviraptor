#[allow(clippy::too_many_arguments)]
fn open_review_request(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    reviewer: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    request_id: &str,
    candidate_id: &str,
    revision: i64,
    candidate_text: &str,
    target_dir: &Path,
) -> Result<(), String> {
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(connection, lease)?;
    let transaction = rusqlite::Transaction::new_unchecked(
        connection,
        rusqlite::TransactionBehavior::Immediate,
    )
    .map_err(|error| format!("无法锁定 Reviewer 请求：{error}"))?;
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&transaction, lease)?;
    let (evidence_revision, manifest_hash) = review_snapshot_manifest(
        &transaction, &lease.root_run_id, target_dir, candidate_text,
    )?;
    transaction
        .execute(
            "UPDATE agent_review_requests SET status='superseded',updated_at=datetime('now','localtime') \
             WHERE root_run_id=?1 AND candidate_id=?2 AND candidate_revision<>?3 \
             AND status IN ('pending','running','confirmed','rejected','insufficient_evidence','needs_evidence')",
            params![lease.root_run_id, candidate_id, revision],
        )
        .map_err(|error| format!("无法收口旧 Reviewer 请求：{error}"))?;
    transaction
        .execute(
            "INSERT INTO agent_review_requests(id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,candidate_json,evidence_revision,manifest_hash,status,lease_epoch,fencing_token) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'running',?10,?11) \
             ON CONFLICT(candidate_id,candidate_revision) DO NOTHING",
            params![
                request_id,
                lease.root_run_id,
                reviewer.assignment_id,
                reviewer.run_id,
                candidate_id,
                revision,
                candidate_text,
                evidence_revision,
                manifest_hash,
                lease.lease_epoch,
                lease.fencing_token
            ],
        )
        .map_err(|error| format!("无法保存 Reviewer 请求：{error}"))?;
    let stored: (String, String, String, String, String, i64, String, i64, String, i64, String) = transaction
        .query_row(
            "SELECT id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,candidate_json,lease_epoch,fencing_token,evidence_revision,manifest_hash \
             FROM agent_review_requests WHERE candidate_id=?1 AND candidate_revision=?2",
            params![candidate_id, revision],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ))
            },
        )
        .map_err(|error| format!("无法确认 Reviewer 请求：{error}"))?;
    if stored
        != (
            request_id.to_string(),
            lease.root_run_id.clone(),
            reviewer.assignment_id.clone(),
            reviewer.run_id.clone(),
            candidate_id.to_string(),
            revision,
            candidate_text.to_string(),
            lease.lease_epoch,
            lease.fencing_token.clone(),
            evidence_revision,
            manifest_hash,
        )
    {
        return Err("review_request_replay_conflict".into());
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交 Reviewer 请求：{error}"))
}

fn fail_review_request(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    request_id: &str,
) -> Result<(), String> {
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(connection, lease)?;
    let changed = connection
        .execute(
            "UPDATE agent_review_requests SET status='failed',finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
             WHERE id=?1 AND root_run_id=?2 AND lease_epoch=?3 AND fencing_token=?4 AND status IN ('pending','running')",
            params![
                request_id,
                lease.root_run_id,
                lease.lease_epoch,
                lease.fencing_token
            ],
        )
        .map_err(|error| format!("无法失败收口 Reviewer 请求：{error}"))?;
    if changed == 1 {
        return Ok(());
    }
    let already_decided: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_review_requests WHERE id=?1 AND root_run_id=?2 \
             AND lease_epoch=?3 AND fencing_token=?4 AND status IN ('confirmed','rejected','insufficient_evidence','needs_evidence','failed'))",
            params![
                request_id,
                lease.root_run_id,
                lease.lease_epoch,
                lease.fencing_token
            ],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法确认 Reviewer 请求失败收口：{error}"))?;
    if already_decided {
        Ok(())
    } else {
        Err("review_request_failure_fencing_or_state_conflict".into())
    }
}

fn finish_failed_review(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    reviewer: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    request_id: &str,
    reason: &str,
) -> Result<(), String> {
    let transaction = rusqlite::Transaction::new_unchecked(
        connection, rusqlite::TransactionBehavior::Immediate,
    ).map_err(|error| format!("review_failure_cleanup_lock:{error}"))?;
    fail_review_request(&transaction, lease, request_id)?;
    stop_failed_child_preserving_usage_in_transaction(&transaction, lease, reviewer, reason)?;
    transaction.commit().map_err(|error| format!("review_failure_cleanup_commit:{error}"))
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn persist_review_decision(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    reviewer: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    request_id: &str,
    candidate_id: &str,
    revision: i64,
    verdict: &str,
    reason_codes: &JsonValue,
    missing_evidence: &JsonValue,
    confidence: f64,
    summary: &str,
    target_dir: &Path,
) -> Result<String, String> {
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(connection, lease)?;
    let transaction = rusqlite::Transaction::new_unchecked(
        connection,
        rusqlite::TransactionBehavior::Immediate,
    )
    .map_err(|error| format!("无法锁定 Reviewer 决策：{error}"))?;
    let message_id = persist_review_decision_in_transaction(
        &transaction, lease, reviewer, request_id, candidate_id, revision, verdict,
        reason_codes, missing_evidence, confidence, summary, target_dir, false,
    )?;
    transaction.commit().map_err(|error| format!("无法提交 Reviewer 决策：{error}"))?;
    Ok(message_id)
}

