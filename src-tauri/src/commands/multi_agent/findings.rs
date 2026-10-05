fn receive_expected_child_message(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    run_id: &str,
    message_id: &str,
    expected_kind: &str,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::mailbox;
    let message = mailbox::deliver_expected(connection, lease, run_id, message_id)?;
    if message.kind != expected_kind {
        return Err("mailbox_expected_kind_mismatch".into());
    }
    mailbox::acknowledge(connection, lease, run_id, message_id)?;
    Ok(message.payload)
}

fn agent_root_run_id(context: &AgentRunContext) -> Option<String> {
    let current = context.run.as_ref()?.run_id.clone();
    let connection = db::open(&context.db_path).ok()?;
    connection
        .query_row(
            "SELECT COALESCE(NULLIF(root_run_id,''),id) FROM agent_runs WHERE id=?1",
            [&current],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .or(Some(current))
}

#[allow(clippy::too_many_arguments)]
fn stage_agent_finding(
    context: &AgentRunContext,
    stage: &str,
    kind: &str,
    record_key: &str,
    title: &str,
    severity: &str,
    value: &JsonValue,
) -> Result<(), String> {
    stage_agent_finding_with_active_guard(
        context, stage, kind, record_key, title, severity, value, false,
    )
}

#[allow(clippy::too_many_arguments)]
fn stage_agent_finding_with_active_guard(
    context: &AgentRunContext,
    stage: &str,
    kind: &str,
    record_key: &str,
    title: &str,
    severity: &str,
    value: &JsonValue,
    require_active_attempt: bool,
) -> Result<(), String> {
    let Some(root_run_id) = agent_root_run_id(context) else {
        #[cfg(test)]
        {
            let connection = db::open(&context.db_path)?;
            return insert_finding(
                &connection,
                &context.scan_id,
                &context.target_url,
                stage,
                kind,
                record_key,
                title,
                severity,
                value,
            );
        }
        #[cfg(not(test))]
        return Err("agent_finding_root_run_missing".to_string());
    };
    let connection = db::open(&context.db_path)?;
    // Authorization needs a stronger boundary than an earlier cancel check:
    // the scan may pause after its third GET but before its finding is staged.
    // An immediate transaction serializes the pause transition and the write;
    // the post-write check also catches a pause performed by a DB trigger.
    let single = native_single_policy(&connection, context)?;
    let transaction = if require_active_attempt || single {
        Some(
            rusqlite::Transaction::new_unchecked(
                &connection,
                rusqlite::TransactionBehavior::Immediate,
            )
            .map_err(|error| format!("authorization_candidate_lock_failed:{error}"))?,
        )
    } else {
        None
    };
    let writer = transaction.as_deref().unwrap_or(&connection);
    let active_attempt = |db: &rusqlite::Connection| -> Result<(), String> {
        let active: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2 AND status='scanning')",
            params![context.scan_id, context.attempt_number], |row| row.get(0),
        ).map_err(|error| format!("authorization_candidate_status_failed:{error}"))?;
        if active {
            Ok(())
        } else {
            Err("authorization_attempt_cancelled".into())
        }
    };
    if require_active_attempt {
        active_attempt(writer)?;
    }
    if single {
        agent_single_require_fresh_on(writer, context)?;
    }
    let redacted = crate::agent_runtime::secrets::redact_json(value);
    let record_text = redacted.to_string();
    let redacted_title = crate::agent_runtime::secrets::redact_text_with(title, None);
    // Inventory, request facts and the coverage ledger are not vulnerability
    // claims. Keep their historical result surfaces, but do not manufacture a
    // CandidateFinding (and hence a Reviewer) solely to display them. The
    // allowlist is intentionally exact: an unknown type still takes the
    // stricter candidate gate instead of bypassing vulnerability review.
    let is_observation = matches!(
        (stage, kind),
        ("frontend-recon", "api")
            | (AGENT_EVIDENCE_STAGE, "evidence")
            | (AGENT_COVERAGE_STAGE, "coverage")
            | ("native-agent", "discovered_host")
    );
    if is_observation {
        let persist = || {
            insert_finding(
                writer,
                &context.scan_id,
                &context.target_url,
                stage,
                kind,
                record_key,
                &redacted_title,
                severity,
                &redacted,
            )
        };
        if single {
            single_finding_write(writer, persist)?;
        } else {
            persist()?;
        }
        if single {
            agent_single_require_fresh_on(writer, context)?;
        }
        if require_active_attempt {
            active_attempt(writer)?;
        }
        if let Some(transaction) = transaction {
            transaction.commit().map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let id = format!(
        "finding-candidate-{}",
        &crate::agent_runtime::store::stable_hash(&format!(
            "{root_run_id}:{}:{}:{stage}:{kind}:{record_key}",
            context.scan_id, context.target_url
        ))[..24]
    );
    let persist = || {
        writer
        .execute(
            "INSERT INTO agent_finding_candidates(id,root_run_id,scan_id,target_url,stage,kind,record_key,title,severity,record_json,status) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'pending') \
             ON CONFLICT(root_run_id,scan_id,target_url,stage,kind,record_key) DO UPDATE SET title=excluded.title,severity=excluded.severity,\
             record_json=excluded.record_json,candidate_revision=0,status='pending',reviewer_run_id='',published_at='',updated_at=datetime('now','localtime') \
             WHERE (agent_finding_candidates.title IS NOT excluded.title OR agent_finding_candidates.severity IS NOT excluded.severity \
             OR agent_finding_candidates.record_json IS NOT excluded.record_json) \
             AND NOT (agent_finding_candidates.status='pending' AND agent_finding_candidates.candidate_revision>0)",
            params![
                id,
                root_run_id,
                context.scan_id,
                context.target_url,
                stage,
                kind,
                record_key,
                redacted_title,
                severity,
                record_text
            ],
        )
        .map_err(|error| format!("无法暂存 Agent finding 候选：{error}"))
    };
    let changed = if single {
        single_finding_write(writer, persist)?
    } else {
        persist()?
    };
    if changed == 0 {
        let (existing_title, existing_severity, existing_record): (String, String, String) = writer
            .query_row(
                "SELECT title,severity,record_json FROM agent_finding_candidates WHERE root_run_id=?1 AND scan_id=?2 AND target_url=?3 AND stage=?4 AND kind=?5 AND record_key=?6",
                params![root_run_id, context.scan_id, context.target_url, stage, kind, record_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| format!("无法核对 Agent finding 候选：{error}"))?;
        if existing_title != redacted_title
            || existing_severity != severity
            || existing_record != record_text
        {
            return Err("finding_candidate_review_in_progress".into());
        }
    }
    if require_active_attempt {
        active_attempt(writer)?;
    }
    if single {
        agent_single_require_fresh_on(writer, context)?;
    }
    if let Some(transaction) = transaction {
        transaction
            .commit()
            .map_err(|error| format!("authorization_candidate_commit_failed:{error}"))?;
    }
    Ok(())
}

fn pending_agent_finding_candidates(
    connection: &rusqlite::Connection,
    root_run_id: &str,
) -> Result<Vec<JsonValue>, String> {
    let mut statement = connection
        .prepare(
            // An insufficiency is not a rejection. Keep its frozen finding in
            // the next candidate bundle if *new* evidence creates a revision.
            "SELECT id,stage,kind,record_key,title,severity,record_json FROM agent_finding_candidates \
             WHERE root_run_id=?1 AND status IN ('pending','insufficient_evidence') ORDER BY created_at,id",
        )
        .map_err(|error| format!("无法读取 Agent finding 候选：{error}"))?;
    let rows = statement
        .query_map([root_run_id], |row| {
            let record: String = row.get(6)?;
            let record: JsonValue = serde_json::from_str(&record).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(serde_json::json!({
                "id": row.get::<_,String>(0)?,
                "stage": row.get::<_,String>(1)?,
                "kind": row.get::<_,String>(2)?,
                "recordKey": row.get::<_,String>(3)?,
                "title": row.get::<_,String>(4)?,
                "severity": row.get::<_,String>(5)?,
                "record": record,
            }))
        })
        .map_err(|error| format!("无法遍历 Agent finding 候选：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解码 Agent finding 候选：{error}"))
}

fn review_attempt_active(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<bool, String> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2 AND status='scanning')",
        params![lease.scan_id, lease.attempt_number], |row| row.get(0),
    ).map_err(|error| format!("review_attempt_status_unavailable:{error}"))
}

#[cfg(test)]
fn settle_agent_finding_candidates(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    reviewer_run_id: &str,
    revision: i64,
    verdict: &str,
    target_dir: &Path,
) -> Result<(), String> {
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(connection, lease)?;
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定 Agent finding 候选：{error}"))?;
    settle_agent_finding_candidates_in_transaction(
        &transaction,
        lease,
        reviewer_run_id,
        revision,
        verdict,
        target_dir,
    )?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交 finding Reviewer 门禁：{error}"))
}

fn settle_agent_finding_candidates_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    reviewer_run_id: &str,
    revision: i64,
    verdict: &str,
    target_dir: &Path,
) -> Result<(), String> {
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(transaction, lease)?;
    if !review_attempt_active(transaction, lease)? {
        return Err("review_attempt_not_active".into());
    }
    if !matches!(verdict, "confirmed" | "rejected" | "insufficient_evidence") {
        return Err("review_verdict_invalid".into());
    }
    let (reviewed_json, candidate_id): (String, String) = transaction
        .query_row(
            "SELECT q.candidate_json,q.candidate_id FROM agent_review_requests q \
             JOIN agent_review_decisions d ON d.id=q.decision_id AND d.root_run_id=q.root_run_id \
               AND d.candidate_id=q.candidate_id AND d.candidate_revision=q.candidate_revision \
               AND d.reviewer_run_id=q.reviewer_run_id AND d.verdict=q.status \
             JOIN agent_runs r ON r.id=q.reviewer_run_id AND r.root_run_id=q.root_run_id \
               AND r.role='evidence_reviewer' AND r.lane='review' \
               AND r.status='terminal' AND r.terminal_state='completed' \
             JOIN agent_assignments a ON a.id=q.assignment_id AND a.coordinator_run_id=q.root_run_id \
               AND a.child_run_id=r.id AND a.state='completed' \
             JOIN agent_messages m ON m.root_run_id=q.root_run_id AND m.from_run_id=q.reviewer_run_id \
               AND m.to_run_id=q.root_run_id AND m.kind='review_decision' \
               AND m.assignment_id=q.assignment_id AND m.evidence_revision=q.candidate_revision \
               AND m.correlation_id=q.id \
               AND m.delivered_at<>'' AND m.acknowledged_at<>'' \
             WHERE q.root_run_id=?1 AND q.reviewer_run_id=?2 AND q.candidate_revision=?3 \
               AND q.status=?4 AND d.verdict=?4 AND q.lease_epoch=?5 AND q.fencing_token=?6 \
             LIMIT 1",
            params![lease.root_run_id, reviewer_run_id, revision, verdict, lease.lease_epoch, lease.fencing_token],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("无法核验 Reviewer 发布凭据：{error}"))?
        .ok_or_else(|| "review_decision_not_acknowledged_or_fenced".to_string())?;
    verify_review_snapshot(
        transaction,
        &lease.root_run_id,
        &candidate_id,
        revision,
        target_dir,
    )?;
    let reviewed: JsonValue = serde_json::from_str(&reviewed_json)
        .map_err(|_| "review_candidate_snapshot_invalid".to_string())?;
    let frozen = reviewed
        .get("findingCandidates")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "review_candidate_snapshot_invalid".to_string())?;
    let current = pending_agent_finding_candidates(transaction, &lease.root_run_id)?;
    if frozen != &current {
        return Err("review_candidate_snapshot_changed".into());
    }
    let bound = transaction
        .execute(
            "UPDATE agent_finding_candidates SET candidate_revision=?1,reviewer_run_id=?2,updated_at=datetime('now','localtime') \
             WHERE root_run_id=?3 AND status IN ('pending','insufficient_evidence')",
            params![revision, reviewer_run_id, lease.root_run_id],
        )
        .map_err(|error| format!("无法绑定 finding 候选 revision：{error}"))?;
    if bound != frozen.len() {
        return Err("review_candidate_binding_write_conflict".into());
    }
    if verdict == "confirmed" {
        let published = transaction
            .execute(
                "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
                 SELECT scan_id,target_url,stage,kind,record_key,title,severity,record_json FROM agent_finding_candidates \
                 WHERE root_run_id=?1 AND candidate_revision=?2 AND status IN ('pending','insufficient_evidence') \
                 ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET title=excluded.title,severity=excluded.severity,\
                 record_json=excluded.record_json,updated_at=datetime('now','localtime')",
                params![lease.root_run_id, revision],
            )
            .map_err(|error| format!("Reviewer confirmed 后发布 finding 失败：{error}"))?;
        if published != frozen.len() {
            return Err("review_candidate_publication_write_conflict".into());
        }
    }
    let candidate_status = match verdict {
        "confirmed" => "published",
        "rejected" => "rejected",
        _ => "insufficient_evidence",
    };
    let settled = transaction
        .execute(
            "UPDATE agent_finding_candidates SET status=?1,published_at=CASE WHEN ?1='published' THEN datetime('now','localtime') ELSE '' END,\
             updated_at=datetime('now','localtime') WHERE root_run_id=?2 AND candidate_revision=?3 AND status IN ('pending','insufficient_evidence')",
            params![candidate_status, lease.root_run_id, revision],
        )
        .map_err(|error| format!("无法结束 finding 候选：{error}"))?;
    if settled != frozen.len() {
        return Err("review_candidate_settlement_write_conflict".into());
    }
    // Row counts alone do not prove publication: an AFTER trigger could alter
    // or remove a row. Confirm the final contents before the enclosing delivery
    // transaction commits its decision, acknowledgement and child completion.
    let verified: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM agent_finding_candidates c WHERE c.root_run_id=?1 \
         AND c.candidate_revision=?2 AND c.reviewer_run_id=?3 AND c.status=?4 \
         AND ((?4='published' AND c.published_at<>'' AND EXISTS(SELECT 1 FROM sentinel_findings f \
           WHERE f.scan_id=c.scan_id AND f.target_url=c.target_url AND f.stage=c.stage \
             AND f.kind=c.kind AND f.record_key=c.record_key AND f.title=c.title \
             AND f.severity=c.severity AND f.record_json=c.record_json)) \
           OR (?4<>'published' AND c.published_at=''))",
            params![
                lease.root_run_id,
                revision,
                reviewer_run_id,
                candidate_status
            ],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法核验 finding 发布结果：{error}"))?;
    if verified != frozen.len() as i64 {
        return Err("review_candidate_publication_postcondition_failed".into());
    }
    // The review decision may have arrived before an operator paused the
    // attempt. Recheck in the same write transaction before publication can
    // commit; a pause transition cannot interleave across this lock.
    if !review_attempt_active(transaction, lease)? {
        return Err("review_attempt_not_active".into());
    }
    Ok(())
}
