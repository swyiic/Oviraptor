// G3 only: gated actual changed Mapper fact SDK, then local original-frame replay.
#[test]
fn native_sdk_log_coordinator_changed_fact_real_gate_uses_next_original_call_and_zero_sdk_replay() {
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive, arrived) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |request| {
        if request.contains("You are the Root Coordinator") && request.contains("mapper-output") {
            let _ = arrive.send(());
            let _ = released
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture(
        "sdk-changed-fact-gate",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    drop(f.parent.take());
    let path = f.context.db_path.clone();
    let scan = f.context.scan_id.clone();
    let (live, result) = std::thread::scope(|scope| {
        let (done, rx) = mpsc::channel();
        let context = &mut f.context;
        scope.spawn(move || {
            let _real = RealSpecialistTransport::enter();
            let result = (|| {
                let mut session = multi_agent_prepare(context)?;
                let db = db::open(&context.db_path)?;
                let root = native_coordinator_root_context(context, &session.lease);
                let tx = rusqlite::Transaction::new_unchecked(
                    &db,
                    rusqlite::TransactionBehavior::Immediate,
                )
                .unwrap();
                let frame = NativeCoordinatorFrame::mapper(&tx, &session.lease, &session.mapper)?;
                tx.commit().unwrap();
                let before = root_tick_sdk_page(&context.db_path, &context.scan_id);
                let replay = native_coordinator_tick_for_frame(&root, &session.lease, &frame)?;
                let after = root_tick_sdk_page(&context.db_path, &context.scan_id);
                assert!(replay.replayed);
                assert_eq!(
                    before, after,
                    "same saved paid frame produces no fresh SDK diagnostic"
                );
                let root_id = session.lease.root_run_id.clone();
                multi_agent_finish_execution(
                    context,
                    &mut session,
                    &AgentTargetOutcome::incomplete("SDK trace gate only"),
                )?;
                Ok::<_, String>((after, root_id))
            })();
            let _ = done.send(result);
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        let live = root_tick_sdk_page(&path, &scan);
        assert!(rx.try_recv().is_err());
        let _ = release.send(());
        (live, rx.recv_timeout(Duration::from_secs(10)).unwrap())
    });
    let (done, root) = result.unwrap();
    assert_eq!(
        root_tick_sdk_stages(&root_tick_sdk_rows(&live, 1)),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "validated",
            "terminal"
        ]
    );
    let before = root_tick_sdk_rows(&live, 2);
    assert_eq!(root_tick_sdk_stages(&before), ["prepared", "sent"]);
    let after = root_tick_sdk_rows(&done, 2);
    assert_eq!(
        root_tick_sdk_stages(&after),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "validated",
            "terminal"
        ]
    );
    assert_ne!(
        before[0]["dispatchKey"],
        root_tick_sdk_rows(&live, 1)[0]["dispatchKey"]
    );
    let db = db::open(&path).unwrap();
    root_tick_sdk_assert_original(&db, &root, &before);
    root_tick_sdk_assert_original(&db, &root, &after);
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "original bootstrap + actual independent Mapper + changed Root, no fourth provider call"
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, &root, Some(""), "model_requests")
            .unwrap()
            .consumed,
        2
    );
}
