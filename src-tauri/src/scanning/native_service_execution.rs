//! The service's outer durable operation journal. Mature scan admission,
//! execution, accounting and pause publication remain in their original owner.
use super::*;
use crate::scanning as owner;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

const EXECUTION_SCHEMA: &str = r#"
DROP TRIGGER scan_service_operation_no_update;
DROP TRIGGER scan_service_operation_no_delete;
ALTER TABLE scan_service_operations RENAME TO scan_service_operations_v1;
CREATE TABLE scan_service_operations(project_id INTEGER NOT NULL REFERENCES projects(id),operation_id TEXT NOT NULL,arguments_sha256 TEXT NOT NULL,instance_id TEXT NOT NULL REFERENCES scan_service_store(instance_id),scan_id TEXT NOT NULL UNIQUE,receipt_json TEXT NOT NULL,PRIMARY KEY(project_id,operation_id));
INSERT INTO scan_service_operations SELECT * FROM scan_service_operations_v1;
DROP TABLE scan_service_operations_v1;
CREATE TRIGGER scan_service_operation_no_update BEFORE UPDATE ON scan_service_operations BEGIN SELECT RAISE(ABORT,'service operation immutable');END;
CREATE TRIGGER scan_service_operation_no_delete BEFORE DELETE ON scan_service_operations BEGIN SELECT RAISE(ABORT,'service operation immutable');END;
CREATE TABLE scan_service_execution_schema(version INTEGER PRIMARY KEY CHECK(version=2));
INSERT INTO scan_service_execution_schema VALUES(2);
CREATE TRIGGER scan_service_execution_schema_no_update BEFORE UPDATE ON scan_service_execution_schema BEGIN SELECT RAISE(ABORT,'execution schema immutable');END;
CREATE TRIGGER scan_service_execution_schema_no_delete BEFORE DELETE ON scan_service_execution_schema BEGIN SELECT RAISE(ABORT,'execution schema immutable');END;
CREATE TABLE scan_service_execution_births(scan_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL,executable_sha256 TEXT NOT NULL,canonical_database_path TEXT NOT NULL);
CREATE TRIGGER scan_service_execution_births_no_update BEFORE UPDATE ON scan_service_execution_births BEGIN SELECT RAISE(ABORT,'service execution birth immutable');END;
CREATE TRIGGER scan_service_execution_births_no_delete BEFORE DELETE ON scan_service_execution_births BEGIN SELECT RAISE(ABORT,'service execution birth immutable');END;
CREATE TABLE scan_service_actions(project_id INTEGER NOT NULL REFERENCES projects(id),operation_id TEXT NOT NULL,verb TEXT NOT NULL,scan_id TEXT NOT NULL,arguments_sha256 TEXT NOT NULL,grant_revision INTEGER NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','completed','rejected','uncertain')),receipt_json TEXT NOT NULL DEFAULT '',PRIMARY KEY(project_id,operation_id));
CREATE TRIGGER scan_service_actions_binding_immutable BEFORE UPDATE OF project_id,operation_id,verb,scan_id,arguments_sha256,grant_revision ON scan_service_actions BEGIN SELECT RAISE(ABORT,'service action immutable');END;
CREATE TRIGGER scan_service_actions_state_guard BEFORE UPDATE ON scan_service_actions WHEN OLD.state<>'prepared' OR NEW.state NOT IN ('completed','rejected','uncertain') OR NEW.receipt_json='' BEGIN SELECT RAISE(ABORT,'service action transition invalid');END;
CREATE TRIGGER scan_service_actions_no_delete BEFORE DELETE ON scan_service_actions BEGIN SELECT RAISE(ABORT,'service action immutable');END;
"#;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Start,
    Pause,
    Resume,
    Retry,
    CancelDraft,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Retry => "retry",
            Self::CancelDraft => "cancel_draft",
        }
    }
    fn permission(self) -> &'static str {
        match self {
            Self::Start | Self::Resume | Self::Retry => "scan.execute",
            _ => "scan.control",
        }
    }
}

fn original_drafts_only(db: &Connection) -> Result<bool, String> {
    db.query_row("SELECT NOT EXISTS(SELECT 1 FROM sentinel_scans WHERE status<>'draft' OR attempt_count<>0 OR task_path<>'') AND NOT EXISTS(SELECT 1 FROM sentinel_scan_attempts) AND NOT EXISTS(SELECT 1 FROM agent_runs) AND NOT EXISTS(SELECT 1 FROM native_branch_dispatches) AND NOT EXISTS(SELECT 1 FROM sentinel_processes) AND NOT EXISTS(SELECT 1 FROM analyzer_container_receipts) AND NOT EXISTS(SELECT 1 FROM agent_root_budget_attempts)", [], |r|r.get(0)).map_err(|_| "scan_execution_schema_history_unknown".into())
}

fn operations_snapshot(db: &Connection) -> Result<Vec<Vec<String>>, String> {
    db.prepare("SELECT project_id,operation_id,arguments_sha256,instance_id,scan_id,receipt_json FROM scan_service_operations ORDER BY project_id,operation_id")
        .and_then(|mut s|s.query_map([], |r| Ok(vec![r.get::<_,i64>(0)?.to_string(),r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?]))?.collect())
        .map_err(|_| "scan_execution_schema_receipts_unknown".into())
}

fn verify_execution_schema(db: &Connection) -> Result<(), String> {
    let tables:i64=db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name IN ('scan_service_operations','scan_service_execution_schema','scan_service_execution_births','scan_service_actions')",[],|r|r.get(0)).map_err(|_| "scan_execution_schema_unknown")?;
    let triggers:i64=db.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND name IN ('scan_service_operation_no_update','scan_service_operation_no_delete','scan_service_execution_schema_no_update','scan_service_execution_schema_no_delete','scan_service_execution_births_no_update','scan_service_execution_births_no_delete','scan_service_actions_binding_immutable','scan_service_actions_state_guard','scan_service_actions_no_delete')",[],|r|r.get(0)).map_err(|_| "scan_execution_schema_unknown")?;
    let obsolete: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='scan_service_operations_v1')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "scan_execution_schema_unknown")?;
    if tables != 4 || triggers != 9 || obsolete {
        return Err("scan_execution_schema_unknown".into());
    }
    for sql in ["SELECT project_id,operation_id,verb,scan_id,arguments_sha256,grant_revision,state,receipt_json FROM main.scan_service_actions LIMIT 0", "SELECT scan_id,instance_id,executable_sha256,canonical_database_path FROM main.scan_service_execution_births LIMIT 0"] {
        db.prepare(sql).map_err(|_| "scan_execution_schema_unknown")?;
    }
    Ok(())
}

pub(super) fn enable(service: &Service) -> Result<(), String> {
    service.require_custody(&service.database)?;
    let tx =
        rusqlite::Transaction::new_unchecked(&service.database, TransactionBehavior::Immediate)
            .map_err(|_| "scan_execution_schema_unavailable")?;
    service.require_custody(&tx)?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='scan_service_execution_schema')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "scan_execution_schema_unknown")?;
    if exists {
        let versions: Vec<i64> = tx
            .prepare("SELECT version FROM main.scan_service_execution_schema")
            .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
            .map_err(|_| "scan_execution_schema_unknown")?;
        if versions != [2] {
            return Err("scan_execution_schema_unknown".into());
        }
        verify_execution_schema(&tx)?;
        service.require_custody(&tx)?;
        tx.commit()
            .map_err(|_| "scan_execution_schema_commit_unconfirmed")?;
        return Ok(());
    }
    let partial:bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name IN ('scan_service_operations_v1','scan_service_actions','scan_service_execution_births'))", [], |r|r.get(0)).map_err(|_| "scan_execution_schema_unknown")?;
    if partial || !original_drafts_only(&tx)? {
        return Err("scan_execution_schema_requires_independent_new_store".into());
    }
    let before = operations_snapshot(&tx)?;
    tx.execute_batch(EXECUTION_SCHEMA)
        .map_err(|_| "scan_execution_schema_upgrade_failed")?;
    if operations_snapshot(&tx)? != before {
        return Err("scan_execution_schema_receipts_changed".into());
    }
    verify_execution_schema(&tx)?;
    service.require_custody(&tx)?;
    tx.commit()
        .map_err(|_| "scan_execution_schema_commit_unconfirmed".to_string())
}

fn owned_task_on(
    db: &Connection,
    instance: &str,
    context: &Context,
    task: &str,
) -> Result<(String, i64), String> {
    valid_text(task, 128, false)?;
    db.query_row("SELECT s.status,s.attempt_count FROM scan_service_operations o JOIN sentinel_scans s ON s.id=o.scan_id WHERE o.instance_id=?1 AND o.project_id=?2 AND s.project_id=?2 AND s.id=?3 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id)", params![instance,context.host_bound_project_id,task], |r|Ok((r.get(0)?,r.get(1)?))).map_err(|_| "scan_task_scope_unavailable".into())
}

fn executable_sha256() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|_| "scan_execution_binary_unavailable")?;
    let mut file = File::open(path).map_err(|_| "scan_execution_binary_unavailable")?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let size = file
            .read(&mut bytes)
            .map_err(|_| "scan_execution_binary_unavailable")?;
        if size == 0 {
            break;
        }
        hash.update(&bytes[..size]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn startup_snapshot(db: &Connection, task: &str) -> Result<Vec<Vec<Vec<String>>>, String> {
    let mut result = Vec::new();
    for sql in [
        "SELECT * FROM sentinel_scans WHERE id=?1 ORDER BY id",
        "SELECT * FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number",
        "SELECT * FROM sentinel_targets WHERE scan_id=?1 ORDER BY id",
    ] {
        let mut statement = db.prepare(sql).map_err(|_| "scan_action_outcome_unknown")?;
        let columns = statement.column_count();
        let rows = statement
            .query_map([task], |row| {
                (0..columns)
                    .map(|i| {
                        row.get::<_, rusqlite::types::Value>(i)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|_| "scan_action_outcome_unknown")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "scan_action_outcome_unknown")?;
        result.push(rows);
    }
    Ok(result)
}

fn engine_error(code: &str) -> &str {
    if code.len() <= 128
        && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        && [
            "scan_",
            "web_",
            "native_",
            "source_",
            "tool_",
            "model_",
            "protection_",
            "administrative_",
        ]
        .iter()
        .any(|p| code.starts_with(p))
    {
        code
    } else {
        "scan_domain_operation_rejected"
    }
}

pub(super) fn perform(
    service: &Service,
    context: &Context,
    action: Action,
    task: &str,
) -> Result<Value, String> {
    service.authorize(context, action.permission())?;
    service.require_custody(&service.database)?;
    let arguments_sha256 = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&json!([action.name(), task]))
                .map_err(|_| "scan_arguments_invalid")?
        )
    );
    let tx =
        rusqlite::Transaction::new_unchecked(&service.database, TransactionBehavior::Immediate)
            .map_err(|_| "scan_action_claim_unavailable")?;
    service.require_custody(&tx)?;
    let old:Option<(String,String,String)>=tx.query_row("SELECT arguments_sha256,state,receipt_json FROM scan_service_actions WHERE project_id=?1 AND operation_id=?2",params![context.host_bound_project_id,context.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_| "scan_action_lookup_failed")?;
    if let Some((old_hash, state, receipt)) = old {
        if old_hash != arguments_sha256 {
            return Err("scan_operation_conflict".into());
        }
        if state == "prepared" {
            return Err("scan_action_outcome_uncertain_no_automatic_replay".into());
        }
        let value = serde_json::from_str(&receipt).map_err(|_| "scan_operation_receipt_invalid")?;
        service.require_custody(&tx)?;
        tx.commit()
            .map_err(|_| "scan_action_read_commit_unconfirmed")?;
        return Ok(value);
    }
    let (status, attempt) = owned_task_on(&tx, &service.instance_id, context, task)?;
    if matches!(action, Action::Start | Action::Resume | Action::Retry) {
        let binary = executable_sha256()?;
        let birth:Option<(String,String,String)>=tx.query_row("SELECT instance_id,executable_sha256,canonical_database_path FROM scan_service_execution_births WHERE scan_id=?1",[task],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_| "scan_execution_birth_unknown")?;
        match birth {
            Some((instance, exe, path))
                if instance == service.instance_id
                    && exe == binary
                    && Path::new(&path) == service.db_path => {}
            Some(_) => return Err("scan_execution_build_or_store_changed_requires_new_task".into()),
            None if matches!(action, Action::Start) && status == "draft" && attempt == 0 => {
                let n = tx
                    .execute(
                        "INSERT INTO scan_service_execution_births VALUES(?1,?2,?3,?4)",
                        params![
                            task,
                            service.instance_id,
                            binary,
                            service.db_path.to_str().ok_or("scan_store_path_invalid")?
                        ],
                    )
                    .map_err(|_| "scan_execution_birth_write_failed")?;
                if n != 1 {
                    return Err("scan_execution_birth_write_failed".into());
                }
            }
            None => return Err("scan_execution_original_service_birth_required".into()),
        }
    }
    let n=tx.execute("INSERT INTO scan_service_actions(project_id,operation_id,verb,scan_id,arguments_sha256,grant_revision,state) VALUES(?1,?2,?3,?4,?5,?6,'prepared')",params![context.host_bound_project_id,context.operation_id,action.name(),task,arguments_sha256,context.grant_revision]).map_err(|_| "scan_action_claim_failed")?;
    if n != 1 {
        return Err("scan_action_claim_failed".into());
    }
    service.require_custody(&tx)?;
    tx.commit()
        .map_err(|_| "scan_action_claim_commit_unconfirmed")?;
    let before = startup_snapshot(&service.database, task)?;
    // Do not compensate an ambiguous mature commit. The durable prepared
    // action remains non-replayable until its real original outcome is known.
    let result=match action {
        Action::Start|Action::Resume|Action::Retry => owner::start_web_scan_with_environment(
            &service._environment,&service.db_path,service._environment.data_root(),task.into(),
            match action {Action::Start=>owner::WebStartMode::Confirm,Action::Resume=>owner::WebStartMode::Resume,_=>owner::WebStartMode::Retry},
        ).map(|scan|json!({"taskId":scan.id,"status":scan.status,"attemptNumber":scan.attempt_count})),
        Action::Pause => owner::request_sentinel_pause(&service.db_path,task).and_then(|attempt| {
            // A busy finalizer leaves pausing; it never claims rollback of a
            // provider/network effect or fabricates an exited worker.
            let _=owner::finish_sentinel_pause(&service.db_path,task,attempt);
            read_task(service,context,task)
        }),
        Action::CancelDraft => owner::claim_scan_control(&service.db_path,task).and_then(|_control| {
            owner::cancel_draft_scan_in(&service.database,task).map(|_|json!({"taskId":task,"deletedDraft":true}))
        }),
    };
    service.require_custody(&service.database)?;
    let (state, outcome) = match result {
        Ok(value) => ("completed", json!({"accepted":true,"task":value})),
        Err(code) => {
            // Original startup publishes the complete attempt atomically and
            // never spawns before confirmed commit. Only an unchanged complete
            // original scan/attempt/target snapshot proves no attempt was
            // published. Changed state, ambiguous commit, spawn failure or a
            // failed read stays uncertain and must never replay automatically.
            let unchanged =
                startup_snapshot(&service.database, task).is_ok_and(|after| after == before);
            if unchanged {
                (
                    "rejected",
                    json!({"accepted":false,"disposition":"rejected","code":engine_error(&code)}),
                )
            } else {
                (
                    "uncertain",
                    json!({"accepted":false,"disposition":"uncertain","executionStarted":"unknown","code":engine_error(&code)}),
                )
            }
        }
    };
    let receipt = json!({"kind":"scan_service_action_receipt","liveState":false,"operationId":context.operation_id,"action":action.name(),"hostBoundProjectId":context.host_bound_project_id,"taskId":task,"outcome":outcome});
    let tx =
        rusqlite::Transaction::new_unchecked(&service.database, TransactionBehavior::Immediate)
            .map_err(|_| "scan_action_outcome_uncertain_no_automatic_replay")?;
    service.require_custody(&tx)?;
    let n=tx.execute("UPDATE scan_service_actions SET state=?1,receipt_json=?2 WHERE project_id=?3 AND operation_id=?4 AND arguments_sha256=?5 AND state='prepared'",params![state,receipt.to_string(),context.host_bound_project_id,context.operation_id,arguments_sha256]).map_err(|_| "scan_action_outcome_uncertain_no_automatic_replay")?;
    let confirmed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM scan_service_actions WHERE project_id=?1 AND operation_id=?2 AND state=?3 AND receipt_json=?4)",params![context.host_bound_project_id,context.operation_id,state,receipt.to_string()],|r|r.get(0)).map_err(|_| "scan_action_outcome_uncertain_no_automatic_replay")?;
    if n != 1 || !confirmed {
        return Err("scan_action_outcome_uncertain_no_automatic_replay".into());
    }
    service.require_custody(&tx)?;
    tx.commit()
        .map_err(|_| "scan_action_outcome_uncertain_no_automatic_replay")?;
    Ok(receipt)
}

pub(super) fn read_task(service: &Service, context: &Context, task: &str) -> Result<Value, String> {
    service.require_custody(&service.database)?;
    let (status, attempt) = owned_task_on(&service.database, &service.instance_id, context, task)?;
    service.require_custody(&service.database)?;
    let status = match status.as_str() {
        "draft"
        | "queued"
        | "scanning"
        | "pausing"
        | "paused"
        | "partial"
        | "completed"
        | "failed"
        | "cancelled"
        | "completed_with_gaps"
        | "limited"
        | "protected_stop"
        | "persistence_failure"
        | "resume_incompatible"
        | "manual_review"
        | "legacy_backend_removed" => status.as_str(),
        _ => "unknown",
    };
    Ok(
        json!({"taskId":task,"hostBoundProjectId":context.host_bound_project_id,"status":status,"attemptNumber":attempt,"executionAvailable":true,"asOf":chrono::Utc::now().to_rfc3339()}),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ListTasks {
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Cursor {
    instance: String,
    project_id: i64,
    order: String,
    updated_at: String,
    task_id: String,
}

impl ListTasks {
    pub(super) fn validate(&self, instance: &str, project: i64) -> Result<Option<Cursor>, String> {
        if !(1..=20).contains(&self.limit.unwrap_or(20)) {
            return Err("scan_arguments_invalid".into());
        }
        let Some(cursor) = &self.cursor else {
            return Ok(None);
        };
        if cursor.len() > 2048 {
            return Err("scan_cursor_invalid".into());
        }
        let raw = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| "scan_cursor_invalid")?;
        let parsed: Cursor = serde_json::from_slice(&raw).map_err(|_| "scan_cursor_invalid")?;
        if parsed.instance != instance
            || parsed.project_id != project
            || parsed.order != "updated_at_desc_task_id_desc"
        {
            return Err("scan_cursor_scope_changed".into());
        }
        valid_text(&parsed.updated_at, 64, false)?;
        valid_text(&parsed.task_id, 128, false)?;
        Ok(Some(parsed))
    }
}

fn observed_text(value: &str) -> String {
    // Redact the full value before clipping; 64 Unicode scalars take at most
    // 384 JSON bytes including worst-case control escaping.
    crate::agent_runtime::secrets::redact_text_with(value, None)
        .chars()
        .take(64)
        .collect()
}

pub(super) fn list(
    service: &Service,
    context: &Context,
    input: ListTasks,
) -> Result<Value, String> {
    service.authorize(context, "draft.read")?;
    let cursor = input.validate(&service.instance_id, context.host_bound_project_id)?;
    let limit = input.limit.unwrap_or(20);
    service.require_custody(&service.database)?;
    let tx = service
        .database
        .unchecked_transaction()
        .map_err(|_| "scan_list_snapshot_unavailable")?;
    service.require_custody(&tx)?;
    let total:i64=tx.query_row("SELECT COUNT(*) FROM sentinel_scans s WHERE s.project_id=?1 AND EXISTS(SELECT 1 FROM scan_service_operations o WHERE o.scan_id=s.id AND o.project_id=s.project_id AND o.instance_id=?2) AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id)",params![context.host_bound_project_id,service.instance_id],|r|r.get(0)).map_err(|_| "scan_list_read_failed")?;
    let mut ids:Vec<(String,String)>=tx.prepare("SELECT s.id,s.updated_at FROM sentinel_scans s WHERE s.project_id=?1 AND EXISTS(SELECT 1 FROM scan_service_operations o WHERE o.scan_id=s.id AND o.project_id=s.project_id AND o.instance_id=?2) AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id) AND (?3 IS NULL OR s.updated_at<?3 OR (s.updated_at=?3 AND s.id<?4)) ORDER BY s.updated_at DESC,s.id DESC LIMIT ?5")
        .and_then(|mut s|s.query_map(params![context.host_bound_project_id,service.instance_id,cursor.as_ref().map(|c|&c.updated_at),cursor.as_ref().map(|c|&c.task_id),limit+1],|r|Ok((r.get(0)?,r.get(1)?)))?.collect()).map_err(|_| "scan_list_read_failed")?;
    let has_more = ids.len() > limit as usize;
    ids.truncate(limit as usize);
    let mut items = Vec::with_capacity(ids.len());
    for (id, _) in &ids {
        let original = owner::sentinel_scan_by_id(&tx, id)?;
        items.push(json!({"taskId":original.id,"title":observed_text(&original.task_name),"status":observed_text(&original.status),"attemptNumber":original.attempt_count,"updatedAt":observed_text(&original.updated_at),"archived":!original.archived_at.is_empty()}));
    }
    let next_cursor = if has_more {
        let (id, updated) = ids.last().ok_or("scan_cursor_invalid")?;
        valid_text(updated, 64, false)?;
        Some(
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    instance: service.instance_id.clone(),
                    project_id: context.host_bound_project_id,
                    order: "updated_at_desc_task_id_desc".into(),
                    updated_at: updated.clone(),
                    task_id: id.clone(),
                })
                .map_err(|_| "scan_cursor_invalid")?,
            ),
        )
    } else {
        None
    };
    service.require_custody(&tx)?;
    tx.commit().map_err(|_| "scan_list_snapshot_unconfirmed")?;
    Ok(
        json!({"items":items,"total":total,"returned":items.len(),"hasMore":has_more,"nextCursor":next_cursor,"limit":limit,"hostBoundProjectId":context.host_bound_project_id,"asOf":chrono::Utc::now().to_rfc3339(),"scope":{"ownedServiceTasksOnly":true,"deleted":false,"order":"updated_at_desc_task_id_desc","asOfIsPersistentSnapshot":false}}),
    )
}

pub(super) fn busy(service: &Service) -> Result<bool, String> {
    service.require_custody(&service.database)?;
    let tx =
        rusqlite::Transaction::new_unchecked(&service.database, TransactionBehavior::Immediate)
            .map_err(|_| "scan_service_busy_unknown")?;
    service.require_custody(&tx)?;
    let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE status IN ('queued','scanning','pausing')) OR EXISTS(SELECT 1 FROM scan_service_actions WHERE state IN ('prepared','uncertain')) OR EXISTS(SELECT 1 FROM sentinel_processes) OR EXISTS(SELECT 1 FROM analyzer_container_receipts WHERE cleanup_status<>'confirmed')",[],|r|r.get(0)).map_err(|_| "scan_service_busy_unknown")?;
    if active {
        return Ok(true);
    }
    let scans: Vec<String> = tx
        .prepare("SELECT id FROM sentinel_scans ORDER BY id")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(|_| "scan_service_busy_unknown")?;
    let mut guards = Vec::new();
    for scan in scans {
        match owner::claim_scan_quiescence_in(&tx, &service.db_path, &scan) {
            Ok(owners) => guards.extend(owners),
            Err(_) => return Ok(true),
        }
    }
    // Joining physical callers is not evidence that a provider invoice or a
    // request outcome is settled. Reuse the original read-only historical proof;
    // it accepts expired deadlines without creating a fresh execution grant.
    let roots:Vec<String>=tx.prepare("SELECT root_run_id FROM agent_root_budget_attempts UNION SELECT id FROM agent_runs WHERE backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id='' ORDER BY 1")
        .and_then(|mut s|s.query_map([],|r|r.get(0))?.collect()).map_err(|_| "scan_service_busy_unknown")?;
    for root in roots {
        if crate::agent_runtime::multi_agent::budget::continuation::verify_settled_history_on(
            &tx, &root,
        )
        .is_err()
        {
            return Ok(true);
        }
    }
    service.require_custody(&tx)?;
    tx.commit().map_err(|_| "scan_service_busy_unknown")?;
    drop(guards);
    Ok(false)
}

#[cfg(test)]
#[path = "native_service_execution_tests.rs"]
mod tests;
