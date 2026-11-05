// Read-only scan/agent cost & productivity stats (OpenCode-style `stats` borrow).
// Aggregates committed ledger rows only; never mutates execution state.

pub(crate) fn scan_agent_stats_inner(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: Option<i64>,
) -> Result<JsonValue, String> {
    if attempt_number.is_some_and(|value| value < 1) {
        return Err("invalid_stats_attempt".into());
    }
    let scan_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !scan_exists {
        return Err("scan_not_found".into());
    }

    let (stored_input, stored_output, stored_cached, stored_total, stored_requests): (i64, i64, i64, i64, i64) =
        connection
            .query_row(
                "SELECT input_tokens, output_tokens, cached_tokens, total_tokens, llm_requests FROM sentinel_scans WHERE id=?1",
                [scan_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap_or((0, 0, 0, 0, 0));

    let (run_tokens, run_cached, run_requests, coordinator_runs, executor_runs): (i64, i64, i64, i64, i64) =
        connection.query_row(
            "SELECT COALESCE(SUM(used_tokens),0), COALESCE(SUM(used_cached_tokens),0), COALESCE(SUM(used_requests),0), \
             COALESCE(SUM(CASE WHEN role='coordinator' THEN 1 ELSE 0 END),0), COUNT(*) \
             FROM agent_runs WHERE scan_id=?1 AND backend='native' AND (?2 IS NULL OR attempt_number=?2)",
            params![scan_id, attempt_number],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|error| error.to_string())?;

    let mut regroup: HashMap<String, (i64, i64, i64, i64, HashMap<String, i64>)> = HashMap::new();
    let tool_rows = connection
        .prepare(
            "SELECT t.tool_name, t.status, COALESCE(t.error_class,''), COUNT(*) \
             FROM tool_invocations t JOIN agent_runs r ON r.id=t.run_id \
             WHERE r.scan_id=?1 AND r.backend='native' AND (?2 IS NULL OR r.attempt_number=?2) \
             GROUP BY t.tool_name, t.status, t.error_class",
        )
        .map_err(|error| error.to_string())?
        .query_map(params![scan_id, attempt_number], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (name, status, error_class, count) in tool_rows {
        let slot = regroup
            .entry(name)
            .or_insert((0, 0, 0, 0, HashMap::new()));
        slot.0 += count;
        match status.as_str() {
            "completed" | "succeeded" | "success" => slot.1 += count,
            "running" => slot.3 += count,
            _ => {
                slot.2 += count;
                if !error_class.is_empty() {
                    *slot.4.entry(error_class).or_default() += count;
                }
            }
        }
    }
    let tool_invocations_total: i64 = regroup.values().map(|row| row.0).sum();
    let mut tool_list: Vec<JsonValue> = regroup
        .into_iter()
        .map(|(name, (calls, completed, failed, running, errors))| {
            serde_json::json!({
                "toolName": name,
                "calls": calls,
                "completed": completed,
                "failed": failed,
                "running": running,
                "errorClasses": errors,
            })
        })
        .collect();
    tool_list.sort_by(|left, right| {
        right["calls"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&left["calls"].as_i64().unwrap_or(0))
            .then_with(|| {
                left["toolName"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(right["toolName"].as_str().unwrap_or(""))
            })
    });

    let findings_by_severity: Vec<JsonValue> = connection
        .prepare(
            "SELECT severity, COUNT(*) FROM sentinel_findings WHERE scan_id=?1 GROUP BY severity ORDER BY COUNT(*) DESC",
        )
        .map_err(|error| error.to_string())?
        .query_map([scan_id], |row| {
            Ok(serde_json::json!({
                "severity": row.get::<_, String>(0)?,
                "count": row.get::<_, i64>(1)?,
            }))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let exploit_status: Vec<JsonValue> = connection
        .prepare(
            "SELECT deep_status, COUNT(*) FROM agent_finding_exploits WHERE scan_id=?1 GROUP BY deep_status",
        )
        .ok()
        .map(|mut statement| {
            statement
                .query_map([scan_id], |row| {
                    Ok(serde_json::json!({
                        "status": row.get::<_, String>(0)?,
                        "count": row.get::<_, i64>(1)?,
                    }))
                })
                .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        })
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_default();

    let pending_questions: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_operator_questions WHERE scan_id=?1 AND status='pending'",
            [scan_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let pending_high_risk: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM high_risk_action_approvals WHERE scan_id=?1 AND status='pending'",
            [scan_id],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let tool_hits = connection
        .prepare(
            "SELECT COALESCE(t.contract_key,''), t.tool_name, COALESCE(t.input_summary_json,'') \
             FROM tool_invocations t JOIN agent_runs r ON r.id=t.run_id \
             WHERE r.scan_id=?1 AND r.backend='native' \
             AND (?2 IS NULL OR r.attempt_number=?2) \
             LIMIT 4000",
        )
        .map_err(|error| error.to_string())?
        .query_map(params![scan_id, attempt_number], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let finding_rows = connection
        .prepare(
            "SELECT f.id, f.title, f.severity, f.record_key, f.target_url, \
             COALESCE(e.deep_status,'pending'), COALESCE(e.requests_used,0), COALESCE(e.iterations_used,0) \
             FROM sentinel_findings f \
             LEFT JOIN agent_finding_exploits e ON e.scan_id=f.scan_id AND e.finding_id=f.id \
             WHERE f.scan_id=?1 ORDER BY f.severity, f.id LIMIT 500",
        )
        .map_err(|error| error.to_string())?
        .query_map([scan_id], |row| {
            let finding_id: i64 = row.get(0)?;
            let record_key: String = row.get(3)?;
            let target_url: String = row.get(4)?;
            let mut contract_tools: HashMap<String, i64> = HashMap::new();
            for (contract_key, tool_name, summary) in &tool_hits {
                let key_hit = !record_key.is_empty()
                    && !contract_key.is_empty()
                    && (contract_key == &record_key
                        || contract_key.contains(&record_key)
                        || record_key.contains(contract_key.as_str()));
                let summary_hit = (!target_url.is_empty() && summary.contains(&target_url))
                    || (record_key.len() >= 4 && summary.contains(&record_key));
                if key_hit || summary_hit {
                    *contract_tools.entry(tool_name.clone()).or_insert(0) += 1;
                }
            }
            let tool_calls: i64 = contract_tools.values().sum();
            Ok(serde_json::json!({
                "findingId": finding_id,
                "title": row.get::<_, String>(1)?,
                "severity": row.get::<_, String>(2)?,
                "recordKey": record_key,
                "targetUrl": target_url,
                "deepStatus": row.get::<_, String>(5)?,
                "requestsUsed": row.get::<_, i64>(6)?,
                "iterationsUsed": row.get::<_, i64>(7)?,
                "contractToolCalls": tool_calls,
                "toolsByName": contract_tools,
            }))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut attempt_statement = connection
        .prepare(
            "SELECT attempt_number, status, total_tokens_delta, llm_requests_delta, input_tokens_delta, output_tokens_delta, cached_tokens_delta \
             FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number",
        )
        .map_err(|error| error.to_string())?;
    let attempts = attempt_statement
        .query_map([scan_id], |row| {
            Ok(serde_json::json!({
                "attemptNumber": row.get::<_, i64>(0)?,
                "status": row.get::<_, String>(1)?,
                "totalTokens": row.get::<_, i64>(2)?,
                "modelRequests": row.get::<_, i64>(3)?,
                "inputTokens": row.get::<_, i64>(4)?,
                "outputTokens": row.get::<_, i64>(5)?,
                "cachedTokens": row.get::<_, i64>(6)?,
            }))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    Ok(serde_json::json!({
        "schemaVersion": 1,
        "scanId": scan_id,
        "attemptNumber": attempt_number,
        "recordedAt": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "tokens": {
            "ledgerTotal": stored_total,
            "ledgerInput": stored_input,
            "ledgerOutput": stored_output,
            "ledgerCached": stored_cached,
            "ledgerModelRequests": stored_requests,
            "runSummedTokens": run_tokens,
            "runSummedCached": run_cached,
            "runSummedRequests": run_requests,
        },
        "agents": {
            "coordinatorRuns": coordinator_runs,
            "nativeRuns": executor_runs,
        },
        "tools": tool_list,
        "toolInvocationsTotal": tool_invocations_total,
        "findingsBySeverity": findings_by_severity,
        "findingRollup": finding_rows,
        "exploitStatus": exploit_status,
        "operator": {
            "pendingQuestions": pending_questions,
            "pendingHighRiskActions": pending_high_risk,
        },
        "attempts": attempts,
    }))
}

fn summary_field(value: &JsonValue, keys: &[&str]) -> String {
    keys.iter().find_map(|key| value.get(*key).and_then(JsonValue::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_string)).unwrap_or_default()
}

#[tauri::command]
pub fn list_scan_request_log(state: State<AppState>, scan_id: String) -> Result<Vec<JsonValue>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT t.tool_name, t.status, COALESCE(t.input_summary_json,'{}'), t.started_at, COALESCE(t.error_class,'') \
             FROM tool_invocations t JOIN agent_runs r ON r.id=t.run_id \
             WHERE r.scan_id=?1 AND r.backend='native' \
             AND t.tool_name IN ('replay_http','probe_sql_injection','probe_command_execution','browser_action','plan_ssrf_channel','targeted_discovery') \
             ORDER BY t.id DESC LIMIT 80",
        )
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query_map([&scan_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rows.reverse();
    let mut previous_input: HashMap<String, String> = HashMap::new();
    let mut items = Vec::new();
    for (tool, status, summary_text, started_at, error_class) in rows {
        let summary: JsonValue = serde_json::from_str(&summary_text).unwrap_or(JsonValue::Null);
        let method = summary_field(&summary, &["method"]).to_ascii_uppercase();
        let url = summary_field(&summary, &["url", "targetUrl"]);
        let param = summary_field(&summary, &["param", "parameter"]);
        let key = if url.is_empty() { tool.clone() } else { url.clone() };
        let diff = match previous_input.get(&key) {
            Some(prior) if prior == &summary_text => "与上一发相同",
            Some(_) => "参数与上一发不同",
            None => "首次",
        };
        previous_input.insert(key, summary_text);
        items.push(serde_json::json!({
            "tool": tool,
            "method": if method.is_empty() { "—" } else { method.as_str() },
            "url": url,
            "param": param,
            "status": status,
            "errorClass": error_class,
            "startedAt": started_at,
            "diff": diff,
        }));
    }
    Ok(items)
}

#[tauri::command]
pub fn get_scan_agent_stats(
    state: State<AppState>,
    scan_id: String,
    attempt_number: Option<i64>,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    scan_agent_stats_inner(&connection, &scan_id, attempt_number)
}

#[tauri::command]
pub fn export_scan_agent_stats(state: State<AppState>, scan_id: String) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    let value = scan_agent_stats_inner(&connection, &scan_id, None)?;
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod scan_stats_tests {
    use super::scan_agent_stats_inner;
    use rusqlite::Connection;

    #[test]
    fn missing_scan_errors() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY, input_tokens INTEGER, output_tokens INTEGER, cached_tokens INTEGER, total_tokens INTEGER, llm_requests INTEGER);")
            .unwrap();
        assert!(scan_agent_stats_inner(&c, "missing", None).is_err());
    }
}
