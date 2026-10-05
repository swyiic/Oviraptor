#[test]
fn native_process_observer_cancellation_strong_kill_flushes_tail_and_stops_descendants() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let result = f.run(
        "printf 'ready\\n'; printf 'password=partial-synthetic-secret' >&2;
        (sleep 0.2; printf escaped > escaped) & printf ready > cancel; wait",
        &|| f.root.join("cancel").exists(),
        &mut |event| journal.append(event, &mut |_| Ok(())).map(|_| ()),
    );
    assert!(result.run.cancelled);
    assert!(!result.run.succeeded());
    let page = f.read(0, 300);
    assert!(page.rows.iter().any(|row| row.message == "ready"));
    assert!(page
        .rows
        .iter()
        .any(|row| row.hint.stream == "stderr" && row.message.contains("<redacted:")));
    assert!(!format!("{:?}", page.rows).contains("partial-synthetic-secret"));
    let count = page.rows.len();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!f.root.join("escaped").exists());
    assert_eq!(
        f.read(0, 300).rows.len(),
        count,
        "no late observer append after return"
    );
}

#[test]
fn native_process_observer_oversized_record_is_omitted_whole_and_flood_is_reported_as_gap() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let script = "printf 'password=oversize-secret'; i=0; while [ $i -lt 17000 ]; do printf x; i=$((i+1)); done;
        printf '\\n'; i=0; while [ $i -lt 8192 ]; do printf 'flood-line\\n'; i=$((i+1)); done";
    let result = f.run(script, &|| false, &mut |event| {
        std::thread::sleep(Duration::from_millis(4));
        journal.append(event, &mut |_| Ok(())).map(|_| ())
    });
    assert!(
        result.log_error.is_some(),
        "queue loss must never claim complete logs"
    );
    let page = f.read(0, 300);
    assert!(!format!("{:?}", page.rows).contains("oversize-secret"));
    assert!(
        page.rows.iter().any(|row| row.gap),
        "overflow or oversize must persist a visible gap"
    );
    assert!(
        page.rows
            .iter()
            .any(|row| row.hint.stream == "gap" && row.message.contains("queue")),
        "a queue overflow must have its own durable gap marker"
    );
}

#[test]
fn native_process_observer_failed_append_never_notifies_or_claims_complete_capture() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let admin = Connection::open(&f.db).unwrap();
    admin
        .execute_batch(
            "CREATE TRIGGER deny_fixture_row BEFORE INSERT ON native_process_log_rows
        BEGIN SELECT RAISE(ABORT,'fixture storage denial'); END;",
        )
        .unwrap();
    let mut notified = 0;
    let result = f.run("printf 'plain\\n'", &|| false, &mut |event| {
        journal
            .append(event, &mut |_| {
                notified += 1;
                Ok(())
            })
            .map(|_| ())
    });
    assert!(result.log_error.is_some());
    assert_eq!(notified, 0);
    assert!(f.read(0, 300).rows.is_empty());
    assert_eq!(result.run.stdout, b"plain\n");
    assert_eq!(
        admin
            .query_row(
                "SELECT row_count FROM native_process_log_executions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
