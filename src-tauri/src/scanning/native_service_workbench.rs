//! Bounded public adapter for the original Source/Greybox/CI startup owner.
//! This journal never interprets an accepted operation as a completed analysis.
use super::*;
use crate::scanning as owner;

const SCHEMA: &str = r#"
CREATE TABLE scan_service_workbench_schema(version INTEGER PRIMARY KEY CHECK(version=1));
INSERT INTO scan_service_workbench_schema VALUES(1);
CREATE TABLE scan_service_workbench_starts(project_id INTEGER NOT NULL REFERENCES projects(id),operation_id TEXT NOT NULL,instance_id TEXT NOT NULL REFERENCES scan_service_store(instance_id),source_ref TEXT NOT NULL REFERENCES scan_service_sources(source_ref),arguments_sha256 TEXT NOT NULL,grant_revision INTEGER NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','completed','rejected','uncertain')),receipt_json TEXT NOT NULL DEFAULT '',PRIMARY KEY(project_id,operation_id));
CREATE TABLE scan_service_source_tasks(scan_id TEXT PRIMARY KEY REFERENCES sentinel_scans(id),project_id INTEGER NOT NULL REFERENCES projects(id),instance_id TEXT NOT NULL REFERENCES scan_service_store(instance_id),source_ref TEXT NOT NULL REFERENCES scan_service_sources(source_ref));
CREATE TRIGGER scan_service_workbench_schema_no_update BEFORE UPDATE ON scan_service_workbench_schema BEGIN SELECT RAISE(ABORT,'workbench schema immutable');END;
CREATE TRIGGER scan_service_workbench_schema_no_delete BEFORE DELETE ON scan_service_workbench_schema BEGIN SELECT RAISE(ABORT,'workbench schema immutable');END;
CREATE TRIGGER scan_service_source_tasks_no_update BEFORE UPDATE ON scan_service_source_tasks BEGIN SELECT RAISE(ABORT,'source task binding immutable');END;
CREATE TRIGGER scan_service_source_tasks_no_delete BEFORE DELETE ON scan_service_source_tasks BEGIN SELECT RAISE(ABORT,'source task binding immutable');END;
CREATE TRIGGER scan_service_workbench_starts_binding BEFORE UPDATE OF project_id,operation_id,instance_id,source_ref,arguments_sha256,grant_revision ON scan_service_workbench_starts BEGIN SELECT RAISE(ABORT,'workbench binding immutable');END;
CREATE TRIGGER scan_service_workbench_starts_state BEFORE UPDATE ON scan_service_workbench_starts WHEN OLD.state<>'prepared' OR NEW.state NOT IN ('completed','rejected','uncertain') OR NEW.receipt_json='' BEGIN SELECT RAISE(ABORT,'workbench transition invalid');END;
CREATE TRIGGER scan_service_workbench_starts_no_delete BEFORE DELETE ON scan_service_workbench_starts BEGIN SELECT RAISE(ABORT,'workbench operation immutable');END;
"#;

pub(super) fn enable(service: &Service) -> Result<(), String> {
    service.require_custody(&service.database)?;
    let tx =
        rusqlite::Transaction::new_unchecked(&service.database, TransactionBehavior::Immediate)
            .map_err(|_| "scan_workbench_schema_unavailable")?;
    service.require_custody(&tx)?;
    let objects:i64=tx.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name IN ('scan_service_workbench_schema','scan_service_workbench_starts','scan_service_source_tasks','scan_service_workbench_schema_no_update','scan_service_workbench_schema_no_delete','scan_service_source_tasks_no_update','scan_service_source_tasks_no_delete','scan_service_workbench_starts_binding','scan_service_workbench_starts_state','scan_service_workbench_starts_no_delete')",[],|r|r.get(0)).map_err(|_| "scan_workbench_schema_unknown")?;
    if objects == 0 {
        tx.execute_batch(SCHEMA)
            .map_err(|_| "scan_workbench_schema_unavailable")?;
    } else if objects != 10 {
        return Err("scan_workbench_schema_partial".into());
    }
    let versions: Vec<i64> = tx
        .prepare("SELECT version FROM scan_service_workbench_schema")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(|_| "scan_workbench_schema_unknown")?;
    if versions != [1] {
        return Err("scan_workbench_schema_unknown".into());
    }
    for line in SCHEMA.lines().filter(|s| s.starts_with("CREATE ")) {
        let expected = line.trim_end_matches(';');
        let name = expected
            .split_whitespace()
            .nth(2)
            .ok_or("scan_workbench_schema_unknown")?
            .split('(')
            .next()
            .unwrap();
        let actual: String = tx
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .map_err(|_| "scan_workbench_schema_unknown")?;
        if actual != expected {
            return Err("scan_workbench_schema_unknown".into());
        }
    }
    service.require_custody(&tx)?;
    tx.commit()
        .map_err(|_| "scan_workbench_schema_commit_unconfirmed".to_string())
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Start {
    source_ref: String,
    title: String,
    scan_type: String,
    #[serde(default)]
    urls: Vec<String>,
    #[serde(default)]
    skill_ids: Vec<i64>,
    #[serde(default)]
    instruction: String,
    scan_mode: Option<String>,
    max_budget_usd: Option<f64>,
    #[serde(default)]
    environment: String,
    #[serde(default)]
    max_critical: i64,
    #[serde(default = "default_max_high")]
    max_high: i64,
    #[serde(default)]
    block_release: bool,
}
fn default_max_high() -> i64 {
    5
}
impl Start {
    pub(super) fn validate(&self) -> Result<(), String> {
        if !uuid::Uuid::parse_str(&self.source_ref).is_ok_and(|v| v.to_string() == self.source_ref)
            || !matches!(self.scan_type.as_str(), "code" | "greybox" | "cicd")
            || self.urls.len() > 200
            || (self.scan_type == "greybox" && self.urls.is_empty())
            || self.skill_ids.len() > 16384
            || self
                .skill_ids
                .iter()
                .any(|n| !(1..=MAX_INTEGER).contains(n))
            || self
                .scan_mode
                .as_ref()
                .is_some_and(|s| !matches!(s.as_str(), "quick" | "standard" | "deep"))
            || !(0..=10000).contains(&self.max_critical)
            || !(0..=10000).contains(&self.max_high)
        {
            return Err("scan_workbench_arguments_invalid".into());
        }
        valid_text(&self.title, 120, false)?;
        valid_text(&self.environment, 80, true)?;
        if self.instruction.len() > 16 * 1024
            || self
                .instruction
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err("scan_workbench_arguments_invalid".into());
        }
        owner::workbench_budget(self.max_budget_usd)?;
        // Preserve the existing original URL normalizer and closed public URL
        // contract; a workbench may contain up to its original 200 targets.
        for chunk in self.urls.chunks(32) {
            NewDraft {
                title: self.title.clone(),
                targets: chunk.to_vec(),
                scan_mode: self.scan_mode.clone(),
            }
            .validate()?;
        }
        Ok(())
    }
    pub(super) fn validate_tool(&self) -> Result<(), String> {
        if self.urls.len() > 128 || self.skill_ids.len() > 128 {
            return Err("scan_workbench_tool_bounds_invalid".into());
        }
        self.validate()
    }
    fn original(
        &self,
        project: i64,
        path: &Path,
    ) -> Result<crate::scanning_models::WorkbenchScanInput, String> {
        Ok(crate::scanning_models::WorkbenchScanInput {
            project_id: project,
            task_name: self.title.clone(),
            scan_type: self.scan_type.clone(),
            urls: self.urls.clone(),
            source_path: path.to_str().ok_or("scan_source_path_invalid")?.into(),
            skill_ids: self.skill_ids.clone(),
            instruction: self.instruction.clone(),
            scan_mode: self.scan_mode.clone().unwrap_or_default(),
            scope_mode: "full".into(),
            diff_base: String::new(),
            max_budget_usd: self.max_budget_usd,
            environment: self.environment.clone(),
            auth_profile_name: String::new(),
            auth_type: String::new(),
            auth_header_name: String::new(),
            auth_value: String::new(),
            auth_session_id: String::new(),
            auth_session_ids: Vec::new(),
            auth_session_scope_id: String::new(),
            ci_provider: String::new(),
            repository_url: String::new(),
            branch: String::new(),
            commit_sha: String::new(),
            build_id: String::new(),
            max_critical: self.max_critical,
            max_high: self.max_high,
            block_release: self.block_release,
        })
    }
}

fn require_claim(
    db: &Connection,
    service: &Service,
    context: &Context,
    input: &Start,
    hash: &str,
) -> Result<(), String> {
    service.require_custody(db)?;
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM scan_service_workbench_starts WHERE project_id=?1 AND operation_id=?2 AND instance_id=?3 AND source_ref=?4 AND arguments_sha256=?5 AND grant_revision=?6 AND state='prepared' AND receipt_json='')",params![context.host_bound_project_id,context.operation_id,service.instance_id,input.source_ref,hash,context.grant_revision],|r|r.get(0)).map_err(|_| "scan_workbench_claim_unknown")?;
    if valid {
        Ok(())
    } else {
        Err("scan_workbench_claim_changed".into())
    }
}

#[cfg(unix)]
fn publish_ownership_on(
    service: &Service,
    context: &Context,
    input: &Start,
    hash: &str,
    binary: &str,
    tx: &Connection,
    scan: &crate::scanning_models::SentinelScan,
) -> Result<(), String> {
    require_claim(tx, service, context, &input, &hash)?;
    let verified = sources::resolve_on(service, tx, context, &input.source_ref)?;
    if scan.project_id != Some(context.host_bound_project_id)
        || scan.source_path != verified.path().to_str().ok_or("scan_source_path_invalid")?
        || scan.scan_type != input.scan_type
        || scan.attempt_count != 1
    {
        return Err("scan_workbench_publication_scope_changed".into());
    }
    let receipt = json!({"kind":"original_workbench_publication_receipt","liveState":false,"taskId":scan.id,"hostBoundProjectId":context.host_bound_project_id,"attemptNumber":scan.attempt_count,"executionRequested":true,"dispatchEvidenceIncluded":false,"sourceRef":input.source_ref,"snapshotOnly":true,"diffSupported":false});
    let n = tx
        .execute(
            "INSERT INTO scan_service_operations VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                context.host_bound_project_id,
                context.operation_id,
                hash,
                service.instance_id,
                scan.id,
                receipt.to_string()
            ],
        )
        .map_err(|_| "scan_operation_publication_conflict")?;
    let m = tx
        .execute(
            "INSERT INTO scan_service_source_tasks VALUES(?1,?2,?3,?4)",
            params![
                scan.id,
                context.host_bound_project_id,
                service.instance_id,
                input.source_ref
            ],
        )
        .map_err(|_| "scan_workbench_publication_conflict")?;
    let b = tx
        .execute(
            "INSERT INTO scan_service_execution_births VALUES(?1,?2,?3,?4)",
            params![
                scan.id,
                service.instance_id,
                binary,
                service.db_path.to_str().ok_or("scan_store_path_invalid")?
            ],
        )
        .map_err(|_| "scan_execution_birth_write_failed")?;
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM scan_service_operations o JOIN scan_service_source_tasks t ON t.scan_id=o.scan_id JOIN scan_service_execution_births b ON b.scan_id=o.scan_id WHERE o.project_id=?1 AND o.operation_id=?2 AND o.arguments_sha256=?3 AND o.instance_id=?4 AND o.scan_id=?5 AND o.receipt_json=?6 AND t.project_id=?1 AND t.instance_id=?4 AND t.source_ref=?7 AND b.instance_id=?4 AND b.executable_sha256=?8 AND b.canonical_database_path=?9)",params![context.host_bound_project_id,context.operation_id,hash,service.instance_id,scan.id,receipt.to_string(),input.source_ref,binary,service.db_path.to_str().ok_or("scan_store_path_invalid")?],|r|r.get(0)).map_err(|_| "scan_workbench_publication_unconfirmed")?;
    if n != 1 || m != 1 || b != 1 || !exact {
        return Err("scan_workbench_publication_unconfirmed".into());
    }
    service.require_custody(tx)
}

// The one task-creating call delegates to the mature workbench startup owner.
// No caller can select a DB, source path, model endpoint or private birth key.
#[cfg(unix)]
pub(super) fn start(service: &Service, context: &Context, input: Start) -> Result<Value, String> {
    service.authorize(context, "scan.execute")?;
    input.validate()?;
    service.require_custody(&service.database)?;
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&input).map_err(|_| "scan_workbench_arguments_invalid")?)
    );
    let binary = execution::executable_sha256()?;
    let tx =
        rusqlite::Transaction::new_unchecked(&service.database, TransactionBehavior::Immediate)
            .map_err(|_| "scan_workbench_claim_unavailable")?;
    service.require_custody(&tx)?;
    let source = sources::resolve_on(service, &tx, context, &input.source_ref)?;
    let old:Option<(String,String,String)>=tx.query_row("SELECT arguments_sha256,state,receipt_json FROM scan_service_workbench_starts WHERE project_id=?1 AND operation_id=?2 AND instance_id=?3",params![context.host_bound_project_id,context.operation_id,service.instance_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_| "scan_workbench_lookup_failed")?;
    if let Some((old_hash, state, receipt)) = old {
        if old_hash != hash {
            return Err("scan_operation_conflict".into());
        }
        if state == "prepared" {
            return Err("scan_action_outcome_uncertain_no_automatic_replay".into());
        }
        let value = serde_json::from_str(&receipt).map_err(|_| "scan_operation_receipt_invalid")?;
        service.require_custody(&tx)?;
        tx.commit()
            .map_err(|_| "scan_workbench_read_commit_unconfirmed")?;
        return Ok(value);
    }
    let conflicting:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM scan_service_operations WHERE project_id=?1 AND operation_id=?2)",params![context.host_bound_project_id,context.operation_id],|r|r.get(0)).map_err(|_| "scan_workbench_lookup_failed")?;
    if conflicting {
        return Err("scan_operation_conflict".into());
    }
    let changed = tx
        .execute(
            "INSERT INTO scan_service_workbench_starts VALUES(?1,?2,?3,?4,?5,?6,'prepared','')",
            params![
                context.host_bound_project_id,
                context.operation_id,
                service.instance_id,
                input.source_ref,
                hash,
                context.grant_revision
            ],
        )
        .map_err(|_| "scan_workbench_claim_failed")?;
    if changed != 1 {
        return Err("scan_workbench_claim_failed".into());
    }
    require_claim(&tx, service, context, &input, &hash)?;
    tx.commit()
        .map_err(|_| "scan_workbench_claim_commit_unconfirmed")?;
    let original = input.original(context.host_bound_project_id, source.path())?;
    let result = owner::start_workbench_scan_publishing_with_environment(
        &service._environment,
        &service.db_path,
        service._environment.data_root(),
        owner::WorkbenchStartRequest::New(Box::new(original)),
        |tx| {
            require_claim(tx, service, context, &input, &hash)?;
            sources::resolve_on(service, tx, context, &input.source_ref).map(|_| ())
        },
        |tx, scan| publish_ownership_on(service, context, &input, &hash, &binary, tx, scan),
    );
    service.require_custody(&service.database)?;
    let task:Option<String>=service.database.query_row("SELECT scan_id FROM scan_service_operations WHERE project_id=?1 AND operation_id=?2 AND instance_id=?3 AND arguments_sha256=?4",params![context.host_bound_project_id,context.operation_id,service.instance_id,hash],|r|r.get(0)).optional().map_err(|_| "scan_action_outcome_uncertain_no_automatic_replay")?;
    let (state, outcome) = match result {
        Ok(scan) if task.as_deref() == Some(scan.id.as_str()) => (
            "completed",
            json!({"accepted":true,"task":{"taskId":scan.id,"status":scan.status,"attemptNumber":scan.attempt_count}}),
        ),
        Ok(_) => (
            "uncertain",
            json!({"accepted":false,"disposition":"uncertain","executionStarted":"unknown","code":"scan_workbench_publication_unconfirmed"}),
        ),
        Err(code) if task.is_none() && !code.starts_with("workbench_start_commit_unconfirmed:") => {
            (
                "rejected",
                json!({"accepted":false,"disposition":"rejected","code":execution::engine_error(&code)}),
            )
        }
        Err(code) => (
            "uncertain",
            json!({"accepted":false,"disposition":"uncertain","executionStarted":"unknown","code":execution::engine_error(&code)}),
        ),
    };
    let receipt = json!({"kind":"scan_service_workbench_receipt","liveState":false,"operationId":context.operation_id,"hostBoundProjectId":context.host_bound_project_id,"taskId":task,"sourceRef":input.source_ref,"outcome":outcome});
    let tx =
        rusqlite::Transaction::new_unchecked(&service.database, TransactionBehavior::Immediate)
            .map_err(|_| "scan_action_outcome_uncertain_no_automatic_replay")?;
    require_claim(&tx, service, context, &input, &hash)?;
    let n=tx.execute("UPDATE scan_service_workbench_starts SET state=?1,receipt_json=?2 WHERE project_id=?3 AND operation_id=?4 AND state='prepared'",params![state,receipt.to_string(),context.host_bound_project_id,context.operation_id]).map_err(|_| "scan_action_outcome_uncertain_no_automatic_replay")?;
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM scan_service_workbench_starts WHERE project_id=?1 AND operation_id=?2 AND state=?3 AND receipt_json=?4)",params![context.host_bound_project_id,context.operation_id,state,receipt.to_string()],|r|r.get(0)).map_err(|_| "scan_action_outcome_uncertain_no_automatic_replay")?;
    if n != 1 || !exact {
        return Err("scan_action_outcome_uncertain_no_automatic_replay".into());
    }
    service.require_custody(&tx)?;
    tx.commit()
        .map_err(|_| "scan_action_outcome_uncertain_no_automatic_replay")?;
    Ok(receipt)
}

#[cfg(unix)]
fn task_source_on(
    service: &Service,
    db: &Connection,
    context: &Context,
    task: &str,
) -> Result<sources::VerifiedSource, String> {
    service.require_custody(db)?;
    let (reference,path):(String,String)=db.query_row("SELECT t.source_ref,s.source_path FROM scan_service_source_tasks t JOIN sentinel_scans s ON s.id=t.scan_id JOIN scan_service_operations o ON o.scan_id=t.scan_id WHERE t.scan_id=?1 AND t.project_id=?2 AND t.instance_id=?3 AND s.project_id=?2 AND o.project_id=?2 AND o.instance_id=?3 AND s.scan_type IN ('code','greybox','cicd') AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=t.scan_id)",params![task,context.host_bound_project_id,service.instance_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_| "scan_source_task_scope_unavailable")?;
    let verified = sources::resolve_on(service, db, context, &reference)?;
    if verified.path() != Path::new(&path) {
        return Err("scan_source_task_path_changed".into());
    }
    Ok(verified)
}

#[cfg(unix)]
pub(super) fn verify_task_on(
    service: &Service,
    db: &Connection,
    context: &Context,
    task: &str,
) -> Result<(), String> {
    task_source_on(service, db, context, task).map(|_| ())
}

// The existing action journal checks original service birth before entering.
// A Source Resume retains its desktop meaning: original workbench retry, never
// WebStartMode::Resume or a claim that old paid work became a cheap continuation.
#[cfg(unix)]
pub(super) fn retry(
    service: &Service,
    context: &Context,
    task: &str,
    resume: bool,
) -> Result<Value, String> {
    let source = task_source_on(service, &service.database, context, task)?;
    if resume {
        let status: String = service
            .database
            .query_row(
                "SELECT status FROM sentinel_scans WHERE id=?1",
                [task],
                |r| r.get(0),
            )
            .map_err(|_| "scan_task_scope_unavailable")?;
        if !matches!(status.as_str(), "paused" | "partial") {
            return Err("scan_workbench_resume_state_ineligible".into());
        }
        if let Some(refusal) = owner::retired_backend_resume_refusal(&service.database, task)? {
            return Err(refusal);
        }
    }
    let scan = owner::start_workbench_scan_publishing_with_environment(
        &service._environment,
        &service.db_path,
        service._environment.data_root(),
        owner::WorkbenchStartRequest::Retry(task.into()),
        |tx| task_source_on(service, tx, context, task).map(|_| ()),
        |tx, scan| {
            let verified = task_source_on(service, tx, context, task)?;
            if verified.path() != source.path() || scan.id != task {
                return Err("scan_source_task_scope_changed".into());
            }
            service.require_custody(tx)
        },
    )?;
    Ok(json!({"taskId":scan.id,"status":scan.status,"attemptNumber":scan.attempt_count}))
}

pub(super) fn unresolved_on(db: &Connection) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM scan_service_workbench_starts WHERE state IN ('prepared','uncertain'))",[],|r|r.get(0)).map_err(|_| "scan_workbench_busy_unknown".into())
}

#[cfg(test)]
#[path = "native_service_workbench_tests.rs"]
mod tests;
