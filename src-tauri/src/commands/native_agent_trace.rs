// Native execution traces come from the append-only run ledger and mailbox.
// Historical external records have a separate read-only import projection.
fn native_trace_available(connection: &rusqlite::Connection, scan_id: &str) -> Result<bool, String> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND backend='native')",
        [scan_id],
        |row| row.get(0),
    ).map_err(|error| format!("无法检查 Native 运行记录：{error}"))
}

// Source tools commit their execution and completion receipt atomically. They
// do not emit a separate start event. Model call IDs can repeat across rounds,
// so use the persisted assignment/round/index as the trace invocation identity.
fn native_source_trace_tool(value: &JsonValue) -> Option<(String, String)> {
    let assignment = value.get("assignmentId")?.as_str()?;
    let round = value.get("sourceRound")?.as_i64()?;
    let index = value.get("callIndex")?.as_i64()?;
    let name = value.pointer("/call/name")?.as_str()?;
    if assignment.is_empty() || round < 1 || index < 0 || name.is_empty() {
        return None;
    }
    Some((name.into(), format!("source:{assignment}:{round}:{index}")))
}

fn collect_native_agent_trace(
    connection: &rusqlite::Connection,
    scan_id: &str,
    include_events: bool,
    latest_attempt_only: bool,
) -> Result<(AgentTraceSummary, Vec<AgentTraceEvent>), String> {
    let (
        task_name, project_name, status, scan_type, _, stored_requests, stored_input,
        stored_output, stored_cached, stored_total, created_at, updated_at,
    ) = agent_trace_base(connection, scan_id)?;
    let attempt = if latest_attempt_only {
        connection.query_row(
            "SELECT MAX(attempt_number) FROM agent_runs WHERE scan_id=?1 AND backend='native'",
            [scan_id], |row| row.get::<_, Option<i64>>(0),
        ).map_err(|error| error.to_string())?
    } else { None };
    let mut runs = connection.prepare(
        "SELECT id,role,used_tokens,used_cached_tokens,used_requests
         FROM agent_runs WHERE scan_id=?1 AND backend='native'
           AND (?2 IS NULL OR attempt_number=?2) ORDER BY created_at,id",
    ).map_err(|error| error.to_string())?;
    let rows = runs.query_map(params![scan_id, attempt], |row| Ok((
        row.get::<_, String>(0)?, row.get::<_, String>(1)?,
        row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?,
    ))).map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    let run_count = rows.iter().filter(|row| row.1 == "coordinator").count() as i64;
    let agent_count = rows.len() as i64;
    let mut requests = 0i64;
    let mut tokens = 0i64;
    let mut cached = 0i64;
    for (_, _, used_tokens, used_cached, used_requests) in &rows {
        requests += used_requests;
        tokens += used_tokens;
        cached += used_cached;
    }
    let mut events = Vec::new();
    let mut message_count = 0i64;
    let mut reasoning_count = 0i64;
    let mut tool_call_count = 0i64;
    let mut tool_result_count = 0i64;
    let mut tools: HashMap<String, (i64, i64)> = HashMap::new();
    let mut source_invocations = HashSet::new();
    let mut rounds = 0i64;
    let mut model_agents = HashSet::new();
    let mut statement = connection.prepare(
        "SELECT e.run_id,e.sequence,e.event_type,e.payload_json,e.created_at,r.role,r.target_url
         FROM agent_events e JOIN agent_runs r ON r.id=e.run_id
         WHERE r.scan_id=?1 AND r.backend='native' AND (?2 IS NULL OR r.attempt_number=?2)
         ORDER BY e.created_at,e.id",
    ).map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id, attempt], |row| Ok((
        row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?,
        row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
        row.get::<_, String>(6)?,
    ))).map_err(|error| error.to_string())?;
    for row in rows {
        let (run_id, sequence, kind, payload, created_at, role, target_url) =
            row.map_err(|error| error.to_string())?;
        let value: JsonValue = serde_json::from_str(&payload).unwrap_or(JsonValue::Null);
        let source_tool = native_source_trace_tool(&value);
        let name = source_tool.as_ref().map(|tool| tool.0.as_str())
            .or_else(|| value.get("tool").and_then(JsonValue::as_str)).unwrap_or("").to_string();
        let call_id = source_tool.as_ref().map(|tool| tool.1.as_str())
            .or_else(|| value.get("invocationId").and_then(JsonValue::as_str)).unwrap_or("").to_string();
        match kind.as_str() {
            "model_round_completed" => { rounds += 1; message_count += 1; model_agents.insert(run_id.clone()); },
            "hypothesis_updated" => reasoning_count += 1,
            "tool_invocation_started" => {
                if source_tool.is_none() || source_invocations.insert((run_id.clone(), call_id.clone())) {
                    tool_call_count += 1;
                    tools.entry(name.clone()).or_default().0 += 1;
                }
            },
            "tool_invocation_completed" => {
                tool_result_count += 1;
                tools.entry(name.clone()).or_default().1 += 1;
                if source_tool.is_some() && source_invocations.insert((run_id.clone(), call_id.clone())) {
                    tool_call_count += 1;
                    tools.entry(name.clone()).or_default().0 += 1;
                }
            },
            _ => {},
        }
        if include_events && events.len() < 800 {
            let redacted = crate::agent_runtime::secrets::redact_text_with(&payload, None);
            let (detail, detail_size, detail_truncated) = trace_preview(&redacted, 20_000);
            events.push(AgentTraceEvent {
                id: format!("native:{run_id}:{sequence}"), session_id: run_id,
                call_id,
                target_url, event_type: kind, role, name, status: "recorded".into(),
                detail, detail_size, detail_truncated, created_at,
            });
        }
    }
    let mut mailbox = connection.prepare(
        "SELECT m.id,COALESCE(sender.id,m.run_id),m.kind,m.payload_json,m.created_at,m.delivered_at,m.acknowledged_at,
                CASE WHEN sender.id IS NOT NULL THEN sender.role
                     WHEN m.from_run_id='' THEN COALESCE(NULLIF(m.from_agent,''),'unknown')
                     ELSE 'unknown' END,r.target_url
         FROM agent_messages m JOIN agent_runs r ON r.id=m.run_id
         LEFT JOIN agent_runs sender ON sender.id=m.from_run_id
           AND sender.backend='native' AND sender.scan_id=r.scan_id
           AND sender.attempt_number=r.attempt_number AND sender.target_url=r.target_url
           AND (sender.id=r.id OR sender.root_run_id=r.id)
           AND sender.role=m.from_agent AND m.root_run_id=r.id
         WHERE r.scan_id=?1 AND r.backend='native' AND (?2 IS NULL OR r.attempt_number=?2)
         ORDER BY m.created_at,m.id",
    ).map_err(|error| error.to_string())?;
    let messages = mailbox.query_map(params![scan_id, attempt], |row| Ok((
        row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
        row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
        row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?,
    ))).map_err(|error| error.to_string())?;
    for row in messages {
        let (id, run_id, kind, payload, created_at, delivered, acknowledged, role, target_url) =
            row.map_err(|error| error.to_string())?;
        message_count += 1;
        if include_events && events.len() < 800 {
            let redacted = crate::agent_runtime::secrets::redact_text_with(&payload, None);
            let (detail, detail_size, detail_truncated) = trace_preview(&redacted, 20_000);
            events.push(AgentTraceEvent {
                id: format!("native-message:{id}"), session_id: run_id, call_id: id,
                target_url, event_type: kind, role, name: "mailbox".into(),
                status: if !acknowledged.is_empty() { "acknowledged" } else if !delivered.is_empty() {
                    "delivered"
                } else { "pending" }.into(),
                detail, detail_size, detail_truncated, created_at,
            });
        }
    }
    events.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
    let mut tools = tools.into_iter().map(|(name, (calls, results))| AgentTraceToolStat {
        name, calls, results,
    }).collect::<Vec<_>>();
    tools.sort_by_key(|item| std::cmp::Reverse(item.calls + item.results));
    let knowledge_id = connection.query_row(
        "SELECT id FROM agent_knowledge_entries WHERE scan_id=?1", [scan_id], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    let model: String = connection.query_row(
        "SELECT CASE WHEN json_valid(plan_json) AND json_type(plan_json,'$.modelProvider')='text'
           THEN json_extract(plan_json,'$.modelProvider') ELSE '' END FROM agent_runs
         WHERE scan_id=?1 AND backend='native' AND (?2 IS NULL OR attempt_number=?2)
           AND role='coordinator' ORDER BY attempt_number DESC LIMIT 1",
        params![scan_id, attempt], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?.unwrap_or_default();
    Ok((AgentTraceSummary {
        scan_id: scan_id.into(), task_name, project_name, status, scan_type,
        source_authority: "native_ledger".into(),
        model, run_count, agent_count, message_count, reasoning_count,
        tool_call_count, tool_result_count,
        llm_requests: stored_requests.max(requests), input_tokens: stored_input,
        output_tokens: stored_output, cached_tokens: stored_cached.max(cached),
        total_tokens: stored_total.max(tokens), hooked_request_count: 0,
        exact_request_capture: false, usage_entry_count: rounds,
        usage_agent_count: model_agents.len() as i64, token_usage_estimated: false,
        // A frozen plan hash is not an instruction hash; no raw prompt is
        // reconstructed from the run ledger.
        instruction_hash: String::new(), tools, knowledge_id, created_at, updated_at,
    }, events))
}
