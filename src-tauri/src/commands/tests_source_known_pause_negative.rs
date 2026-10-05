// The original failed SDK owner survives the actual branch Drop, so the real
// finalizer must roll back both Root and business pause. Nothing is fabricated.
fn source_known_pause_pending_fixture() -> (
    PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::execution_owner::NativeInvocationOwner,
) {
    let kept = std::rc::Rc::new(std::cell::RefCell::new(None));
    let retain = kept.clone();
    let (root, db, actor, result) = source_exhausted_pending_root_using(
        "tool_exhausted",
        "first_tool_received",
        5,
        |root, _, record, _, busy| {
            assert_eq!(
                request_sentinel_pause(&root.join("oviraptor.sqlite3"), &record.scan_id).unwrap(),
                1
            );
            *retain.borrow_mut() = busy.take();
            Ok(json!({}))
        },
    );
    result.unwrap();
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&actor.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pausing"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "running"
    );
    let owner = kept.borrow_mut().take().unwrap();
    (root, db, actor, owner)
}

#[test]
fn source_known_pause_actual_original_damage_or_revocation_keeps_every_physical_row() {
    for sql in [
        "UPDATE agent_runs SET cancel_requested_at='pause-revoked' WHERE role='coordinator'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1",
        "UPDATE agent_assignment_attempts SET failure_class='not original' WHERE state='failed'",
        "UPDATE agent_budget_ledger SET spent_requests=spent_requests-1",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET state='uncertain' WHERE round_number=3",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET response_hash='changed' WHERE round_number=3",
        "UPDATE agent_messages SET payload_json='{}' WHERE kind='evidence_summary'",
        "DROP TRIGGER source_runtime_no_update; UPDATE source_runtime_contracts SET contract_json=json_set(contract_json,'$.budget.tokenLimit',1)",
        "DROP TRIGGER analysis_view_no_update; UPDATE source_analysis_views SET manifest_digest=printf('%064d',1)",
        "DROP TRIGGER source_ci_policy_no_update; UPDATE source_ci_policies SET max_high=max_high+1",
        r#"INSERT INTO agent_events(run_id,sequence,event_type,payload_json) SELECT id,(SELECT coalesce(max(sequence),0)+1 FROM agent_events WHERE run_id=r.id),'terminal_reduced','{"sourceClosureVersion":4}' FROM agent_runs r WHERE role='coordinator'"#,
    ] {
        let (root,db,actor,busy)=source_known_pause_pending_fixture();
        drop(busy);
        db.execute_batch(sql).unwrap(); assert!(db.changes()>0,"fault must change an actual original: {sql}");
        let damaged=source_exit_snapshot(&db);
        assert!(finish_sentinel_pause(&root.join("oviraptor.sqlite3"),&actor.scan_id,1).is_err(),"{sql}");
        assert_eq!(source_exit_snapshot(&db),damaged,"denial changed original rows: {sql}");
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_known_pause_private_writer_faults_rollback_root_and_pause_and_preserve_assets() {
    let (root, db, actor, busy) = source_known_pause_pending_fixture();
    drop(busy);
    let path = root.join("oviraptor.sqlite3");
    let files = source_failed_deletion_cas_files(&root);
    assert!(!files.is_empty());
    db.execute_batch(
        "CREATE TABLE pause_business(payload TEXT); INSERT INTO pause_business VALUES('preserve');
        INSERT INTO assets(id,asset_key) VALUES(987654323,'pause-preserve')",
    )
    .unwrap();
    for sql in [
        "CREATE TRIGGER paid_pause_fault BEFORE UPDATE OF status ON agent_runs WHEN NEW.role='coordinator' BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER paid_pause_fault BEFORE UPDATE OF status ON sentinel_scans BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER paid_pause_fault BEFORE UPDATE OF status ON sentinel_scan_attempts BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER paid_pause_fault BEFORE INSERT ON agent_multi_exit_receipts BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER paid_pause_fault AFTER UPDATE OF status ON sentinel_scans BEGIN UPDATE pause_business SET payload='changed'; END",
        "CREATE TRIGGER paid_pause_fault AFTER UPDATE OF status ON sentinel_scan_attempts BEGIN DELETE FROM assets; END",
        "CREATE TRIGGER paid_pause_fault AFTER UPDATE OF status ON agent_runs BEGIN UPDATE sentinel_scan_attempts SET total_tokens_delta=999; END",
        "CREATE TRIGGER paid_pause_fault AFTER UPDATE OF status ON sentinel_scans BEGIN UPDATE agent_budget_ledger SET spent_requests=0; END",
    ] {
        db.execute_batch(sql).unwrap();let before=source_exit_snapshot(&db);
        assert!(finish_sentinel_pause(&path,&actor.scan_id,1).is_err(),"{sql}");
        assert_eq!(source_exit_snapshot(&db),before,"Root, wall, exit and pause must roll back together: {sql}");
        assert_eq!(source_failed_deletion_cas_files(&root),files);
        db.execute_batch("DROP TRIGGER paid_pause_fault").unwrap();
    }
    let before = source_exit_snapshot(&db);
    assert!(finish_sentinel_pause(&path, &actor.scan_id, 1).unwrap());
    let after = source_exit_snapshot(&db);
    for (name, rows) in &before {
        if name == "sqlite_sequence" {
            let original = rows
                .iter()
                .filter(|row| {
                    row[1] != rusqlite::types::Value::Text("agent_collaboration_events".into())
                })
                .collect::<Vec<_>>();
            let current = after
                .iter()
                .find(|(n, _)| n == name)
                .unwrap()
                .1
                .iter()
                .filter(|row| {
                    row[1] != rusqlite::types::Value::Text("agent_collaboration_events".into())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                current, original,
                "only the original Root collaboration sequence may advance"
            );
            let seq: i64 = db
                .query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name='agent_collaboration_events'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                seq,
                db.query_row(
                    "SELECT max(sequence) FROM agent_collaboration_events",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap()
            );
            continue;
        }
        if ![
            "agent_runs",
            "agent_budget_entries",
            "agent_multi_exit_receipts",
            "agent_collaboration_events",
            "sentinel_scans",
            "sentinel_scan_attempts",
            "native_scan_branches",
            "sentinel_targets",
        ]
        .contains(&name.as_str())
        {
            assert_eq!(
                after.iter().find(|(n, _)| n == name).unwrap().1,
                *rows,
                "foreign original table {name}"
            );
        }
    }
    source_pause_branch_assert_projection_delta(&db,&actor,&before);
    assert_eq!(
        db.query_row("SELECT payload FROM pause_business", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "preserve"
    );
    assert_eq!(
        db.query_row("SELECT asset_key FROM assets WHERE id=987654323", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "pause-preserve"
    );
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    let closed = source_exit_snapshot(&db);
    assert!(!finish_sentinel_pause(&path, &actor.scan_id, 1).unwrap());
    assert_eq!(source_exit_snapshot(&db), closed);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_known_pause_original_busy_or_missing_sdk_exit_cannot_be_repaired_or_closed() {
    let (root, db, actor, busy) = source_known_pause_pending_fixture();
    drop(busy);
    let path = root.join("oviraptor.sqlite3");
    let child: String = db
        .query_row(
            "SELECT child_run_id FROM agent_assignments WHERE state='failed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let initial: String = db
        .query_row(
            "SELECT child_run_id FROM agent_specialist_calls ORDER BY rowid LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for (kind, key) in [
        ("source-model", "source".to_string()),
        ("source-round-sdk", child),
        ("specialist-sdk", initial),
    ] {
        let owner = crate::agent_runtime::execution_owner::probe_native_invocation(
            &path,
            &actor.scan_id,
            1,
            kind,
            &key,
        )
        .unwrap()
        .unwrap();
        let before = source_exit_snapshot(&db);
        assert!(
            finish_sentinel_pause(&path, &actor.scan_id, 1).is_err(),
            "{kind}"
        );
        assert_eq!(source_exit_snapshot(&db), before, "{kind}");
        drop(owner);
    }
    use sha2::Digest;
    let canonical = fs::canonicalize(&path).unwrap();
    let mut directory = canonical.file_name().unwrap().to_os_string();
    directory.push(".invocations");
    let key = serde_json::to_vec(&(&actor.scan_id, 1, "source-model", "source")).unwrap();
    let original = canonical
        .with_file_name(directory)
        .join(format!("{:x}.lock", sha2::Sha256::digest(key)));
    assert!(original.is_file());
    fs::remove_file(&original).unwrap();
    let before = source_exit_snapshot(&db);
    assert_eq!(
        finish_sentinel_pause(&path, &actor.scan_id, 1).unwrap_err(),
        "scan_quiescence_original_source_exit_missing"
    );
    assert!(!original.exists());
    assert_eq!(source_exit_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
