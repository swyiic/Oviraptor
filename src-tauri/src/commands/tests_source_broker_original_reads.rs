#[test]
fn source_broker_scope_real_entry_never_reads_unselected_source() {
    let p = source_broker_original_probe(
        "diff",
        true,
        vec![vec![
            ("repo.inventory", json!({})),
            ("repo.read_slice", json!({"path":"app.py"})),
            ("repo.read_slice", json!({"path":"untouched.py"})),
            ("repo.search", json!({"query":"not selected"})),
            (
                "evidence.submit_candidate",
                json!({"title":"unselected","rationale":"must not be accessible","path":"untouched.py"}),
            ),
        ]],
        None,
        |_, _| {},
    );
    assert!(p.result.is_ok(), "{:?}", p.result);
    assert_eq!(p.calls, 7);
    let inventory = p.output(1, 0);
    assert_eq!(inventory["fileCount"], 1, "{inventory}");
    assert!(!inventory.to_string().contains("untouched.py"));
    assert_eq!(p.output(1, 1)["lines"], json!(["print('changed')"]));
    assert!(p.output(1, 2).get("error").is_some());
    assert_eq!(p.output(1, 3)["matches"], json!([]));
    assert!(p.output(1, 4).get("error").is_some());
    assert_eq!(
        p.db.query_row("SELECT count(*) FROM agent_evidence_nodes", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    p.cleanup();
}

#[test]
fn source_broker_scope_full_and_auto_keep_authorized_files_available() {
    for (mode, base, count) in [("full", true, 2), ("auto", false, 2), ("auto", true, 1)] {
        let p = source_broker_original_probe(
            mode,
            base,
            vec![vec![
                ("repo.inventory", json!({})),
                ("repo.read_slice", json!({"path":"untouched.py"})),
            ]],
            None,
            |_, _| {},
        );
        assert!(p.result.is_ok(), "{:?}", p.result);
        assert_eq!(p.calls, 7);
        // The real SDK has exactly the role's frozen source capabilities, not
        // the old 17-item mixture of Source and Web facade tools.
        let inventory = p.output(1, 0);
        assert_eq!(inventory["fileCount"], count, "{inventory}");
        assert_eq!(
            inventory["analysisManifestDigest"].as_str().unwrap().len(),
            64
        );
        let read = p.output(1, 1);
        assert_eq!(read.get("error").is_none(), count == 2, "{read}");
        p.cleanup();
    }
}

#[test]
fn source_broker_scope_exact_paths_and_selected_dependency_manifests() {
    let paths = [
        ".env.example",
        " space 中文\n.py",
        "./app.py",
        " app.py",
        "../app.py",
        "app.py/",
        "app.py\\",
    ];
    let mut tools: Vec<_> = paths
        .iter()
        .map(|path| ("repo.read_slice", json!({"path":path})))
        .collect();
    tools.extend([
        ("dependency.get_record", json!({"manifest":"package.json"})),
        ("dependency.get_record", json!({"manifest":"Cargo.toml"})),
        ("repo.inventory", json!({})),
    ]);
    let p = source_broker_original_probe("diff", true, vec![tools], None, |_, record| {
        let repo = Path::new(&record.source_path);
        fs::write(repo.join("Cargo.toml"), "[package]\nname = 'unselected'\n").unwrap();
        analysis_view_git(repo, &["add", "."]);
        analysis_view_git(repo, &["commit", "-qm", "base with unselected manifest"]);
        record.diff_base = analysis_view_git(repo, &["rev-parse", "HEAD"]);
        fs::write(repo.join(".env.example"), "PUBLIC_SAMPLE=value\n").unwrap();
        fs::write(repo.join(" space 中文\n.py"), "print('exact path')\n").unwrap();
        fs::write(
            repo.join("package.json"),
            "{\"dependencies\":{\"fixture\":\"1\"}}",
        )
        .unwrap();
    });
    assert!(p.result.is_ok(), "{:?}", p.result);
    assert_eq!(p.calls, 7);
    for (i, path) in paths.iter().enumerate() {
        let answer = p.output(1, i as i64);
        if i < 2 {
            assert_eq!(answer["path"], *path, "{answer}");
            assert!(answer["contentHash"].is_string());
        } else {
            assert!(answer.get("error").is_some(), "{path}: {answer}");
        }
    }
    assert_eq!(p.output(1, 7)["dependencies"][0]["name"], "fixture");
    assert!(p.output(1, 8).get("error").is_some());
    let inventory = p.output(1, 9);
    assert_eq!(inventory["manifests"], json!(["package.json"]));
    assert_eq!(inventory["languages"], json!(["javascript"]));
    assert!(!inventory.to_string().contains("Cargo.toml"));
    p.cleanup();
}
