#[test]
fn assignment_attempt_supervisor_web_inflight_stop_retains_unknown_and_never_resends() {
    use crate::agent_runtime::multi_agent::{budget, supervisor::WorkerSupervisor};
    use std::sync::{atomic::Ordering, mpsc, Arc, Mutex};
    let (arrived_tx, arrived_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Arc::new(Mutex::new(release_rx));
    let (port, seen, stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrived_tx.send(());
        let _ = release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
        (
            200,
            "application/json",
            proposal_model_response("{\"summary\":\"late\"}"),
        )
    }));
    let (root, mut context, lease, child) = specialist_journal_fixture();
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    context.target_dir = root.join("worker");
    fs::create_dir_all(&context.target_dir).unwrap();
    let db = db::open(&context.db_path).unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    context.supervision = Some(guard.ticket());
    std::thread::scope(|scope| {
        let (done_tx, done_rx) = mpsc::channel();
        let (ctx, actor, worker) = (&context, &lease, &child);
        scope.spawn(move || {
            let _ = done_tx.send(multi_agent_child_round_transport(
                ctx,
                actor,
                worker,
                "readonly",
                json!({"fixed":true}),
            ));
        });
        arrived_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(guard);
        let result = done_rx.recv_timeout(Duration::from_secs(3));
        let _ = release_tx.send(());
        assert!(result
            .expect("parent stop cancels the real request before provider response")
            .is_err());
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while seen.lock().unwrap().is_empty() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    let cost = budget::balance(
        &db,
        &lease.root_run_id,
        Some(&child.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!((cost.consumed, cost.indeterminate), (0, 1));
    assert_eq!(
        budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "concurrency_batches"
        )
        .unwrap()
        .reserved,
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_messages WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let before = super::tests::application_table_snapshot(&db);
    assert!(multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        json!({"fixed":true})
    )
    .is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    stop.store(true, Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
