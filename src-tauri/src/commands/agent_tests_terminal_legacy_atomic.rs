// Faults only in disposable full application DB after the canonical bill/publication.
fn terminal_legacy_fault(sql: &str) {
    let (fixture, context, owned, calls) = original_terminal_dispatch(false);
    assert_eq!(calls, 0);
    record_runtime_terminal_facts_original(
        &fixture.path,
        &fixture.scan,
        &context.route,
        &owned.original_terminal,
        &owned.outcome,
    )
    .unwrap()
    .unwrap();
    let db = db::open(&fixture.path).unwrap();
    db.execute_batch(sql).unwrap();
    let before = single_finally_physical(&db);
    let mut tally = AgentPipelineTally::default();
    assert!(
        !record_owned_agent_target_outcome(
            &fixture.path,
            &fixture.scan,
            &context.route,
            &owned,
            &mut tally
        ),
        "failed legacy projection must stop instead of claiming a committed outcome"
    );
    assert_eq!(
        tally.counted(),
        0,
        "no count without complete legacy projection"
    );
    assert_eq!(
        single_finally_physical(&db),
        before,
        "canonical paid facts preserved and partial projection rolled back"
    );
}
#[test]
fn terminal_legacy_atomic_ignored_terminal_checkpoint_rolls_back_target_and_tally() {
    terminal_legacy_fault("CREATE TRIGGER legacy_ignore BEFORE INSERT ON sentinel_checkpoints WHEN NEW.stage='agent_terminal' BEGIN SELECT RAISE(IGNORE); END");
}
#[test]
fn terminal_legacy_atomic_ignored_target_rolls_back_both_checkpoints_and_tally() {
    terminal_legacy_fault("CREATE TRIGGER legacy_ignore BEFORE UPDATE ON sentinel_targets BEGIN SELECT RAISE(IGNORE); END");
}
#[test]
fn terminal_legacy_atomic_checkpoint_collateral_cannot_touch_business_records() {
    terminal_legacy_fault("CREATE TRIGGER legacy_collateral AFTER INSERT ON sentinel_checkpoints WHEN NEW.stage='agent_terminal' BEGIN UPDATE projects SET name='foreign projection side effect'; END");
}
#[test]
fn terminal_legacy_atomic_real_rotation_after_publication_changes_no_current_legacy_row() {
    let (fixture, context, owned, calls) = original_terminal_dispatch(false);
    assert_eq!(calls, 0);
    let (reached_tx, reached_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let path = fixture.path.clone();
    let scan = fixture.scan.clone();
    let route = context.route.clone();
    let worker = thread::spawn(move || {
        ORIGINAL_TERMINAL_LEGACY_PAUSE.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                reached_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
            }))
        });
        let mut tally = AgentPipelineTally::default();
        let continued = record_owned_agent_target_outcome(&path, &scan, &route, &owned, &mut tally);
        (continued, tally.counted())
    });
    reached_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    let db = db::open(&fixture.path).unwrap();
    // Real concurrent writer while actual consumer is paused after original commit.
    // This isolated mutation is not full user Retry/startup acceptance.
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
        [&fixture.scan],
    )
    .unwrap();
    let before = single_finally_physical(&db);
    release_tx.send(()).unwrap();
    let result = worker.join().unwrap();
    assert_eq!(
        result,
        (false, 0),
        "old result cannot project after current attempt changed"
    );
    assert_eq!(single_finally_physical(&db), before);
}

#[test]
fn terminal_legacy_atomic_saved_owned_replay_keeps_rows_and_counts_original_root_once() {
    let (fixture, context, owned, calls) = original_terminal_dispatch(false);
    assert_eq!(calls, 0);
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &fixture.path,
        &fixture.scan,
        &context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    let db = db::open(&fixture.path).unwrap();
    let before = single_finally_physical(&db);
    assert!(record_owned_agent_target_outcome(
        &fixture.path,
        &fixture.scan,
        &context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(
        single_finally_physical(&db),
        before,
        "Saved projection must not rewrite physical rows or timestamps"
    );
    assert_eq!(
        tally.counted(),
        1,
        "the same original Root may not count twice"
    );
}
#[test]
fn terminal_legacy_atomic_private_writer_adaptive_and_fuse_faults_rollback_all_rows() {
    // Lower writer contract only: these synthetic payloads are not execution or
    // WAF evidence. Original identity itself comes from actual admitted entry.
    for sql in [
        "CREATE TRIGGER legacy_fault BEFORE INSERT ON sentinel_checkpoints WHEN NEW.stage='adaptive_routing' BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER legacy_fault BEFORE INSERT ON sentinel_fuse_zone BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER legacy_fault AFTER INSERT ON sentinel_fuse_zone BEGIN UPDATE projects SET name='foreign fuse write'; END",
    ] {
        let (fixture,context,owned,_)=original_terminal_dispatch(false);
        record_runtime_terminal_facts_original(&fixture.path,&fixture.scan,&context.route,&owned.original_terminal,&owned.outcome).unwrap();
        let db=db::open(&fixture.path).unwrap();db.execute_batch(sql).unwrap();
        let before=single_finally_physical(&db);
        // Original producer already stored the initial adaptive route. Request
        // a changed projection so this fault actually reaches its write.
        let mut route=context.route.clone();route.reasons.push("changed lower writer projection".into());
        let result=write_terminal_legacy_projection(&fixture.path,&fixture.scan,&owned.original_terminal,&route,"failed",Some(&serde_json::json!({"fixture":"lower writer only"})),Some("lower writer atomicity"));
        assert!(result.is_err(),"fault must reject complete projection: {sql}");
        assert_eq!(single_finally_physical(&db),before);
    }
}
#[test]
fn terminal_legacy_atomic_private_fuse_upsert_saved_and_update_fault_preserve_original_rows() {
    let (fixture, context, owned, _) = original_terminal_dispatch(false);
    record_runtime_terminal_facts_original(
        &fixture.path,
        &fixture.scan,
        &context.route,
        &owned.original_terminal,
        &owned.outcome,
    )
    .unwrap();
    let payload = serde_json::json!({"fixture":"private writer only"});
    let write = |reason| {
        write_terminal_legacy_projection(
            &fixture.path,
            &fixture.scan,
            &owned.original_terminal,
            &context.route,
            "failed",
            Some(&payload),
            Some(reason),
        )
    };
    write("first lower writer projection").unwrap();
    let db = db::open(&fixture.path).unwrap();
    let before = single_finally_physical(&db);
    write("first lower writer projection").unwrap();
    assert_eq!(single_finally_physical(&db), before);
    db.execute_batch("CREATE TRIGGER legacy_fault BEFORE UPDATE ON sentinel_fuse_zone BEGIN SELECT RAISE(IGNORE); END").unwrap();
    assert!(write("changed lower writer projection").is_err());
    assert_eq!(single_finally_physical(&db), before);
}
