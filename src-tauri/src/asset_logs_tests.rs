use super::*;
use std::cell::Cell;

fn fixture() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    initialize(&connection);
    connection
}

fn initialize(connection: &Connection) {
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
        CREATE TABLE runs(id INTEGER PRIMARY KEY, project_id INTEGER NOT NULL);
        INSERT INTO runs VALUES(1,10),(2,20);
        CREATE TABLE logs(id INTEGER PRIMARY KEY, run_id INTEGER REFERENCES runs(id),
        level TEXT NOT NULL, stage TEXT NOT NULL, message TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT 'fixture-time');",
        )
        .unwrap();
}

#[test]
fn asset_log_append_commits_before_identity_only_notification() {
    let uri = format!(
        "file:asset-log-{}?mode=memory&cache=shared",
        uuid::Uuid::new_v4()
    );
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
        | rusqlite::OpenFlags::SQLITE_OPEN_URI;
    let mut connection = Connection::open_with_flags(&uri, flags).unwrap();
    initialize(&connection);
    let observer = Connection::open_with_flags(&uri, flags).unwrap();
    let count = Cell::new(0);
    append(
        &mut connection,
        1,
        "info",
        "collect",
        "password=private-key",
        |hint| {
            count.set(count.get() + 1);
            assert_eq!(
                serde_json::to_value(hint).unwrap(),
                serde_json::json!({"logId":1,"runId":1,"projectId":10})
            );
            // Another connection must see the committed row during the notification.
            assert_eq!(
                observer
                    .query_row(
                        "SELECT count(*) FROM logs WHERE id=?1",
                        [hint.log_id],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
        },
    )
    .unwrap();
    assert!(connection.is_autocommit());
    assert_eq!(count.get(), 1);
    let stored: String = connection
        .query_row("SELECT message FROM logs", [], |row| row.get(0))
        .unwrap();
    assert!(!stored.contains("private-key"));
    assert!(stored.contains("redacted"));
}

#[test]
fn asset_log_failed_write_or_commit_never_notifies() {
    let mut connection = fixture();
    let count = Cell::new(0);
    assert!(
        append(&mut connection, 99, "info", "test", "bad FK", |_| count
            .set(1))
        .is_err()
    );
    connection.execute_batch("CREATE TABLE pending_fk(id INTEGER REFERENCES runs(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER reject_commit AFTER INSERT ON logs BEGIN INSERT INTO pending_fk VALUES(99); END;").unwrap();
    assert!(
        append(&mut connection, 1, "info", "test", "bad commit", |_| count
            .set(1))
        .is_err()
    );
    assert_eq!(count.get(), 0);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM logs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn asset_log_reads_preserve_project_and_run_filters_order_and_repeated_records() {
    let mut connection = fixture();
    for run in [1, 2, 1, 1] {
        append(&mut connection, run, "info", "test", "repeated", |_| {}).unwrap();
    }
    let rows = read(&connection, None, Some(10), Some(2)).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![4, 3]
    );
    assert_eq!(rows[0].message, rows[1].message);
    assert!(read(&connection, Some(1), Some(20), None)
        .unwrap()
        .is_empty());
    assert_eq!(read(&connection, Some(2), None, None).unwrap()[0].id, 2);
    assert_eq!(read(&connection, None, None, Some(-1)).unwrap().len(), 1);
}

#[test]
fn asset_log_old_raw_rows_are_redacted_on_read_without_rewriting_storage() {
    let connection = fixture();
    let raw = "\x1b[31mpassword=old-private-key\x1b[0m";
    connection
        .execute(
            "INSERT INTO logs(run_id,level,stage,message) VALUES(1,?1,?1,?1)",
            [raw],
        )
        .unwrap();
    let view = read(&connection, None, None, None).unwrap().remove(0);
    for field in [view.message, view.level, view.stage] {
        assert!(!field.contains("old-private-key"));
        assert!(!field.contains('\x1b'));
    }
    assert_eq!(
        connection
            .query_row("SELECT message FROM logs", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        raw
    );
}
