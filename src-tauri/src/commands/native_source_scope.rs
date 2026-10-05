// A task.json is an audit artifact, not a recoverable execution grant. Freeze
// the operator's source selection in the same transaction as the new attempt.
#[derive(Clone,Debug,PartialEq,Eq,serde::Serialize)]
#[serde(rename_all="camelCase")]
struct WorkbenchSourceScope {
    source_path: String,
    canonical_root: String,
    scan_type: String,
    scope_mode: String,
    diff_base: String,
    analysis_policy_version: i64,
}

impl WorkbenchSourceScope {
    fn new(source_path: &str, scan_type: &str, scope_mode: &str, diff_base: &str) -> Result<Self,String> {
        if !matches!(scan_type,"code"|"greybox"|"cicd") || !matches!(scope_mode,"full"|"diff"|"auto") {
            return Err("workbench_source_scope_invalid".into());
        }
        let root = Path::new(source_path).canonicalize().map_err(|_| "workbench_source_root_unavailable")?;
        if !root.is_dir() { return Err("workbench_source_root_unavailable".into()); }
        // Full never inherits a hidden old base from the UI. Diff/auto retain
        // the requested reference, not a later mutable global setting.
        let base = if scope_mode=="full" { "" } else { diff_base.trim() };
        if base.len()>1024 || base.contains('\0') || (scope_mode=="diff" && base.is_empty()) {
            return Err("workbench_source_base_invalid".into());
        }
        Ok(Self {source_path:source_path.into(),canonical_root:root.to_str().ok_or("workbench_source_root_encoding")?.into(),
            scan_type:scan_type.into(),scope_mode:scope_mode.into(),diff_base:base.into(),analysis_policy_version:1})
    }

    fn from_record(r: &WorkbenchStartRecord) -> Result<Self,String> {
        Self::new(&r.source_path,&r.scan_type,&r.scope_mode,&r.diff_base)
    }

    fn verify_task_file(&self,r: &WorkbenchStartRecord,work_dir: &Path) -> Result<(),String> {
        let path=work_dir.join("task.json");
        if fs::symlink_metadata(&path).map_err(|_| "workbench_source_task_unavailable")?.file_type().is_symlink() {
            return Err("workbench_source_task_invalid".into());
        }
        // Share the existing 16 MiB task-artifact ceiling. The reader checks
        // the opened descriptor, bounds the read, and uses NOFOLLOW/NONBLOCK
        // on Unix; a path swapped for a FIFO must not stall publication.
        let bytes=read_workbench_retry_task(&path).map_err(|_| "workbench_source_task_unsafe_or_unavailable")?;
        let value: JsonValue=serde_json::from_slice(&bytes)
            .map_err(|_| "workbench_source_task_invalid")?;
        let text=|key: &str| value.get(key).and_then(JsonValue::as_str).ok_or("workbench_source_task_invalid");
        if text("scanId")?!=r.scan_id || value["attempt"].as_u64()!=Some(u64::from(r.attempt))
            || Self::new(text("sourcePath")?,text("scanType")?,text("scopeMode")?,text("diffBase")?)? != *self {
            return Err("workbench_source_task_mismatch".into());
        }
        Ok(())
    }

    fn verify_worker(&self,source_path: &str,scan_type: &str,diff_base: &str) -> Result<(),String> {
        if Self::new(source_path,scan_type,&self.scope_mode,diff_base)? != *self {
            return Err("workbench_source_worker_scope_mismatch".into());
        }
        Ok(())
    }
}

fn store_workbench_source_scope(connection: &rusqlite::Connection,scan_id: &str,attempt: i64,scope: &WorkbenchSourceScope) -> Result<(),String> {
    let count=connection.execute(
        "INSERT INTO source_scope_contracts(scan_id,attempt_number,source_path,canonical_root,scan_type,scope_mode,diff_base,analysis_policy_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![scan_id,attempt,scope.source_path,scope.canonical_root,scope.scan_type,scope.scope_mode,scope.diff_base,scope.analysis_policy_version],
    ).map_err(|error|format!("workbench_source_scope_not_persisted:{error}"))?;
    if count!=1 { return Err("workbench_source_scope_postcondition".into()); }
    verify_workbench_source_scope(connection,scan_id,attempt,scope)
}

fn load_workbench_source_scope(connection: &rusqlite::Connection,scan_id: &str,attempt: i64) -> Result<WorkbenchSourceScope,String> {
    let scope=connection.query_row(
        "SELECT p.source_path,p.canonical_root,p.scan_type,p.scope_mode,p.diff_base,p.analysis_policy_version FROM source_scope_contracts p
         JOIN sentinel_scans s ON s.id=p.scan_id AND s.attempt_count=p.attempt_number
         JOIN sentinel_scan_attempts a ON a.scan_id=p.scan_id AND a.attempt_number=p.attempt_number
         WHERE p.scan_id=?1 AND p.attempt_number=?2 AND s.status='scanning' AND a.status='scanning'
         AND s.scan_type=p.scan_type AND s.source_path=p.source_path AND p.analysis_policy_version=1
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=p.scan_id)",
        params![scan_id,attempt],|r| Ok(WorkbenchSourceScope{source_path:r.get(0)?,canonical_root:r.get(1)?,
            scan_type:r.get(2)?,scope_mode:r.get(3)?,diff_base:r.get(4)?,analysis_policy_version:r.get(5)?}),
    ).map_err(|_| "workbench_source_scope_unavailable_start_new_attempt")?;
    if WorkbenchSourceScope::new(&scope.source_path,&scope.scan_type,&scope.scope_mode,&scope.diff_base)?!=scope {
        return Err("workbench_source_scope_root_or_contract_changed".into());
    }
    Ok(scope)
}

fn verify_workbench_source_scope(connection: &rusqlite::Connection,scan_id: &str,attempt: i64,expected: &WorkbenchSourceScope) -> Result<(),String> {
    if load_workbench_source_scope(connection,scan_id,attempt)?!=*expected {
        return Err("workbench_source_scope_changed".into());
    }
    Ok(())
}
