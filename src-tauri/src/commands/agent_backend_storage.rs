// Persisted authority must distinguish absence from unreadable/unsupported data.
// These readers never turn a failed decode into permission to select Native.
fn read_scan_backend_plan(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
) -> Result<Option<ScanBackendPlan>, String> {
    let connection = db::open(db_path).map_err(|_| "backend_plan_storage_unavailable")?;
    let stored: Option<String> = connection.query_row(
        "SELECT backend_plan_json FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt_number], |row| row.get(0),
    ).optional().map_err(|_| "backend_plan_read_failed")?;
    if let Some(raw) = stored.filter(|raw| !raw.trim().is_empty()) {
        let value: JsonValue = serde_json::from_str(&raw).map_err(|_| "backend_plan_invalid")?;
        return checked_backend_matrix(&value, scan_id, attempt_number).map(Some);
    }
    let staged: Option<String> = connection.query_row(
        "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url='' AND stage='scan_backend_plan'",
        [scan_id], |row| row.get(0),
    ).optional().map_err(|_| "backend_projection_read_failed")?;
    let Some(raw) = staged else {
        return Ok(None);
    };
    let value: JsonValue = serde_json::from_str(&raw).map_err(|_| "backend_projection_invalid")?;
    if value.get("scanId").and_then(JsonValue::as_str) != Some(scan_id) {
        return Err("backend_projection_scan_mismatch".into());
    }
    let previous = value
        .get("attemptNumber")
        .and_then(JsonValue::as_i64)
        .filter(|attempt| *attempt > 0)
        .ok_or("backend_projection_attempt_invalid")?;
    // A new attempt never inherits a previous attempt's projection implicitly.
    if previous != attempt_number {
        return Ok(None);
    }
    checked_backend_matrix(&value, scan_id, attempt_number).map(Some)
}

fn checked_backend_matrix(
    value: &JsonValue,
    scan_id: &str,
    attempt: i64,
) -> Result<ScanBackendPlan, String> {
    let plan = ScanBackendPlan::from_json(value).ok_or("backend_plan_invalid")?;
    if plan.scan_id != scan_id
        || value.get("attemptNumber").and_then(JsonValue::as_i64) != Some(attempt)
    {
        return Err("backend_plan_scope_mismatch".into());
    }
    let unique = plan
        .targets
        .iter()
        .map(|target| target.url.as_str())
        .collect::<std::collections::HashSet<_>>();
    if unique.len() != plan.targets.len()
        || plan
            .targets
            .iter()
            .any(|target| target.url.trim().is_empty())
    {
        return Err("backend_plan_targets_invalid".into());
    }
    Ok(plan)
}

fn persisted_attempt_backend(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    url: &str,
) -> Result<Option<AgentBackendKind>, String> {
    let connection = db::open(db_path).map_err(|_| "backend_plan_storage_unavailable")?;
    let mut statement = connection.prepare(
        "SELECT backend,plan_json FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role='coordinator'",
    ).map_err(|_| "backend_target_plan_read_failed")?;
    let rows = statement
        .query_map(params![scan_id, attempt_number, url], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| "backend_target_plan_read_failed")?;
    let mut selected = None;
    let mut observed = None;
    for row in rows {
        let (backend, raw) = row.map_err(|_| "backend_target_plan_read_failed")?;
        let backend = AgentBackendKind::parse(&backend).ok_or("backend_target_unsupported")?;
        if observed.is_some_and(|previous| previous != backend) {
            return Err("backend_target_conflict".into());
        }
        observed = Some(backend);
        // Empty initial records are not a frozen plan, but a non-native marker
        // still must prevent automatic Native activation.
        if raw.is_empty() || raw == "{}" {
            if backend != AgentBackendKind::Native {
                return Err("backend_target_not_executable".into());
            }
            continue;
        }
        let value: JsonValue =
            serde_json::from_str(&raw).map_err(|_| "backend_target_plan_invalid")?;
        let plan = AgentExecutionPlan::from_json(&value).ok_or("backend_target_plan_invalid")?;
        if plan.backend != backend
            || plan.attempt_number != attempt_number
            || plan.target_url != url
        {
            return Err("backend_target_plan_mismatch".into());
        }
        selected = Some(backend);
    }
    Ok(selected)
}
