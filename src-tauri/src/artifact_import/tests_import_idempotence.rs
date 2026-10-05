// §13.2 幂等、事务与 reconciliation（§IDM-001 … §IDM-013）。

fn logical_keys(connection: &Connection, kind: RecordKind) -> Vec<String> {
    connection
        .prepare(
            "SELECT DISTINCT r.logical_key FROM import_projection_memberships m
                 JOIN import_record_revisions r ON r.id=m.revision_id
                 WHERE m.current=1 AND r.record_kind=?1 ORDER BY r.logical_key",
        )
        .unwrap()
        .query_map([kind.as_str()], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect()
}

fn single_finding_bundle(root: &Path, severity: &str) -> PathBuf {
    let bundle = source_dir(root).join("bundle");
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "越权", &[], Some(serde_json::json!({"severity":severity,"endpoint":"/api/v1/order","method":"GET","cwe":"CWE-639"})))]),
    );
    bundle
}

#[test]
fn importing_the_same_bundle_twice_adds_nothing() {
    let root = sandbox("idm001");
    single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let first = import_at(&connection, &root);
    let revisions = table_count(&connection, "import_record_revisions");
    let memberships = table_count(&connection, "import_projection_memberships");
    let objects = table_count(&connection, "artifact_objects");
    assert_eq!(first.outcomes[0].status, BundleStatus::Imported);
    let second = import_at(&connection, &root);
    assert_eq!(second.outcomes[0].status, BundleStatus::Unchanged);
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        revisions
    );
    assert_eq!(
        table_count(&connection, "import_projection_memberships"),
        memberships
    );
    assert_eq!(table_count(&connection, "artifact_objects"), objects);
    assert_eq!(table_count(&connection, "import_bundles"), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_same_bundle_in_another_directory_is_not_a_duplicate() {
    let root = sandbox("idm002");
    single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let revisions = table_count(&connection, "import_record_revisions");
    let keys = logical_keys(&connection, RecordKind::FindingCandidate);

    let moved = sandbox("idm002b");
    copy_tree(&root, &moved);
    import_at(&connection, &moved);
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        revisions,
        "同一内容换目录不能新增语义行"
    );
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate),
        keys
    );
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(moved);
}

#[test]
fn touching_mtime_does_not_reimport_but_a_same_length_change_does() {
    let root = sandbox("idm003");
    let bundle = single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let target = bundle.join("findings.sarif");
    assert!(target.is_file(), "改动前目标文件必须存在");
    let times = std::fs::FileTimes::new().set_modified(
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000),
    );
    std::fs::File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_times(times)
        .unwrap();
    let touched = import_at(&connection, &root);
    assert_eq!(
        touched.outcomes[0].status,
        BundleStatus::Unchanged,
        "§IDM-003：只改 mtime 不该重导"
    );
    // Same length, different content: must be re-imported.
    let changed = target.to_string_lossy().to_string().len();
    let original = fs::read_to_string(&target).unwrap();
    let swapped = original.replace("\"high\"", "\"info\"");
    assert_eq!(swapped.len(), original.len(), "样例必须等长变化");
    fs::write(&target, swapped).unwrap();
    let reimported = import_at(&connection, &root);
    assert_eq!(
        reimported.outcomes[0].status,
        BundleStatus::Imported,
        "等长内容变化必须重导（{changed}）"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn changing_any_single_format_reimports_that_bundle() {
    let formats = ["findings.sarif", "llm-hook.jsonl"];
    for format in formats {
        let root = sandbox("idm004");
        let bundle = source_dir(&root).join("bundle");
        write_json(
            &bundle,
            "model-prompt-audit.json",
            &serde_json::json!({"instruction":"r"}),
        );
        match format {
            "findings.sarif" => write_json(&bundle, format, &sarif_document(vec![])),
            "llm-hook.jsonl" => write_file(&bundle, format, "{\"call_id\":\"c1\"}\n"),
            _ => unreachable!("explicit current format list"),
        }
        let db_path = initialize_db(&root);
        let connection = open_connection(&db_path);
        assert_eq!(
            import_at(&connection, &root).outcomes[0].status,
            BundleStatus::Imported
        );
        match format {
            "findings.sarif" => write_json(
                &bundle,
                format,
                &sarif_document(vec![sarif_result(
                    "R-2",
                    "error",
                    "新增结果",
                    &[("src/b.py", 2)],
                    None,
                )]),
            ),
            "llm-hook.jsonl" => write_file(&bundle, format, "{\"call_id\":\"c1\"}\n{\"call_id\":\"c2\"}\n"),
            _ => unreachable!("explicit current format list"),
        }
        assert_eq!(
            import_at(&connection, &root).outcomes[0].status,
            BundleStatus::Imported,
            "§IDM-004：只改 {format} 也必须重导"
        );
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn reordering_the_finding_array_keeps_logical_keys_and_revisions() {
    let root = sandbox("idm005");
    let bundle = source_dir(&root).join("bundle");
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"endpoint":"/api/a","method":"GET","severity":"high"}))),
            sarif_result("v2", "warning", "乙", &[], Some(serde_json::json!({"endpoint":"/api/b","method":"GET","severity":"low"})))]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let before = logical_keys(&connection, RecordKind::FindingCandidate);
    let revisions = table_count(&connection, "import_record_revisions");
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v2", "warning", "乙", &[], Some(serde_json::json!({"endpoint":"/api/b","method":"GET","severity":"low"}))),
            sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"endpoint":"/api/a","method":"GET","severity":"high"})))]),
    );
    import_at(&connection, &root);
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate),
        before,
        "数组顺序不能改变逻辑 key"
    );
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        revisions,
        "顺序变化不该产生新修订"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_field_change_adds_a_revision_and_keeps_the_old_one() {
    let root = sandbox("idm006");
    let bundle = single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let first = logical_keys(&connection, RecordKind::FindingCandidate);
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "越权", &[], Some(serde_json::json!({"severity":"critical","endpoint":"/api/v1/order","method":"GET","cwe":"CWE-639"})))]),
    );
    import_at(&connection, &root);
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate),
        first
    );
    assert!(
        table_count(&connection, "import_record_revisions") >= 2,
        "字段变化必须新增修订"
    );
    let severities: Vec<String> = connection
            .prepare("SELECT envelope_json FROM import_record_revisions WHERE record_kind='finding_candidate'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(Result::ok)
            .map(|text| {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                payload_of(&value)
                    .get("severity")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
    assert!(
        severities.contains(&"high".to_string()),
        "旧修订必须还在：{severities:?}"
    );
    assert!(severities.contains(&"critical".to_string()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rewriting_the_history_but_not_the_projection_is_refused() {
    let root = sandbox("append");
    single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    assert_eq!(table_count(&connection, "import_record_revisions"), 1);
    let update = connection.execute(
        "UPDATE import_record_revisions SET revision_hash='tampered' WHERE id=1",
        [],
    );
    assert!(
        update.is_err(),
        "§9.9：原始修订必须只可追加，实际 {update:?}"
    );
    let delete = connection.execute("DELETE FROM import_record_revisions WHERE id=1", []);
    assert!(delete.is_err(), "历史修订不可删除，实际 {delete:?}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_removed_finding_is_revoked_without_losing_history() {
    let root = sandbox("idm007");
    let bundle = source_dir(&root).join("bundle");
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"endpoint":"/api/a","method":"GET","severity":"high"}))),
            sarif_result("v2", "warning", "乙", &[], Some(serde_json::json!({"endpoint":"/api/b","method":"GET","severity":"low"})))]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate).len(),
        2
    );
    let revisions = table_count(&connection, "import_record_revisions");
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"endpoint":"/api/a","method":"GET","severity":"high"})))]),
    );
    import_at(&connection, &root);
    let current = logical_keys(&connection, RecordKind::FindingCandidate);
    assert_eq!(current.len(), 1, "§IDM-007：消失的条目要撤销当前投影");
    let tombstones: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM import_projection_memberships WHERE tombstone=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tombstones, 1, "撤销要留下墓碑");
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        revisions,
        "撤销不能删除历史修订"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn two_threads_importing_the_same_bundle_commit_once() {
    let root = sandbox("idm008");
    let bundle = single_finding_bundle(&root, "high");
    // A real file database, so the second writer has to cope with a lock.
    let initialized = initialize_db(&root);
    let (left_summary, right_summary) = std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            let connection = open_connection(&initialized);
            import_at(&connection, root.as_path())
        });
        let right = scope.spawn(|| {
            let connection = open_connection(&initialized);
            import_at(&connection, root.as_path())
        });
        (left.join().unwrap(), right.join().unwrap())
    });
    let revisions = table_count(&open_connection(&initialized), "import_record_revisions");
    assert_eq!(revisions, 1, "并发导入只能 commit 一次：{revisions}");
    assert_eq!(
        table_count(
            &open_connection(&initialized),
            "import_projection_memberships"
        ),
        1
    );
    let mut left_codes = codes(&left_summary);
    left_codes.extend(codes(&right_summary));
    assert!(
        !left_summary
            .outcomes
            .iter()
            .chain(right_summary.outcomes.iter())
            .any(|outcome| outcome.status == BundleStatus::Failed),
        "§IDM-008/§IDM-010：并发导入不得留下失败或锁错误：{left_codes:?}"
    );
    assert!(
        !left_codes.iter().any(|code| code == "bundle_failed"),
        "§IDM-010：DB busy 必须有界重试而不是失败：{left_codes:?}"
    );
    assert!(bundle.is_dir());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_locked_database_is_retried_within_the_bound_and_commits_once() {
    let root = sandbox("idm010");
    single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let held = db_path.clone();
    // Another writer takes the SQLite write lock first and releases it shortly
    // after, so the importer has to survive a real SQLITE_BUSY, not just wait out
    // the 10 s busy_timeout that `db::open` installs.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let connection = open_connection(&held);
        connection.pragma_update(None, "busy_timeout", 0).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        let _ = ready_tx.send(());
        std::thread::sleep(std::time::Duration::from_millis(40));
        drop(connection);
    });
    ready_rx.recv().expect("写锁必须真的持有");
    let connection = open_connection(&db_path);
    connection
        .pragma_update(None, "busy_timeout", 0)
        .expect("导入侧同样不等待");
    let summary = import_at(&connection, root.as_path());
    let outcome = &summary.outcomes[0];
    let codes = codes(&summary);
    assert_ne!(
        outcome.status,
        BundleStatus::Failed,
        "§IDM-010：锁只应触发有界重试，不应失败：{codes:?}"
    );
    assert!(
        !codes.iter().any(|code| code == "bundle_failed"),
        "§IDM-010：重试成功后不得留下 bundle_failed：{codes:?}"
    );
    assert_eq!(outcome.revisions, 1, "一次导入只写一条修订");
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        1,
        "§IDM-010：重试不得产生重复修订"
    );
    assert_eq!(
        table_count(&connection, "import_projection_memberships"),
        1,
        "§IDM-010：重试不得产生重复投影"
    );
    holder.join().unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_failed_transaction_leaves_no_half_import() {
    let root = sandbox("idm009");
    single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    // Object storage stays outside the tree that is being imported.
    let cas = root.join("cas");
    let key_path = root.join("artifact-import.key");
    // Break the projection table so the commit fails after the CAS write.
    connection
        .execute("DROP TABLE import_projection_memberships", [])
        .unwrap();
    let context = ImportContext {
        connection: &connection,
        cas_dir: &cas,
        key_path: &key_path,
        roots: std::slice::from_ref(&root),
        limits: &Limits::default(),
    };
    let summary = import_roots(&context);
    assert_eq!(summary.outcomes[0].status, BundleStatus::Failed);
    assert!(has_code(&summary, "bundle_failed"), "{:?}", codes(&summary));
    assert_eq!(
        table_count(&connection, "import_bundles"),
        0,
        "§IDM-009：事务失败不得留下 bundle 行"
    );
    assert_eq!(
        table_count(&connection, "import_record_revisions"),
        0,
        "§IDM-009：事务失败不得留下半成品修订"
    );
    // 缺口 7：原文行也必须一起回滚，任何查询面都看不到失败的导入。
    assert_eq!(
        table_count(&connection, "artifact_objects"),
        0,
        "§IDM-009：对象行必须与投影同一事务"
    );
    assert_eq!(
        table_count(&connection, "import_bundle_files"),
        0,
        "§IDM-009：不得留下文件清单残行"
    );
    assert_eq!(
        table_count(&connection, "import_diagnostics"),
        0,
        "§IDM-009：诊断行同样属于这次事务"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_older_attempt_never_rolls_back_the_latest_one() {
    let root = sandbox("idm011");
    let newest = source_dir(&root).join("attempt-0002");
    write_json(
        &newest,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"endpoint":"/api/a","method":"GET","severity":"high"}))),
            sarif_result("v2", "warning", "乙", &[], Some(serde_json::json!({"endpoint":"/api/b","method":"GET","severity":"low"})))]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let current = logical_keys(&connection, RecordKind::FindingCandidate);
    assert_eq!(current.len(), 2);
    let older = source_dir(&root).join("attempt-0001");
    write_json(
        &older,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"endpoint":"/api/a","method":"GET","severity":"high"})))]),
    );
    let summary = import_at(&connection, &root);
    assert!(
        has_code(&summary, "older_attempt_ignored"),
        "旧 attempt 重导必须被识别：{:?}",
        codes(&summary)
    );
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate),
        current,
        "§IDM-011：最新 attempt 的投影不能被回滚"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn records_the_legacy_writer_already_wrote_are_adopted_not_duplicated() {
    let root = sandbox("idm012");
    let bundle = single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let key = logical_keys(&connection, RecordKind::FindingCandidate)[0].clone();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(8201,'Adopt')", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) \
                 VALUES('adopt-scan',8201,'Adopt','completed','web')",
            [],
        )
        .unwrap();
    connection
            .execute(
                "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) \
                 VALUES('adopt-scan','https://fixture.invalid','s3','vulnerability',?1,'旧导入器已写')",
                [&key],
            )
            .unwrap();
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "越权", &[], Some(serde_json::json!({"severity":"critical","endpoint":"/api/v1/order","method":"GET","cwe":"CWE-639"})))]),
    );
    import_at(&connection, &root);
    let adopted: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM import_projection_memberships WHERE adopted_from='legacy_sentinel_findings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(adopted, 1, "§IDM-012：旧导入器写过的记录要被认领");
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_findings WHERE record_key=?1",
                [&key],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1,
        "认领不得复制第二条历史结果"
    );
    let _ = fs::remove_dir_all(root);
}

/// Two runs under one search root are two scans: importing the second must not
/// revoke what the first projected (§12 Stage 2 scope 缺口 1).
#[test]
fn sibling_bundles_under_one_root_do_not_revoke_each_other() {
    let root = sandbox("idm014");
    for (name, endpoint) in [("run-a", "/api/a"), ("run-b", "/api/b")] {
        write_json(
            &source_dir(&root).join(name),
            "model-prompt-audit.json",
            &serde_json::json!({"instruction":name}),
        );
        write_json(
        &source_dir(&root).join(name),
        "findings.sarif",
        &sarif_document(vec![sarif_result(&format!("v-{name}"), "warning", "越权", &[], Some(serde_json::json!({"endpoint":endpoint,"method":"GET","cwe":"CWE-639"})))]),
    );
    }
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    let summary = import_at(&connection, &root);
    assert_eq!(summary.outcomes.len(), 2, "{:?}", codes(&summary));
    let keys = logical_keys(&connection, RecordKind::FindingCandidate);
    assert_eq!(keys.len(), 2, "两个 sibling 必须各自保留投影：{keys:?}");
    // 再导一次也不能互相撤销。
    import_at(&connection, &root);
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate),
        keys,
        "重复导入后 sibling 投影必须不变"
    );
    let _ = fs::remove_dir_all(root);
}

/// Once the latest attempt deletes a finding, its tombstone has to keep an older
/// attempt from projecting it again (缺口 4).
#[test]
fn a_deleted_latest_attempt_tombstone_blocks_an_older_attempt() {
    let root = sandbox("idm015");
    let newest = source_dir(&root).join("attempt-0002");
    write_json(
        &newest,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"endpoint":"/api/a","method":"GET","cwe":"CWE-639"})))]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate).len(),
        1
    );
    // The newest attempt drops the finding: the projection is revoked and a
    // tombstone is left behind.
    write_json(
        &newest,
        "findings.sarif",
        &sarif_document(vec![]),
    );
    let revoked = import_at(&connection, &root);
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate).len(),
        0,
        "{:?}",
        codes(&revoked)
    );
    // An older attempt that still carries the finding must not resurrect it.
    write_json(
        &source_dir(&root).join("attempt-0001"),
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"endpoint":"/api/a","method":"GET","cwe":"CWE-639"})))]),
    );
    let summary = import_at(&connection, &root);
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate),
        Vec::<String>::new(),
        "§IDM-011：tombstone 之后旧 attempt 不得复活投影：{:?}",
        codes(&summary)
    );
    assert!(
        has_code(&summary, "older_attempt_ignored")
            || has_code(&summary, "tombstone_blocks_restore"),
        "必须有明确诊断说明为何没有投影：{:?}",
        codes(&summary)
    );
    let _ = fs::remove_dir_all(root);
}

/// A scan the user deleted may be re-read from disk but must never project again
/// (缺口 5).
#[test]
fn a_deleted_scan_is_never_projected_again() {
    let root = sandbox("idm016");
    let bundle = source_dir(&root).join("run");
    write_json(
        &bundle,
        "model-prompt-audit.json",
        &serde_json::json!({"instruction":"gone"}),
    );
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"endpoint":"/api/a","method":"GET","cwe":"CWE-639"})))]),
    );
    fs::write(bundle.join(".oviraptor-scan-id"), "deleted-scan\n").unwrap();
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    connection
        .execute(
            "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('deleted-scan')",
            [],
        )
        .unwrap();
    let summary = import_at(&connection, &root);
    assert_eq!(
        logical_keys(&connection, RecordKind::FindingCandidate),
        Vec::<String>::new(),
        "已删除任务不得产生 current projection：{:?}",
        codes(&summary)
    );
    assert!(
        has_code(&summary, "deleted_scan_not_projected"),
        "必须报告为何跳过投影：{:?}",
        codes(&summary)
    );
    assert_eq!(
        table_count(&connection, "import_bundles"),
        1,
        "bundle 仍要登记，审计才知道见过这批结果"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn retired_scan_alias_cannot_hide_an_unrelated_current_projection() {
    let root = sandbox("idm016-legacy-ui");
    let bundle = source_dir(&root).join("run");
    write_json(
        &bundle,
        "model-prompt-audit.json",
        &serde_json::json!({"instruction":"legacy-gone"}),
    );
    write_json(
        &bundle,
        "findings.sarif",
        &sarif_document(vec![sarif_result("v1", "warning", "甲", &[], Some(serde_json::json!({"severity":"high"})))]),
    );
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    connection
        .execute(
            "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('strix-legacy-gone')",
            [],
        )
        .unwrap();
    write_file(&bundle, ".oviraptor-scan-id", "legacy-gone\n");
    let summary = import_at(&connection, &root);
    assert!(
        !logical_keys(&connection, RecordKind::FindingCandidate).is_empty(),
        "a retired alias must not hide a different exact task ID: {:?}",
        codes(&summary)
    );
    assert!(!has_code(&summary, "deleted_scan_not_projected"));
    assert!(!store::scan_marked_deleted(&connection, "legacy-gone").unwrap());
    assert!(store::scan_marked_deleted(&connection, "strix-legacy-gone").unwrap());
    assert_eq!(table_count(&connection, "sentinel_deleted_scans"), 1);
    assert_eq!(table_count(&connection, "import_bundles"), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_shadow_report_separates_matched_and_one_sided_rows() {
    let root = sandbox("shadow");
    single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    import_at(&connection, &root);
    let key = logical_keys(&connection, RecordKind::FindingCandidate)[0].clone();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(8210,'Shadow')", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) \
                 VALUES('shadow-scan',8210,'Shadow','completed','web')",
            [],
        )
        .unwrap();
    // One legacy row the canonical side also produces, one it never will, and one
    // non-vulnerability row that must stay out of the comparison.
    for (record_key, title, kind) in [
        (key.as_str(), "两侧都有", "vulnerability"),
        ("sha256:only-legacy", "只有旧导入器写过", "vulnerability"),
        ("sha256:coverage-row", "覆盖度不是漏洞", "coverage"),
    ] {
        connection
            .execute(
                "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) \
                     VALUES('shadow-scan','https://fixture.invalid','s3',?1,?2,?3)",
                rusqlite::params![kind, record_key, title],
            )
            .unwrap();
    }
    let summary = import_at(&connection, &root);
    assert_eq!(
        summary.shadow.canonical_candidates, 1,
        "{:?}",
        summary.shadow
    );
    assert_eq!(summary.shadow.legacy_findings, 2);
    assert_eq!(summary.shadow.matched, 1);
    assert_eq!(summary.shadow.only_canonical, 0);
    assert_eq!(summary.shadow.only_legacy, 1);
    assert_eq!(
        table_count(&connection, "sentinel_findings"),
        3,
        "§12 Stage 2：shadow compare 只读，不改历史表"
    );
    assert_eq!(
        table_count(&connection, "import_projection_memberships"),
        1,
        "对账不得往当前投影里塞第二行"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn importing_cannot_touch_an_active_attempt() {
    let root = sandbox("idm013");
    single_finding_bundle(&root, "high");
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    connection
        .execute("INSERT INTO projects(id,name) VALUES(8202,'Active')", [])
        .unwrap();
    connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,scan_type,attempt_count) \
                 VALUES('active-scan',8202,'Active','scanning','正在执行','web',1)",
                [],
            )
            .unwrap();
    connection
            .execute(
                "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status,stage,checkpoint,stop_reason,work_dir,backend_plan_json,total_tokens_delta) \
                 VALUES('active-scan',1,'scanning','frontend_recon','进行中','','/tmp/active','{\"backend\":\"native\"}',120)",
                [],
            )
            .unwrap();
    connection
            .execute(
                "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,plan_hash,status,soft_token_budget,hard_token_budget,used_tokens) \
                 VALUES('active-run','active-scan',1,'https://fixture.invalid','native','coordinator','h','running',1000,2000,50)",
                [],
            )
            .unwrap();
    let before = connection
            .query_row(
                "SELECT a.status||'|'||a.stage||'|'||a.backend_plan_json||'|'||r.status||'|'||r.used_tokens||'|'||r.hard_token_budget||'|'||s.status
                 FROM sentinel_scan_attempts a JOIN agent_runs r ON r.scan_id=a.scan_id JOIN sentinel_scans s ON s.id=a.scan_id",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
    import_at(&connection, &root);
    let after = connection
            .query_row(
                "SELECT a.status||'|'||a.stage||'|'||a.backend_plan_json||'|'||r.status||'|'||r.used_tokens||'|'||r.hard_token_budget||'|'||s.status
                 FROM sentinel_scan_attempts a JOIN agent_runs r ON r.scan_id=a.scan_id JOIN sentinel_scans s ON s.id=a.scan_id",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
    assert_eq!(
        before, after,
        "§IDM-013：导入不得改变活动 attempt 的任何状态"
    );
    let _ = fs::remove_dir_all(root);
}
