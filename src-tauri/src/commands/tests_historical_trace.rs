fn historical_trace_fixture() -> (PathBuf, rusqlite::Connection) {
    let root = std::env::temp_dir().join(format!("oviraptor-trace-boundary-{}", Uuid::new_v4()));
    let connection = db::open(&db::initialize(&root).unwrap()).unwrap();
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join(".oviraptor-scan-id"), "trace-boundary").unwrap();
    fs::write(source.join("model-prompt-audit.json"), r#"{"instruction":"review evidence"}"#).unwrap();
    connection.execute(
        "INSERT INTO sentinel_scans(id,task_path) VALUES('trace-boundary',?1),('strix-trace-boundary',?1)",
        [source.to_string_lossy().as_ref()],
    ).unwrap();
    let result = crate::artifact_import::import_roots(&crate::artifact_import::ImportContext {
        connection: &connection, cas_dir: &root.join("cas"), key_path: &root.join("key"), roots: &[source], limits: &crate::artifact_import::Limits::default(),
    });
    assert!(!result.outcomes.is_empty());
    assert_eq!(historical_trace_records(&connection, "trace-boundary", false).unwrap().len(), 1);
    (root, connection)
}

#[test]
fn historical_trace_rejects_alias_and_enforces_exact_membership_boundaries() {
    let (root, connection) = historical_trace_fixture();
    assert!(historical_trace_records(&connection, "strix-trace-boundary", false).unwrap().is_empty());
    for mutation in [
        "UPDATE import_projection_memberships SET source_path='unrelated-source'",
        "UPDATE import_projection_memberships SET current=0",
        "UPDATE import_projection_memberships SET tombstone=1",
        "UPDATE import_bundles SET status='failed'",
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('trace-boundary')",
    ] {
        connection.execute_batch("SAVEPOINT trace_boundary").unwrap();
        connection.execute_batch(mutation).unwrap();
        assert!(historical_trace_records(&connection, "trace-boundary", false).unwrap().is_empty(), "{mutation}");
        connection.execute_batch("ROLLBACK TO trace_boundary; RELEASE trace_boundary").unwrap();
    }
    connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES('strix-trace-boundary')", []).unwrap();
    assert_eq!(historical_trace_records(&connection, "trace-boundary", false).unwrap().len(), 1);
    // Append deliberately unsupported revisions instead of rewriting immutable
    // history. Even if such data reaches storage, the reader must reject it.
    for replacement in [
        "'$.canonicalSchema','unknown'", "'$.claim.authority','native_ledger'",
        "'$.claim.reviewState','confirmed'", "'$.claim.readOnly',json('false')",
        "'$.claim.executionEligible',json('true')", "'$.claim.readOnly',1",
        "'$.claim.executionEligible',0", "'$.provenance.importAdapter','retired_reader'",
        "'$.recordKind','run_state'", "'$.payload.trace_kind','agent_session'",
        "'$.payload.trace_kind','agent_message'", "'$.payload.trace_kind','unknown'",
    ] {
        connection.execute_batch("SAVEPOINT trace_boundary").unwrap();
        connection.execute_batch(&format!(
            "INSERT INTO import_record_revisions(record_key,logical_key,record_kind,revision_hash,adapter,envelope_json) \
             SELECT record_key,logical_key,record_kind,'test-unsupported',adapter,json_set(envelope_json,{replacement}) \
             FROM import_record_revisions LIMIT 1; \
             UPDATE import_projection_memberships SET revision_id=last_insert_rowid();"
        )).unwrap();
        assert!(historical_trace_records(&connection, "trace-boundary", false).unwrap().is_empty(), "{replacement}");
        connection.execute_batch("ROLLBACK TO trace_boundary; RELEASE trace_boundary").unwrap();
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_trace_latest_attempt_does_not_fall_back_to_old_evidence() {
    let (root, connection) = historical_trace_fixture();
    connection.execute("UPDATE import_projection_memberships SET attempt_number=1", []).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES('trace-boundary',2)", []).unwrap();
    assert!(historical_trace_records(&connection, "trace-boundary", true).unwrap().is_empty());
    assert_eq!(historical_trace_records(&connection, "trace-boundary", false).unwrap().len(), 1);
    connection.execute("UPDATE import_projection_memberships SET attempt_number=2", []).unwrap();
    assert_eq!(historical_trace_records(&connection, "trace-boundary", true).unwrap().len(), 1);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_trace_keeps_current_hook_usage_and_pending_requests_source_scoped() {
    let (root, connection) = historical_trace_fixture();
    let source = root.join("source");
    for (directory, tokens) in [("one", 20), ("two", 30)] {
        let run = source.join(directory);
        fs::create_dir_all(&run).unwrap();
        // Identical request IDs are independent across current Hook files.
        // A separate pending request contributes no invented token usage.
        let mut hooks = vec![serde_json::json!({"kind":"model_call_started","requestId":"same-request","recordedAt":"t1"}).to_string()];
        hooks.push(serde_json::json!({"kind":"model_call","requestId":"same-request","recordedAt":"t2","status":"200","request":{"messages":[]},"usage":{"input_tokens":tokens,"total_tokens":tokens}}).to_string());
        if directory == "two" {
            hooks.push(serde_json::json!({"kind":"model_call_started","requestId":"pending-request","recordedAt":"t3"}).to_string());
        }
        fs::write(run.join("llm-hook.jsonl"), hooks.join("\n")).unwrap();
    }
    let audit = serde_json::json!({"captureMode":"full","source":"legacy","captureLevel":"exact", "exactModelRequest":true,
        "model":"test","deployment":"local","fullPower":false,"recordedAt":"t3","instructionSha256":"digest",
        "instructionChars":50,"instruction":"Review evidence\nAuthorization: Bearer do-not-store","notice":"old"});
    fs::write(source.join("model-prompt-audit.json"), audit.to_string()).unwrap();
    crate::artifact_import::import_roots(&crate::artifact_import::ImportContext {
        connection: &connection, cas_dir: &root.join("cas"), key_path: &root.join("key"), roots: std::slice::from_ref(&source), limits: &crate::artifact_import::Limits::default(),
    });
    fs::rename(&source, root.join("offline-source")).unwrap();
    let (summary, events) = collect_historical_agent_trace(&connection, "trace-boundary", true, false).unwrap();
    assert_eq!(summary.message_count, 0);
    assert_eq!(summary.agent_count, 0);
    assert_eq!(summary.tool_call_count, 0);
    assert_eq!(summary.llm_requests, 2, "completed requests in separate Hook files must both count");
    assert_eq!(summary.total_tokens, 50);
    assert_eq!(summary.hooked_request_count, 2);
    let hooks: Vec<_> = events.iter().filter(|event| event.event_type == "model_request").collect();
    assert_eq!(hooks.len(), 3);
    assert_eq!(hooks.iter().filter(|event| event.status == "in_flight").count(), 1);
    assert!(events.iter().all(|event| event.target_url.is_empty()));
    let audit = historical_prompt_audit(&connection, "trace-boundary").unwrap().unwrap();
    assert!(!audit.exact_model_request);
    assert_eq!(audit.capture_level, "generated_instruction");
    assert!(audit.instruction.as_ref().unwrap().contains("Review evidence"));
    assert!(!serde_json::to_string(&audit).unwrap().contains("do-not-store"));
    let detail = read_agent_trace_detail(&connection, "trace-boundary").unwrap();
    assert!(detail.summary.exact_request_capture);
    let displayed = detail.prompt_audit.unwrap();
    assert!(!displayed.exact_model_request, "the command must not undo historical audit normalization");
    assert_eq!(displayed.capture_level, "generated_instruction_and_redacted_model_requests");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn trace_display_privacy_historical_reader_hides_private_payloads_without_rewriting_history() {
    let (root, connection) = historical_trace_fixture();
    let source = root.join("source");
    let hooks = source.join("llm-hook.jsonl");
    let original = serde_json::json!({"kind":"model_call", "requestId":"privacy-fixture",
        "request":{"messages":[{"type":"reasoning","summary":"private-event-reasoning-fixture"}]},
        "response":{"choices":[{"message":{"content":"public answer",
            "reasoning_content":"private-hook-reasoning-fixture"}}]},
        "usage":{"total_tokens":7}}).to_string();
    fs::write(&hooks, &original).unwrap();
    crate::artifact_import::import_roots(&crate::artifact_import::ImportContext {
        connection: &connection, cas_dir: &root.join("cas"), key_path: &root.join("key"), roots: &[source], limits: &crate::artifact_import::Limits::default(),
    });
    let revision_snapshot = || connection.prepare("SELECT envelope_json FROM import_record_revisions ORDER BY id")
        .unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
        .collect::<Result<Vec<_>, _>>().unwrap();
    let before = revision_snapshot();
    let displayed = read_agent_trace_detail(&connection, "trace-boundary").unwrap();
    let serialized = serde_json::to_string(&displayed).unwrap();
    assert!(!serialized.contains("private-hook-reasoning-fixture"));
    assert!(!serialized.contains("private-event-reasoning-fixture"));
    assert!(serialized.contains("public answer"));
    assert_eq!(displayed.summary.reasoning_count, 0);
    assert_eq!(displayed.events.iter().filter(|event| event.event_type == "model_request").count(), 1);
    assert_eq!(revision_snapshot(), before, "display must not rewrite canonical history");
    assert_eq!(fs::read_to_string(hooks).unwrap(), original);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
