// Explicit task-record deletion is not a process killer or a disk purge. Paths
// stored by imports/old releases are evidence, never filesystem authority.
fn delete_sentinel_scan_inner(db_path: &Path, scan_id: &str) -> Result<(), String> {
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
    let _workers = claim_scan_quiescence_with_source_parents(&tx, db_path, scan_id,Some(&mut source_parents))
        .map_err(|e| format!("执行退出或清理尚未确认；任务和证据已保留：{e}"))?;
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
        let prepared=crate::agent_runtime::deleted_scan_audit::prepare_with_source_parents(&tx,scan_id,source_parents).map_err(|error| if error=="deleted_audit_financial_schema_migration_required" {
            "native_paid_audit_retention_required: deleted_audit_financial_schema_migration_required: 原财务关联尚未完成受控迁移；任务与凭证已保留，须先核对、备份再迁移".to_string()
        } else {"native_paid_audit_retention_required: 该任务包含原始请求或财务审计凭证；当前必须保留原任务关联，可以归档。归档不表示执行、未知费用或清理已结清".to_string()})?;
        require_scan_deletion_with_audit_in(&tx,scan_id,Some(&prepared)).map_err(|_|"native_paid_audit_retention_required: 原执行义务尚未全部闭合；任务与原审计来源已保留".to_string())?;
        Some(prepared)
    } else {require_scan_deletion_settled_in(&tx,scan_id)?;None};
    let children = scan_deletion_children_in(&tx, scan_id)?;

    let stamp: String = tx.query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if let Some(audit)=&paid_audit {audit.install_private_writer(&tx)?;audit.persist(&tx,&stamp)?;}
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
    #[cfg(test)]
    if paid_audit.is_some() {agent_tests::paid_deletion_commit_barrier(db_path,scan_id)?;}
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
