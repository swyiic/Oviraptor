use super::*;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

struct Fixture {
    root: PathBuf,
    service: Service,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-source-import-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        for path in [
            &root,
            &root.join("data"),
            &root.join("configuration"),
            &root.join("resources"),
        ] {
            if !path.exists() {
                fs::create_dir(path).unwrap();
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let environment = ScanServiceEnvironment::from_roots(
            &root.join("data"),
            &root.join("configuration"),
            &root.join("resources"),
        )
        .unwrap();
        let mut service = Service::open(environment).unwrap();
        execution::enable(&service).unwrap();
        enable(&service).unwrap();
        service
            .set_execution_project(HostProject {
                project_id: 71,
                name: "Synthetic source project".into(),
                status: "active".into(),
                grant_revision: 1,
                permissions: BTreeSet::from([
                    "source.import".into(),
                    "draft.read".into(),
                    "scan.execute".into(),
                ]),
            })
            .unwrap();
        Self { root, service }
    }
    fn context(&self, operation: &str) -> Context {
        Context {
            host_bound_project_id: 71,
            grant_revision: 1,
            operation_id: operation.into(),
        }
    }
    fn directory(&self) -> PathBuf {
        self.service._environment.data_root().join(DIRECTORY)
    }
    fn import(&self) -> Value {
        import(&self.service, &self.context("source-once"), input()).unwrap()
    }
    fn no_execution(&self) {
        for table in [
            "sentinel_scans",
            "sentinel_scan_attempts",
            "source_scope_contracts",
            "agent_runs",
            "native_branch_dispatches",
            "agent_root_budget_attempts",
            "agent_model_cost_facts",
            "agent_http_request_claims",
        ] {
            assert_eq!(
                self.service
                    .database
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0,
                "{table}"
            );
        }
    }
}
fn make_writable(path: &Path) {
    let metadata = fs::symlink_metadata(path).unwrap();
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_dir() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        for entry in fs::read_dir(path).unwrap() {
            make_writable(&entry.unwrap().path());
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        make_writable(&self.root);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn file(path: &str, bytes: &[u8]) -> InputFile {
    InputFile {
        path: path.into(),
        content_base64: STANDARD.encode(bytes),
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}
fn request(files: Vec<InputFile>) -> Import {
    let mut facts: Vec<_> = files
        .iter()
        .map(|f| FileFact {
            path: f.path.clone(),
            sha256: f.sha256.clone(),
            bytes: STANDARD.decode(&f.content_base64).unwrap().len() as u64,
        })
        .collect();
    facts.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    Import {
        label: "Explicit synthetic snapshot".into(),
        expected_tree_sha256: tree_hash(&facts).unwrap(),
        files,
    }
}
fn input() -> Import {
    request(vec![
        file("src/Exact.rs", b"fn exact() {}\n"),
        file("README.md", b"source fixture\n"),
    ])
}
fn snapshot(db: &Connection) -> Value {
    let names:Vec<String>=db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let mut result = serde_json::Map::new();
    for name in names {
        let mut statement = db
            .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
            .unwrap();
        let width = statement.column_count();
        let mut rows: Vec<String> = statement
            .query_map([], |r| {
                let mut values = Vec::new();
                for n in 0..width {
                    values.push(format!("{:?}", r.get::<_, rusqlite::types::Value>(n)?));
                }
                Ok(values.join("|"))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        rows.sort();
        result.insert(name, json!(rows));
    }
    Value::Object(result)
}

#[test]
fn operator_source_import_preserves_exact_private_snapshot_and_grants_no_execution() {
    let f = Fixture::new();
    let receipt = f.import();
    let reference = receipt["sourceRef"].as_str().unwrap();
    assert_eq!(receipt["executionGranted"], false);
    assert_eq!(receipt["immutable"], true);
    assert_eq!(receipt["files"], 2);
    assert_eq!(receipt["bytes"], 29);
    assert_eq!(receipt["hostBoundProjectId"], 71);
    let path = f.directory().join(reference);
    assert_eq!(
        fs::read(path.join("src/Exact.rs")).unwrap(),
        b"fn exact() {}\n"
    );
    assert_eq!(
        fs::read(path.join("README.md")).unwrap(),
        b"source fixture\n"
    );
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o500);
    assert_eq!(
        fs::metadata(path.join("src")).unwrap().mode() & 0o777,
        0o500
    );
    assert_eq!(
        fs::metadata(path.join("src/Exact.rs")).unwrap().mode() & 0o777,
        0o400
    );
    assert!(!receipt.to_string().contains(f.root.to_str().unwrap()));
    assert!(!receipt.to_string().contains("fn exact"));
    let verified = resolve_on(
        &f.service,
        &f.service.database,
        &f.context("verify"),
        reference,
    )
    .unwrap();
    assert_eq!(verified.path(), path);
    assert_eq!(verified.receipt, receipt);
    f.no_execution();
}

#[test]
fn source_reference_replay_is_order_independent_and_verifies_live_bytes_without_new_rows() {
    let f = Fixture::new();
    let first = f.import();
    let before = snapshot(&f.service.database);
    let changes = f.service.database.total_changes();
    let mut reordered = input();
    reordered.files.reverse();
    assert_eq!(
        import(&f.service, &f.context("source-once"), reordered).unwrap(),
        first
    );
    assert_eq!(snapshot(&f.service.database), before);
    assert_eq!(f.service.database.total_changes(), changes);
    assert_eq!(fs::read_dir(f.directory()).unwrap().count(), 1);
    let mut changed = input();
    changed.label = "Changed operator label".into();
    assert_eq!(
        import(&f.service, &f.context("source-once"), changed).unwrap_err(),
        "scan_source_operation_conflict"
    );
    assert_eq!(snapshot(&f.service.database), before);
    assert_eq!(f.service.database.total_changes(), changes);
    f.no_execution();
}

#[test]
fn source_import_refuses_invalid_paths_aliases_and_file_parent_collisions_before_files_or_sql() {
    let f = Fixture::new();
    let before = snapshot(&f.service.database);
    let changes = f.service.database.total_changes();
    for path in [
        "",
        "/absolute",
        "../escape",
        "a/../escape",
        "./relative",
        "a//b",
        "a\\b",
        "C:drive",
        ".git/config",
        "a/.GIT/index",
        "a\0b",
        "a\nb",
    ] {
        let mut bad = input();
        bad.files[0].path = path.into();
        assert!(
            import(&f.service, &f.context("invalid-path"), bad).is_err(),
            "{path:?}"
        );
    }
    for paths in [["a", "a"], ["A.rs", "a.rs"], ["a", "a/b"], ["Dir", "dir/b"]] {
        let mut bad = input();
        bad.files = paths.into_iter().map(|p| file(p, b"x")).collect();
        assert!(
            import(&f.service, &f.context("invalid-alias"), bad).is_err(),
            "{paths:?}"
        );
    }
    assert!(!f.directory().exists());
    assert_eq!(snapshot(&f.service.database), before);
    assert_eq!(f.service.database.total_changes(), changes);
    f.no_execution();
}

#[test]
fn source_import_refuses_size_base64_and_digest_failures_without_partial_publication() {
    let f = Fixture::new();
    let before = snapshot(&f.service.database);
    let changes = f.service.database.total_changes();
    let mut cases = Vec::new();
    let mut bad = input();
    bad.files.clear();
    cases.push(bad);
    let mut bad = input();
    bad.files = (0..=MAX_FILES)
        .map(|n| file(&format!("{n}.txt"), b"x"))
        .collect();
    cases.push(bad);
    let mut bad = input();
    bad.files = vec![file("too-large", &vec![b'x'; MAX_FILE_BYTES + 1])];
    cases.push(bad);
    let mut bad = input();
    bad.files = (0..5)
        .map(|n| file(&format!("{n}.txt"), &vec![b'x'; MAX_FILE_BYTES]))
        .collect();
    cases.push(bad);
    let mut bad = input();
    bad.files[0].content_base64 = "not_base64!".into();
    cases.push(bad);
    let mut bad = input();
    bad.files[0].sha256 = "0".repeat(64);
    cases.push(bad);
    let mut bad = input();
    bad.expected_tree_sha256 = "0".repeat(64);
    cases.push(bad);
    let mut bad = input();
    bad.files[0].path = "a/".repeat(16) + "file";
    cases.push(bad);
    for bad in cases {
        assert!(import(&f.service, &f.context("invalid-bounds"), bad).is_err());
    }
    assert!(!f.directory().exists());
    assert_eq!(snapshot(&f.service.database), before);
    assert_eq!(f.service.database.total_changes(), changes);
    f.no_execution();
}

#[test]
fn source_references_are_project_instance_and_current_grant_scoped() {
    let f = Fixture::new();
    let receipt = f.import();
    let reference = receipt["sourceRef"].as_str().unwrap();
    let before = snapshot(&f.service.database);
    let changes = f.service.database.total_changes();
    let foreign = Context {
        host_bound_project_id: 72,
        grant_revision: 1,
        operation_id: "foreign".into(),
    };
    assert!(resolve_on(&f.service, &f.service.database, &foreign, reference).is_err());
    let stale = Context {
        host_bound_project_id: 71,
        grant_revision: 0,
        operation_id: "stale".into(),
    };
    assert_eq!(
        import(&f.service, &stale, input()).unwrap_err(),
        "scan_host_scope_permission_denied"
    );
    let other = Fixture::new();
    assert!(resolve_on(
        &other.service,
        &other.service.database,
        &other.context("other"),
        reference
    )
    .is_err());
    assert_eq!(snapshot(&f.service.database), before);
    assert_eq!(f.service.database.total_changes(), changes);
    f.no_execution();
}

#[test]
fn replay_refuses_modified_missing_extra_symlink_hardlink_and_permission_changed_files() {
    for scenario in [
        "modified",
        "missing",
        "extra",
        "empty-directory",
        "symlink",
        "hardlink",
        "permissions",
    ] {
        let f = Fixture::new();
        let receipt = f.import();
        let root = f.directory().join(receipt["sourceRef"].as_str().unwrap());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let file = root.join("README.md");
        match scenario {
            "modified" => {
                fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
                fs::write(&file, b"changed bytes\n").unwrap();
                fs::set_permissions(&file, fs::Permissions::from_mode(0o400)).unwrap();
            }
            "missing" => fs::remove_file(&file).unwrap(),
            "extra" => {
                fs::write(root.join("extra.txt"), b"unlisted").unwrap();
                fs::set_permissions(root.join("extra.txt"), fs::Permissions::from_mode(0o400))
                    .unwrap();
            }
            "empty-directory" => {
                fs::create_dir(root.join("extra")).unwrap();
                fs::set_permissions(root.join("extra"), fs::Permissions::from_mode(0o500)).unwrap();
            }
            "symlink" => {
                fs::remove_file(&file).unwrap();
                symlink(root.join("src/Exact.rs"), &file).unwrap();
            }
            "hardlink" => {
                fs::remove_file(&file).unwrap();
                fs::hard_link(root.join("src/Exact.rs"), &file).unwrap();
            }
            "permissions" => fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap(),
            _ => unreachable!(),
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
        let before = snapshot(&f.service.database);
        let changes = f.service.database.total_changes();
        assert!(
            import(&f.service, &f.context("source-once"), input()).is_err(),
            "{scenario}"
        );
        assert_eq!(snapshot(&f.service.database), before, "{scenario}");
        assert_eq!(f.service.database.total_changes(), changes, "{scenario}");
        f.no_execution();
    }
}

#[test]
fn reference_root_replacement_missing_or_symlink_is_never_recreated_or_adopted() {
    for scenario in ["replace", "missing", "symlink"] {
        let f = Fixture::new();
        let receipt = f.import();
        let reference = receipt["sourceRef"].as_str().unwrap();
        let path = f.directory().join(reference);
        let displaced = f.directory().join("displaced-original");
        fs::rename(&path, &displaced).unwrap();
        match scenario {
            "replace" => {
                fs::create_dir(&path).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
            }
            "symlink" => symlink(&displaced, &path).unwrap(),
            "missing" => {}
            _ => unreachable!(),
        }
        let before = snapshot(&f.service.database);
        let changes = f.service.database.total_changes();
        assert!(
            resolve_on(
                &f.service,
                &f.service.database,
                &f.context("check-root"),
                reference
            )
            .is_err(),
            "{scenario}"
        );
        if scenario == "missing" {
            assert!(!path.exists());
        }
        assert_eq!(
            fs::read(displaced.join("src/Exact.rs")).unwrap(),
            b"fn exact() {}\n"
        );
        assert_eq!(snapshot(&f.service.database), before);
        assert_eq!(f.service.database.total_changes(), changes);
        f.no_execution();
    }
}

#[test]
fn ignored_source_publication_retains_only_unreferenced_private_evidence_and_no_execution() {
    let f = Fixture::new();
    f.service.database.execute_batch("CREATE TRIGGER deny_source BEFORE INSERT ON scan_service_sources BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before = snapshot(&f.service.database);
    assert!(import(&f.service, &f.context("ignored"), input()).is_err());
    assert_eq!(snapshot(&f.service.database), before);
    assert_eq!(fs::read_dir(f.directory()).unwrap().count(), 1);
    let reference = fs::read_dir(f.directory())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    assert!(resolve_on(
        &f.service,
        &f.service.database,
        &f.context("orphan"),
        &reference
    )
    .is_err());
    assert!(fs::read(f.directory().join(reference).join("README.md")).is_ok());
    f.no_execution();
}

#[test]
fn source_schema_reopen_is_idempotent_and_partial_or_unknown_schema_is_not_repaired() {
    let f = Fixture::new();
    let receipt = f.import();
    let before = snapshot(&f.service.database);
    let changes = f.service.database.total_changes();
    enable(&f.service).unwrap();
    assert_eq!(snapshot(&f.service.database), before);
    assert_eq!(f.service.database.total_changes(), changes);
    f.service
        .database
        .execute_batch("DROP TRIGGER scan_service_sources_no_update;")
        .unwrap();
    let before = snapshot(&f.service.database);
    let changes = f.service.database.total_changes();
    assert_eq!(
        enable(&f.service).unwrap_err(),
        "scan_source_schema_partial"
    );
    assert_eq!(snapshot(&f.service.database), before);
    assert_eq!(f.service.database.total_changes(), changes);
    assert!(fs::read(
        f.directory()
            .join(receipt["sourceRef"].as_str().unwrap())
            .join("README.md")
    )
    .is_ok());
    f.no_execution();
}
