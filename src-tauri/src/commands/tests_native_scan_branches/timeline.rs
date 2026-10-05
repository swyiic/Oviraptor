#[test]
fn greybox_branches_write_source_and_runtime_into_one_graph() {
    let root = std::env::temp_dir().join(format!("oviraptor-greybox-graph-{}", Uuid::new_v4()));
    let repo = root.join("repo");
    let scratch = root.join("scratch");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("app.py"), "print('fixture')\n").unwrap();
    let db_path = db::initialize(&root.join("app")).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection.execute(
        "INSERT INTO sentinel_scans(id,project_name,status,scan_type,attempt_count,source_path) \
         VALUES('greybox-join','fixture','scanning','greybox',1,?1)",
        [repo.to_string_lossy().as_ref()],
    ).unwrap();
    let snapshot = crate::native_pipeline::snapshot::RepositorySnapshot::capture(&repo, &scratch, None).unwrap();
    snapshot.store(&connection, "greybox-join", 1).unwrap();
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,backend,role,status) VALUES('web-run','greybox-join',1,'native','coordinator','completed')",
        [],
    ).unwrap();
    connection.execute(
        "INSERT INTO tool_invocations(run_id,tool_name,status,input_summary_json) \
         VALUES('web-run','replay_http','completed','{\"method\":\"GET\",\"path\":\"/api/orders\"}')",
        [],
    ).unwrap();
    connection.execute(
        "INSERT INTO tool_invocations(run_id,tool_name,status,progress_signature) \
         VALUES('web-run','replay_http','completed','POST|/api/orders')",
        [],
    ).unwrap();
    register_native_branches(&connection, "greybox-join", 1, &["source", "web"]).unwrap();
    let source_report = json!({
        "sourceClaims": [
            {"method":"GET","path":"/api/orders","line":4,"claim":"订单详情"},
            {"method":"PUT","path":"/api/orders","line":9,"claim":"订单只能由创建者修改"}
        ]
    });
    finish_native_branch(&db_path, "greybox-join", 1, "source", "completed", "source done", &source_report).unwrap();
    let early: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_evidence_nodes WHERE root_run_id='greybox:greybox-join:1'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(early, 0, "源码分支单独结束时还不能汇合，Web 请求还没到齐");
    finish_native_branch(&db_path, "greybox-join", 1, "web", "completed", "web done", &json!({})).unwrap();
    let nodes: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_evidence_nodes WHERE root_run_id='greybox:greybox-join:1'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(nodes, 4, "两条源码声明和两次运行时请求要在同一张图里");
    let supports: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_evidence_edges WHERE root_run_id='greybox:greybox-join:1' AND kind='supports'",
        [],
        |row| row.get(0),
    ).unwrap();
    let contradictions: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_evidence_edges WHERE root_run_id='greybox:greybox-join:1' AND kind='contradicts'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(supports, 1, "只有 GET 对 GET 是 supports");
    assert_eq!(contradictions, 3, "同一路径上方法不一致的每一对都要留下 contradicts");
    let checkpoint: String = connection.query_row(
        "SELECT current_checkpoint FROM sentinel_scans WHERE id='greybox-join'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert!(checkpoint.contains("greybox_graph:source=2"), "{checkpoint}");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn collaboration_sequence_supports_incremental_timeline_recovery() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
         VALUES('sequence-run','source-regression',1,'https://authorized.example.test','native','coordinator','prepared')",
        [],
    ).unwrap();
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
         VALUES('sequence-unchanged','source-regression',1,'https://authorized.example.test','native','spa_api_mapper','prepared')",
        [],
    ).unwrap();

    let full = native_scan_status(&connection, "source-regression").unwrap();
    let first_sequence = full["latestSequence"].as_i64().unwrap();
    assert!(first_sequence > 0);
    assert!(full["timeline"].as_array().unwrap().iter().any(|item| {
        item["eventType"] == "agent_run" && item["id"] == "sequence-run"
            && item["sequence"].as_i64().unwrap_or_default() > 0
    }));
    assert!(full["timeline"].as_array().unwrap().iter().any(|item| {
        item["eventType"] == "agent_run" && item["id"] == "sequence-unchanged"
            && item["sequence"].as_i64().unwrap_or_default() == first_sequence
    }));

    connection.execute(
        "UPDATE agent_runs SET status='running',updated_at=datetime('now','localtime') WHERE id='sequence-run'",
        [],
    ).unwrap();
    let incremental = native_scan_status_after(
        &connection,
        "source-regression",
        Some(first_sequence),
    ).unwrap();
    assert_eq!(incremental["isIncremental"], true);
    assert!(incremental["latestSequence"].as_i64().unwrap() > first_sequence);
    let events = incremental["timeline"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["id"], "sequence-run");
    assert_eq!(events[0]["status"], "running");
    assert!(native_scan_status_after(&connection, "source-regression",
        incremental["latestSequence"].as_i64()).unwrap()["timeline"]
        .as_array().unwrap().is_empty());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retry_discards_cursor_from_previous_attempt() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
         VALUES('old-run','source-regression',1,'https://authorized.example.test','native','coordinator','prepared')",
        [],
    ).unwrap();
    let old = native_scan_status(&connection, "source-regression").unwrap();
    let old_cursor = old["latestSequence"].as_i64().unwrap();
    assert!(old_cursor > 0);
    connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id='source-regression'", []).unwrap();
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
         VALUES('new-run','source-regression',2,'https://authorized.example.test','native','coordinator','prepared')",
        [],
    ).unwrap();

    // Even a cursor ahead of the new attempt's watermark is irrelevant once
    // the attempt identity changes; the new conversation starts in full.
    let new = native_scan_status_for_attempt(
        &connection, "source-regression", Some(old_cursor + 100), Some(1),
    ).unwrap();
    assert_eq!(new["attemptNumber"], 2);
    assert_eq!(new["isIncremental"], false);
    assert!(new["timeline"].as_array().unwrap().iter().any(|event| event["id"] == "new-run"));
    assert!(!new["timeline"].as_array().unwrap().iter().any(|event| event["id"] == "old-run"));
    let delta = native_scan_status_for_attempt(
        &connection, "source-regression", new["latestSequence"].as_i64(), Some(2),
    ).unwrap();
    assert_eq!(delta["isIncremental"], true);
    assert!(delta["timeline"].as_array().unwrap().is_empty());
    assert!(native_scan_status_for_attempt(&connection, "source-regression", None, Some(0)).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn gap_mailbox_timeline_shows_bounded_proposal_and_deferred_assessment_not_raw_payload() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
         VALUES('gap-timeline-root','source-regression',1,'https://authorized.example.test','native','coordinator','terminal')",
        [],
    ).unwrap();
    let proposal = serde_json::json!({
        "schemaVersion":2, "summary":"Review owner control", "nextStep":"request_new_contract",
        "proposal":{
            "gapCode":"missing_owner_control", "missingEvidence":["owner X response"],
            "proposedContracts":["operator_approved_control_group"],
            "supportingFactRefs":["ev-123"], "prerequisites":["operator control group"],
            "estimatedCost":{"modelTokens":1000,"modelRequests":1,"targetRequests":3},
            "sideEffectClass":"read_only"
        },
        "secretBody":"Bearer abcdefghijklmnopqrstuvwxyz123"
    });
    let assessment = serde_json::json!({
        "schemaVersion":2, "summary":"Deferred", "decision":"deferred_requires_new_evidence_revision",
        "reasonCode":"proposal_is_not_a_verified_execution_contract",
        "secretBody":"Bearer abcdefghijklmnopqrstuvwxyz123"
    });
    for (id, kind, payload) in [
        ("gap-timeline-proposal", "gap_proposed", proposal),
        ("gap-timeline-assessment", "proposal_assessed", assessment),
    ] {
        connection.execute(
            "INSERT INTO agent_messages(id,run_id,root_run_id,dedup_key,from_agent,to_agent,kind,payload_json) \
             VALUES(?1,'gap-timeline-root','gap-timeline-root',?1,'deep_investigator','coordinator',?2,?3)",
            params![id, kind, payload.to_string()],
        ).unwrap();
    }
    let status = native_scan_status(&connection, "source-regression").unwrap();
    let events = status["timeline"].as_array().unwrap();
    let proposed = events.iter().find(|item| item["id"] == "gap-timeline-proposal").unwrap();
    assert_eq!(proposed["gapDetail"]["gapCode"], "missing_owner_control");
    assert_eq!(proposed["gapDetail"]["estimatedCost"]["targetRequests"], 3);
    assert_eq!(proposed["gapDetail"]["proposedContracts"][0], "operator_approved_control_group");
    let assessed = events.iter().find(|item| item["id"] == "gap-timeline-assessment").unwrap();
    assert_eq!(assessed["gapAssessment"]["reasonCode"], "proposal_is_not_a_verified_execution_contract");
    assert_eq!(assessed["gapAssessment"]["decision"], "deferred_requires_new_evidence_revision");
    assert!(!status.to_string().contains("abcdefghijklmnopqrstuvwxyz123"));
    let proposal_v3 = serde_json::json!({
        "schemaVersion":3, "summary":"Request a future control group", "nextStep":"request_new_contract",
        "proposal":{
            "gapCode":"missing_owner_control", "missingEvidence":["owner X response"],
            "proposedContracts":["new_attempt_control_group_request"],
            "supportingFactRefs":[], "prerequisites":["new operator authorization"],
            "estimatedCost":{"modelTokens":1000,"modelRequests":1,"targetRequests":3},
            "sideEffectClass":"read_only"
        }
    });
    let assessment_v3 = serde_json::json!({
        "schemaVersion":3, "decision":"deferred_requires_new_evidence_revision",
        "reasonCode":"operator_approval_new_attempt_required",
        "newAttemptRequired":true, "targetRequestsGranted":0
    });
    let (_, projected_proposal) = gap_timeline_projection("gap_proposed", &proposal_v3).unwrap();
    assert_eq!(projected_proposal["proposedContracts"][0], "new_attempt_control_group_request");
    let (_, projected_assessment) = gap_timeline_projection("proposal_assessed", &assessment_v3).unwrap();
    assert_eq!(projected_assessment["newAttemptRequired"], true);
    assert_eq!(projected_assessment["targetRequestsGranted"], 0);
    let mut false_approval = assessment_v3.clone();
    false_approval["targetRequestsGranted"] = serde_json::json!(3);
    assert!(gap_timeline_projection("proposal_assessed", &false_approval).is_none());
    let mut wrong_contract = proposal_v3;
    wrong_contract["proposal"]["proposedContracts"] = serde_json::json!(["operator_approved_control_group"]);
    assert!(gap_timeline_projection("gap_proposed", &wrong_contract).is_none());
    let unsupported = serde_json::json!({
        "schemaVersion":2, "nextStep":"request_new_contract",
        "proposal":{
            "gapCode":"test", "sideEffectClass":"read_only", "missingEvidence":["missing"],
            "prerequisites":[], "supportingFactRefs":[], "proposedContracts":["arbitrary_shell"],
            "estimatedCost":{"modelTokens":0,"modelRequests":0,"targetRequests":0}
        }
    });
    assert!(gap_timeline_projection("gap_proposed", &unsupported).is_none());
    assert!(gap_timeline_projection("proposal_assessed", &serde_json::json!({
        "schemaVersion":2, "decision":"approved", "reasonCode":"proposal_is_not_a_verified_execution_contract"
    })).is_none());
    fs::remove_dir_all(root).unwrap();
}
