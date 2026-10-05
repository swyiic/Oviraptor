// Privately owned projection transaction. Never writes original finance/Native state.
fn write_terminal_legacy_projection(
    path: &Path,
    scan: &str,
    original: &OriginalAgentTerminalIdentity,
    route: &FrontendRoute,
    status: &str,
    terminal: Option<&JsonValue>,
    fuse: Option<&str>,
) -> Result<(), String> {
    if original.db_path != path {
        return Err("terminal_legacy_database_scope_conflict".into());
    }
    let mut db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| e.to_string())?;
    db.busy_timeout(Duration::from_millis(250))
        .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;
    db.authorizer(Some(terminal_legacy_authorize))
        .map_err(|e| e.to_string())?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    require_original_terminal_projection(&tx, original, scan, route, terminal)?;
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
            params![scan, route.url],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count != 1 {
        return Err("terminal_legacy_target_missing_or_ambiguous".into());
    }
    if let Some(value) = terminal {
        terminal_legacy_checkpoint(&tx, scan, &route.url, "agent_terminal", value)?;
    }
    let reason = route.reason_text();
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1 AND url=?2 AND status=?3 AND value_score=?4 AND scan_mode=?5 AND routing_reason=?6)",params![scan,route.url,status,route.score,route.mode,reason],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact {
        let stamp: String = tx
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let changed=tx.execute("UPDATE sentinel_targets SET status=?3,value_score=?4,scan_mode=?5,routing_reason=?6,updated_at=?7 WHERE scan_id=?1 AND url=?2",params![scan,route.url,status,route.score,route.mode,reason,stamp]).map_err(|e|e.to_string())?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1 AND url=?2 AND status=?3 AND value_score=?4 AND scan_mode=?5 AND routing_reason=?6 AND updated_at=?7)",params![scan,route.url,status,route.score,route.mode,reason,stamp],|r|r.get(0)).map_err(|e|e.to_string())?;
        if changed != 1 || !exact {
            return Err("terminal_legacy_target_write_conflict".into());
        }
    }
    terminal_legacy_checkpoint(&tx, scan, &route.url, "adaptive_routing", &route.as_json())?;
    if let Some(reason) = fuse {
        terminal_legacy_fuse(&tx, scan, &route.url, reason)?;
    }
    require_original_terminal_projection(&tx, original, scan, route, terminal)?;
    tx.commit().map_err(|e| e.to_string())
}
fn terminal_legacy_checkpoint(
    db: &rusqlite::Connection,
    scan: &str,
    url: &str,
    stage: &str,
    value: &JsonValue,
) -> Result<(), String> {
    let payload = value.to_string();
    let saved: Option<String> = db
        .query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3",
            params![scan, url, stage],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if saved.as_deref() == Some(payload.as_str()) {
        return Ok(());
    }
    let stamp: String = db
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let changed=db.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=excluded.updated_at",params![scan,url,stage,payload,stamp]).map_err(|e|e.to_string())?;
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage=?3 AND raw_json=?4 AND updated_at=?5)",params![scan,url,stage,payload,stamp],|r|r.get(0)).map_err(|e|e.to_string())?;
    if changed != 1 || !exact {
        return Err("terminal_legacy_checkpoint_write_conflict".into());
    }
    Ok(())
}
fn terminal_legacy_authorize(
    c: rusqlite::hooks::AuthContext<'_>,
) -> rusqlite::hooks::Authorization {
    use rusqlite::hooks::{AuthAction, Authorization};
    let direct = c.database_name == Some("main") && c.accessor.is_none();
    let allowed = match c.action {
        AuthAction::Insert { table_name } if direct => {
            matches!(table_name, "sentinel_checkpoints" | "sentinel_fuse_zone")
        }
        AuthAction::Update {
            table_name,
            column_name,
        } if direct => match table_name {
            "sentinel_targets" => matches!(
                column_name,
                "status" | "value_score" | "scan_mode" | "routing_reason" | "updated_at"
            ),
            "sentinel_checkpoints" => matches!(column_name, "raw_json" | "updated_at"),
            "sentinel_fuse_zone" => matches!(
                column_name,
                "asset_id"
                    | "company"
                    | "url"
                    | "source_scan_id"
                    | "reason"
                    | "verdict"
                    | "note"
                    | "evidence"
                    | "archived"
                    | "updated_at"
            ),
            _ => false,
        },
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => true,
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}

fn require_original_terminal_projection(
    db: &rusqlite::Connection,
    original: &OriginalAgentTerminalIdentity,
    scan: &str,
    route: &FrontendRoute,
    terminal: Option<&JsonValue>,
) -> Result<(), String> {
    if let Some(reduction) = original_multi_terminal_on(db, original, scan, route)? {
        let terminal = terminal.ok_or("runtime_terminal_original_projection_missing")?;
        if terminal["status"].as_str() != Some(reduction.state.to_sentinel_status())
            || terminal["code"].as_str() != Some(reduction.code.as_str())
            || terminal["detail"]
                .as_str()
                .map(|s| crate::agent_runtime::secrets::redact_text_with(s, None))
                .as_deref()
                != Some(reduction.reason.as_str())
        {
            return Err("runtime_terminal_original_projection_conflict".into());
        }
    }
    Ok(())
}
