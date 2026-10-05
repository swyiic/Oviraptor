// Versioned, read-only union of Web invocations and Source tool receipts. The
// legacy numeric Web-only cursor remains supported by its original command.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeExecutionHistoryCursor {
    schema_version: u32,
    scan_id: String,
    attempt_number: i64,
    recorded_at: String,
    record_key: String,
}

#[tauri::command]
pub fn get_native_attempt_execution_history(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    before: Option<String>,
    limit: Option<i64>,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    native_attempt_execution_history(
        &connection,
        &scan_id,
        attempt_number,
        before.as_deref(),
        limit.unwrap_or(50),
    )
}

fn native_attempt_execution_history(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt: i64,
    before: Option<&str>,
    limit: i64,
) -> Result<JsonValue, String> {
    let cursor = before
        .map(|text| {
            if text.len() > 4096 {
                return Err("invalid_execution_history_cursor".to_string());
            }
            let value: NativeExecutionHistoryCursor = serde_json::from_str(text)
                .map_err(|_| "invalid_execution_history_cursor".to_string())?;
            if value.schema_version != 2
                || value.scan_id != scan_id
                || value.attempt_number != attempt
                || value.recorded_at.is_empty()
                || value.recorded_at.len() > 64
                || !(value.record_key.starts_with("source:")
                    || value.record_key.starts_with("web:"))
            {
                return Err("invalid_execution_history_cursor".into());
            }
            Ok(value)
        })
        .transpose()?;
    if attempt < 1 {
        return Err("invalid_execution_history_attempt".into());
    }
    let snapshot = if connection.is_autocommit() {
        Some(
            rusqlite::Transaction::new_unchecked(
                connection,
                rusqlite::TransactionBehavior::Deferred,
            )
            .map_err(|_| "execution_history_snapshot_failed")?,
        )
    } else {
        None
    };
    let tx = snapshot.as_deref().unwrap_or(connection);
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts a JOIN sentinel_scans s ON s.id=a.scan_id
         WHERE a.scan_id=?1 AND a.attempt_number=?2
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id))",
        params![scan_id,attempt], |row| row.get(0),
    ).map_err(|_| "execution_history_attempt_lookup_failed")?;
    if !exists {
        return Err("attempt_not_found".into());
    }
    let page_size = limit.clamp(1, 100) as usize;
    // Page small identities first, not potentially large arguments/results. A
    // source receipt's ordering uses its immutable model-response timestamp,
    // which is also when its planned row was committed, never a claimed start.
    let mut statement = tx.prepare(
        "WITH history AS (
          SELECT 'web:'||printf('%020d',t.id) AS record_key,t.started_at AS recorded_at,
                 t.id AS web_id,'' AS assignment_id,0 AS round_number,0 AS call_index
          FROM tool_invocations t JOIN agent_runs r ON r.id=t.run_id
          WHERE r.scan_id=?1 AND r.attempt_number=?2 AND r.backend='native'
          UNION ALL
          SELECT 'source:'||t.assignment_id||':'||printf('%020d',t.round_number)||':'||printf('%020d',t.call_index),
                 m.finished_at,0,t.assignment_id,t.round_number,t.call_index
          FROM agent_source_tool_receipts t
          JOIN agent_source_model_rounds m ON m.assignment_id=t.assignment_id AND m.round_number=t.round_number AND m.child_run_id=t.child_run_id
          JOIN agent_runs r ON r.id=m.child_run_id
          JOIN agent_runs root ON root.id=m.root_run_id
          WHERE root.scan_id=?1 AND root.attempt_number=?2 AND root.backend='native'
            AND r.scan_id=root.scan_id AND r.attempt_number=root.attempt_number AND r.backend='native'
        ) SELECT record_key,recorded_at,web_id,assignment_id,round_number,call_index FROM history
          WHERE (?3 IS NULL OR recorded_at<?3 OR (recorded_at=?3 AND record_key<?4))
          ORDER BY recorded_at DESC,record_key DESC LIMIT ?5",
    ).map_err(|_| "execution_history_query_failed")?;
    let keys = statement
        .query_map(
            params![
                scan_id,
                attempt,
                cursor.as_ref().map(|c| &c.recorded_at),
                cursor.as_ref().map(|c| &c.record_key),
                (page_size + 1) as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .map_err(|_| "execution_history_read_failed")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "execution_history_decode_failed")?;
    let has_older = keys.len() > page_size;
    let mut invocations = Vec::new();
    for (key, recorded_at, web_id, assignment, round, index) in keys.into_iter().take(page_size) {
        let mut item = if assignment.is_empty() {
            native_web_history_item(tx, web_id)?
        } else {
            native_source_history_item(tx, &assignment, round, index)?
        };
        item["id"] = json!(key);
        item["recordedAt"] = json!(recorded_at);
        invocations.push(item);
    }
    invocations.reverse();
    let older_cursor = invocations
        .first()
        .map(|first| {
            serde_json::to_string(&NativeExecutionHistoryCursor {
                schema_version: 2,
                scan_id: scan_id.into(),
                attempt_number: attempt,
                recorded_at: first["recordedAt"].as_str().unwrap_or_default().into(),
                record_key: first["id"].as_str().unwrap_or_default().into(),
            })
        })
        .transpose()
        .map_err(|_| "execution_history_cursor_encode_failed")?;
    drop(statement);
    if let Some(snapshot) = snapshot {
        snapshot
            .commit()
            .map_err(|_| "execution_history_snapshot_failed")?;
    }
    Ok(
        json!({"schemaVersion":2,"scanId":scan_id,"attemptNumber":attempt,
        "invocations":invocations,"hasOlder":has_older,"olderCursor":older_cursor}),
    )
}

fn native_web_history_item(
    connection: &rusqlite::Connection,
    id: i64,
) -> Result<JsonValue, String> {
    connection.query_row(
        "SELECT t.invocation_id,t.run_id,r.role,t.tool_name,t.status,t.policy_decision,
         t.started_at,t.finished_at,t.request_artifact_id,t.response_artifact_id,t.error_class
         FROM tool_invocations t JOIN agent_runs r ON r.id=t.run_id WHERE t.id=?1",[id],|row| {
        let error: String = row.get(10)?;
        Ok(json!({"origin":"web_invocation","timeBasis":"tool_started","receiptIntegrity":"not_checked",
            "invocationId":row.get::<_,String>(0)?,"runId":row.get::<_,String>(1)?,"role":row.get::<_,String>(2)?,
            "toolName":row.get::<_,String>(3)?,"status":row.get::<_,String>(4)?,"policyDecision":row.get::<_,String>(5)?,
            "startedAt":row.get::<_,String>(6)?,"finishedAt":row.get::<_,String>(7)?,
            "hasRequestArtifact":!row.get::<_,String>(8)?.is_empty(),"hasResponseArtifact":!row.get::<_,String>(9)?.is_empty(),
            "errorClass":crate::agent_runtime::secrets::redact_text_with(&error,None).chars().take(200).collect::<String>()}))
    }).map_err(|_| "execution_history_web_record_missing".into())
}

struct NativeSourceHistoryRecord {
    run_id: String,
    role: String,
    name: String,
    state: String,
    args: String,
    call_id: String,
    output: String,
    receipt_hash: String,
    sequence: i64,
    event: Option<String>,
    event_at: String,
    model_tool: Option<String>,
    bound: bool,
}

fn native_source_history_item(
    connection: &rusqlite::Connection,
    assignment: &str,
    round: i64,
    index: i64,
) -> Result<JsonValue, String> {
    let NativeSourceHistoryRecord {run_id,role,name,state,args,call_id,output,receipt_hash,sequence,event,event_at,model_tool,bound} = connection.query_row(
        "SELECT r.id,r.role,t.tool_name,t.state,t.arguments_json,t.call_id,t.output_json,t.receipt_hash,t.event_sequence,
            e.payload_json,COALESCE(e.created_at,''),
            CASE WHEN json_valid(m.response_json) THEN json_extract(m.response_json,'$.toolCalls['||t.call_index||']') END,
            COALESCE(a.child_run_id=r.id AND a.coordinator_run_id=root.id AND a.role=r.role AND m.role=r.role
              AND r.root_run_id=root.id AND r.parent_run_id=root.id AND root.role='coordinator'
              AND r.assignment_id=a.id AND r.target_url=root.target_url AND a.target_key=root.target_url
              AND m.state='received' AND m.finished_at<>'' AND t.round_number>0
              AND r.role IN ('repo_mapper','source_analyst'),0)
         FROM agent_source_tool_receipts t
         JOIN agent_source_model_rounds m ON m.assignment_id=t.assignment_id AND m.round_number=t.round_number AND m.child_run_id=t.child_run_id
         JOIN agent_runs r ON r.id=m.child_run_id JOIN agent_runs root ON root.id=m.root_run_id
         LEFT JOIN agent_assignments a ON a.id=t.assignment_id
         LEFT JOIN agent_events e ON e.run_id=r.id AND e.sequence=t.event_sequence AND e.event_type='tool_invocation_completed'
         WHERE t.assignment_id=?1 AND t.round_number=?2 AND t.call_index=?3",
        params![assignment,round,index],|r|Ok(NativeSourceHistoryRecord {
            run_id:r.get(0)?,role:r.get(1)?,name:r.get(2)?,state:r.get(3)?,args:r.get(4)?,call_id:r.get(5)?,
            output:r.get(6)?,receipt_hash:r.get(7)?,sequence:r.get(8)?,event:r.get(9)?,event_at:r.get(10)?,
            model_tool:r.get(11)?,bound:r.get(12)?,
        }),
    ).map_err(|_| "execution_history_source_record_missing")?;
    let parse = |text: &str| serde_json::from_str::<JsonValue>(text).ok();
    let arguments = parse(&args);
    let result = parse(&output);
    let tool = json!({"id":call_id,"name":name,"arguments":arguments});
    let known_tool = crate::native_pipeline::tools::SOURCE_TOOLS.contains(&name.as_str());
    let input_matches = known_tool
        && bound
        && arguments.is_some()
        && model_tool.as_deref().and_then(parse) == Some(tool.clone());
    let payload = json!({"sourceRound":round,"assignmentId":assignment,"callIndex":index,
        "call":tool,"output":result,"targetRequestsDelta":0});
    // This is local receipt consistency, not review approval or permission to
    // replay. No output body, arguments, model text or denial message is exposed.
    let matched = input_matches
        && state == "completed"
        && result.is_some()
        && sequence > 0
        && event.as_deref().and_then(parse) == Some(payload.clone())
        && receipt_hash == crate::agent_runtime::store::stable_hash(&payload.to_string());
    let planned = input_matches
        && state == "planned"
        && sequence == 0
        && receipt_hash.is_empty()
        && result == Some(json!({}));
    let denied = matched
        && result
            .as_ref()
            .is_some_and(|value| value.get("error").is_some());
    Ok(
        json!({"origin":"source_receipt","timeBasis":"model_response_received",
        "invocationId":format!("source:{assignment}:{round}:{index}"),"runId":run_id,"role":role,
        "toolName":if known_tool {name.as_str()}else{"unknown_source_tool"},
        "status":if denied {"refused"}else if matched {"completed"}else if planned {"planned"}else{"unverified"},
        "policyDecision":if denied {"deny"}else{""},
        "receiptIntegrity":if matched {"matched"}else if planned {"pending"}else{"unverified"},
        "resultKind":if name=="assignment.finish" {"control"}else{"data"},
        "startedAt":"","finishedAt":if matched {event_at}else{String::new()},
        "hasRequestArtifact":false,"hasResponseArtifact":false,
        "errorClass":if denied {"source_tool_refused"}else if !matched && !planned {"source_receipt_unverified"}else{""}}),
    )
}
