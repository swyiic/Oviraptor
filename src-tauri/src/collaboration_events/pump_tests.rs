use super::*;
use crate::collaboration_events::tests_support::Fixture;
use std::sync::mpsc;

#[test]
fn pending_commits_are_coalesced_without_delaying_shutdown() {
    let signal = CommitWake::default();
    let stopped = AtomicBool::new(false);
    let observed = signal.snapshot();
    signal.notify();
    let start = std::time::Instant::now();
    wait_for_work(&signal, observed, &stopped);
    assert!(start.elapsed() >= COMMIT_COALESCE);
    assert!(start.elapsed() < Duration::from_secs(1));
    signal.stop(&stopped);
    let start = std::time::Instant::now();
    wait_for_work(&signal, observed, &stopped);
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn startup_skips_existing_hints_but_not_new_commits() {
    let fixture = Fixture::new();
    let writer = fixture.writer();
    Fixture::insert(&writer, "old");
    let cursor = initial_cursor(&fixture.path).unwrap();
    assert_eq!(cursor, 1);
    Fixture::insert(&writer, "new");
    let events = read_batch(&fixture.path, cursor).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sequence, 2);
}

#[test]
fn only_committed_trigger_rows_are_visible_and_payloads_never_leave_pump() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let transaction = writer.transaction().unwrap();
    Fixture::insert(&transaction, "secret-notification-body");
    assert!(read_batch(&fixture.path, 0).unwrap().is_empty());
    transaction.rollback().unwrap();
    assert!(read_batch(&fixture.path, 0).unwrap().is_empty());
    Fixture::insert(&writer, "secret-notification-body");
    let events = read_batch(&fixture.path, 0).unwrap();
    assert_eq!(events.len(), 1);
    let json = serde_json::to_value(&events[0]).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"sequence":1,"scanId":"task-a","attemptNumber":2,
        "entityType":"message","entityId":"1","eventType":"inserted"})
    );
    assert!(!json.to_string().contains("secret-notification-body"));
}

#[test]
fn failed_emit_retains_unsent_cursor_and_drains_all_later_batches() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let transaction = writer.transaction().unwrap();
    for _ in 0..600 {
        Fixture::insert(&transaction, "local synthetic message");
    }
    transaction.commit().unwrap();
    let mut cursor = 0;
    let stopped = AtomicBool::new(false);
    let mut delivered = Vec::new();
    let result = drain_batch(&fixture.path, &mut cursor, &stopped, &mut |event| {
        if event.sequence == 3 {
            return Err("synthetic transport failure".into());
        }
        delivered.push(event.sequence);
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(cursor, 2);
    let mut batches = 0;
    loop {
        batches += 1;
        let full = drain_batch(&fixture.path, &mut cursor, &stopped, &mut |event| {
            delivered.push(event.sequence);
            Ok(())
        })
        .unwrap();
        if !full {
            break;
        }
    }
    assert_eq!(batches, 3);
    assert_eq!(cursor, 600);
    assert_eq!(delivered, (1..=600).collect::<Vec<_>>());
}

#[test]
fn missing_database_is_not_created_and_read_errors_do_not_advance_cursor() {
    let fixture = Fixture::new();
    let path = fixture.path.with_file_name("missing.sqlite3");
    let mut cursor = 42;
    assert!(initial_cursor(&path).is_err());
    assert!(
        drain_batch(&path, &mut cursor, &AtomicBool::new(false), &mut |_| {
            panic!("missing database cannot emit an event")
        })
        .is_err()
    );
    assert!(!path.exists());
    assert_eq!(cursor, 42);
    let read_only = reader(&fixture.path).unwrap();
    assert!(read_only
        .execute("DELETE FROM agent_collaboration_events", [])
        .is_err());
}

#[test]
fn unrelated_database_commits_never_cross_event_identity() {
    let first = Fixture::new();
    let second = Fixture::new();
    let writer = second.writer();
    Fixture::insert(&writer, "other database");
    assert!(read_batch(&first.path, 0).unwrap().is_empty());
    assert_eq!(read_batch(&second.path, 0).unwrap().len(), 1);
}

#[test]
fn stopping_during_batch_prevents_later_emits_and_drop_wakes_worker() {
    let fixture = Fixture::new();
    let writer = fixture.writer();
    Fixture::insert(&writer, "one");
    Fixture::insert(&writer, "two");
    let control = EventPumpControl {
        stopped: Arc::new(AtomicBool::new(false)),
        commits: Arc::new(CommitWake::default()),
    };
    let stopped = Arc::clone(&control.stopped);
    let mut control = Some(control);
    let mut cursor = 0;
    let mut count = 0;
    assert!(
        !drain_batch(&fixture.path, &mut cursor, &stopped, &mut |_| {
            count += 1;
            drop(control.take());
            Ok(())
        })
        .unwrap()
    );
    assert_eq!(cursor, 1);
    assert_eq!(count, 1);
    assert!(stopped.load(Ordering::Acquire));
}

#[test]
fn commit_during_delivery_wakes_next_read_and_worker_stops_without_interval_delay() {
    let fixture = Fixture::new();
    let writer = fixture.writer();
    Fixture::insert(&writer, "first");
    let control = EventPumpControl {
        stopped: Arc::new(AtomicBool::new(false)),
        commits: Arc::clone(&COMMITS),
    };
    let (event_tx, event_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = {
        let path = fixture.path.clone();
        let commits = Arc::clone(&control.commits);
        let stopped = Arc::clone(&control.stopped);
        thread::spawn(move || {
            run(path, 0, commits, stopped, |event| {
                event_tx.send(event.sequence).unwrap();
                if event.sequence == 1 {
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                Ok(())
            });
            done_tx.send(()).unwrap();
        })
    };
    let first = event_rx.recv_timeout(Duration::from_secs(2));
    Fixture::insert(&writer, "commit during read-to-wait gap");
    release_tx.send(()).unwrap();
    let second = event_rx.recv_timeout(Duration::from_secs(2));
    control.stop();
    let done = done_rx.recv_timeout(Duration::from_secs(2));
    worker.join().unwrap();
    assert_eq!(first.unwrap(), 1);
    assert_eq!(second.unwrap(), 2);
    done.unwrap();
}
