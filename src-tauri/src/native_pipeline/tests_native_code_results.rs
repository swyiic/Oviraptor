//! Analyzer recovery and frozen source-plan regressions.

use super::tests_native_code::{plan_for, spec};
use super::*;
use crate::native_pipeline::analyzer::{self, AnalyzerStatus};
use crate::native_pipeline::snapshot::DiffBaseState;
use crate::native_pipeline::tools::SourceBroker;
use crate::native_pipeline::NativeSourcePlan;
use serde_json::json;
use std::fs;
use std::time::Duration;

#[test]
fn code_011_crash_recovery_reads_the_last_analyzer_run_instead_of_paying_again() {
    let root = sandbox("code-011");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let target = spec("/nonexistent/semgrep-here", &root, process_limits(30, 4096));
    let steps = analyzer::planned_args(&target).unwrap();
    let digest = analyzer::rule_pack_digest_of(&target.rule_pack);
    let key = analyzer::invocation_key(&target, &steps, &digest);
    let sarif = analyzer::sarif_path(&target);
    write_json(
        &target.scratch_dir,
        "semgrep.sarif",
        &sarif_document(vec![], "semgrep"),
    );
    connection
        .execute(
            "INSERT INTO analyzer_runs(invocation_key,scan_id,attempt_number,engine,status,version,
                rule_pack_digest,sarif_path,network_disabled,repository_read_only,evidence_json)
             VALUES(?1,?2,?3,'semgrep','ran','1.2.3',?4,?5,1,1,?6)",
            rusqlite::params![
                key,
                target.scan_id,
                target.attempt_number,
                digest,
                sarif.to_string_lossy(),
                json!({"sarifSha256": crate::artifact_import::canonical::sha256_hex(&fs::read(&sarif).unwrap())}).to_string()
            ],
        )
        .unwrap();

    let started = std::time::Instant::now();
    let outcome = analyzer::run(&connection, &target, &*never_cancelled()).unwrap();
    assert!(outcome.reused, "崩溃恢复不得重跑分析器");
    assert_eq!(outcome.status, AnalyzerStatus::Ran { exit: Some(0) });
    assert_eq!(outcome.version, "1.2.3");
    assert_eq!(outcome.invocation_key, key);
    assert!(
        outcome.network_disabled && outcome.repository_read_only,
        "隔离声明要从记录里读回来：{:?}",
        outcome.evidence_json()
    );
    assert!(outcome.duration < Duration::from_millis(500));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        table_count(&connection, "analyzer_runs"),
        1,
        "复用不得再写一行执行记录"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_013_analyzer_results_come_back_through_the_importer_not_stdout() {
    let root = sandbox("code-013");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    make_agent_run(&connection, "run-code-013", "code-scan", "coordinator");
    let scratch = scratch_dir(&root);
    write_file(&scratch, ".oviraptor-scan-id", "code-scan");
    write_json(
        &scratch,
        "findings.sarif",
        &sarif_document(
            vec![sarif_result(
                "py/sql-injection",
                "error",
                "untrusted input in SQL",
                "python/app.py",
                2,
            )],
            "semgrep",
        ),
    );
    import_directory(&connection, &root, &scratch);
    connection.execute(
        "INSERT INTO analyzer_runs(invocation_key,scan_id,attempt_number,engine,status,gap_code,version,rule_pack_digest)
         VALUES('k1','code-scan',1,'semgrep','ran','','1.79.0','digest-1')",
        [],
    )
    .unwrap();

    let mut broker = SourceBroker::new(
        capture(&root, None),
        "code-scan",
        1,
        "run-code-013",
        "run-code-013",
        1,
    );
    let listed = broker
        .call(&connection, "analyzer.list_results", &json!({}))
        .unwrap();
    assert_eq!(listed["runs"].as_array().unwrap().len(), 1);
    assert_eq!(listed["runs"][0]["rulePackDigest"], json!("digest-1"));
    let candidates = listed["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "{listed:?}");
    assert_eq!(candidates[0]["ruleId"], json!("py/sql-injection"));
    assert_eq!(candidates[0]["reviewState"], json!("candidate"));
    let key = candidates[0]["key"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    let one = broker
        .call(&connection, "analyzer.get_result", &json!({"key": key}))
        .unwrap();
    assert_eq!(one["reviewState"], json!("candidate"));
    assert_eq!(
        one["envelope"]
            .pointer("/payload/rule_id")
            .and_then(json_text),
        Some("py/sql-injection")
    );
    let missing = broker
        .call(&connection, "analyzer.get_result", &json!({"key": "nope"}))
        .unwrap_err();
    assert_eq!(missing.code, "result_not_found");
    // Dependency declarations come from the manifest, not from a downloaded SBOM.
    let dependency = broker
        .call(
            &connection,
            "dependency.get_record",
            &json!({"manifest": "python/requirements.txt"}),
        )
        .unwrap();
    let names: Vec<&str> = dependency["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert!(names.contains(&"requests"), "{names:?}");
    assert!(names.contains(&"flask"), "{names:?}");
    // And the inventory only ever speaks the frozen paths.
    let inventory = broker
        .call(&connection, "repo.inventory", &json!({"prefix": "python"}))
        .unwrap();
    assert!(inventory["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["path"]
            .as_str()
            .unwrap_or_default()
            .starts_with("python/")));
    let _ = fs::remove_dir_all(root);
}

fn json_text(value: &serde_json::Value) -> Option<&str> {
    value.as_str()
}

#[test]
fn code_014_the_sandbox_flags_describe_the_run_that_actually_happened() {
    let root = sandbox("code-014");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    // A host binary is not a sandbox: the outcome must say so rather than claim the
    // container guarantees it never had.
    let host = spec("/usr/bin/false", &root, process_limits(30, 4096));
    let outcome = analyzer::run(&connection, &host, &*never_cancelled()).unwrap();
    assert!(!outcome.repository_read_only);
    assert!(!outcome.network_disabled);

    // A resumed run reads the flags back from the row that recorded them.
    let mut pinned = host.clone();
    pinned.image = Some(
        "registry.example/semgrep@sha256:1111111111111111111111111111111111111111111111111111111111111111"
            .to_string(),
    );
    pinned.attempt_number = 7;
    let steps = analyzer::planned_args(&pinned).unwrap();
    let digest = analyzer::rule_pack_digest_of(&pinned.rule_pack);
    let key = analyzer::invocation_key(&pinned, &steps, &digest);
    write_json(
        &pinned.scratch_dir,
        "semgrep.sarif",
        &sarif_document(vec![], "semgrep"),
    );
    connection
        .execute(
            "INSERT INTO analyzer_runs(invocation_key,scan_id,attempt_number,engine,status,version,
                rule_pack_digest,network_disabled,repository_read_only,sarif_path,evidence_json)
             VALUES(?1,?2,7,'semgrep','ran','1.79.0',?3,0,0,?4,?5)",
            rusqlite::params![
                key,
                pinned.scan_id,
                digest,
                analyzer::sarif_path(&pinned).to_string_lossy(),
                json!({"sarifSha256": crate::artifact_import::canonical::sha256_hex(&fs::read(analyzer::sarif_path(&pinned)).unwrap())}).to_string()
            ],
        )
        .unwrap();
    let reused = analyzer::run(&connection, &pinned, &*never_cancelled()).unwrap();
    assert!(reused.reused);
    assert!(
        !reused.network_disabled && !reused.repository_read_only,
        "复用不得把当时没有的隔离声明出来：{:?}",
        reused.evidence_json()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_012_the_plan_freezes_for_an_attempt_and_refuses_a_rewrite() {
    let root = sandbox("code-012");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let snapshot = capture(&root, None);
    let plan = plan_for(&snapshot, "code-plan", vec![]);
    plan.store(&connection).unwrap();
    let loaded = NativeSourcePlan::load(&connection, "code-plan", 1)
        .unwrap()
        .expect("冻结计划要能读回来");
    assert_eq!(loaded, plan);
    assert_eq!(loaded.diff_base_state, DiffBaseState::Missing.as_str());

    let mut moved = plan.clone();
    moved.analyzers = vec!["codeql".to_string()];
    let error = moved.store(&connection).unwrap_err();
    assert!(error.contains("不能改写"), "{error}");
    // The same plan again is a no-op, not a second row.
    plan.store(&connection).unwrap();
    assert_eq!(table_count(&connection, "native_scan_plans"), 1);
    let _ = fs::remove_dir_all(root);
}
