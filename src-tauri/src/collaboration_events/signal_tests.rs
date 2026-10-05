use super::*;
use crate::collaboration_events::tests_support::Fixture;
use std::{sync::mpsc, thread, time::Instant};

#[test]
fn commits_before_wait_are_not_lost_and_coalesce() {
    let signal = CommitWake::default();
    let stopped = AtomicBool::new(false);
    let before_read = signal.snapshot();
    for _ in 0..3 {
        signal.notify();
    }
    let start = Instant::now();
    signal.wait(Some(before_read), &stopped, Duration::from_secs(2));
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(signal.snapshot(), before_read + 3);
}

#[test]
fn idle_wait_reconciles_without_a_notification() {
    let signal = CommitWake::default();
    let start = Instant::now();
    signal.wait(
        Some(signal.snapshot()),
        &AtomicBool::new(false),
        Duration::from_millis(40),
    );
    assert!(start.elapsed() >= Duration::from_millis(40));
}

#[test]
fn stop_interrupts_both_idle_and_error_waits() {
    for backoff in [false, true] {
        let signal = Arc::new(CommitWake::default());
        let stopped = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let waiter = {
            let signal = Arc::clone(&signal);
            let stopped = Arc::clone(&stopped);
            thread::spawn(move || {
                let observed = (!backoff).then(|| signal.snapshot());
                ready_tx.send(()).unwrap();
                signal.wait(observed, &stopped, Duration::from_secs(5));
                done_tx.send(()).unwrap();
            })
        };
        ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        signal.stop(&stopped);
        let outcome = done_rx.recv_timeout(Duration::from_secs(1));
        waiter.join().unwrap();
        outcome.unwrap();
        assert!(stopped.load(Ordering::Acquire));
    }
}

#[test]
fn commit_storm_does_not_bypass_error_backoff() {
    let signal = Arc::new(CommitWake::default());
    let notifier = {
        let signal = Arc::clone(&signal);
        thread::spawn(move || {
            for _ in 0..12 {
                signal.notify();
                thread::sleep(Duration::from_millis(5));
            }
        })
    };
    let start = Instant::now();
    signal.wait(None, &AtomicBool::new(false), Duration::from_millis(80));
    assert!(start.elapsed() >= Duration::from_millis(80));
    notifier.join().unwrap();
}

#[test]
fn centralized_connection_commit_wakes_without_polling() {
    let fixture = Fixture::new();
    let writer = fixture.writer();
    let observed = COMMITS.snapshot();
    Fixture::insert(&writer, "committed");
    let start = Instant::now();
    COMMITS.wait(
        Some(observed),
        &AtomicBool::new(false),
        Duration::from_secs(2),
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_ne!(COMMITS.snapshot(), observed);
}

#[test]
fn installed_hook_preserves_passive_checkpoint_at_default_threshold() {
    let fixture = Fixture::new();
    let connection = fixture.writer();
    // One committed write above 1000 pages; NOOP observes but cannot perform a
    // checkpoint, so this assertion actually checks the installed callback.
    connection
        .execute("CREATE TABLE checkpoint_padding (value BLOB)", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO checkpoint_padding VALUES (zeroblob(6000000))",
            [],
        )
        .unwrap();
    let (frames, checkpointed): (i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(NOOP)", [], |row| {
            Ok((row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert!(frames >= 1000);
    assert_eq!(frames, checkpointed);
}

#[test]
fn pinned_reader_does_not_turn_checkpoint_contention_into_failed_commit() {
    let fixture = Fixture::new();
    let connection = fixture.writer();
    connection
        .execute("CREATE TABLE checkpoint_padding (value BLOB)", [])
        .unwrap();
    let reader = fixture.writer();
    reader
        .execute_batch("BEGIN; SELECT * FROM test_messages;")
        .unwrap();
    connection
        .execute(
            "INSERT INTO checkpoint_padding VALUES (zeroblob(6000000))",
            [],
        )
        .unwrap();
    let (frames, checkpointed): (i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(NOOP)", [], |row| {
            Ok((row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert!(checkpointed < frames);
    reader.execute_batch("ROLLBACK").unwrap();
    Fixture::insert(&connection, "checkpoint may now finish");
    let (frames, checkpointed): (i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(NOOP)", [], |row| {
            Ok((row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(frames, checkpointed);
}
