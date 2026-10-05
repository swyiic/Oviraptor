// Historical trace rendering reads the canonical projection exclusively. No
// external path, source SQLite database, JSONL log, or sealed original is opened.
struct HistoricalTraceRecord {
    id: i64,
    bundle: i64,
    attempt: i64,
    data: JsonValue,
}

impl HistoricalTraceRecord {
    fn text(&self, key: &str) -> &str {
        self.data.get(key).and_then(JsonValue::as_str).unwrap_or("")
    }

    fn source_key(&self) -> String {
        format!("{}:{}:{}", self.bundle, self.attempt, self.text("trace_source"))
    }
}

fn historical_trace_records(
    connection: &rusqlite::Connection,
    scan_id: &str,
    latest_attempt_only: bool,
) -> Result<Vec<HistoricalTraceRecord>, String> {
    let mut statement = connection.prepare(
        "SELECT m.id,m.bundle_row_id,m.attempt_number,r.record_kind,r.adapter,r.envelope_json,m.source_path,s.task_path \
         FROM import_projection_memberships m \
         JOIN import_record_revisions r ON r.id=m.revision_id \
         JOIN import_bundles b ON b.id=m.bundle_row_id \
         JOIN sentinel_scans s ON s.id=?1 \
         WHERE m.current=1 AND m.tombstone=0 AND b.status IN ('imported','unchanged') \
           AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d \
               WHERE d.scan_id=s.id) \
           AND m.scope_key='scan='||s.id AND s.task_path<>'' \
         ORDER BY m.attempt_number,m.id LIMIT 100001"
    ).map_err(|error| format!("无法准备历史 trace：{error}"))?;
    let mut rows = statement.query_map([scan_id], |row| Ok((
        row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?,
        row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
        row.get::<_, String>(6)?, row.get::<_, String>(7)?,
    ))).map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    if rows.len() > 100_000 {
        return Err("历史 trace 超过单次展示上限；请按任务拆分导出，不截断为完整记录".into());
    }
    // Bundles may live below task_path (reports/ or attempt directories). Check
    // path components, never string prefixes, and never reopen offline sources.
    rows.retain(|row| {
        let source = Path::new(&row.6);
        let task = Path::new(&row.7);
        !source.components().chain(task.components()).any(|component| matches!(component, std::path::Component::ParentDir))
            && source.starts_with(task)
    });
    let stored_attempt = if latest_attempt_only {
        connection.query_row("SELECT MAX(attempt_number) FROM sentinel_scan_attempts WHERE scan_id=?1", [scan_id], |row| row.get::<_, Option<i64>>(0))
            .map_err(|error| error.to_string())?
    } else { None };
    // Scope the attempt ceiling before rejecting unsupported formats/claims.
    // Otherwise a rejected latest attempt could reveal older evidence as current.
    let ceiling = stored_attempt.into_iter().chain(rows.iter().map(|row| row.2)).max();
    let mut records = Vec::new();
    for (id, bundle, attempt, kind, adapter, text, _, _) in rows {
        if latest_attempt_only && Some(attempt) != ceiling { continue; }
        // Never parse retired adapters, including malformed persisted payloads.
        if adapter != "model_audit" || kind != "event_trace" { continue; }
        let envelope: JsonValue = serde_json::from_str(&text).map_err(|error| format!("历史 trace envelope 损坏：{error}"))?;
        if !current_audit_trace_envelope(&envelope) { continue; }
        let mut data = envelope.get("extensions").and_then(JsonValue::as_object).cloned().unwrap_or_default();
        data.extend(envelope.get("payload").and_then(JsonValue::as_object).cloned().unwrap_or_default());
        if !matches!(data.get("trace_kind").and_then(JsonValue::as_str), Some("model_hook" | "prompt_audit")) { continue; }
        records.push(HistoricalTraceRecord { id, bundle, attempt, data: retained_trace_value(&JsonValue::Object(data)) });
    }
    records.sort_by(|a,b| (a.attempt, a.source_key(), a.data.get("line").and_then(JsonValue::as_i64).unwrap_or(0), a.id)
        .cmp(&(b.attempt, b.source_key(), b.data.get("line").and_then(JsonValue::as_i64).unwrap_or(0), b.id)));
    Ok(records)
}

fn current_audit_trace_envelope(envelope: &JsonValue) -> bool {
    [
        ("/canonicalSchema", "oviraptor.artifact.v1"),
        ("/recordKind", "event_trace"),
        ("/provenance/importAdapter", "model_audit"),
        ("/claim/authority", "historical_external"),
        ("/claim/reviewState", "unreviewed"),
    ].iter().all(|(path, expected)| envelope.pointer(path).and_then(JsonValue::as_str) == Some(*expected))
        && envelope.pointer("/claim/readOnly").and_then(JsonValue::as_bool) == Some(true)
        && envelope.pointer("/claim/executionEligible").and_then(JsonValue::as_bool) == Some(false)
}

fn historical_prompt_audit(connection: &rusqlite::Connection, scan_id: &str) -> Result<Option<ModelPromptAudit>, String> {
    let records = historical_trace_records(connection, scan_id, true)?;
    Ok(records.iter().rev().filter(|row| row.text("trace_kind") == "prompt_audit").find_map(|row| {
        let mut audit: ModelPromptAudit = serde_json::from_value(row.data.clone()).ok()?;
        audit.exact_model_request = false;
        audit.capture_level = "generated_instruction".into();
        audit.instruction = audit.instruction.as_deref().map(retained_trace_text);
        audit.notice = "历史外部提示词审计：只读、未复核、脱敏展示；不是 Native 模型请求，不赋予任何执行权限。".into();
        Some(audit)
    }))
}

fn add_historical_trace_usage(total: &mut llm_hook::UsageTotals, part: &llm_hook::UsageTotals) {
    total.requests = total.requests.saturating_add(part.requests);
    total.failed_requests = total.failed_requests.saturating_add(part.failed_requests);
    total.input_tokens = total.input_tokens.saturating_add(part.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(part.output_tokens);
    total.cached_tokens = total.cached_tokens.saturating_add(part.cached_tokens);
    total.total_tokens = total.total_tokens.saturating_add(part.total_tokens);
}

fn collect_historical_agent_trace(
    connection: &rusqlite::Connection,
    scan_id: &str,
    include_events: bool,
    latest_attempt_only: bool,
) -> Result<(AgentTraceSummary, Vec<AgentTraceEvent>), String> {
    let (task_name, project_name, status, scan_type, _, _, _, _, _, _, created_at, updated_at) = agent_trace_base(connection, scan_id)?;
    let records = historical_trace_records(connection, scan_id, latest_attempt_only)?;
    let hooks: Vec<_> = records.iter().filter(|row| row.text("trace_kind") == "model_hook").collect();
    // Request IDs are file-scoped. Pending requests contribute no invented usage.
    let mut hook_sources: HashMap<String, Vec<JsonValue>> = HashMap::new();
    for row in &hooks {
        hook_sources.entry(row.source_key()).or_default().push(row.data.clone());
    }
    let mut usage = llm_hook::UsageTotals::default();
    for values in hook_sources.values() {
        add_historical_trace_usage(&mut usage, &llm_hook::usage_from_records(values));
    }
    let hooked_request_count = usage.requests;
    let token_usage_estimated = hooks.iter().any(|row| row.data.get("usageEstimated").and_then(JsonValue::as_bool) == Some(true));
    let exact_request_capture = hooks.iter().any(|row| row.data.get("request").is_some_and(JsonValue::is_object));
    let completed_hooks: HashSet<_> = hooks.iter().filter(|row| row.text("kind") == "model_call")
        .map(|row| (row.source_key(), row.text("requestId").to_string())).collect();
    let mut model = String::new();
    let mut events = Vec::new();
    for row in &hooks {
        if model.is_empty() { model = row.text("model").to_string(); }
        if !include_events || (row.text("kind") == "model_call_started" && completed_hooks.contains(&(row.source_key(), row.text("requestId").to_string()))) { continue; }
        let (detail, detail_size, detail_truncated) = trace_preview(&row.data.to_string(), 20_000);
        events.push(AgentTraceEvent {
            id: format!("import:{}", row.id), session_id: row.source_key(), call_id: row.text("requestId").to_string(), target_url: String::new(),
            event_type: "model_request".into(), role: "model".into(), name: row.text("model").into(),
            status: if row.text("status").is_empty() { if row.text("kind") == "model_call_started" { "in_flight" } else { "recorded" } } else { row.text("status") }.into(),
            detail, detail_size, detail_truncated, created_at: row.text("recordedAt").into(),
        });
    }
    events.sort_by(|a,b| a.created_at.cmp(&b.created_at));
    let knowledge_id = connection.query_row("SELECT id FROM agent_knowledge_entries WHERE scan_id=?1", [scan_id], |row| row.get(0)).optional().map_err(|error| error.to_string())?;
    Ok((AgentTraceSummary {
        scan_id: scan_id.into(), task_name, project_name, status, scan_type, model,
        source_authority: "historical_external".into(),
        // Imported audits do not establish Native runs, agents, messages or tools.
        run_count: 0, agent_count: 0, message_count: 0, reasoning_count: 0, tool_call_count: 0, tool_result_count: 0,
        llm_requests: usage.requests, input_tokens: usage.input_tokens, output_tokens: usage.output_tokens, cached_tokens: usage.cached_tokens, total_tokens: usage.total_tokens,
        hooked_request_count, exact_request_capture, usage_entry_count: 0, usage_agent_count: 0,
        token_usage_estimated, instruction_hash: String::new(), tools: Vec::new(), knowledge_id, created_at, updated_at,
    }, events))
}
