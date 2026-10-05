// Old trace databases are unrecognized inputs, not an alternate Native history.
fn retired_trace_databases(root: &Path) -> Vec<Vec<u8>> {
    let mut databases = vec![b"not a database".to_vec()];
    for (index, schema) in [
        "CREATE TABLE agent_sessions(session_id TEXT PRIMARY KEY);\
         CREATE TABLE agent_messages(id INTEGER PRIMARY KEY,session_id TEXT,message_data TEXT,created_at TEXT);\
         INSERT INTO agent_sessions VALUES('old-session');\
         INSERT INTO agent_messages VALUES(1,'old-session','{\"type\":\"message\",\"content\":\"old-trace\"}','t1');",
        "CREATE TABLE unknown_schema(payload TEXT);",
        "CREATE TABLE agent_sessions(session_id TEXT); CREATE TABLE agent_messages(id INTEGER);",
        "CREATE TABLE agent_sessions(session_id TEXT PRIMARY KEY);\
         CREATE TABLE agent_messages(id INTEGER PRIMARY KEY,session_id TEXT,message_data TEXT,created_at TEXT);\
         INSERT INTO agent_sessions VALUES('old-session');\
         INSERT INTO agent_messages VALUES(1,'old-session',NULL,'t1');",
    ].iter().enumerate() {
        let path = root.join(format!("fixture-{index}.db"));
        build_sqlite(&path, schema);
        databases.push(fs::read(path).unwrap());
    }
    databases
}

#[test]
fn sqlite_retirement_direct_dispatch_never_opens_a_snapshot() {
    let root = sandbox("sqlite-retired-dispatch");
    let scratch = root.join("scratch");
    let manifest = manifest::Manifest { bundle_id: "retired-db".into(), canonical: String::new(), files: Vec::new() };
    for bytes in retired_trace_databases(&root) {
        for name in ["agents.db", ".state/agents.db", "nested/.state/agents.db"] {
            for limit in [0, 20] {
                let payloads = vec![(name.into(), bytes.clone())];
                let limits = Limits { records: limit, ..Limits::default() };
                let context = adapters::ParseContext {
                    bundle_id: &manifest.bundle_id, manifest: &manifest,
                    payloads: &payloads, limits: &limits,
                };
                let (rows, notes) = adapters::parse_bundle(&context);
                assert!(rows.is_empty(), "retired database produced records: {name}");
                assert_eq!(notes.len(), 1);
                assert_eq!(notes[0].code, "unrecognized_artifact");
                assert!(!scratch.exists(), "retired input created a scratch snapshot");
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sqlite_retirement_old_only_tree_does_not_create_bundle_or_objects() {
    let root = sandbox("sqlite-retired-only");
    let source = source_dir(&root);
    for (index, bytes) in retired_trace_databases(&root).into_iter().enumerate() {
        let dir = source.join(format!("case-{index}/.state"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("agents.db"), bytes).unwrap();
        write_file(&dir, "agents.db-wal", "ignored WAL");
        write_file(&dir, "agents.db-shm", "ignored SHM");
    }
    let before = fingerprint_tree(&source);
    let (bundles, notes) = discovery::discover(&[&source], 8);
    assert!(bundles.is_empty());
    assert!(notes.is_empty());
    let connection = open_connection(&initialize_db(&root));
    assert!(import_at(&connection, &root).outcomes.is_empty());
    for table in ["import_bundles", "import_bundle_files", "artifact_objects",
        "import_record_revisions", "import_projection_memberships", "sentinel_findings", "agent_runs"] {
        assert_eq!(table_count(&connection, table), 0, "{table}");
    }
    assert!(fingerprint_tree(&root.join("cas")).is_empty());
    assert!(!root.join("scratch").exists());
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sqlite_retirement_mixed_bundle_ignores_databases_and_sidecar_changes() {
    let root = sandbox("sqlite-retired-mixed");
    let source = source_dir(&root);
    let old_dir = source.join(".state");
    fs::create_dir_all(&old_dir).unwrap();
    fs::write(old_dir.join("agents.db"), &retired_trace_databases(&root)[1]).unwrap();
    write_file(&old_dir, "agents.db-wal", "ignored WAL");
    write_file(&old_dir, "agents.db-shm", "ignored SHM");
    write_json(&source, "model-prompt-audit.json", &serde_json::json!({"instruction":"current audit"}));
    write_json(&source, "findings.sarif", &sarif_document(vec![sarif_result(
        "CURRENT", "warning", "current finding", &[("src/current.rs", 2)], None,
    )]));
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    let first = import_at(&connection, &root);
    assert_eq!(first.outcomes.len(), 1);
    assert_eq!(first.outcomes[0].status, BundleStatus::Imported);
    assert_eq!(table_count(&connection, "import_bundle_files"), 2);
    assert_eq!(table_count(&connection, "artifact_objects"), 2);
    assert_eq!(current_envelopes(&connection, RecordKind::FindingCandidate).len(), 1);
    assert_eq!(current_envelopes(&connection, RecordKind::EventTrace).len(), 1);
    assert_eq!(table_count(&connection, "agent_runs"), 0);
    assert_eq!(fingerprint_tree(&source), before);
    assert!(!root.join("scratch").exists());
    let revisions = table_count(&connection, "import_record_revisions");
    for name in ["agents.db", "agents.db-wal", "agents.db-shm"] {
        write_file(&old_dir, name, "changed ignored bytes");
    }
    let changed = fingerprint_tree(&source);
    let second = import_at(&connection, &root);
    assert_eq!(second.outcomes[0].status, BundleStatus::Unchanged);
    assert_eq!(table_count(&connection, "import_record_revisions"), revisions);
    assert_eq!(table_count(&connection, "artifact_objects"), 2);
    assert_eq!(fingerprint_tree(&source), changed);
    fs::remove_dir_all(root).unwrap();
}
