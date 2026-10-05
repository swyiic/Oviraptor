//! GRY-001 … GRY-007 — greybox: one evidence graph for the source branch and the
//! browser branch, contradictions that survive, a lane that does not widen, and a
//! conclusion label that cannot claim more than the graph holds.

use super::*;
use crate::agent_runtime::evidence_graph::contract::EvidenceProvenance;
use crate::agent_runtime::evidence_graph::store::{
    list_evidence_edges_at_revision, list_evidence_nodes_at_revision,
};
use crate::native_pipeline::greybox::{
    self, claim_read_only_lane, read_only_lane_holders, route_binding, Branch, Conclusion,
    GreyboxGraph,
};
use serde_json::json;
use std::fs;

fn graph(connection: &rusqlite::Connection, root: &std::path::Path, run_id: &str) -> GreyboxGraph {
    make_agent_run(connection, run_id, "greybox-scan", "coordinator");
    GreyboxGraph::new(
        run_id,
        run_id,
        1,
        "https://target.example",
        &capture(root, None),
    )
}

#[test]
fn gry_001_source_and_browser_land_in_one_graph() {
    let root = sandbox("gry-001");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let graph = graph(&connection, &root, "root-001");
    let static_node = graph
        .record_source_claim(
            &connection,
            "sql-injection-python-app",
            "拼接了请求参数",
            "python/app.py",
            2,
        )
        .unwrap();
    let runtime_node = graph
        .record_runtime_observation(
            &connection,
            "GET /user/1",
            json!({"endpoint": "/user/1", "status": 200}),
        )
        .unwrap();

    let nodes = list_evidence_nodes_at_revision(&connection, "root-001", 1).unwrap();
    assert_eq!(nodes.len(), 2, "两条分支要在同一张图里：{nodes:?}");
    assert!(nodes.iter().any(|node| node.id == static_node));
    assert!(nodes.iter().any(|node| node.id == runtime_node));
    let provenance: Vec<_> = nodes
        .iter()
        .map(|node| (node.kind.as_str(), node.provenance))
        .collect();
    assert!(
        provenance
            .iter()
            .any(|(_, value)| *value == EvidenceProvenance::SourceDerived),
        "源码分支必须标 source_derived：{provenance:?}"
    );
    assert!(
        provenance
            .iter()
            .any(|(_, value)| *value == EvidenceProvenance::Observed),
        "浏览器分支必须标 observed：{provenance:?}"
    );
    // One graph means one query surface: no second table holds the browser half.
    assert_eq!(
        table_count(&connection, "agent_evidence_nodes"),
        2,
        "两条分支不能各写一份存储"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gry_002_endpoint_to_source_binding_is_stable() {
    let root = sandbox("gry-002");
    let snapshot = capture(&root, None);
    let binding = |tree: &str, symbol: &str| {
        route_binding(
            tree,
            "GET",
            "/user/1?page=2",
            "UserController",
            symbol,
            "python/app.py",
            2,
        )
    };
    assert_eq!(
        binding(&snapshot.tree_hash, "load"),
        binding(&snapshot.tree_hash, "load"),
        "同一 commit 的同一映射必须稳定"
    );
    assert_ne!(
        binding(&snapshot.tree_hash, "load"),
        binding(&snapshot.tree_hash, "update"),
        "不同 symbol 不能撞进同一个映射"
    );
    assert_ne!(
        binding(&snapshot.tree_hash, "load"),
        binding(
            "0000000000000000000000000000000000000000000000000000000000000000",
            "load"
        ),
        "换了 tree hash 就是另一次冻结"
    );
    // The query string and the trailing slash are the same route.
    assert_eq!(
        route_binding(
            &snapshot.tree_hash,
            "get",
            "/user/1",
            "UserController",
            "load",
            "python/app.py",
            2
        ),
        binding(&snapshot.tree_hash, "load")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gry_003_identity_source_and_runtime_form_one_candidate_chain() {
    let root = sandbox("gry-003");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let graph = graph(&connection, &root, "root-003");
    let static_node = graph
        .record_source_claim(
            &connection,
            "idor-user-endpoint",
            "按 id 直接取对象",
            "python/app.py",
            2,
        )
        .unwrap();
    let identity = graph
        .record_runtime_observation(
            &connection,
            "identity:member",
            json!({"identity": "member", "role": "user"}),
        )
        .unwrap();
    let request = graph
        .record_runtime_observation(
            &connection,
            "GET /user/2 as member",
            json!({"endpoint": "/user/2", "status": 200}),
        )
        .unwrap();
    graph
        .support(
            &connection,
            &request,
            &static_node,
            "同身份重放取到他人对象",
        )
        .unwrap();
    graph
        .support(&connection, &identity, &request, "该请求确实由这个身份发出")
        .unwrap();

    let edges = list_evidence_edges_at_revision(&connection, "root-003", 1).unwrap();
    assert_eq!(edges.len(), 2, "证据链要连着三条事实：{edges:?}");
    assert_eq!(
        graph.conclusion_of(&connection, &static_node).unwrap(),
        Conclusion::RuntimeVerified
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gry_004_a_contradiction_adds_an_edge_and_keeps_the_static_evidence() {
    let root = sandbox("gry-004");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let graph = graph(&connection, &root, "root-004");
    let static_node = graph
        .record_source_claim(
            &connection,
            "cache-header-mismatch",
            "源码里输出了私有缓存头",
            "src/server.js",
            2,
        )
        .unwrap();
    let runtime_node = graph
        .record_runtime_observation(
            &connection,
            "GET /user/1 cache",
            json!({"endpoint": "/user/1", "cacheControl": "public"}),
        )
        .unwrap();
    graph
        .contradict(
            &connection,
            &runtime_node,
            &static_node,
            "实测头与源码不一致",
        )
        .unwrap();

    let edges = list_evidence_edges_at_revision(&connection, "root-004", 1).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].kind.as_str(), "contradicts");
    assert_eq!(edges[0].from_node_id, runtime_node);
    assert_eq!(edges[0].to_node_id, static_node);
    let nodes = list_evidence_nodes_at_revision(&connection, "root-004", 1).unwrap();
    assert_eq!(nodes.len(), 2, "矛盾不能删掉静态证据：{nodes:?}");
    assert!(nodes.iter().any(|node| node.id == static_node));
    assert_eq!(
        graph.conclusion_of(&connection, &static_node).unwrap(),
        Conclusion::Contradicted
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gry_005_the_read_only_lane_holds_exactly_one_run_per_target() {
    let root = sandbox("gry-005");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    make_agent_run(&connection, "lane-a", "greybox-scan", "coordinator");
    make_agent_run(&connection, "lane-b", "greybox-scan", "coordinator");
    let target = "https://target.example";

    claim_read_only_lane(&connection, target, "lane-a").unwrap();
    assert_eq!(
        read_only_lane_holders(&connection, target).unwrap(),
        vec!["lane-a".to_string()]
    );
    // The same run re-claiming is not a second slot.
    claim_read_only_lane(&connection, target, "lane-a").unwrap();
    assert_eq!(
        read_only_lane_holders(&connection, target).unwrap().len(),
        1,
        "容量必须还是 1"
    );
    let error = claim_read_only_lane(&connection, target, "lane-b").unwrap_err();
    assert!(error.contains("lane-a"), "{error}");
    // A different target is a different slot.
    claim_read_only_lane(&connection, "https://other.example", "lane-b").unwrap();
    // Nothing here widened the target-touching lane.
    assert_eq!(
        table_count(
            &connection,
            "agent_assignments WHERE lane='target_touching'"
        ),
        0,
        "目标触碰不能被这个阶段放开"
    );
    assert_eq!(greybox::READ_ONLY_LANE_CAPACITY, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gry_006_a_failed_source_branch_closes_the_web_branch_with_a_gap() {
    let root = sandbox("gry-006");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let graph = graph(&connection, &root, "root-006");
    let closed = graph.close_branch(
        Branch::Source,
        Some("analyzer_missing"),
        &["semgrep 未安装".to_string()],
    );
    assert!(!closed.ok);
    assert_eq!(closed.branch, Branch::Source);
    assert!(closed.gaps.contains(&"semgrep 未安装".to_string()));
    assert!(
        closed
            .gaps
            .contains(&"source_branch_failed:analyzer_missing".to_string()),
        "{:?}",
        closed.gaps
    );
    assert!(closed
        .gaps
        .contains(&"web_branch_closed_with_gap".to_string()));
    // A runtime-only failure does not pretend the source half ran either.
    let runtime_only = graph.close_branch(Branch::Runtime, Some("cdp_timeout"), &[]);
    assert_eq!(
        runtime_only.gaps,
        vec!["runtime_branch_failed:cdp_timeout".to_string()]
    );
    let clean = graph.close_branch(Branch::Source, None, &[]);
    assert!(clean.ok && clean.gaps.is_empty());

    // And this branch has no path back to a Strix process.
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/native_pipeline/greybox.rs"
    ))
    .unwrap()
    .to_ascii_lowercase();
    for banned in ["strix_", "launch_strix"] {
        assert!(!text.contains(banned), "greybox.rs 引用了 {banned}");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gry_007_a_source_only_conclusion_is_never_labelled_runtime_verified() {
    let root = sandbox("gry-007");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let graph = graph(&connection, &root, "root-007");
    let static_node = graph
        .record_source_claim(
            &connection,
            "unchecked-redirect",
            "跳转目标没有校验",
            "src/server.js",
            2,
        )
        .unwrap();
    let conclusion = graph.conclusion_of(&connection, &static_node).unwrap();
    assert_eq!(conclusion, Conclusion::SourceOnly);
    assert!(!conclusion.is_runtime_verified());
    assert_eq!(conclusion.as_str(), "source_only");

    // An observation that is not linked to the claim still does not verify it.
    let unlinked = graph
        .record_runtime_observation(
            &connection,
            "GET /other",
            json!({"endpoint": "/other", "status": 200}),
        )
        .unwrap();
    assert!(list_evidence_edges_at_revision(&connection, "root-007", 1)
        .unwrap()
        .is_empty());
    assert_eq!(
        graph.conclusion_of(&connection, &static_node).unwrap(),
        Conclusion::SourceOnly,
        "{unlinked} 没有被连到候选上"
    );
    let _ = fs::remove_dir_all(root);
}
