#[test]
fn directive_scope_distinguishes_missing_scan_from_corrupt_attempt() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    assert_eq!(scan_directive_scope(&connection, "missing-scan", "team").unwrap_err(), "任务不存在");
    connection.execute(
        "UPDATE sentinel_scans SET attempt_count='broken' WHERE id='source-regression'",
        [],
    ).unwrap();
    let error = scan_directive_scope(&connection, "source-regression", "team").unwrap_err();
    assert!(error.contains("指令任务尝试"), "corruption must not look like an unknown scan: {error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn attempt_mailbox_history_is_scoped_paginated_and_redacted() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    connection.execute("INSERT OR IGNORE INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES('source-regression',1)", []).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES('source-regression',2)", []).unwrap();
    for (id, attempt) in [("history-one", 1), ("history-two", 2)] {
        connection.execute(
            "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
             VALUES(?1,'source-regression',?2,'https://authorized.example.test','native','coordinator','terminal')",
            params![id, attempt],
        ).unwrap();
    }
    for (id, root_id, timestamp, summary) in [
        ("old-1", "history-one", "2026-01-01 01:00:00", "Bearer abcdefghijklmnopqrstuvwxyz123"),
        ("old-2", "history-one", "2026-01-01 02:00:00", "second message"),
        ("old-3", "history-one", "2026-01-01 03:00:00", "third message"),
        ("new-1", "history-two", "2026-01-01 04:00:00", "new attempt"),
    ] {
        connection.execute(
            "INSERT INTO agent_messages(id,run_id,root_run_id,dedup_key,from_agent,to_agent,kind,created_at,payload_json) \
             VALUES(?1,?2,?2,?1,'coordinator','evidence_reviewer','gap_proposed',?3,?4)",
            params![id, root_id, timestamp, serde_json::json!({"summary":summary}).to_string()],
        ).unwrap();
    }
    let page = native_attempt_mailbox_history(&connection, "source-regression", 1, None, None, 2).unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 2);
    assert_eq!(page["messages"][0]["id"], "old-2");
    assert_eq!(page["messages"][1]["id"], "old-3");
    assert_eq!(page["hasOlder"], true);
    let older = native_attempt_mailbox_history(
        &connection, "source-regression", 1,
        page["olderCursor"]["createdAt"].as_str(), page["olderCursor"]["id"].as_str(), 2,
    ).unwrap();
    assert_eq!(older["messages"][0]["id"], "old-1");
    assert!(!older.to_string().contains("abcdefghijklmnopqrstuvwxyz123"));
    assert!(!native_scan_status(&connection, "source-regression").unwrap().to_string()
        .contains("abcdefghijklmnopqrstuvwxyz123"));
    assert_eq!(older["hasOlder"], false);
    let latest = native_attempt_mailbox_history(&connection, "source-regression", 2, None, None, 2).unwrap();
    assert_eq!(latest["messages"].as_array().unwrap().len(), 1);
    assert_eq!(latest["messages"][0]["id"], "new-1");
    assert!(native_attempt_mailbox_history(&connection, "source-regression", 3, None, None, 2).is_err());
    assert!(native_attempt_mailbox_history(&connection, "source-regression", 1, Some("2026"), None, 2).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn attempt_tool_history_is_scoped_paginated_and_never_returns_tool_inputs() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    connection.execute("INSERT OR IGNORE INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES('source-regression',1)", []).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number) VALUES('source-regression',2)", []).unwrap();
    for (id, attempt) in [("tool-run-old", 1), ("tool-run-new", 2)] {
        connection.execute(
            "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
             VALUES(?1,'source-regression',?2,'https://authorized.example.test','native','web_executor','terminal')",
            params![id, attempt],
        ).unwrap();
    }
    for (invocation, run, when, status) in [
        ("old-1", "tool-run-old", "2026-01-01 01:00:00", "confirmed"),
        ("old-2", "tool-run-old", "2026-01-01 02:00:00", "refused"),
        ("old-3", "tool-run-old", "2026-01-01 02:00:00", "interrupted"),
        ("new-1", "tool-run-new", "2026-01-01 03:00:00", "confirmed"),
    ] {
        connection.execute(
            "INSERT INTO tool_invocations(run_id,invocation_id,tool_name,input_summary_json,policy_decision,status,started_at,finished_at,request_artifact_id,response_artifact_id,error_class) \
             VALUES(?1,?2,'replay_http',?3,'allow',?4,?5,?5,'request-safe-id','response-safe-id','secret=fixture-private')",
            params![run, invocation, r#"{"secret":"fixture-private"}"#, status, when],
        ).unwrap();
    }
    let page = native_attempt_tool_history(&connection, "source-regression", 1, None, None, 2).unwrap();
    assert_eq!(page["invocations"].as_array().unwrap().len(), 2);
    assert_eq!(page["invocations"][0]["invocationId"], "old-2");
    assert_eq!(page["invocations"][1]["invocationId"], "old-3");
    assert_eq!(page["hasOlder"], true);
    let older = native_attempt_tool_history(
        &connection, "source-regression", 1,
        page["olderCursor"]["startedAt"].as_str(), page["olderCursor"]["id"].as_i64(), 2,
    ).unwrap();
    assert_eq!(older["invocations"][0]["invocationId"], "old-1");
    assert!(!older.to_string().contains("fixture-private"));
    assert_eq!(older["hasOlder"], false);
    assert_eq!(native_attempt_tool_history(&connection, "source-regression", 2, None, None, 2)
        .unwrap()["invocations"][0]["invocationId"], "new-1");
    assert!(native_attempt_tool_history(&connection, "source-regression", 3, None, None, 2).is_err());
    assert!(native_attempt_tool_history(&connection, "source-regression", 1, Some("2026"), None, 2).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn open_directive_drafts_survive_status_reload_without_exposing_fencing_tokens() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    let first = crate::agent_runtime::multi_agent::directive::create_draft(
        &connection,
        "source-regression",
        1,
        "coordinator-run",
        "https://authorized.example.test",
        "coordinator",
        "先检查登录接口",
        7,
        "private-fencing-token",
    )
    .unwrap();
    let second = crate::agent_runtime::multi_agent::directive::create_draft(
        &connection,
        "source-regression",
        1,
        "coordinator-run",
        "https://authorized.example.test",
        "coordinator",
        "再核对 F-12",
        7,
        "private-fencing-token",
    )
    .unwrap();

    let serialized = serde_json::to_value(&first).unwrap();
    assert!(serialized.get("boundLeaseEpoch").is_none());
    assert!(serialized.get("boundFencingToken").is_none());
    assert!(!serialized.to_string().contains("private-fencing-token"));

    let status = native_scan_status(&connection, "source-regression").unwrap();
    let drafts = status["directiveDrafts"].as_array().unwrap();
    assert_eq!(drafts.len(), 2, "刷新后必须恢复所有尚未确认的草案");
    assert_eq!(drafts[0]["threadKey"], "team");
    assert!(status["timeline"].as_array().unwrap().iter()
        .filter(|item| item["eventType"] == "directive_draft")
        .all(|item| item["threadKey"] == "team"));
    let restored_ids = drafts
        .iter()
        .filter_map(|draft| draft["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(restored_ids, std::collections::HashSet::from([first.id.as_str(), second.id.as_str()]));
    assert!(!status.to_string().contains("private-fencing-token"));

    crate::agent_runtime::multi_agent::directive::cancel_draft(
        &connection,
        "source-regression",
        &first.id,
        first.revision,
        &first.draft_hash,
    )
    .unwrap();
    let reloaded = native_scan_status(&connection, "source-regression").unwrap();
    let remaining = reloaded["directiveDrafts"].as_array().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["id"], second.id);

    fs::remove_dir_all(root).unwrap();
}
