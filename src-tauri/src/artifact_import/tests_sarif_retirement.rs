// Vendor metadata is opaque evidence, never a classification or authority input.
#[test]
fn sarif_retirement_vendor_metadata_cannot_override_standard_fields() {
    for kind in [None, Some("fail"), Some("pass"), Some("open"), Some("notApplicable")] {
        for level in ["warning", "error", "none", "note"] {
            for rule in ["CURRENT-RULE", "strix-coverage/old-rule"] {
                let mut result = sarif_result(rule, level, "local fixture", &[], None);
                if let Some(kind) = kind { result["kind"] = serde_json::json!(kind); }
                let plain = sarif_document(vec![result.clone()]);
                result["properties"] = serde_json::json!({
                    "kind":"open", "coverage_outcome":"tested",
                    "strix":{"coverage_outcome":"not_tested"},
                    "executionEligible":true, "reviewState":"confirmed"
                });
                let decorated = sarif_document(vec![result.clone()]);
                let parse = |value: &serde_json::Value| {
                    let (rows, notes) = parse_audit_fixture("findings.sarif", &serde_json::to_vec(value).unwrap(), &Limits::default());
                    assert!(notes.is_empty()); assert_eq!(rows.len(), 1);
                    rows.into_iter().next().unwrap()
                };
                let plain = parse(&plain);
                let decorated = parse(&decorated);
                let expected = if matches!(level, "warning" | "error") && matches!(kind, None | Some("fail")) {
                    RecordKind::FindingCandidate
                } else { RecordKind::Coverage };
                assert_eq!(plain.record_kind, expected, "rule name must be opaque: {rule}");
                assert_eq!(decorated.record_kind, expected, "vendor properties must be opaque");
                assert_eq!(plain.payload, decorated.payload);
                assert!(decorated.payload.get("coverage_outcome").is_none());
                assert!(decorated.extensions.get("coverage_outcome").is_none());
                assert_eq!(decorated.extensions["sarif_result"]["properties"], result["properties"]);
                assert_eq!(decorated.claim, canonical::Claim::historical_external());
            }
        }
    }
}

#[test]
fn sarif_retirement_v3_signature_reparses_without_mutating_source_or_native_rows() {
    let root = sandbox("sarif-version-retirement");
    let source = source_dir(&root);
    write_json(&source, "findings.sarif", &sarif_document(vec![sarif_result(
        "CURRENT-RULE", "warning", "local fixture", &[], None,
    )]));
    let before = fingerprint_tree(&source);
    let connection = open_connection(&initialize_db(&root));
    import_at(&connection, &root);
    let original: String = connection.query_row("SELECT envelope_json FROM import_record_revisions LIMIT 1", [], |row| row.get(0)).unwrap();
    let count = table_count(&connection, "import_record_revisions");
    let signature: String = connection.query_row("SELECT signature FROM import_bundles", [], |row| row.get(0)).unwrap();
    let (manifest, _) = signature.rsplit_once("\u{1}adapter=").unwrap();
    connection.execute("UPDATE import_bundles SET signature=?1", [format!("{manifest}\u{1}adapter=3")]).unwrap();
    assert!(!has_code(&import_at(&connection, &root), "bundle_unchanged"), "v3 cached interpretations must be reconciled");
    assert_eq!(table_count(&connection, "import_record_revisions"), count);
    assert_eq!(connection.query_row("SELECT envelope_json FROM import_record_revisions LIMIT 1", [], |row| row.get::<_, String>(0)).unwrap(), original);
    assert_eq!(current_envelopes(&connection, RecordKind::FindingCandidate).len(), 1);
    assert!(has_code(&import_at(&connection, &root), "bundle_unchanged"));
    for table in ["sentinel_findings", "agent_runs", "agent_source_review_decisions"] {
        assert_eq!(table_count(&connection, table), 0);
    }
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sarif_retirement_replaces_old_membership_but_preserves_immutable_revision() {
    let root = sandbox("sarif-old-membership");
    let source = source_dir(&root);
    let original = sarif_result("CURRENT-RULE", "warning", "local fixture", &[],
        Some(serde_json::json!({"kind":"open"})));
    write_json(&source, "findings.sarif", &sarif_document(vec![original.clone()]));
    let before = fingerprint_tree(&source);
    let db = open_connection(&initialize_db(&root));
    let limits = Limits::default();
    let (bundles, notes) = discovery::discover(&[&source], 8);
    assert!(notes.is_empty());
    let bundle = &bundles[0];
    let (manifest, payloads) = manifest::build(&source, &source, &bundle.files, &limits).unwrap();
    let context = adapters::ParseContext {
        bundle_id: &manifest.bundle_id, manifest: &manifest, payloads: &payloads, limits: &limits,
    };
    // Seed a v3 interpretation without retaining a production compatibility reader.
    let mut old_input = original.clone();
    old_input["kind"] = serde_json::json!("open");
    let (mut records, notes) = adapters::sarif::records(&context, "findings.sarif",
        &serde_json::to_vec(&sarif_document(vec![old_input])).unwrap());
    assert!(notes.is_empty());
    let mut old = records.remove(0);
    assert_eq!(old.record_kind, RecordKind::Coverage);
    old.provenance.adapter_version = 3;
    old.extensions["sarif_result"] = original;
    old.finalize_revision_hash();
    let old_text = serde_json::to_string(&old).unwrap();
    let scope = scope::resolve(&source, bundle, &payloads);
    let scope_key = scope.reconcile_key();
    let signature = format!("{}\u{1}{}\u{1}adapter=3", manifest.canonical, source.display());
    let bundle_id = store::write_bundle(&db, &manifest.bundle_id, &source, &source,
        &bundle.attempt_key, scope.attempt_number, &signature, "imported", 1, 1).unwrap();
    let (revision, _) = store::insert_revision(&db, &old, &old_text).unwrap();
    let origin = store::MembershipOrigin { scope_key: &scope_key, source_path: &source,
        attempt_number: scope.attempt_number };
    store::add_membership(&db, &origin, &old, revision, bundle_id, "").unwrap();
    assert_eq!(current_envelopes(&db, RecordKind::Coverage).len(), 1);
    assert!(!has_code(&import_at(&db, &root), "bundle_unchanged"));
    assert!(current_envelopes(&db, RecordKind::Coverage).is_empty());
    let current = current_envelopes(&db, RecordKind::FindingCandidate);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["claim"]["executionEligible"], false);
    assert_eq!(table_count(&db, "import_record_revisions"), 2);
    let stored: String = db.query_row("SELECT envelope_json FROM import_record_revisions WHERE id=?1",
        [revision], |row| row.get(0)).unwrap();
    assert_eq!(stored, old_text);
    assert!(has_code(&import_at(&db, &root), "bundle_unchanged"));
    assert_eq!(fingerprint_tree(&source), before);
    fs::remove_dir_all(root).unwrap();
}
