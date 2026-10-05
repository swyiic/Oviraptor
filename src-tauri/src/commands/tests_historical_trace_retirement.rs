// Persisted, retired projections remain inert even if they predate reader removal.
#[test]
fn historical_trace_retirement_current_source_membership_uses_path_components() {
    let (root, connection) = historical_trace_fixture();
    let source = root.join("source");
    for (path, accepted) in [
        (source.clone(), true),
        (source.join("reports/current-audit"), true),
        (source.join("attempt-0002/reports"), true),
        (root.join("source-other"), false),
        (source.join("../unrelated-source"), false),
        (root.join("other"), false),
        (PathBuf::new(), false),
    ] {
        connection.execute("UPDATE import_projection_memberships SET source_path=?1", [path.to_string_lossy().as_ref()]).unwrap();
        assert_eq!(historical_trace_records(&connection, "trace-boundary", false).unwrap().len(), usize::from(accepted), "{}", path.display());
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_trace_retirement_rejects_persisted_formats_without_generic_fallback() {
    let (root, connection) = historical_trace_fixture();
    let original: String = connection.query_row("SELECT envelope_json FROM import_record_revisions LIMIT 1", [], |row| row.get(0)).unwrap();
    let source = fs::read(root.join("source/model-prompt-audit.json")).unwrap();
    for (adapter, kind, trace_kind) in [
        ("sqlite_trace", "event_trace", "agent_message"),
        ("retired_reader", "run_state", "run_state"),
        ("retired_reader", "usage", "usage"),
        ("model_audit", "event_trace", "agent_session"),
        ("model_audit", "event_trace", "agent_message"),
        ("model_audit", "event_trace", "unknown"),
        ("model_audit", "usage", "model_hook"),
        ("retired_reader", "event_trace", "model_hook"),
    ] {
        connection.execute_batch("SAVEPOINT retired_trace").unwrap();
        let mut envelope: JsonValue = serde_json::from_str(&original).unwrap();
        envelope["recordKind"] = json!(kind);
        envelope["provenance"]["importAdapter"] = json!(adapter);
        envelope["payload"] = json!({
            "trace_kind":trace_kind, "trace_source":"old/session.jsonl", "trace_session_key":"session",
            "kind":"model_call", "requestId":"old-request", "model":"retired-model",
            "type":"function_call", "name":"retired-tool", "call_id":"call",
            "content":"retired body", "targets_info":[{"original":"https://example.invalid"}],
            "instruction_sha256":"old-hash", "provider_data":{"model":"retired-model"},
            "usage":{"total_tokens":999}, "total_tokens":999, "requests":7,
            "agents":[{"agent_id":"old-agent"}]
        });
        connection.execute(
            "INSERT INTO import_record_revisions(record_key,logical_key,record_kind,revision_hash,adapter,envelope_json) SELECT record_key,logical_key,?1,'retired-format',?2,?3 FROM import_record_revisions LIMIT 1",
            rusqlite::params![kind, adapter, envelope.to_string()],
        ).unwrap();
        connection.execute("UPDATE import_projection_memberships SET revision_id=last_insert_rowid()", []).unwrap();
        assert!(historical_trace_records(&connection, "trace-boundary", false).unwrap().is_empty(), "{adapter}/{kind}/{trace_kind}");
        let (summary, events) = collect_historical_agent_trace(&connection, "trace-boundary", true, false).unwrap();
        assert!(events.is_empty());
        assert!(summary.model.is_empty() && summary.instruction_hash.is_empty() && summary.tools.is_empty());
        assert_eq!((summary.run_count, summary.agent_count, summary.message_count, summary.tool_call_count, summary.total_tokens, summary.usage_agent_count), (0, 0, 0, 0, 0, 0));
        connection.execute_batch("ROLLBACK TO retired_trace; RELEASE retired_trace").unwrap();
    }
    assert_eq!(connection.query_row("SELECT envelope_json FROM import_record_revisions LIMIT 1", [], |row| row.get::<_, String>(0)).unwrap(), original);
    assert_eq!(fs::read(root.join("source/model-prompt-audit.json")).unwrap(), source);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_trace_retirement_rejected_latest_attempt_does_not_reveal_older_audit() {
    let (root, connection) = historical_trace_fixture();
    connection.execute("UPDATE import_projection_memberships SET attempt_number=1", []).unwrap();
    for (adapter, kind, envelope) in [
        ("retired_reader", "event_trace", "'not-json'"),
        ("model_audit", "event_trace", "json_set(envelope_json,'$.payload.trace_kind','unknown')"),
        ("model_audit", "event_trace", "json_set(envelope_json,'$.claim.readOnly',json('false'))"),
        ("retired_reader", "run_state", "envelope_json"),
    ] {
        connection.execute_batch("SAVEPOINT rejected_latest").unwrap();
        connection.execute_batch(&format!(
            "INSERT INTO import_record_revisions(record_key,logical_key,record_kind,revision_hash,adapter,envelope_json) SELECT record_key,logical_key,'{kind}','rejected-latest','{adapter}',{envelope} FROM import_record_revisions LIMIT 1; \
             INSERT INTO import_projection_memberships(scope_key,source_path,record_key,logical_key,revision_id,bundle_row_id,attempt_number) SELECT scope_key,source_path,record_key,logical_key,last_insert_rowid(),bundle_row_id,2 FROM import_projection_memberships LIMIT 1;"
        )).unwrap();
        assert!(historical_trace_records(&connection, "trace-boundary", true).unwrap().is_empty(), "{adapter}/{kind}/{envelope}");
        assert_eq!(historical_trace_records(&connection, "trace-boundary", false).unwrap().len(), 1);
        connection.execute_batch("ROLLBACK TO rejected_latest; RELEASE rejected_latest").unwrap();
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
