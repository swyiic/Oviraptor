#[tauri::command]
pub fn get_native_attempt_mailbox_history(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    before_created_at: Option<String>,
    before_id: Option<String>,
    limit: Option<i64>,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    native_attempt_mailbox_history(
        &connection, &scan_id, attempt_number,
        before_created_at.as_deref(), before_id.as_deref(), limit.unwrap_or(50),
    )
}

fn native_attempt_mailbox_history(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    before_created_at: Option<&str>,
    before_id: Option<&str>,
    limit: i64,
) -> Result<JsonValue, String> {
    if attempt_number < 1 || before_created_at.is_some() != before_id.is_some()
        || before_created_at.is_some_and(str::is_empty) || before_id.is_some_and(str::is_empty)
    {
        return Err("invalid_attempt_mailbox_cursor".into());
    }
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts a JOIN sentinel_scans s ON s.id=a.scan_id \
         WHERE a.scan_id=?1 AND a.attempt_number=?2 \
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id))",
        params![scan_id, attempt_number], |row| row.get(0),
    ).map_err(|error| format!("无法核对执行轮次：{error}"))?;
    if !exists { return Err("attempt_not_found".into()); }
    let page_size = limit.clamp(1, 100) as usize;
    let mut statement = connection.prepare(
        "SELECT m.id,m.created_at,m.from_agent,m.from_run_id,m.to_agent,m.to_run_id,m.kind, \
         m.correlation_id,m.assignment_id,m.evidence_revision,m.delivered_at,m.acknowledged_at,m.payload_json \
         FROM agent_messages m \
         JOIN agent_runs root ON root.id=m.root_run_id AND root.role='coordinator' \
         JOIN agent_runs owner ON owner.id=m.run_id AND owner.scan_id=root.scan_id \
           AND owner.attempt_number=root.attempt_number AND (owner.id=root.id OR owner.root_run_id=root.id) \
         WHERE root.scan_id=?1 AND root.attempt_number=?2 \
           AND (?3 IS NULL OR m.created_at<?3 OR (m.created_at=?3 AND m.id<?4)) \
         ORDER BY m.created_at DESC,m.id DESC LIMIT ?5",
    ).map_err(|error| format!("无法读取 Agent 消息：{error}"))?;
    let rows = statement.query_map(
        params![scan_id, attempt_number, before_created_at, before_id, (page_size + 1) as i64],
        |row| {
            let payload: String = row.get(12)?;
            let value = serde_json::from_str::<JsonValue>(&payload).unwrap_or(JsonValue::Null);
            let kind: String = row.get(6)?;
            let summary = value.get("summary").or_else(|| value.get("verdict"))
                .or_else(|| value.get("text")).and_then(JsonValue::as_str).unwrap_or(&kind);
            let summary = crate::agent_runtime::secrets::redact_text_with(summary, None)
                .chars().take(500).collect::<String>();
            let delivered: String = row.get(10)?;
            let acknowledged: String = row.get(11)?;
            Ok(json!({
                "id":row.get::<_,String>(0)?, "createdAt":row.get::<_,String>(1)?,
                "fromRole":row.get::<_,String>(2)?, "fromRunId":row.get::<_,String>(3)?,
                "toRole":row.get::<_,String>(4)?, "toRunId":row.get::<_,String>(5)?,
                "kind":kind, "correlationId":row.get::<_,String>(7)?,
                "assignmentId":row.get::<_,String>(8)?, "evidenceRevision":row.get::<_,i64>(9)?,
                "deliveryState":if delivered.is_empty(){"pending"}else{"delivered"},
                "ackState":if acknowledged.is_empty(){"pending"}else{"acknowledged"},
                "summary":summary,
            }))
        },
    ).map_err(|error| format!("无法查询 Agent 消息：{error}"))?;
    let mut messages: Vec<JsonValue> = rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析 Agent 消息：{error}"))?;
    let has_older = messages.len() > page_size;
    messages.truncate(page_size);
    messages.reverse();
    let older_cursor = messages.first().map(|first| json!({
        "createdAt":first["createdAt"], "id":first["id"],
    }));
    Ok(json!({"scanId":scan_id,"attemptNumber":attempt_number,
        "messages":messages,"hasOlder":has_older,"olderCursor":older_cursor}))
}

#[tauri::command]
pub fn get_native_attempt_tool_history(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    before_started_at: Option<String>,
    before_id: Option<i64>,
    limit: Option<i64>,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    native_attempt_tool_history(
        &connection, &scan_id, attempt_number,
        before_started_at.as_deref(), before_id, limit.unwrap_or(50),
    )
}

fn native_attempt_tool_history(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    before_started_at: Option<&str>,
    before_id: Option<i64>,
    limit: i64,
) -> Result<JsonValue, String> {
    if attempt_number < 1 || before_started_at.is_some() != before_id.is_some()
        || before_started_at.is_some_and(str::is_empty) || before_id.is_some_and(|id| id < 1)
    {
        return Err("invalid_attempt_tool_cursor".into());
    }
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts a JOIN sentinel_scans s ON s.id=a.scan_id \
         WHERE a.scan_id=?1 AND a.attempt_number=?2 \
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id))",
        params![scan_id, attempt_number], |row| row.get(0),
    ).map_err(|error| format!("无法核对执行轮次：{error}"))?;
    if !exists { return Err("attempt_not_found".into()); }
    let page_size = limit.clamp(1, 100) as usize;
    let mut statement = connection.prepare(
        "SELECT t.id,t.invocation_id,t.run_id,r.role,t.tool_name,t.status,t.policy_decision, \
         t.started_at,t.finished_at,t.request_artifact_id,t.response_artifact_id,t.error_class \
         FROM tool_invocations t JOIN agent_runs r ON r.id=t.run_id \
         WHERE r.scan_id=?1 AND r.attempt_number=?2 \
           AND (?3 IS NULL OR t.started_at<?3 OR (t.started_at=?3 AND t.id<?4)) \
         ORDER BY t.started_at DESC,t.id DESC LIMIT ?5",
    ).map_err(|error| format!("无法读取工具调用：{error}"))?;
    let rows = statement.query_map(
        params![scan_id, attempt_number, before_started_at, before_id, (page_size + 1) as i64],
        |row| {
            let error: String = row.get(11)?;
            Ok(json!({
                "id":row.get::<_,i64>(0)?, "invocationId":row.get::<_,String>(1)?,
                "runId":row.get::<_,String>(2)?, "role":row.get::<_,String>(3)?,
                "toolName":row.get::<_,String>(4)?, "status":row.get::<_,String>(5)?,
                "policyDecision":row.get::<_,String>(6)?, "startedAt":row.get::<_,String>(7)?,
                "finishedAt":row.get::<_,String>(8)?,
                "hasRequestArtifact":!row.get::<_,String>(9)?.is_empty(),
                "hasResponseArtifact":!row.get::<_,String>(10)?.is_empty(),
                "errorClass":crate::agent_runtime::secrets::redact_text_with(&error, None)
                    .chars().take(200).collect::<String>(),
            }))
        },
    ).map_err(|error| format!("无法查询工具调用：{error}"))?;
    let mut invocations: Vec<JsonValue> = rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析工具调用：{error}"))?;
    let has_older = invocations.len() > page_size;
    invocations.truncate(page_size);
    invocations.reverse();
    let older_cursor = invocations.first().map(|first| json!({
        "startedAt":first["startedAt"], "id":first["id"],
    }));
    Ok(json!({"scanId":scan_id,"attemptNumber":attempt_number,
        "invocations":invocations,"hasOlder":has_older,"olderCursor":older_cursor}))
}
