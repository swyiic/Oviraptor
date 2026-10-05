//! CI-001 … CI-007 — the CI surface: a freeze that says what was scanned, a gate that
//! reads only stored reviewer decisions, exit codes a pipeline can branch on, and an
//! export that goes back through the same canonical importer.

use super::*;
use crate::agent_runtime::review::{insert_review_decision, ReviewDecision, ReviewVerdict};
use crate::native_pipeline::analyzer::{AnalyzerEngine, AnalyzerOutcome, AnalyzerStatus};
use crate::native_pipeline::ci::{self, CiFreeze, CiScope, GatePolicy, GateStatus};
use crate::native_pipeline::NativeSourcePlan;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::time::Duration;

fn ran(engine: AnalyzerEngine, version: &str, digest: &str) -> AnalyzerOutcome {
    AnalyzerOutcome {
        sarif_sha256: String::new(),
        engine,
        status: AnalyzerStatus::Ran { exit: Some(0) },
        version: version.to_string(),
        rule_pack_digest: digest.to_string(),
        image_digest: String::new(),
        network_disabled: true,
        repository_read_only: true,
        args: Vec::new(),
        sarif_path: None,
        truncated: false,
        duration: Duration::from_millis(12),
        invocation_key: format!("{}-key", engine.as_str()),
        reused: false,
    }
}

fn outcomes() -> Vec<AnalyzerOutcome> {
    vec![
        ran(AnalyzerEngine::Semgrep, "1.79.0", "rules-digest-1"),
        ran(AnalyzerEngine::CodeQL, "2.16.0", "rules-digest-1"),
    ]
}

#[test]
fn ci_freeze_accounts_for_every_analyzers_rule_pack_and_is_order_independent() {
    let root = sandbox("ci-all-rule-digests");
    let snapshot = capture(&root, Some("main"));
    let baseline = CiFreeze::of(&snapshot, &outcomes(), CiScope::Diff);
    let mut changed = outcomes();
    changed[1].rule_pack_digest = "second-engine-new-rules".into();
    let freeze = CiFreeze::of(&snapshot, &changed, CiScope::Diff);
    assert_ne!(
        freeze.rule_pack_digest, baseline.rule_pack_digest,
        "changing only the second engine's rules must change the freeze"
    );
    changed.reverse();
    assert_eq!(freeze, CiFreeze::of(&snapshot, &changed, CiScope::Diff));
    fs::remove_dir_all(root).unwrap();
}

fn freeze_at(root: &Path, base: Option<&str>, scope: CiScope) -> CiFreeze {
    CiFreeze::of(&capture_at(root, base), &outcomes(), scope)
}

/// Capture the same fixture into its own scratch so two runs never share a directory.
fn capture_at(
    root: &Path,
    base: Option<&str>,
) -> crate::native_pipeline::snapshot::RepositorySnapshot {
    let scratch = root.join(format!("scratch-{}", std::process::id()));
    crate::native_pipeline::snapshot::RepositorySnapshot::capture(
        &fixture_repository(root),
        &scratch,
        base,
    )
    .unwrap()
}

fn confirm(
    connection: &Connection,
    root: &str,
    reviewer: &str,
    candidate: &str,
    severity: &str,
    verdict: ReviewVerdict,
) {
    confirm_at(connection, root, reviewer, candidate, severity, "", verdict)
}

fn confirm_at(
    connection: &Connection,
    root: &str,
    reviewer: &str,
    candidate: &str,
    severity: &str,
    location: &str,
    verdict: ReviewVerdict,
) {
    let evidence = if location.is_empty() {
        vec![format!("{candidate}@1")]
    } else {
        vec![location.to_string()]
    };
    let decision = ReviewDecision {
        reason_codes: vec![format!("{verdict:?}").to_lowercase()],
        evidence_refs: evidence,
        confidence: 0.8,
        severity: severity.to_string(),
        ..ReviewDecision::new(root, candidate, 1, reviewer, verdict)
    };
    insert_review_decision(connection, &decision).unwrap();
}

fn open_ci_root(root: &Path, tag: &str) -> Connection {
    let db_path = initialize_db(root);
    let connection = open_connection(&db_path);
    make_agent_run(&connection, "ci-root", "ci-scan", "coordinator");
    make_reviewer_run(&connection, "ci-reviewer", "ci-root", "ci-scan");
    let _ = tag;
    connection
}

fn complete_coverage() -> Vec<String> {
    Vec::new()
}

#[test]
fn source_specialists_unreviewed_ci_requires_review_even_with_complete_nonempty_coverage() {
    let root = sandbox("ci-unreviewed-source");
    let freeze = freeze_at(&root, None, CiScope::Full);
    assert!(freeze.file_count > 0);
    for enabled in [true, false] {
        let policy = GatePolicy::from_workbench(
            &json!({"maxCritical":100,"maxHigh":100,"blockRelease":enabled}),
        )
        .unwrap();
        let report = ci::evaluate_unreviewed(&freeze, &[], None, policy).unwrap();
        assert_eq!(report.status, GateStatus::CoverageIncomplete);
        assert_eq!(report.gaps, vec!["source_review_not_completed"]);
        assert!(report.decisions.is_empty());
        assert!(report.blocking.is_empty());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_ci_policy_limits_are_independent_and_count_only_confirmed_decisions() {
    let root = sandbox("ci-workbench-severity-limits");
    let connection = open_ci_root(&root, "limits");
    let freeze = freeze_at(&root, None, CiScope::Full);
    for (candidate, severity, verdict) in [
        ("c1", "critical", ReviewVerdict::Confirmed),
        ("h1", "HIGH", ReviewVerdict::Confirmed),
        ("h2", "high", ReviewVerdict::Confirmed),
        ("rejected", "critical", ReviewVerdict::Rejected),
        ("unproven", "critical", ReviewVerdict::InsufficientEvidence),
    ] {
        confirm(
            &connection,
            "ci-root",
            "ci-reviewer",
            candidate,
            severity,
            verdict,
        );
    }
    for (critical, high, enabled, expected) in [
        (1, 2, true, GateStatus::Warning),
        (0, 999, true, GateStatus::Blocked),
        (999, 1, true, GateStatus::Blocked),
        (0, 0, false, GateStatus::Warning),
        (1, 2, false, GateStatus::Warning),
    ] {
        let policy = GatePolicy::from_workbench(
            &json!({"maxCritical":critical,"maxHigh":high,"blockRelease":enabled}),
        )
        .unwrap();
        let report = ci::evaluate(&connection, "ci-root", &freeze, &[], None, policy).unwrap();
        assert_eq!(
            report.status, expected,
            "limits {critical}/{high}/{enabled}"
        );
        assert_eq!(report.blocking.len(), 3);
        assert_eq!(
            report.as_json()["policy"]["releaseLimits"]["blockRelease"],
            enabled
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_ci_policy_disabled_release_block_does_not_hide_missing_coverage_or_infrastructure() {
    let root = sandbox("ci-workbench-not-security-bypass");
    let connection = open_ci_root(&root, "limits");
    let freeze = freeze_at(&root, None, CiScope::Full);
    confirm(
        &connection,
        "ci-root",
        "ci-reviewer",
        "critical",
        "critical",
        ReviewVerdict::Confirmed,
    );
    let policy =
        GatePolicy::from_workbench(&json!({"maxCritical":0,"maxHigh":0,"blockRelease":false}))
            .unwrap();
    let report = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &["analyzer_missing".into()],
        None,
        policy.clone(),
    )
    .unwrap();
    assert_eq!(report.status, GateStatus::CoverageIncomplete);
    let report = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &[],
        Some("sandbox_unavailable"),
        policy,
    )
    .unwrap();
    assert_eq!(report.status, GateStatus::InfraFailed);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ci_001_the_same_commit_input_and_rules_reproduce_the_same_logical_keys() {
    let root = sandbox("ci-001");
    let first_root = root.join("first");
    let second_root = root.join("second");
    fs::create_dir_all(&first_root).unwrap();
    fs::create_dir_all(&second_root).unwrap();
    // Two identical checkouts of the same fixture repository.
    let shared = fixture_repository(&root);
    let keys_of = |directory: &Path| -> (Vec<String>, String) {
        let connection = open_ci_root(directory, "x");
        let snapshot =
            RepositorySnapshot::capture(&shared, &directory.join("scratch"), None).unwrap();
        let bundle = directory.join("bundle");
        write_file(&bundle, ".oviraptor-scan-id", "ci-bundle");
        write_json(
            &bundle,
            "findings.sarif",
            &sarif_document(
                vec![
                    sarif_result("py/sql-injection", "error", "sql", "python/app.py", 2),
                    sarif_result(
                        "js/open-redirect",
                        "warning",
                        "redirect",
                        "src/server.js",
                        2,
                    ),
                ],
                "semgrep",
            ),
        );
        import_directory(&connection, directory, &bundle);
        (
            current_logical_keys(&connection, "ci-bundle"),
            snapshot.tree_hash.clone(),
        )
    };
    let (first, first_tree) = keys_of(&first_root);
    let (second, second_tree) = keys_of(&second_root);
    assert_eq!(first.len(), 2, "{first:?}");
    assert_eq!(first_tree, second_tree, "两次运行的输入必须是同一棵树");
    assert_eq!(
        first, second,
        "同样的 commit 和规则必须给出同样的 logical key"
    );
    // A second import of the same bundle into the same database adds nothing.
    let db_path = initialize_db(&first_root);
    let connection = open_connection(&db_path);
    let before = table_count(&connection, "import_record_revisions");
    import_directory(&connection, &first_root, &first_root.join("bundle"));
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        before,
        "重跑不能产生第二条语义记录"
    );
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(second_root);
}

#[test]
fn ci_002_passed_and_warning_exit_zero_while_a_blocking_finding_exits_two() {
    let root = sandbox("ci-002");
    let connection = open_ci_root(&root, "x");
    let freeze = freeze_at(&root, None, CiScope::Full);
    let policy = GatePolicy::default();

    let passed = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        policy,
    )
    .unwrap();
    assert_eq!(passed.status, GateStatus::Passed);
    assert_eq!(passed.exit_code(), 0);

    confirm(
        &connection,
        "ci-root",
        "ci-reviewer",
        "medium-one",
        "medium",
        ReviewVerdict::Confirmed,
    );
    let warned = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(warned.status, GateStatus::Warning);
    assert_eq!(warned.exit_code(), 0, "warning 不能挡住发布");

    confirm(
        &connection,
        "ci-root",
        "ci-reviewer",
        "high-one",
        "high",
        ReviewVerdict::Confirmed,
    );
    let blocked = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(blocked.status, GateStatus::Blocked);
    assert_eq!(blocked.exit_code(), 2);
    assert_eq!(blocked.blocking.len(), 1);
    assert_eq!(blocked.as_json()["exitCode"], json!(2));

    // Raising the threshold to one makes the same answer a warning, not a block.
    let tolerated = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy {
            allowed_blocking: 1,
            ..GatePolicy::default()
        },
    )
    .unwrap();
    assert_eq!(tolerated.exit_code(), 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ci_003_incomplete_coverage_exits_three_and_infrastructure_failure_exits_four() {
    let root = sandbox("ci-003");
    let connection = open_ci_root(&root, "x");
    let freeze = freeze_at(&root, None, CiScope::Full);

    let covered = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(covered.exit_code(), 0);

    let partial = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &["analyzer_missing".to_string()],
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(partial.status, GateStatus::CoverageIncomplete);
    assert_eq!(partial.exit_code(), 3);
    assert!(partial.reasons.join(" ").contains("analyzer_missing"));
    assert!(!partial.status.is_green(), "覆盖不完整不能算绿色");

    let broken = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        Some("镜像拉取失败"),
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(broken.status, GateStatus::InfraFailed);
    assert_eq!(broken.exit_code(), 4);

    // A confirmed blocking finding still outranks the gap: the build must fail on it.
    confirm(
        &connection,
        "ci-root",
        "ci-reviewer",
        "high-gap",
        "high",
        ReviewVerdict::Confirmed,
    );
    let both = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &["analyzer_missing".to_string()],
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(both.status, GateStatus::Blocked);
    assert_eq!(both.exit_code(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ci_004_candidates_history_and_rejected_findings_do_not_block() {
    let root = sandbox("ci-004");
    let connection = open_ci_root(&root, "x");
    let freeze = freeze_at(&root, None, CiScope::Full);

    // A candidate the reviewer never answered.
    make_agent_run(&connection, "ci-candidate-author", "ci-scan", "coordinator");
    let mut broker = crate::native_pipeline::tools::SourceBroker::new(
        capture(&root, None),
        "ci-scan",
        1,
        "ci-root",
        "ci-candidate-author",
        1,
    );
    broker
        .call(
            &connection,
            "evidence.submit_candidate",
            &json!({"title": "疑似高危", "severity": "critical", "rationale": "拼接了请求参数", "path": "python/app.py", "line": 2}),
        )
        .unwrap();
    let report = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(report.exit_code(), 0, "候选不能阻断：{:?}", report.reasons);

    // A rejected critical stays a rejection.
    confirm(
        &connection,
        "ci-root",
        "ci-reviewer",
        "rejected-critical",
        "critical",
        ReviewVerdict::Rejected,
    );
    let report = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(report.exit_code(), 0, "已拒绝不能阻断");

    // A confirmed critical belonging to another investigation is history, not this
    // gate's input.
    make_agent_run(&connection, "other-root", "other-scan", "coordinator");
    make_reviewer_run(&connection, "other-reviewer", "other-root", "other-scan");
    confirm(
        &connection,
        "other-root",
        "other-reviewer",
        "old-critical",
        "critical",
        ReviewVerdict::Confirmed,
    );
    let report = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(report.exit_code(), 0, "别处的历史结论不能挡住这次发布");
    assert_eq!(
        report.decisions.len(),
        1,
        "gate 只读本 root 的决定：{:?}",
        report
            .decisions
            .iter()
            .map(|row| row.candidate_id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(report.decisions[0].candidate_id, "rejected-critical");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ci_005_an_unusable_diff_base_never_scans_an_empty_set_and_reports_pass() {
    let root = sandbox("ci-005");
    let connection = open_ci_root(&root, "x");
    let snapshot = capture(&root, Some("main"));
    assert_eq!(
        snapshot.diff_base,
        crate::native_pipeline::snapshot::DiffBaseState::NoRepository,
        "fixture 里没有 git 仓库，base 必须判为不可用"
    );
    let freeze = CiFreeze::of(&snapshot, &outcomes(), CiScope::Diff);
    assert_eq!(
        freeze.scope,
        CiScope::Full,
        "不可用的 base 不能留下 diff 范围"
    );

    let inconclusive = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(inconclusive.status, GateStatus::Inconclusive);
    assert_eq!(inconclusive.exit_code(), 3);
    assert!(!inconclusive.status.is_green());

    // The explicit whole-tree policy may continue, and says what it did.
    let forced = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy {
            full_scan_on_bad_base: true,
            ..GatePolicy::default()
        },
    )
    .unwrap();
    assert_ne!(forced.status, GateStatus::Inconclusive);
    assert!(
        forced
            .gaps
            .iter()
            .any(|gap| gap.starts_with("diff_base_unusable_fallback_full")),
        "{:?}",
        forced.gaps
    );

    // An empty tree is never a pass, whatever the policy says.
    let empty_root = sandbox("ci-005-empty");
    fs::create_dir_all(empty_root.join("repo")).unwrap();
    let empty =
        RepositorySnapshot::capture(&empty_root.join("repo"), &empty_root.join("scratch"), None)
            .unwrap();
    assert!(empty.is_empty());
    let empty_freeze = CiFreeze::of(&empty, &outcomes(), CiScope::Full);
    let connection = open_ci_root(&empty_root, "y");
    let report = ci::evaluate(
        &connection,
        "ci-root",
        &empty_freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(report.status, GateStatus::Inconclusive);
    assert_eq!(report.exit_code(), 3);
    assert!(report.reasons.join(" ").contains("空集"));
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(empty_root);
}

#[test]
fn ci_006_standalone_sarif_imports_without_the_removed_json_writer() {
    // The product JSON/bundle contract is tested through the actual source
    // export IPC helper in commands::tests::source_findings_* and bundle_import_*.
    // This standalone gate view is SARIF only, not a second JSON export API.
    let root = sandbox("ci-006");
    let connection = open_ci_root(&root, "x");
    let freeze = freeze_at(&root, None, CiScope::Full);
    confirm_at(
        &connection,
        "ci-root",
        "ci-reviewer",
        "sql-injection",
        "high",
        "python/app.py:2",
        ReviewVerdict::Confirmed,
    );
    let report = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(report.exit_code(), 2);
    let exported = report.findings();
    assert_eq!(exported.len(), 1);
    let bundle = root.join("bundle");
    write_file(&bundle, ".oviraptor-scan-id", "ci-bundle");
    let bytes = serde_json::to_vec_pretty(&report.as_sarif()).unwrap();
    fs::write(bundle.join("findings.sarif"), &bytes).unwrap();
    let active_runs = table_count(&connection, "agent_runs");
    import_directory(&connection, &root, &bundle);
    let candidates = current_candidates(&connection, "ci-bundle");
    assert_eq!(candidates.len(), exported.len());
    let row = &candidates[0];
    assert_eq!(
        payload_text(row, "rule_id"),
        exported[0]["id"].as_str().unwrap()
    );
    assert_eq!(field_text(row, "severity"), "high");
    assert_eq!(
        row["payload"]["fingerprints"]["oviraptorFindingId"],
        exported[0]["id"]
    );
    assert_eq!(row["claim"]["executionEligible"], false);
    assert_eq!(row["claim"]["readOnly"], true);
    assert_eq!(table_count(&connection, "agent_runs"), active_runs);
    assert_eq!(fs::read(bundle.join("findings.sarif")).unwrap(), bytes);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ci_007_the_freeze_carries_head_base_tree_rules_and_analyzers_and_cannot_be_edited() {
    let root = sandbox("ci-007");
    let connection = open_ci_root(&root, "x");
    let snapshot = capture(&root, Some("main"));
    let freeze = CiFreeze::of(&snapshot, &outcomes(), CiScope::Diff);
    let json = freeze.as_json();
    assert_eq!(json["headSha"], json!(snapshot.commit_sha));
    assert_eq!(json["baseSha"], json!(snapshot.base_sha));
    assert_eq!(json["treeHash"], json!(snapshot.tree_hash));
    assert_eq!(json["rulePackDigest"], json!("rules-digest-1"));
    assert_eq!(json["diffBase"], json!("no_repository"));
    let engines = json["analyzers"].as_array().unwrap();
    assert_eq!(
        engines.len(),
        2,
        "两个分析器的版本都要写进冻结：{engines:?}"
    );
    assert_eq!(engines[0]["engine"], json!("codeql"));
    assert_eq!(engines[0]["version"], json!("2.16.0"));
    assert_eq!(engines[1]["engine"], json!("semgrep"));
    assert_eq!(json["scope"], json!("full"), "不可用的 base 不能留下 diff");

    // Deriving it again is the same value; a different snapshot is not.
    assert_eq!(freeze, freeze_at(&root, Some("main"), CiScope::Diff));
    let changed = {
        let mut rows = outcomes();
        rows[0].version = "1.80.0".to_string();
        CiFreeze::of(&snapshot, &rows, CiScope::Diff)
    };
    assert_ne!(
        freeze.stable_text(),
        changed.stable_text(),
        "分析器版本变化必须改变冻结"
    );

    // The gate reads the freeze, never a mutable copy of it.
    let report = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(report.freeze, freeze);
    assert_eq!(
        report.as_json()["freeze"]["treeHash"],
        json!(snapshot.tree_hash)
    );

    // And the pipeline plan beside it is write-once per attempt.
    let plan =
        NativeSourcePlan::for_snapshot("ci-scan", 1, "cicd", &snapshot, &["semgrep"], vec![]);
    plan.store(&connection).unwrap();
    assert!(NativeSourcePlan::load(&connection, "ci-scan", 1)
        .unwrap()
        .is_some());
    let mut moved = plan.clone();
    moved.analyzers.push("codeql".to_string());
    assert!(moved.store(&connection).is_err(), "冻结过的计划不能改写");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ci_008_the_gate_reads_only_stored_reviewer_decisions() {
    let root = sandbox("ci-008");
    let connection = open_ci_root(&root, "x");
    let freeze = freeze_at(&root, None, CiScope::Full);
    // A finding written straight into the projection must not move the gate.
    connection
        .execute(
            "INSERT INTO sentinel_findings(scan_id,stage,kind,title,severity)
             VALUES('ci-scan','ci','injected-by-test','未经评审的投影','critical')",
            [],
        )
        .unwrap();
    let report = ci::evaluate(
        &connection,
        "ci-root",
        &freeze,
        &complete_coverage(),
        None,
        GatePolicy::default(),
    )
    .unwrap();
    assert_eq!(report.exit_code(), 0, "未经评审的投影结论不能阻断发布");
    let _ = fs::remove_dir_all(root);
}
