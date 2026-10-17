//! Exercise the real SQLite transaction interface and unchanged clock guards.
//! This reader-only schema has no production execution or budget authority.
use super::remaining;
use rusqlite::{types::Value, Connection};

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE agent_runs(id TEXT PRIMARY KEY,created_at TEXT);
        CREATE TABLE agent_budget_clock_origins(root_run_id TEXT,started_at TEXT);
        CREATE TABLE agent_budget_limits(root_run_id TEXT,dimension TEXT,hard_limit INTEGER);
        CREATE TABLE agent_budget_entries(root_run_id TEXT,assignment_id TEXT,dimension TEXT,kind TEXT,amount INTEGER);
        CREATE TABLE caller_owned_marker(value TEXT);
        INSERT INTO agent_runs VALUES('root',datetime('now','localtime','-1 second'));
        INSERT INTO agent_budget_clock_origins SELECT id,created_at FROM agent_runs;
        INSERT INTO agent_budget_limits VALUES('root','wall_time_ms',120000);").unwrap();
    db
}

fn snapshot(db: &Connection) -> Vec<Vec<Vec<Value>>> {
    ["SELECT rowid,* FROM agent_runs ORDER BY rowid",
        "SELECT rowid,* FROM agent_budget_clock_origins ORDER BY rowid",
        "SELECT rowid,* FROM agent_budget_limits ORDER BY rowid",
        "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid",
        "SELECT rowid,* FROM caller_owned_marker ORDER BY rowid"]
        .iter().map(|sql| {
            let mut statement = db.prepare(sql).unwrap();
            let columns = statement.column_count();
            let rows = statement.query_map([], |row| (0..columns).map(|column| row.get(column)).collect::<rusqlite::Result<Vec<Value>>>()).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        }).collect()
}

#[test]
fn remaining_reuses_caller_transaction_without_nesting_commit_or_writes() {
    let db = fixture();
    let initial = snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("INSERT INTO caller_owned_marker VALUES('caller transaction')", []).unwrap();
    let before = snapshot(&tx);
    let changes = tx.total_changes();
    assert!(!tx.is_autocommit());
    assert!(remaining(&tx, "root").unwrap() > std::time::Duration::ZERO);
    assert!(!tx.is_autocommit(), "The caller still owns its original transaction");
    assert_eq!(remaining(&tx, "missing").unwrap_err(), "budget_clock_origin_missing");
    assert!(!tx.is_autocommit(), "A read error cannot end the caller's transaction");
    assert_eq!(tx.total_changes(), changes);
    assert_eq!(snapshot(&tx), before);
    tx.rollback().unwrap();
    assert!(db.is_autocommit());
    assert_eq!(snapshot(&db), initial, "The caller can still roll back its own write");
}

#[test]
fn remaining_keeps_backwards_and_exhausted_clock_rejection_without_writes() {
    for backwards in [true, false] {
        let db = fixture();
        if backwards {
            // A persisted sample ahead of the real clock must still fail;
            // the snapshot fix cannot silently clamp it to current elapsed.
            db.execute_batch("INSERT INTO agent_budget_entries VALUES('root','child','wall_time_ms','reserve',60000);
                INSERT INTO agent_budget_entries VALUES('root','child','wall_time_ms','consume',60000);").unwrap();
        } else {
            db.execute_batch("UPDATE agent_runs SET created_at='2000-01-01 00:00:00';
                UPDATE agent_budget_clock_origins SET started_at='2000-01-01 00:00:00';").unwrap();
        }
        let before = snapshot(&db);
        let changes = db.total_changes();
        assert_eq!(remaining(&db, "root").unwrap_err(), if backwards {
            "budget_clock_moved_backwards"
        } else { "budget_wall_time_exhausted" });
        assert!(db.is_autocommit());
        assert_eq!(db.total_changes(), changes);
        assert_eq!(snapshot(&db), before);
    }
}

#[test]
fn remaining_keeps_reserved_and_indeterminate_rejection_without_repair() {
    for indeterminate in [false, true] {
        let db = fixture();
        db.execute("INSERT INTO agent_budget_entries VALUES('root','child','wall_time_ms','reserve',1)", []).unwrap();
        if indeterminate {
            db.execute("INSERT INTO agent_budget_entries VALUES('root','child','wall_time_ms','forfeit',1)", []).unwrap();
        }
        let before = snapshot(&db);
        let changes = db.total_changes();
        assert_eq!(remaining(&db, "root").unwrap_err(), "budget_clock_unsettled_sample");
        assert!(db.is_autocommit());
        assert_eq!(db.total_changes(), changes);
        assert_eq!(snapshot(&db), before);
    }
}
