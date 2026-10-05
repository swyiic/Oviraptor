// Real provider arrival proves the original model request remains in flight.
fn ordered_inflight_closure_case(second: bool) {
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive_tx, arrive) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let release = Arc::new(Mutex::new(release));
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        if second && count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return (200, "application/json", proposal_model_response(valid_proposal_text()));
        }
        let _ = arrive_tx.send(());
        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200, "application/json", proposal_model_response(valid_proposal_text()))
    }));
    let (f, id) = ordered_exec_fixture("ordered-inflight-close", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    if second { ordered_exec_apply(&f).unwrap(); }
    let paid = ordered_exec_receipts(&db, &id);
    let native = ordered_exec_native(&db, &f.actor.root_run_id);
    let outcome = AgentTargetOutcome::incomplete("original ordered transport must exit before terminal publication");
    let (closing, status, returned, unchanged) = std::thread::scope(|scope| {
        let (done_tx, done) = mpsc::channel();
        let f = &f;
        scope.spawn(move || { let _ = done_tx.send(ordered_exec_apply(f)); });
        arrive.recv_timeout(Duration::from_secs(5)).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let closing = finish_coordinator_run(&db, &f.actor, &outcome);
        let unchanged = super::tests::application_table_snapshot(&db) == before;
        let status: String = db.query_row("SELECT status FROM agent_runs WHERE id=?1", [&f.actor.root_run_id], |r|r.get(0)).unwrap();
        let child: String = db.query_row("SELECT child_run_id FROM agent_directive_ordered_actions WHERE directive_id=?1 AND state='executing'", [&id], |r|r.get(0)).unwrap();
        // Use the original child stop path to exit transport; do not release
        // the blocked provider response until the SDK cancellation has returned.
        db.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1", [&child]).unwrap();
        let early = done.recv_timeout(Duration::from_secs(3));
        release_tx.send(()).unwrap();
        let returned = match early { Ok(result) => Some(result), Err(_) => { let _ = done.recv_timeout(Duration::from_secs(5)); None } };
        (closing, status, returned, unchanged)
    });
    assert!(closing.is_err(), "Root published terminal while its original SDK transport was still awaiting the provider");
    assert!(closing.unwrap_err().contains("ordered_transport_not_idle"));
    assert!(unchanged, "busy original transport must roll back every application row");
    assert_eq!(status, "running");
    assert!(returned.is_some_and(|r|r.is_err()), "original cancellation must exit before provider release");
    let job = crate::agent_runtime::multi_agent::directive::ordered_execution::load(&db, &id, if second { 2 } else { 1 }).unwrap().unwrap();
    let fee = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid", [&job.child.assignment_id]).unwrap();
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid", [&job.child.assignment_id]).unwrap()==fee);
    assert_eq!(ordered_exec_projection(&db, &id)["actions"][usize::from(second)]["state"], "outcome_unknown");
    assert_eq!(ordered_exec_receipts(&db, &id), paid);
    assert_eq!(ordered_exec_native(&db, &f.actor.root_run_id), native);
    let closed = super::tests::application_table_snapshot(&db);
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == closed);
    let capture_deadline = std::time::Instant::now()+Duration::from_secs(1);
    while seen.lock().unwrap().is_empty() && std::time::Instant::now()<capture_deadline { std::thread::yield_now(); }
    assert_eq!(seen.lock().unwrap().len(), if second { 2 } else { 1 });
}

#[test]
fn ordered_inflight_actual_terminal_waits_for_original_transport_exit_and_preserves_unknown_fee() {
    ordered_inflight_closure_case(false);
}
#[test]
fn ordered_inflight_second_actual_call_cannot_close_or_change_first_paid_receipt() {
    ordered_inflight_closure_case(true);
}

fn ordered_inflight_lock_path(f: &RootTickFixture, child: &str) -> PathBuf {
    use sha2::Digest;
    let key = serde_json::to_vec(&(&f.actor.scan_id, f.actor.attempt_number, "ordered-assessment-sdk", child)).unwrap();
    let db = fs::canonicalize(&f.context.db_path).unwrap();
    let mut name = db.file_name().unwrap().to_os_string(); name.push(".invocations");
    db.with_file_name(name).join(format!("{:x}.lock", sha2::Sha256::digest(key)))
}
#[test]
fn ordered_inflight_missing_original_lock_cannot_be_replaced_by_another_idle_owner() {
    for foreign in [false, true] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (503, "application/json", r#"{"error":{"message":"fixture unavailable"}}"#.into())));
        let (f, id) = ordered_exec_fixture("ordered-inflight-missing-owner", &format!("http://127.0.0.1:{port}/v1"));
        assert!(ordered_exec_apply(&f).is_err());
        let db = db::open(&f.context.db_path).unwrap();
        let job = crate::agent_runtime::multi_agent::directive::ordered_execution::load(&db, &id, 1).unwrap().unwrap();
        let path = ordered_inflight_lock_path(&f, &job.child.run_id);
        assert!(path.is_file());
        // Explicit temporary fixture corruption after the actual SDK has returned.
        fs::remove_file(&path).unwrap();
        if foreign {
            drop(crate::agent_runtime::execution_owner::claim_native_invocation(
                &f.context.db_path, &f.actor.scan_id, f.actor.attempt_number,
                "ordered-assessment-sdk", "foreign-child",
            ).unwrap());
        }
        let before = super::tests::application_table_snapshot(&db);
        let error = finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("missing original lifetime proof")).unwrap_err();
        assert!(error.contains("ordered_transport_original_exit_proof_missing"), "{error}");
        assert!(super::tests::application_table_snapshot(&db)==before);
        assert!(!path.exists(), "probe must not manufacture an original exit proof");
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
#[test]
fn ordered_inflight_probe_does_not_create_files_for_a_pristine_unstarted_action() {
    let (f, id) = ordered_exec_fixture("ordered-inflight-no-create", "http://127.0.0.1:9/v1");
    let job = ordered_closure_prepared(&f);
    let path = ordered_inflight_lock_path(&f, &job.child.run_id);
    assert!(!path.exists());
    let db = db::open(&f.context.db_path).unwrap();
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("original action never dispatched")).unwrap();
    ordered_closure_assert_cancelled(&db, &job);
    assert!(!path.exists());
    assert_eq!(ordered_exec_projection(&db, &id)["actions"][0]["state"], "cancelled_before_dispatch");
}
#[cfg(unix)]
#[test]
fn ordered_inflight_symlink_directory_cannot_supply_an_original_exit_proof() {
    let (f, _id) = ordered_exec_fixture("ordered-inflight-symlink", "http://127.0.0.1:9/v1");
    let job = ordered_closure_prepared(&f);
    let path = ordered_inflight_lock_path(&f, &job.child.run_id);
    let parent = path.parent().unwrap();
    // Preserve the fixture parent's existing lock inodes while simulating a
    // damaged directory; restore them before asserting. No real data touched.
    let original = parent.with_extension("original-invocations");
    fs::rename(parent, &original).unwrap();
    // A symlink to an existing empty directory must fail even when the file is absent.
    let other = f.context.target_dir.join("fake-invocations");
    fs::create_dir_all(&other).unwrap();
    std::os::unix::fs::symlink(&other, parent).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let error = finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("foreign lifetime directory")).unwrap_err();
    fs::remove_file(parent).unwrap();
    fs::rename(&original, parent).unwrap();
    assert!(error.contains("native_invocation_directory_not_regular"), "{error}");
    assert!(super::tests::application_table_snapshot(&db)==before);
    assert!(!path.exists());
}
