// Behaviour regressions from the 2026-09-23 independent audit.

#[test]
fn marker_aliases_and_parents_preserve_deleted_scan_identity() {
    for marker in [".oviraptor-scan-id", ".asset-atlas-scan-id"] {
        for parent in [false, true] {
            let root = sandbox("marker-regression");
            let bundle = single_finding_bundle(&root, "high");
            let directory = if parent { bundle.parent().unwrap() } else { &bundle };
            write_file(directory, marker, "deleted-marker-scan\n");
            let before = fingerprint_tree(&source_dir(&root));
            let connection = open_connection(&initialize_db(&root));
            connection.execute(
                "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('deleted-marker-scan')", [],
            ).unwrap();
            let summary = import_at(&connection, &root);
            assert!(has_code(&summary, "deleted_scan_not_projected"),
                "marker={marker}, parent={parent}: {:?}", codes(&summary));
            assert!(logical_keys(&connection, RecordKind::FindingCandidate).is_empty());
            assert_eq!(before, fingerprint_tree(&source_dir(&root)));
            fs::remove_dir_all(root).unwrap();
        }
    }
}

fn secret_bundle(root: &Path, name: &str) -> PathBuf {
    let bundle = source_dir(root).join(name);
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("secret-finding", "warning", "credential fixture", &[], Some(serde_json::json!({"endpoint":"/private","poc_description":format!("password={SECRET_MARKER}")})))]),
    );
    write_json(&bundle, "model-prompt-audit.json", &serde_json::json!({"instruction":name}));
    bundle
}

#[test]
fn secret_original_survives_partial_bundle_change_and_cross_bundle_reuse() {
    let root = sandbox("secret-reuse");
    let bundle = secret_bundle(&root, "first");
    let connection = open_connection(&initialize_db(&root));
    let first = import_at(&connection, &root);
    assert_eq!(first.outcomes[0].status, BundleStatus::Imported);
    let original_cas = fingerprint_tree(&root.join("cas"));
    write_json(&bundle, "model-prompt-audit.json", &serde_json::json!({"instruction":"first revised audit"}));
    secret_bundle(&root, "second");
    let before = fingerprint_tree(&source_dir(&root));
    let second = import_at(&connection, &root);
    assert_eq!(second.outcomes.len(), 2);
    assert!(second.outcomes.iter().all(|o| o.status == BundleStatus::Imported),
        "shared ciphertext is valid, not CAS corruption: {:?}", codes(&second));
    let current_cas = fingerprint_tree(&root.join("cas"));
    for (name, fingerprint) in original_cas {
        assert_eq!(Some(&fingerprint), current_cas.get(&name), "never overwrite existing CAS");
    }
    assert_eq!(before, fingerprint_tree(&source_dir(&root)));
    let third = import_at(&connection, &root);
    assert!(third.outcomes.iter().all(|o| o.status == BundleStatus::Unchanged));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_secret_objects_fail_closed_on_tamper_wrong_key_or_plaintext() {
    for failure in ["tamper", "key", "plaintext"] {
        let root = sandbox("secret-corruption");
        let bundle = secret_bundle(&root, "first");
        let connection = open_connection(&initialize_db(&root));
        assert_eq!(import_at(&connection, &root).outcomes[0].status, BundleStatus::Imported);
        let bytes = fs::read(bundle.join("findings.sarif")).unwrap();
        let hash = canonical::sha256_hex(&bytes);
        let path = root.join("cas").join(&hash[..2]).join(&hash);
        match failure {
            "tamper" => {
                let mut sealed = fs::read(&path).unwrap();
                *sealed.last_mut().unwrap() ^= 1;
                fs::write(&path, sealed).unwrap();
            }
            "key" => fs::write(root.join("artifact-import.key"), [7u8; 32]).unwrap(),
            _ => fs::write(&path, bytes).unwrap(),
        }
        let cas_before = fingerprint_tree(&root.join("cas"));
        let revisions = table_count(&connection, "import_record_revisions");
        write_json(&bundle, "model-prompt-audit.json", &serde_json::json!({"instruction":"first revised audit"}));
        let outcome = import_at(&connection, &root);
        assert_eq!(outcome.outcomes[0].status, BundleStatus::Failed, "{failure}");
        assert_eq!(revisions, table_count(&connection, "import_record_revisions"));
        assert_eq!(cas_before.get(&format!("{}/{hash}", &hash[..2])),
            fingerprint_tree(&root.join("cas")).get(&format!("{}/{hash}", &hash[..2])));
        assert!(!format!("{:?}", outcome).contains(SECRET_MARKER));
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn bundle_transaction_reserves_the_writer_before_its_first_statement() {
    let root = sandbox("immediate-transaction");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let contender = open_connection(&db_path);
    contender.busy_timeout(std::time::Duration::ZERO).unwrap();
    let transaction = service::begin_bundle_transaction(&connection).unwrap();
    assert!(contender.execute_batch("BEGIN IMMEDIATE").is_err(),
        "a bundle must reserve the write lock before performing any reads or writes");
    drop(transaction);
    contender.execute_batch("BEGIN IMMEDIATE; ROLLBACK").unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn simultaneous_first_use_creates_one_installation_key() {
    let root = sandbox("key-race");
    let path = root.join("key");
    let barrier = std::sync::Barrier::new(16);
    let blobs = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..16).map(|_| scope.spawn(|| {
            barrier.wait();
            let key = sealing::Key::load_or_create(&path).unwrap();
            sealing::seal(&key, b"shared installation secret").unwrap()
        })).collect();
        workers.into_iter().map(|worker| worker.join().unwrap()).collect::<Vec<_>>()
    });
    let key = sealing::Key::load_or_create(&path).unwrap();
    for blob in blobs {
        assert_eq!(sealing::open(&key, &blob).unwrap(), b"shared installation secret");
    }
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_secret_imports_share_a_complete_ciphertext_and_key() {
    let root = sandbox("concurrent-secret-import");
    secret_bundle(&root, "shared");
    let db_path = initialize_db(&root);
    let barrier = std::sync::Barrier::new(4);
    let results = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4).map(|_| scope.spawn(|| {
            let connection = open_connection(&db_path);
            barrier.wait();
            import_at(&connection, &root)
        })).collect();
        workers.into_iter().map(|worker| worker.join().unwrap()).collect::<Vec<_>>()
    });
    for result in results {
        assert_eq!(result.outcomes.len(), 1);
        assert_ne!(result.outcomes[0].status, BundleStatus::Failed, "{:?}", codes(&result));
    }
    let connection = open_connection(&db_path);
    let revisions = table_count(&connection, "import_record_revisions");
    assert!(import_at(&connection, &root).outcomes.iter().all(|o| o.status == BundleStatus::Unchanged));
    assert_eq!(revisions, table_count(&connection, "import_record_revisions"));
    fs::remove_dir_all(root).unwrap();
}
