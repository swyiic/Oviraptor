fn source_report_fixture(scan: &str, attempt: i64, root: &str) -> serde_json::Value {
    serde_json::json!({"format":"oviraptor-source-review-v1","schemaVersion":1,
        "executionEligible":true,"review":{"scanId":scan,"attemptNumber":attempt,
        "rootRunId":root,"materialDigest":"material-1","status":"audited",
        "counts":{"confirmed":1},"findings":[{"id":"decision-1","sourceDecisionId":"decision-1",
        "scanId":scan,"attemptNumber":attempt,"rootRunId":root,"materialDigest":"material-1",
        "candidateDigest":"candidate-1","reviewState":"confirmed","title":"Shared title",
        "path":"src/app.rs","severity":"high","executionEligible":true}]}})
}

fn source_sarif_fixture(bundle: &serde_json::Value) -> serde_json::Value {
    let rows = bundle["review"]["findings"].as_array().unwrap();
    let mut summary = bundle.clone();
    summary["review"]
        .as_object_mut()
        .unwrap()
        .remove("findings");
    serde_json::json!({"version":"2.1.0","runs":[{"tool":{"driver":{"name":"oviraptor-source-review"}},
        "properties":{"oviraptorSourceReview":summary},"results":rows.iter().map(|row|serde_json::json!({
            "ruleId":row["sourceDecisionId"],"kind":"fail","level":"error","message":{"text":row["title"]},
            "partialFingerprints":{"oviraptorSourceDecisionId":row["sourceDecisionId"],"candidateDigest":row["candidateDigest"]},
            "properties":{"oviraptorSourceFinding":row}})).collect::<Vec<_>>()}]})
}

fn source_container_fixture() -> serde_json::Value {
    let report = source_report_fixture("container-task", 1, "container-root");
    super::report_bundle::build(report.clone(), source_sarif_fixture(&report)).unwrap()
}

fn refresh_container_hashes(bundle: &mut serde_json::Value) {
    for (key, name) in [
        ("json", "source-review.json"),
        ("sarif", "source-review.sarif"),
    ] {
        let bytes = canonical::canonical_json(&bundle["documents"][key]);
        bundle["manifest"][key] = serde_json::json!({"name":name,"bytes":bytes.len(),"sha256":canonical::sha256_hex(bytes.as_bytes())});
    }
}

#[test]
fn source_container_rejects_incomplete_corrupt_or_inconsistent_pairs_before_any_write() {
    for mutation in [
        "missing",
        "extra",
        "schema",
        "digest",
        "size",
        "name",
        "changed",
        "foreign",
        "timestamp",
        "candidate",
        "extra_run",
        "invalid_kind",
        "invalid_count",
        "truncated",
    ] {
        for direct in [false, true] {
            let root = sandbox(mutation);
            let source = source_dir(&root);
            let db = open_connection(&initialize_db(&root));
            let mut value = source_container_fixture();
            match mutation {
                "missing" => {
                    value["documents"].as_object_mut().unwrap().remove("sarif");
                }
                "extra" => value["documents"]["other"] = serde_json::json!({}),
                "schema" => value["schemaVersion"] = serde_json::json!(2),
                "digest" => value["manifest"]["sarif"]["sha256"] = serde_json::json!("wrong"),
                "size" => value["manifest"]["json"]["bytes"] = serde_json::json!(0),
                "name" => {
                    value["manifest"]["json"]["name"] = serde_json::json!("../../outside.json")
                }
                "changed" => {
                    value["documents"]["json"]["review"]["scanId"] = serde_json::json!("changed")
                }
                "foreign" => {
                    let other = source_report_fixture("foreign-task", 1, "container-root");
                    value["documents"]["sarif"] = source_sarif_fixture(&other);
                    refresh_container_hashes(&mut value);
                }
                "timestamp" => {
                    value["documents"]["sarif"]["runs"][0]["properties"]["oviraptorSourceReview"]
                        ["exportedAt"] = serde_json::json!("different");
                    refresh_container_hashes(&mut value);
                }
                "candidate" => {
                    value["documents"]["sarif"]["runs"][0]["results"][0]["properties"]
                        ["oviraptorSourceFinding"]["severity"] = serde_json::json!("low");
                    refresh_container_hashes(&mut value);
                }
                "extra_run" => {
                    let run = value["documents"]["sarif"]["runs"][0].clone();
                    value["documents"]["sarif"]["runs"]
                        .as_array_mut()
                        .unwrap()
                        .push(run);
                    refresh_container_hashes(&mut value);
                }
                "invalid_kind" => {
                    value["documents"]["sarif"]["runs"][0]["results"][0]["kind"] =
                        serde_json::json!("pass");
                    refresh_container_hashes(&mut value);
                }
                "invalid_count" => {
                    value["documents"]["json"]["review"]["counts"]["confirmed"] =
                        serde_json::json!(0);
                    value["documents"]["sarif"] = source_sarif_fixture(&value["documents"]["json"]);
                    refresh_container_hashes(&mut value);
                }
                _ => {}
            }
            let mut bytes = serde_json::to_vec(&value).unwrap();
            if mutation == "truncated" {
                bytes.pop();
            }
            if direct {
                let cas = root.join("cas");
                let key = root.join("key");
                let limits = Limits::default();
                let context = ImportContext {
                    connection: &db,
                    cas_dir: &cas,
                    key_path: &key,
                    roots: &[],
                    limits: &limits,
                };
                assert!(
                    super::import_sentinel_snapshot(&context, &bytes, None).is_err(),
                    "{mutation}"
                );
            } else {
                fs::write(source.join("source-review-bundle-bad.json"), &bytes).unwrap();
                let result = import_at(&db, &root);
                assert_eq!(
                    result.outcomes[0].status,
                    BundleStatus::Failed,
                    "{mutation}: {result:?}"
                );
            }
            for table in [
                "import_bundles",
                "import_bundle_files",
                "import_record_revisions",
                "import_projection_memberships",
                "artifact_objects",
            ] {
                assert_eq!(table_count(&db, table), 0, "{mutation}: {table}");
            }
            assert!(!root.join("cas").exists());
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn source_container_and_generic_records_share_premerge_limit() {
    for mixed in [false, true] {
        let root = sandbox("container-limits");
        let source = source_dir(&root);
        let db = open_connection(&initialize_db(&root));
        let value = source_container_fixture();
        write_json(&source, "source-review-bundle-a.json", &value);
        let limit = if mixed {
            write_json(
                &source,
                "ordinary.sarif",
                &serde_json::json!({"version":"2.1.0","runs":[{"tool":{"driver":{"name":"generic"}},"results":[{"ruleId":"generic","message":{"text":"generic"}}]}]}),
            );
            4
        } else {
            write_json(&source, "source-review-bundle-b.json", &value);
            7
        };
        let limits = Limits {
            records: limit,
            ..Limits::default()
        };
        let result = import_dir(&db, &root, &limits);
        assert_eq!(
            result.outcomes[0].status,
            BundleStatus::Failed,
            "{result:?}"
        );
        assert_eq!(table_count(&db, "import_record_revisions"), 0);
        assert!(!root.join("cas").exists());
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_container_preserves_document_pointers_and_zero_result_summaries() {
    for zero in [false, true] {
        let root = sandbox("container-provenance");
        let source = source_dir(&root);
        let db = open_connection(&initialize_db(&root));
        let mut report = source_report_fixture("container-task", 1, "container-root");
        if zero {
            report["review"]["findings"] = serde_json::json!([]);
            report["review"]["counts"]["confirmed"] = serde_json::json!(0);
        }
        let mut value =
            super::report_bundle::build(report.clone(), source_sarif_fixture(&report)).unwrap();
        value["executionEligible"] = serde_json::json!(true);
        write_json(&source, "source-review-bundle-a.json", &value);
        let before = fingerprint_tree(&source);
        let result = import_at(&db, &root);
        assert_eq!(
            result.outcomes[0].status,
            BundleStatus::Imported,
            "{result:?}"
        );
        assert_eq!(current_envelopes(&db, RecordKind::RunState).len(), 2);
        let findings = current_envelopes(&db, RecordKind::FindingCandidate);
        assert_eq!(findings.len(), if zero { 0 } else { 1 });
        for record in current_envelopes(&db, RecordKind::RunState)
            .into_iter()
            .chain(findings)
        {
            let pointer = record["provenance"]["sourceRecordPointer"]
                .as_str()
                .unwrap();
            assert!(pointer.starts_with("/documents/"));
            assert!(value.pointer(pointer).is_some(), "{pointer}");
            assert_eq!(record["claim"]["authority"], "historical_external");
            assert_eq!(record["claim"]["executionEligible"], false);
        }
        assert_eq!(
            import_at(&db, &root).outcomes[0].status,
            BundleStatus::Unchanged
        );
        assert_eq!(fingerprint_tree(&source), before);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reports_exact_scopes_attempts_and_generic_runs_coexist() {
    let root = sandbox("source-scopes");
    let source = source_dir(&root);
    let db = open_connection(&initialize_db(&root));
    for (name, scan, attempt, run) in [
        ("a", "task-a", 1, "root-a"),
        ("b", "task-b", 1, "root-a"),
        ("c", "task-a", 2, "root-a"),
        ("d", "task-a", 2, "root-b"),
    ] {
        let report = source_report_fixture(scan, attempt, run);
        write_json(&source, &format!("source-review-{name}.json"), &report);
        write_json(
            &source,
            &format!("source-review-{name}.sarif"),
            &source_sarif_fixture(&report),
        );
    }
    // A mixed SARIF file retains ordinary runs and their original pointer index.
    let mut mixed = source_sarif_fixture(&source_report_fixture("task-b", 1, "root-a"));
    mixed["runs"].as_array_mut().unwrap().push(serde_json::json!({"tool":{"driver":{"name":"ordinary"}},
        "properties":{"note":"ordinary"},"results":[{"ruleId":"other","level":"warning","message":{"text":"Other"}}]}));
    write_json(&source, "mixed.sarif", &mixed);
    let before = fingerprint_tree(&source);
    let result = import_at(&db, &root);
    assert_eq!(
        result.outcomes[0].status,
        BundleStatus::Imported,
        "{result:?}"
    );
    assert!(result.outcomes[0]
        .diagnostics
        .iter()
        .any(|d| d.code == "older_attempt_ignored"));
    let findings = current_envelopes(&db, RecordKind::FindingCandidate);
    assert_eq!(
        findings.len(),
        4,
        "two roots in newest task-a, task-b, and ordinary SARIF"
    );
    assert_eq!(
        findings
            .iter()
            .filter(|r| r["extensions"]["scanId"] == "task-a")
            .count(),
        2
    );
    for row in findings {
        assert_eq!(row["claim"]["executionEligible"], false);
        assert_eq!(row["claim"]["readOnly"], true);
        assert_eq!(row["claim"]["reviewState"], "unreviewed");
        if row["extensions"]["scanId"] == "task-a" {
            assert_eq!(row["extensions"]["attemptNumber"], 2);
        }
    }
    assert!(current_envelopes(&db, RecordKind::RunState)
        .iter()
        .any(|r| r["provenance"]["sourceRecordPointer"] == "/runs/1"));
    assert_eq!(
        result.outcomes[0]
            .committed_records
            .as_ref()
            .unwrap()
            .iter()
            .filter(|r| r.record_kind == "finding_candidate")
            .count(),
        8
    );
    assert_eq!(
        import_at(&db, &root).outcomes[0].status,
        BundleStatus::Unchanged
    );
    assert_eq!(fingerprint_tree(&source), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reports_invalid_native_documents_fail_atomically() {
    for mutation in [
        "foreign_scan",
        "foreign_attempt",
        "foreign_root",
        "foreign_material",
        "duplicate",
        "count",
        "schema",
        "pass",
        "fingerprint",
        "malformed",
    ] {
        let root = sandbox(mutation);
        let source = source_dir(&root);
        let db = open_connection(&initialize_db(&root));
        let valid = source_report_fixture("task-a", 1, "root-a");
        write_json(&source, "source-review-valid.json", &valid);
        let mut report = valid.clone();
        match mutation {
            "foreign_scan" => {
                report["review"]["findings"][0]["scanId"] = serde_json::json!("task-b")
            }
            "foreign_attempt" => {
                report["review"]["findings"][0]["attemptNumber"] = serde_json::json!(2)
            }
            "foreign_root" => {
                report["review"]["findings"][0]["rootRunId"] = serde_json::json!("root-b")
            }
            "foreign_material" => {
                report["review"]["findings"][0]["materialDigest"] = serde_json::json!("material-b")
            }
            "duplicate" => {
                let row = report["review"]["findings"][0].clone();
                report["review"]["findings"]
                    .as_array_mut()
                    .unwrap()
                    .push(row);
                report["review"]["counts"]["confirmed"] = serde_json::json!(2);
            }
            "count" => report["review"]["counts"]["confirmed"] = serde_json::json!(2),
            "schema" => report["schemaVersion"] = serde_json::json!(2),
            _ => {}
        }
        if matches!(mutation, "pass" | "fingerprint") {
            let mut sarif = source_sarif_fixture(&report);
            if mutation == "pass" {
                sarif["runs"][0]["results"][0]["kind"] = serde_json::json!("pass");
            } else {
                sarif["runs"][0]["results"][0]["partialFingerprints"]["candidateDigest"] =
                    serde_json::json!("foreign");
            }
            write_json(&source, "bad.sarif", &sarif);
        } else if mutation == "malformed" {
            write_file(&source, "source-review-bad.json", "{");
        } else {
            write_json(&source, "source-review-bad.json", &report);
        }
        let result = import_at(&db, &root);
        assert_eq!(
            result.outcomes[0].status,
            BundleStatus::Failed,
            "{mutation}: {result:?}"
        );
        for table in [
            "import_bundles",
            "import_bundle_files",
            "import_record_revisions",
            "import_projection_memberships",
            "artifact_objects",
        ] {
            assert_eq!(table_count(&db, table), 0, "{mutation}:{table}");
        }
        assert!(
            !root.join("cas").exists(),
            "invalid native document must fail before CAS publication"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reports_duplicate_inputs_cannot_bypass_record_limit() {
    let root = sandbox("source-limit");
    let source = source_dir(&root);
    let db = open_connection(&initialize_db(&root));
    let report = source_report_fixture("task-a", 1, "root-a");
    write_json(&source, "source-review-a.json", &report);
    write_json(&source, "source-review-b.json", &report);
    let limits = Limits {
        records: 3,
        ..Limits::default()
    };
    assert_eq!(
        import_dir(&db, &root, &limits).outcomes[0].status,
        BundleStatus::Failed
    );
    assert_eq!(table_count(&db, "import_record_revisions"), 0);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reports_conflicting_views_preserve_values_and_both_origins() {
    let root = sandbox("source-conflict");
    let source = source_dir(&root);
    let db = open_connection(&initialize_db(&root));
    let first = source_report_fixture("task-a", 1, "root-a");
    let mut second = first.clone();
    second["review"]["findings"][0]["severity"] = serde_json::json!("low");
    write_json(&source, "source-review-a.json", &first);
    write_json(
        &source,
        "source-review-b.sarif",
        &source_sarif_fixture(&second),
    );
    let result = import_at(&db, &root);
    assert_eq!(result.outcomes[0].status, BundleStatus::Imported);
    let rows = current_envelopes(&db, RecordKind::FindingCandidate);
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.to_string().contains("severity")));
    assert_eq!(
        result.outcomes[0]
            .committed_records
            .as_ref()
            .unwrap()
            .iter()
            .filter(|r| r.record_kind == "finding_candidate")
            .count(),
        2
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_reports_rebind_old_sarif_memberships_without_erasing_generic_history() {
    for case in ["same_path", "moved", "native_only", "ambiguous"] {
        let root = sandbox("source-migration");
        let source = source_dir(&root);
        let db = open_connection(&initialize_db(&root));
        let mut sarif = source_sarif_fixture(&source_report_fixture("task-a", 1, "root-a"));
        if case != "native_only" {
            sarif["runs"].as_array_mut().unwrap().push(serde_json::json!({"tool":{"driver":{"name":"other"}},
                "results":[{"ruleId":"unrelated","level":"warning","message":{"text":"Keep me"}}]}));
        }
        write_json(&source, "old.sarif", &sarif);
        let limits = Limits::default();
        let (bundles, notes) = discovery::discover(&[source.as_path()], 8);
        assert!(notes.is_empty());
        let bundle = &bundles[0];
        let (manifest, payloads) =
            manifest::build(&source, &bundle.source_dir, &bundle.files, &limits).unwrap();
        let context = adapters::ParseContext {
            bundle_id: &manifest.bundle_id,
            manifest: &manifest,
            payloads: &payloads,
            limits: &limits,
            };
        // Reproduce v2's ordinary-SARIF canonical records. Masking only the tag
        // bypasses v3 dispatch; restoring the original header yields v2 content.
        let mut masked = sarif.clone();
        masked["runs"][0]["properties"]["oviraptorSourceReview"]
            .as_object_mut()
            .unwrap()
            .remove("format");
        let (mut records, notes) =
            adapters::sarif::records(&context, "old.sarif", &serde_json::to_vec(&masked).unwrap());
        assert!(notes.is_empty());
        for record in &mut records {
            record.provenance.adapter_version = 2;
            if case == "ambiguous"
                && record.record_kind == RecordKind::FindingCandidate
                && record
                    .provenance
                    .source_record_pointer
                    .starts_with("/runs/0")
            {
                record.conflicts.push(
                    serde_json::json!({"field":"severity","value":"low","origin":"other-adapter"}),
                );
                record.finalize_revision_hash();
            }
            if record.record_kind == RecordKind::RunState {
                record.extensions["sarif_run_properties"] = sarif["runs"][0]["properties"].clone();
                record.finalize_revision_hash();
            }
        }
        let old_scope = scope::resolve(&source, bundle, &payloads);
        let old_key = old_scope.reconcile_key();
        let row_id = store::write_bundle(
            &db,
            &manifest.bundle_id,
            &source,
            &source,
            "",
            0,
            "adapter=2",
            "imported",
            records.len(),
            records.len(),
        )
        .unwrap();
        let old_native_keys = records
            .iter()
            .filter(|r| r.provenance.source_record_pointer.starts_with("/runs/0"))
            .map(|r| r.logical_key.clone())
            .collect::<Vec<_>>();
        let origin = store::MembershipOrigin {
            scope_key: &old_key,
            source_path: &source,
            attempt_number: old_scope.attempt_number,
        };
        for record in &records {
            let (revision, _) =
                store::insert_revision(&db, record, &serde_json::to_string(record).unwrap())
                    .unwrap();
            store::add_membership(&db, &origin, record, revision, row_id, "").unwrap();
        }
        let old_revision_count = table_count(&db, "import_record_revisions");
        if case == "moved" {
            fs::rename(source.join("old.sarif"), source.join("renamed.sarif")).unwrap();
        }
        let result = import_at(&db, &root);
        if case == "ambiguous" {
            assert_eq!(result.outcomes[0].status, BundleStatus::Failed);
            assert!(result.outcomes[0]
                .diagnostics
                .iter()
                .any(|d| d.detail.contains("source_report_legacy_merge_ambiguous")));
            assert_eq!(
                table_count(&db, "import_record_revisions"),
                old_revision_count
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM import_projection_memberships WHERE current=1",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                3,
                "migration demotions must roll back with the transaction"
            );
            drop(db);
            fs::remove_dir_all(root).unwrap();
            continue;
        }
        assert_eq!(
            result.outcomes[0].status,
            BundleStatus::Imported,
            "{result:?}"
        );
        assert!(result.outcomes[0].revoked >= 2);
        for key in old_native_keys {
            assert_eq!(db.query_row("SELECT count(*) FROM import_projection_memberships WHERE current=1 AND logical_key=?1",[key],|r|r.get::<_,i64>(0)).unwrap(),0);
        }
        let current = current_envelopes(&db, RecordKind::FindingCandidate);
        assert_eq!(current.len(), if case == "native_only" { 1 } else { 2 });
        if case != "native_only" {
            assert!(current
                .iter()
                .any(|r| r["payload"]["rule_id"] == "unrelated"));
        }
        assert_eq!(db.query_row("SELECT count(*) FROM import_projection_memberships WHERE current=1 AND scope_key='scan=task-a'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert!(table_count(&db, "import_record_revisions") > old_revision_count);
        assert_eq!(
            import_at(&db, &root).outcomes[0].status,
            BundleStatus::Unchanged
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_reports_deleted_task_stays_deleted_and_empty_reviews_remain_history() {
    let root = sandbox("source-deleted");
    let source = source_dir(&root);
    let db = open_connection(&initialize_db(&root));
    let report = source_report_fixture("deleted-task", 1, "root-a");
    db.execute(
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('deleted-task')",
        [],
    )
    .unwrap();
    write_json(&source, "source-review-deleted.json", &report);
    write_json(
        &source,
        "source-review-deleted.sarif",
        &source_sarif_fixture(&report),
    );
    let mut empty = source_report_fixture("empty-task", 1, "root-a");
    empty["review"]["findings"] = serde_json::json!([]);
    empty["review"]["counts"]["confirmed"] = serde_json::json!(0);
    write_json(&source, "source-review-empty.json", &empty);
    write_json(
        &source,
        "source-review-empty.sarif",
        &source_sarif_fixture(&empty),
    );
    let result = import_at(&db, &root);
    assert_eq!(result.outcomes[0].status, BundleStatus::Imported);
    assert!(result.outcomes[0]
        .diagnostics
        .iter()
        .any(|d| d.code == "deleted_scan_not_projected"));
    assert!(current_envelopes(&db, RecordKind::FindingCandidate).is_empty());
    let runs = current_envelopes(&db, RecordKind::RunState);
    assert_eq!(runs.len(), 2);
    assert!(runs
        .iter()
        .all(|r| r["extensions"]["scanId"] == "empty-task"));
    assert_eq!(table_count(&db, "import_bundle_files"), 4);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
