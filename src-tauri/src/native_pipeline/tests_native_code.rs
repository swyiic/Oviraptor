//! CODE-001 … CODE-011 — the Native code pipeline: a frozen snapshot, bounded analyzers,
//! the canonical importer as the only path into findings, and a broker that refuses
//! everything §10.2 item 6 takes away.

use super::*;
use crate::native_pipeline::analyzer::{self, AnalyzerEngine, AnalyzerSpec, AnalyzerStatus};
use crate::native_pipeline::process;
use crate::native_pipeline::snapshot::RepositorySnapshot;
use crate::native_pipeline::NativeSourcePlan;
use std::fs;

/// Every plan field a code scan needs, from a snapshot and an analyzer list.
pub(super) fn plan_for(
    snapshot: &RepositorySnapshot,
    scan_id: &str,
    gaps: Vec<String>,
) -> NativeSourcePlan {
    NativeSourcePlan::for_snapshot(scan_id, 1, "code", snapshot, &["semgrep"], gaps)
}

pub(super) fn spec(
    program: &str,
    root: &std::path::Path,
    limits: process::ProcessLimits,
) -> AnalyzerSpec {
    let rule_pack = root.join("rules");
    fs::create_dir_all(&rule_pack).unwrap();
    fs::write(rule_pack.join("no-sql.yaml"), "rules: []\n").unwrap();
    AnalyzerSpec {
        engine: AnalyzerEngine::Semgrep,
        program: std::path::PathBuf::from(program),
        image: None,
        rule_pack,
        languages: vec!["python".to_string()],
        repository: root.join("repo"),
        scratch_dir: scratch_dir(root),
        scan_id: "code-scan".to_string(),
        attempt_number: 1,
        limits,
    }
}

#[test]
fn code_001_a_code_scan_completes_without_any_strix_reference() {
    let root = sandbox("code-001");
    let snapshot = capture(&root, None);
    let plan = plan_for(&snapshot, "code-scan", vec![]);
    assert_eq!(plan.backend, "native");
    assert_eq!(plan.scan_type, "code");
    assert_eq!(plan.tree_hash, snapshot.tree_hash);
    assert!(!plan.tools.is_empty());

    // The whole module is the pipeline this scan runs on, and none of it may reach for
    // a Strix executable, adapter or table.
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/native_pipeline")
        .to_path_buf();
    let mut files = vec![
        "analyzer.rs",
        "ci.rs",
        "container.rs",
        "greybox.rs",
        "mod.rs",
        "process.rs",
        "snapshot.rs",
        "tools.rs",
        "tools/analyzer_results.rs",
        "tools/assignment.rs",
        "tools/dependencies.rs",
        "tools/policy.rs",
        "tools/repository.rs",
    ];
    files.retain(|name| source.join(name).is_file());
    assert_eq!(files.len(), 13, "native_pipeline 的文件清单变了，检查守卫");
    for name in files {
        let text = fs::read_to_string(source.join(name))
            .unwrap()
            .to_ascii_lowercase();
        for banned in ["strix_", "launch_strix", "strixcli", "strixcli"] {
            assert!(
                !text.contains(banned),
                "{name} 里出现了 {banned}：源码流水线不能引用 Strix 执行端"
            );
        }
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_002_the_snapshot_is_reproducible_and_the_source_stays_frozen() {
    let root = sandbox("code-002");
    let repo = fixture_repository(&root);
    let scratch = scratch_dir(&root);
    let first = RepositorySnapshot::capture(&repo, &scratch, None).unwrap();
    let second = RepositorySnapshot::capture(&repo, &scratch, None).unwrap();
    assert_eq!(
        first.tree_hash, second.tree_hash,
        "同一棵树必须得到同一个 hash"
    );
    assert_eq!(first.files, second.files);
    first.verify_unchanged().unwrap();

    write_file(&repo, "src/server.js", "// edited during the scan\n");
    let error = first.verify_unchanged().unwrap_err();
    assert!(error.contains("src/server.js"), "{error}");
    let third = RepositorySnapshot::capture(&repo, &scratch, None).unwrap();
    assert_ne!(third.tree_hash, first.tree_hash, "内容变了 hash 必须变");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_003_exclusions_symlinks_and_every_manifest_of_a_monorepo() {
    let root = sandbox("code-003");
    let snapshot = capture(&root, None);
    let paths = snapshot
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    for banned in ["node_modules", "target", "dist", "vendor", ".venv", ".git"] {
        assert!(
            !paths.iter().any(|path| path.starts_with(banned)),
            "{banned} 的内容进入了快照：{paths:?}"
        );
        assert!(
            snapshot
                .gaps
                .iter()
                .any(|gap| gap == &format!("excluded_directory:{banned}")),
            "{banned} 被跳过了但没有留下 gap：{:?}",
            snapshot.gaps
        );
    }
    // A link out of the repository is refused; a link inside it is still not followed
    // twice over.
    assert!(snapshot
        .gaps
        .iter()
        .any(|gap| gap == "symlink_outside_repository:escape"));
    assert!(
        !paths.iter().any(|path| path.starts_with("escape/")),
        "越出仓库的符号链接被读取了：{paths:?}"
    );
    // A linked directory is reported, not walked: walking it would record the same file
    // a second time under the target's own path.
    assert!(snapshot
        .gaps
        .iter()
        .any(|gap| gap == "symlink_directory_not_traversed:link-to-src"));
    assert!(
        !paths.iter().any(|path| path.starts_with("link-to-src/")),
        "链接目录被走了一遍，同一个文件会出现两次：{paths:?}"
    );
    // A linked file keeps its own identity and is read through the link.
    assert!(paths.iter().any(|path| path == "link-to-readme"));
    assert_eq!(
        snapshot.content_hash_of("link-to-readme"),
        snapshot.content_hash_of("README.md")
    );
    let mut sorted = paths.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), paths.len(), "快照里出现了重复路径：{paths:?}");
    assert!(
        !fs::read_to_string(root.join("outside/secret.env"))
            .unwrap()
            .is_empty(),
        "fixture 本身要能被读到，才说明是快照挡住了它"
    );

    let languages = snapshot.languages();
    assert!(
        languages.contains(&"javascript".to_string()),
        "{languages:?}"
    );
    assert!(languages.contains(&"python".to_string()), "{languages:?}");
    assert!(snapshot.manifests.contains(&"package.json".to_string()));
    assert!(snapshot
        .manifests
        .contains(&"python/requirements.txt".to_string()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_004_analyzer_sarif_reaches_findings_only_through_the_canonical_importer() {
    let root = sandbox("code-004");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
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
                3,
            )],
            "semgrep",
        ),
    );
    let before = table_count(&connection, "sentinel_findings");
    import_directory(&connection, &root, &scratch);
    let candidates = current_candidates(&connection, "code-scan");
    assert!(
        candidates
            .iter()
            .any(|row| payload_text(row, "rule_id") == "py/sql-injection"),
        "SARIF 结果没有进入 canonical 记录：{candidates:?}"
    );
    assert_eq!(
        table_count(&connection, "sentinel_findings"),
        before,
        "分析器输出不得直接写 Finding"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_005_a_missing_analyzer_is_a_gap_and_the_limits_are_auditable() {
    let root = sandbox("code-005");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);

    // Absent binary: gap, never a clean result.
    let missing = analyzer::run(
        &connection,
        &spec("/nonexistent/semgrep-here", &root, process_limits(30, 4096)),
        &*never_cancelled(),
    )
    .unwrap();
    assert!(missing.status.is_gap());
    assert_eq!(missing.status.gap_code(), "analyzer_missing");
    // A host binary is not a sandbox: without a pinned image the run has neither the
    // read-only mount nor the network cut, and the outcome says so.
    assert!(!missing.network_disabled);
    assert!(!missing.repository_read_only);
    assert!(!missing.produced_results());
    assert_eq!(missing.gap_reason(), "analyzer_missing");

    // An analyzer that fails: also a gap, and the stderr tail is kept for the audit.
    let broken = spec("/usr/bin/false", &root, process_limits(30, 4096));
    let failed = analyzer::run(&connection, &broken, &*never_cancelled()).unwrap();
    assert!(failed.status.is_gap());
    assert_eq!(failed.status.gap_code(), "analyzer_failed");
    assert!(!failed.produced_results());

    // Cancelled before the first step. A different attempt, so it is its own audit row.
    let mut cancelled_spec = broken.clone();
    cancelled_spec.attempt_number = 2;
    let cancelled = analyzer::run(&connection, &cancelled_spec, &|| true).unwrap();
    assert_eq!(cancelled.status, AnalyzerStatus::Cancelled);
    assert_eq!(cancelled.status.gap_code(), "analyzer_cancelled");

    // Every one of those left a row a CI log can read.
    let stored: Vec<(String, String, String)> = {
        let mut statement = connection
            .prepare("SELECT engine,status,gap_code FROM analyzer_runs ORDER BY invocation_key")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .filter_map(|row| row.ok())
            .collect()
    };
    assert_eq!(stored.len(), 3, "三次执行都要留下记录：{stored:?}");
    for (_, status, gap) in &stored {
        assert_ne!(status, "ran", "{gap} 不能记成 ran");
        assert!(!gap.is_empty(), "{status} 必须带 gap code");
    }

    // The runner's own bounds: a deadline, a cancel and an output cap.
    let timed = process::run(
        std::path::Path::new("/bin/sleep"),
        &["30".to_string()],
        None,
        &process_limits(1, 4096),
        &|| false,
    )
    .unwrap();
    assert!(timed.timed_out, "超时没有被记录");
    assert!(!timed.succeeded());
    let stopped = process::run(
        std::path::Path::new("/bin/sleep"),
        &["30".to_string()],
        None,
        &process_limits(30, 4096),
        &|| true,
    )
    .unwrap();
    assert!(stopped.cancelled, "取消没有被记录");
    let loud = root.join("loud.txt");
    fs::write(&loud, "x".repeat(200_000)).unwrap();
    let capped = process::run(
        std::path::Path::new("/bin/cat"),
        &[loud.to_string_lossy().to_string()],
        None,
        &process_limits(30, 1024),
        &|| false,
    )
    .unwrap();
    assert!(capped.truncated, "stdout 上限没有生效");
    assert!(capped.stdout.len() <= 1024);
    assert!(capped.succeeded());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_006_pinned_image_read_only_mount_no_network_and_a_digest_for_the_rules() {
    let root = sandbox("code-006");
    let base = spec("/usr/bin/docker", &root, process_limits(30, 4096));

    // An image without a digest is not a pinned image.
    let mut floating = base.clone();
    floating.image = Some("registry.example/semgrep:latest".to_string());
    let error = analyzer::planned_args(&floating).unwrap_err();
    assert!(error.contains("digest"), "{error}");

    let mut pinned = base.clone();
    pinned.image = Some(
        "registry.example/semgrep@sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
    );
    let steps = analyzer::planned_args(&pinned).unwrap();
    let joined = steps
        .iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains("--network none"), "{joined}");
    assert!(joined.contains("--read-only"), "{joined}");
    assert!(joined.contains(":ro"), "仓库必须只读挂载：{joined}");
    assert!(joined.contains("/rules:ro"), "规则包必须只读挂载：{joined}");
    assert!(
        !joined.contains("&&") && !joined.contains(';'),
        "参数里不允许 shell 控制字符：{joined}"
    );
    // A rule path smuggled as a shell payload is refused outright.
    let mut hostile = pinned.clone();
    hostile.rule_pack = std::path::PathBuf::from("/rules; rm -rf /");
    assert!(analyzer::planned_args(&hostile).is_err());

    let digest_before = analyzer::rule_pack_digest_of(&base.rule_pack);
    assert_eq!(digest_before.len(), 64);
    fs::write(base.rule_pack.join("extra.yaml"), "rules: [x]\n").unwrap();
    let digest_after = analyzer::rule_pack_digest_of(&base.rule_pack);
    assert_ne!(digest_before, digest_after, "规则改动必须改变 digest");
    let _ = fs::remove_dir_all(root);
}
