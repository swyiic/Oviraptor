// Explicit task-record deletion is not a process killer or a disk purge. Paths
// stored by imports/old releases are evidence, never filesystem authority.
fn delete_sentinel_scan_inner(db_path: &Path, scan_id: &str) -> Result<(), String> {
    delete_sentinel_scan_inner_mode(db_path, scan_id, false)
}

// `force` is the operator hard-override (user-confirmed in the UI). It deletes a
// scan that is DEAD (no live local process — the one quiescence fact it still
// requires) even when the crash left orphaned budget reservations or unsettled
// assignments behind: it skips the per-root clean-exit proof AND the settlement
// gate. It never bypasses the live-process check, never fabricates a clean-exit
// receipt, and still ARCHIVES every retained financial row verbatim (marked
// `forced` so the archive is honestly distinguishable from a settled deletion),
// so the "this was deleted unreconciled" fact is preserved rather than lost.
fn delete_sentinel_scan_inner_mode(db_path: &Path, scan_id: &str, force: bool) -> Result<(), String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains('\\')
        || scan_id.contains("..") || scan_id.chars().any(char::is_control)
    { return Err("任务 ID 非法".into()); }
    let _lifecycle = claim_scan_control(db_path, scan_id)?;
    let mut connection = db::open(db_path)?;
    connection.pragma_update(None, "synchronous", "FULL").map_err(|e| e.to_string())?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let status: Option<String> = tx.query_row(
        "SELECT status FROM sentinel_scans WHERE id=?1", [scan_id], |r| r.get(0),
    ).optional().map_err(|e| e.to_string())?;
    let tombstoned: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        [scan_id], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    let Some(status) = status else {
        // Retry after an ambiguous IPC/commit only observes the durable result.
        // It must not delete anything else or refresh the original tombstone.
        return if tombstoned { crate::agent_runtime::deleted_scan_audit::verify_deleted(&tx,scan_id)?;Ok(()) } else { Err("任务不存在".into()) };
    };
    if tombstoned { return Err("删除状态不一致；任务和证据已保留，需要人工核对".into()); }
    require_scan_without_retired_data(&tx,scan_id)?;
    if verified_administrative_closure(&tx,scan_id)?.is_some() {
        return Err("人工结案不等于执行结清；结案回执与原任务证据必须保留，不能删除".into());
    }
    let handoff: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM native_web_closure_handoffs WHERE scan_id=?1)",
        [scan_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if handoff { return Err("该任务保留人工结案交接记录；可以归档，不能删除或重新创建交接".into()); }
    if !matches!(status.as_str(), "draft" | "paused" | "completed" | "completed_with_gaps" | "failed" | "cancelled"
        | "imported" | "partial" | "recon_only" | "manual_review" | "limited" | "protected_stop"
        | "resume_incompatible" | "persistence_failure" | "fuse_excluded" | "deferred") {
        return Err("任务仍在执行或状态未确认；请先请求暂停，等待执行退出后再删除".into());
    }
    // Includes historical attempts, removed targets and detached recon owners.
    // All acquired locks survive through COMMIT; no PID is ever signalled.
    let mut source_parents=crate::agent_runtime::deleted_scan_audit::OriginalSourceParents::default();
    if force {
        // The scan is dead; prove only that no local worker is still touching
        // these rows, then skip the per-attempt clean-exit proof the crash
        // never wrote. `source_parents` stays empty: the forced archive below
        // does not walk the per-root idle guards that would populate it.
        require_no_live_process_in(&tx, scan_id)
            .map_err(|e| format!("仍有本地执行进程未退出；任务和证据已保留：{e}"))?;
    } else {
        let _workers = claim_scan_quiescence_with_source_parents(&tx, db_path, scan_id,Some(&mut source_parents))
            .map_err(|e| format!("执行退出或清理尚未确认；任务和证据已保留：{e}"))?;
    }
    // Tick evidence has RESTRICT; Single's immutable journal/exit receipts do
    // not. Both still need the original run, events and source rows to verify.
    // Refuse before tombstones or cascades even if an old status was supplied.
    let retained: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_root_tick_receipts p JOIN agent_runs r
         ON r.id=p.root_run_id WHERE r.scan_id=?1)
         OR EXISTS(SELECT 1 FROM agent_root_tick_timeline_receipts p
         WHERE p.scan_id=?1)
         OR EXISTS(SELECT 1 FROM agent_root_model_journal p JOIN agent_runs r
         ON r.id=p.root_run_id WHERE r.scan_id=?1)
         OR EXISTS(SELECT 1 FROM agent_single_exit_receipts p JOIN agent_runs r
         ON r.id=p.root_run_id WHERE r.scan_id=?1)
         OR EXISTS(SELECT 1 FROM agent_single_projection_receipts p JOIN agent_runs r
         ON r.id=p.root_run_id WHERE r.scan_id=?1)
         OR EXISTS(SELECT 1 FROM agent_root_budget_attempts p JOIN agent_runs r
         ON r.id=p.root_run_id WHERE r.scan_id=?1)", [scan_id], |r| r.get(0),
    ).map_err(|e| format!("删除前原付费审计检查失败；任务已保留：{e}"))?;
    let paid_audit = if retained {
        let prepared=crate::agent_runtime::deleted_scan_audit::prepare_with_source_parents_mode(&tx,scan_id,source_parents,force).map_err(|error| if error=="deleted_audit_financial_schema_migration_required" {
            "native_paid_audit_retention_required: deleted_audit_financial_schema_migration_required: 原财务关联尚未完成受控迁移；任务与凭证已保留，须先核对、备份再迁移".to_string()
        } else {"native_paid_audit_retention_required: 该任务包含原始请求或财务审计凭证；当前必须保留原任务关联，可以归档。归档不表示执行、未知费用或清理已结清".to_string()})?;
        // Hard override deletes a dead-but-unsettled scan: the retained finance
        // rows are still captured and archived (forced), but the settlement gate
        // is skipped. Normal deletion keeps requiring full closure.
        if !force {
            require_scan_deletion_with_audit_in(&tx,scan_id,Some(&prepared)).map_err(|_|"native_paid_audit_retention_required: 原执行义务尚未全部闭合；任务与原审计来源已保留".to_string())?;
        }
        Some(prepared)
    } else {if !force {require_scan_deletion_settled_in(&tx,scan_id)?;}None};
    let children = scan_deletion_children_in(&tx, scan_id)?;

    let stamp: String = tx.query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if let Some(audit)=&paid_audit {audit.install_private_writer(&tx,force)?;audit.persist(&tx,&stamp)?;}
    let inserted = tx.execute(
        "INSERT INTO sentinel_deleted_scans(scan_id,deleted_at) VALUES(?1,?2)",
        params![scan_id, stamp],
    ).map_err(|e| e.to_string())?;
    if inserted != 1 { return Err("删除标记未确认，未删除任务".into()); }

    // Old retry children are independent tasks. Detach only their parent link;
    // verify the complete rows so ignored/corrupt writes cannot destroy history.
    let changed = tx.execute(
        "UPDATE sentinel_scans SET previous_scan_id='' WHERE previous_scan_id=?1", [scan_id],
    ).map_err(|e| e.to_string())?;
    if changed != children.len() { return Err("后续任务关联更新未确认，未删除任务".into()); }

    // Compatibility for old databases without these ON DELETE CASCADE edges.
    // Native evidence and control tables with declared FKs cascade atomically.
    let legacy_tables = ["sentinel_validations", "sentinel_opportunities", "sentinel_findings",
        "sentinel_checkpoints", "sentinel_targets", "sentinel_processes"];
    for table in legacy_tables {
        tx.execute(&format!("DELETE FROM {table} WHERE scan_id=?1"), [scan_id])
            .map_err(|e| e.to_string())?;
    }
    tx.execute("DELETE FROM browser_auth_sessions WHERE owner_scan_id=?1", [scan_id])
        .map_err(|e| e.to_string())?;
    if tx.execute("DELETE FROM sentinel_scans WHERE id=?1", [scan_id]).map_err(|e| e.to_string())? != 1 {
        return Err("任务删除未确认，已回滚".into());
    }
    // Detect silent IGNORE and AFTER-trigger corruption before acknowledging.
    let valid: bool = tx.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 OR previous_scan_id=?1)
         AND EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1 AND deleted_at=?2)
         AND NOT EXISTS(SELECT 1 FROM browser_auth_sessions WHERE owner_scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1)",
        params![scan_id, stamp], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if !valid { return Err("删除结果未确认，已回滚".into()); }
    for table in legacy_tables {
        let remaining: bool = tx.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE scan_id=?1)"),
            [scan_id], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if remaining { return Err("关联记录删除未确认，已回滚".into()); }
    }
    for (id, expected) in children {
        let actual = tx.query_row("SELECT * FROM sentinel_scans WHERE id=?1", [&id], |r| {
            (0..expected.len()).map(|i| r.get::<_, rusqlite::types::Value>(i)).collect::<Result<Vec<_>, _>>()
        }).map_err(|e| e.to_string())?;
        if actual != expected { return Err("后续任务内容发生异常变化，已回滚".into()); }
    }
    if let Some(audit)=&paid_audit {audit.verify_deleted(&tx)?;}
    tx.commit().map_err(|e| format!("任务删除提交结果未确认；请刷新后核对，不要清理文件：{e}"))
}

fn scan_deletion_children_in(connection: &rusqlite::Connection, scan_id: &str)
    -> Result<Vec<(String, Vec<rusqlite::types::Value>)>, String>
{
    let mut statement = connection.prepare("SELECT * FROM sentinel_scans WHERE previous_scan_id=?1 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let parent_column = statement.column_index("previous_scan_id").map_err(|e| e.to_string())?;
    let count = statement.column_count();
    let rows = statement.query_map([scan_id], |r| {
        let mut values = (0..count).map(|i| r.get::<_, rusqlite::types::Value>(i)).collect::<Result<Vec<_>, _>>()?;
        values[parent_column] = String::new().into();
        Ok((r.get::<_, String>("id")?, values))
    }).map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    Ok(rows)
}

fn require_scan_deletion_settled_in(connection: &rusqlite::Connection, scan_id: &str) -> Result<(), String> {
    require_scan_deletion_with_audit_in(connection,scan_id,None)
}
fn require_scan_deletion_with_audit_in(connection: &rusqlite::Connection, scan_id: &str,
    audit:Option<&crate::agent_runtime::deleted_scan_audit::Prepared>) -> Result<(), String> {
    if let Some(audit)=audit {audit.verify_scope(connection,scan_id)?;}
    let verified_original=audit.is_some();
    // Status labels, expired leases and operator attestations cannot settle
    // requests. Keep every known unresolved obligation available for review.
    let unresolved: bool = connection.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM native_scan_branches WHERE scan_id=?1 AND status='pending')
         OR EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND ((status NOT IN ('completed','failed','cancelled') AND NOT (?2 AND status='terminal' AND orchestration_policy IN ('single','multi')))
             OR terminal_code IN ('request_reconciliation_required','evidence_integrity')))
         OR EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.coordinator_run_id
             WHERE r.scan_id=?1 AND (a.state NOT IN ('completed','failed','cancelled','lease_expired') OR a.budget_settled_at=''))
         OR EXISTS(SELECT 1 FROM agent_lane_leases WHERE scan_id=?1)
         OR EXISTS(SELECT 1 FROM agent_budget_ledger b JOIN agent_runs r ON r.id=b.root_run_id
             WHERE r.scan_id=?1 AND (b.reserved_tokens<>0 OR b.reserved_requests<>0))
         OR EXISTS(SELECT 1 FROM agent_specialist_calls c JOIN agent_runs r ON r.id=c.root_run_id
             WHERE r.scan_id=?1 AND c.state<>'received')
         OR EXISTS(SELECT 1 FROM agent_web_model_journal d JOIN agent_runs r ON r.id=d.root_run_id
             WHERE r.scan_id=?1 AND d.phase='dispatch' AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal t
               WHERE t.call_id=d.call_id AND t.phase IN ('received','unsent') AND {}))
         OR EXISTS(SELECT 1 FROM tool_invocations t JOIN agent_runs r ON r.id=t.run_id
             WHERE r.scan_id=?1 AND (t.status IN ('running','interrupted') OR t.finished_at=''))
         OR EXISTS(SELECT 1 FROM agent_http_request_claims c LEFT JOIN tool_invocations t
             ON t.run_id=c.run_id AND t.invocation_id=c.invocation_id
             WHERE c.scan_id=?1 AND (c.response_status=0 OR c.received_at='' OR t.id IS NULL OR t.status<>'completed'))
         OR EXISTS(SELECT 1 FROM agent_external_surface_captures WHERE scan_id=?1 AND (state<>'received' OR artifact_id=''))
         OR EXISTS(SELECT 1 FROM agent_authorization_probe_claims WHERE scan_id=?1 AND artifact_id='')
         OR EXISTS(SELECT 1 FROM agent_user_directives WHERE scan_id=?1 AND status NOT IN ('completed','failed','rejected'))",crate::agent_runtime::multi_agent::budget::WEB_RECEIPT_BINDING),
        params![scan_id,verified_original], |r| r.get(0),
    ).map_err(|e| format!("删除前执行义务检查失败；任务已保留：{e}"))?;
    if unresolved { return Err("仍有未结算的执行、指令或请求回执；请先完成核对，不能通过删除清除未知结果".into()); }
    // The old model summary cannot settle independent target or other costs.
    // Keep append-only sources even after legitimate settled task deletion.
    let mut statement=connection.prepare("SELECT DISTINCT e.root_run_id FROM agent_budget_entries e
        JOIN agent_runs r ON r.id=e.root_run_id WHERE r.scan_id=?1").map_err(|e|e.to_string())?;
    let roots=statement.query_map([scan_id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?
        .collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    for root in roots {
        for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
            let balance=crate::agent_runtime::multi_agent::budget::balance(connection,&root,None,dimension)?;
            if balance.reserved!=0 || balance.indeterminate!=0 {
                return Err("追加预算账本仍有未结算占用；任务与审计来源已保留，请先核对".into());
            }
        }
    }
    Ok(())
}

// Gold-standard end-to-end validation of the operator hard-override against a
// REAL crashed scan. It is #[ignore]d so normal `cargo test` and CI never run
// it, and it operates only on COPIES of a snapshot the caller points it at —
// never the live database. Invoke explicitly, e.g.:
//   OVIRAPTOR_FORCE_TEST_SNAPSHOT=/abs/copy.sqlite3 \
//   OVIRAPTOR_FORCE_TEST_SCAN=<dead-unsettled-scan-id> \
//   cargo test --lib force_delete_live_data -- --ignored --nocapture
#[cfg(test)]
mod force_delete_live_data_tests {
    use super::*;

    fn count(db: &Path, sql: &str, scan: &str) -> i64 {
        let c = rusqlite::Connection::open(db).unwrap();
        c.query_row(sql, [scan], |r| r.get(0)).unwrap()
    }

    #[test]
    #[ignore]
    fn forced_delete_removes_dead_but_unsettled_scan_and_archives_raw() {
        let snapshot = std::env::var("OVIRAPTOR_FORCE_TEST_SNAPSHOT")
            .expect("set OVIRAPTOR_FORCE_TEST_SNAPSHOT to a DB copy path");
        let scan = std::env::var("OVIRAPTOR_FORCE_TEST_SCAN").expect("set OVIRAPTOR_FORCE_TEST_SCAN");
        let snapshot = PathBuf::from(snapshot);
        let dir = std::env::temp_dir().join(format!("oviraptor-forcetest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // (1) The normal path must still REFUSE this dead-but-unsettled scan —
        // otherwise there would be nothing for the override to override.
        let w1 = dir.join("normal.sqlite3");
        std::fs::copy(&snapshot, &w1).unwrap();
        let normal = delete_sentinel_scan_inner(&w1, &scan);
        assert!(normal.is_err(), "normal delete must refuse; got {normal:?}");

        // (2) The forced path must SUCCEED, cascade-delete the scan, archive the
        // retained finance rows raw (marked forced), and leave them in place.
        let w2 = dir.join("forced.sqlite3");
        std::fs::copy(&snapshot, &w2).unwrap();
        let attempts_before = count(
            &w2,
            "SELECT COUNT(*) FROM agent_root_budget_attempts WHERE json_extract(contract_json,'$.root.scan')=?1",
            &scan,
        );
        assert!(attempts_before > 0, "test scan must carry retained budget attempts");
        // Counted via the retained budget-attempt keys (not via agent_runs,
        // which the cascade removes) so the same query is valid before & after.
        let tick_sql = "SELECT COUNT(*) FROM agent_root_tick_receipts WHERE lease_attempt_id IN (SELECT id FROM agent_root_budget_attempts WHERE json_extract(contract_json,'$.root.scan')=?1)";
        let tick_before = count(&w2, tick_sql, &scan);
        assert!(tick_before > 0, "test scan must carry retained tick receipts");

        let forced = delete_sentinel_scan_inner_mode(&w2, &scan, true);
        assert!(forced.is_ok(), "forced delete must succeed; got {forced:?}");

        // Scan row and cascading runs/ledger are gone.
        assert_eq!(count(&w2, "SELECT COUNT(*) FROM sentinel_scans WHERE id=?1", &scan), 0);
        assert_eq!(count(&w2, "SELECT COUNT(*) FROM agent_runs WHERE scan_id=?1", &scan), 0);

        // The forced archive is persisted and honestly marked forced.
        let conn = rusqlite::Connection::open(&w2).unwrap();
        let audit_json: String = conn
            .query_row(
                "SELECT audit_json FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
                [&scan],
                |r| r.get(0),
            )
            .expect("a forced archive row must exist");
        assert!(audit_json.contains("\"forced\":true"), "archive must be marked forced");

        // The retained financial record survives verbatim — nothing was lost.
        assert_eq!(
            count(
                &w2,
                "SELECT COUNT(*) FROM agent_root_budget_attempts WHERE json_extract(contract_json,'$.root.scan')=?1",
                &scan,
            ),
            attempts_before,
        );
        assert_eq!(count(&w2, tick_sql, &scan), tick_before);

        // The deletion verifies end to end (the forced bundle re-verifies without
        // demanding closure, every other integrity check intact).
        assert!(crate::agent_runtime::deleted_scan_audit::verify_deleted(&conn, &scan).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
