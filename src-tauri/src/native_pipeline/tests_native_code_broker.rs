//! Source Broker scope, candidate, and resource-budget regressions.

use super::*;
use crate::agent_runtime::review::{
    insert_review_decision, list_review_decisions_for_candidate, ReviewDecision, ReviewVerdict,
};
use crate::native_pipeline::tools::SourceBroker;
use serde_json::json;
use std::fs;

#[test]
fn code_007_results_are_pinned_to_the_snapshot_and_the_content_hash() {
    let root = sandbox("code-007");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    make_agent_run(&connection, "run-code-007", "code-scan", "coordinator");
    let snapshot = capture(&root, None);
    let mut broker = SourceBroker::new(
        snapshot.clone(),
        "code-scan",
        1,
        "run-code-007",
        "run-code-007",
        1,
    );

    let listed = broker
        .call(
            &connection,
            "repo.read_slice",
            &json!({"path": "python/app.py"}),
        )
        .unwrap();
    let recorded = snapshot.content_hash_of("python/app.py").unwrap();
    assert_eq!(listed["contentHash"], json!(recorded));
    assert_eq!(listed["totalLines"], json!(2));
    assert!(listed["lines"][0]
        .as_str()
        .unwrap()
        .contains("def load(cur, pid)"));

    // Editing the original cannot change the evidence quoted from the frozen copy.
    write_file(
        &root.join("repo"),
        "python/app.py",
        "def load():\n    pass\n",
    );
    let frozen = broker
        .call(
            &connection,
            "repo.read_slice",
            &json!({"path": "python/app.py"}),
        )
        .unwrap();
    assert_eq!(frozen, listed);
    assert!(snapshot.verify_unchanged().is_err());
    fs::remove_file(snapshot.frozen_root.join("python/app.py")).unwrap();
    write_file(
        &snapshot.frozen_root,
        "python/app.py",
        "tampered frozen copy",
    );
    let error = broker
        .call(
            &connection,
            "repo.read_slice",
            &json!({"path":"python/app.py"}),
        )
        .unwrap_err();
    assert_eq!(error.code, "snapshot_integrity");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_008_a_rule_hit_or_a_model_suspicion_is_only_ever_a_candidate() {
    let root = sandbox("code-008");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    make_agent_run(&connection, "run-code-008", "code-scan", "coordinator");
    let snapshot = capture(&root, None);
    let mut broker = SourceBroker::new(snapshot, "code-scan", 1, "run-code-008", "run-code-008", 1);

    let submitted = broker
        .call(
            &connection,
            "evidence.submit_candidate",
            &json!({
                "title": "疑似 SQL 注入",
                "severity": "high",
                "path": "python/app.py",
                "line": 2,
                "rationale": "拼接了请求参数"
            }),
        )
        .unwrap();
    assert_eq!(submitted["reviewState"], json!("candidate"));
    assert_eq!(submitted["confirmable"], json!(false));

    // The same call cannot declare itself confirmed.
    let refusal = broker
        .call(
            &connection,
            "evidence.submit_candidate",
            &json!({"title": "x", "rationale": "y", "status": "confirmed"}),
        )
        .unwrap_err();
    assert_eq!(refusal.code, "confirmation_not_model_side");

    let confirmed = table_count(
        &connection,
        "agent_evidence_nodes WHERE kind='finding' AND provenance='observed'",
    );
    assert_eq!(confirmed, 0, "模型侧不能产生已确认 Finding");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_009_counter_evidence_lets_the_reviewer_reject_the_candidate() {
    let root = sandbox("code-009");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    make_agent_run(&connection, "root-009", "code-scan", "coordinator");
    make_reviewer_run(&connection, "reviewer-009", "root-009", "code-scan");
    let mut broker = SourceBroker::new(
        capture(&root, None),
        "code-scan",
        1,
        "root-009",
        "root-009",
        1,
    );
    let submitted = broker
        .call(
            &connection,
            "evidence.submit_candidate",
            &json!({
                "title": "sql-injection-python-app",
                "severity": "high",
                "path": "python/app.py",
                "line": 2,
                "rationale": "拼接了请求参数"
            }),
        )
        .unwrap();
    let candidate_id = submitted["id"].as_str().unwrap().to_string();

    let decision = ReviewDecision {
        verdict: ReviewVerdict::Rejected,
        reason_codes: vec!["sanitizer_present".to_string()],
        counter_evidence_refs: vec!["python/app.py:2".to_string()],
        confidence: 0.9,
        severity: "high".to_string(),
        ..ReviewDecision::new(
            "root-009",
            &candidate_id,
            1,
            "reviewer-009",
            ReviewVerdict::Rejected,
        )
    };
    insert_review_decision(&connection, &decision).unwrap();
    let stored = list_review_decisions_for_candidate(&connection, &candidate_id).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].verdict, ReviewVerdict::Rejected);
    // The candidate node is still there for the audit trail.
    let nodes = table_count(
        &connection,
        "agent_evidence_nodes WHERE kind='candidate_finding'",
    );
    assert_eq!(nodes, 1, "拒绝不能删除证据");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_010_the_broker_refuses_shell_paths_binaries_rules_and_network() {
    let root = sandbox("code-010");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    make_agent_run(&connection, "run-code-010", "code-scan", "coordinator");
    let mut broker = SourceBroker::new(
        capture(&root, None),
        "code-scan",
        1,
        "run-code-010",
        "run-code-010",
        1,
    );
    let cases = [
        ("shell.exec", json!({"command": "ls"}), "arbitrary_shell"),
        (
            "repo.read_slice",
            json!({"path": "/etc/passwd"}),
            "absolute_path",
        ),
        (
            "repo.read_slice",
            json!({"path": "../../secrets"}),
            "path_escape",
        ),
        (
            "repo.read_slice",
            json!({"path": "src/../../../x"}),
            "path_escape",
        ),
        (
            "repo.read_slice",
            json!({"path": "src/server.js", "binary": "/tmp/mine"}),
            "self_chosen_binary",
        ),
        (
            "analyzer.list_results",
            json!({"rules": "https://example/pack.yaml"}),
            "self_downloaded_rules",
        ),
        (
            "repo.search",
            json!({"query": "token", "url": "https://example.com"}),
            "unapproved_network",
        ),
        ("dependency.get_record", json!({}), "invalid_arguments"),
        ("write_file", json!({"path": "a"}), "source_modification"),
        ("nope.unknown", json!({}), "unknown_tool"),
    ];
    for (name, arguments, code) in cases {
        let denial = broker.call(&connection, name, &arguments).unwrap_err();
        assert_eq!(
            denial.code, code,
            "{name} {arguments} 被拒为 {:?}",
            denial.code
        );
    }
    // An excluded path is not readable either.
    let denial = broker
        .call(
            &connection,
            "repo.read_slice",
            &json!({"path": "node_modules/left-pad/index.js"}),
        )
        .unwrap_err();
    assert_eq!(denial.code, "path_not_in_snapshot");
    // The allowed surface answers for real.
    let inventory = broker
        .call(&connection, "repo.inventory", &json!({"limit": 5}))
        .unwrap();
    assert_eq!(inventory["files"].as_array().unwrap().len(), 5);
    let searched = broker
        .call(&connection, "repo.search", &json!({"query": "cur.execute"}))
        .unwrap();
    assert!(searched["matches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["path"] == json!("python/app.py")));
    // No diff base means no incremental answer either.
    let denial = broker
        .call(&connection, "git.changed_files", &json!({}))
        .unwrap_err();
    assert_eq!(denial.code, "diff_base_unavailable");
    // Callgraph is not a capability this build has, and says so.
    let denial = broker
        .call(&connection, "callgraph.get_slice", &json!({}))
        .unwrap_err();
    assert_eq!(denial.code, "unsupported_capability");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_010_search_has_a_total_work_budget_and_rejects_oversized_queries() {
    let root = sandbox("code-010-search-budget");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    make_agent_run(
        &connection,
        "run-code-010-budget",
        "code-scan",
        "coordinator",
    );
    let repo = fixture_repository(&root);
    let block = "x".repeat(1024 * 1024);
    for index in 0..17 {
        write_file(&repo, &format!("a-bulk/{index:03}.txt"), &block);
    }
    let snapshot = capture_at(&repo, &scratch_dir(&root), None);
    let mut broker = SourceBroker::new(
        snapshot,
        "code-scan",
        1,
        "run-code-010-budget",
        "run-code-010-budget",
        1,
    );
    let result = broker
        .call(
            &connection,
            "repo.search",
            &json!({"query": "not-present", "prefix": "a-bulk/"}),
        )
        .unwrap();
    assert_eq!(result["scannedFiles"], 16);
    assert_eq!(result["scannedBytes"], 16 * 1024 * 1024);
    assert_eq!(result["truncated"], true);
    assert!(result["matches"].as_array().unwrap().is_empty());

    let denial = broker
        .call(
            &connection,
            "repo.search",
            &json!({"query": "q".repeat(257)}),
        )
        .unwrap_err();
    assert_eq!(denial.code, "invalid_arguments");
    let _ = fs::remove_dir_all(root);
}
