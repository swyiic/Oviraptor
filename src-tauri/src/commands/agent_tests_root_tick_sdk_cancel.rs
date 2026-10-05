#[test]
fn native_sdk_log_coordinator_cancel_returns_before_provider_reply_under_original_uncertain_owner()
{
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive, arrived) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrive.send(());
        let _ = released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        (
            200,
            "application/json",
            sdk_exact_model_body(&root_tick_valid_text("late canceled body")),
        )
    }));
    let f = root_tick_fixture(
        "sdk-root-cancel-gate",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let (before, result) = std::thread::scope(|scope| {
        let (done, rx) = mpsc::channel();
        let context = &f.context;
        let actor = &f.actor;
        scope.spawn(move || {
            let _ = done.send(native_coordinator_tick(context, actor));
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        let before = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
        db.execute(
            "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
            [&f.context.scan_id],
        )
        .unwrap();
        let result = rx.recv_timeout(Duration::from_secs(2));
        let _ = release.send(());
        (before, result.unwrap())
    });
    assert!(result.is_err());
    assert_eq!(
        root_tick_sdk_stages(&root_tick_sdk_rows(&before, 1)),
        ["prepared", "sent"]
    );
    let page = root_tick_sdk_page(&f.context.db_path, &f.context.scan_id);
    let rows = root_tick_sdk_rows(&page, 1);
    assert_eq!(
        root_tick_sdk_stages(&rows),
        ["prepared", "sent", "cost_saved", "terminal"]
    );
    assert_eq!(rows.last().unwrap()["terminalState"], "uncertain");
    root_tick_sdk_assert_original(&db, &f.actor.root_run_id, &rows);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let snapshot = super::tests::application_table_snapshot(&db);
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), snapshot);
    assert_eq!(
        root_tick_sdk_page(&f.context.db_path, &f.context.scan_id),
        page
    );
}
