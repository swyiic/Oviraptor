// All input rows are existing production scheduler invoices in temporary DBs.
// No model, target, browser, installation or real database is called.
fn immutable_unique_fixture(
    tag: &str,
) -> (
    std::path::PathBuf,
    rusqlite::Connection,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler},
    };
    let (directory, path, root, lease) = multi_agent_test_root(tag, 1000, 10);
    let db = db::open(&path).unwrap();
    db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
    assert_eq!(
        db.query_row("PRAGMA recursive_triggers", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute(
        "UPDATE agent_runs SET created_at=datetime('now','-3 seconds','localtime') WHERE id=?1",
        [&root],
    )
    .unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "immutable-unique-owner",
        &json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    assert!(
        budget::balance(&db, &root, None, "wall_time_ms")
            .unwrap()
            .consumed
            >= 2800
    );
    (directory, db, root, lease, child)
}

fn immutable_unique_physical(db: &rusqlite::Connection) -> Vec<Vec<String>> {
    [
        "agent_runs",
        "agent_assignments",
        "agent_assignment_attempts",
        "agent_budget_clock_origins",
        "agent_budget_limits",
        "agent_budget_entries",
    ]
    .into_iter()
    .map(|table| {
        let mut q = db
            .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
            .unwrap();
        let count = q.column_count();
        q.query_map([], |r| {
            Ok(format!(
                "{:?}",
                (0..count)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()?
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    })
    .collect()
}

#[test]
fn budget_immutable_clock_rejects_all_original_root_replace_collisions_recursive_off() {
    for unchanged in [false, true] {
        let (directory, db, root, _lease, _child) =
            immutable_unique_fixture("immutable-clock-replace");
        let physical = immutable_unique_physical(&db);
        let before = super::tests::application_table_snapshot(&db);
        let time = if unchanged {
            "started_at"
        } else {
            "datetime('now','localtime')"
        };
        let result = db.execute(
            &format!(
                "INSERT OR REPLACE INTO agent_budget_clock_origins(root_run_id,started_at)
            SELECT root_run_id,{time} FROM agent_budget_clock_origins WHERE root_run_id=?1"
            ),
            [&root],
        );
        assert!(
            result.is_err(),
            "same original Root must not replace its immutable origin: {result:?}"
        );
        assert!(super::tests::application_table_snapshot(&db) == before);
        assert_eq!(immutable_unique_physical(&db), physical);
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn budget_immutable_limits_rejects_full_composite_replace_collisions_recursive_off() {
    for delta in [0, 1] {
        let (directory, db, root, _lease, _child) =
            immutable_unique_fixture("immutable-limit-replace");
        let physical = immutable_unique_physical(&db);
        let before = super::tests::application_table_snapshot(&db);
        let result = db.execute(
            "INSERT OR REPLACE INTO agent_budget_limits(root_run_id,dimension,hard_limit)
            SELECT root_run_id,dimension,hard_limit+?2 FROM agent_budget_limits
            WHERE root_run_id=?1 AND dimension='wall_time_ms'",
            params![root, delta],
        );
        assert!(
            result.is_err(),
            "same original limit cannot be replaced or widened: {result:?}"
        );
        assert!(super::tests::application_table_snapshot(&db) == before);
        assert_eq!(immutable_unique_physical(&db), physical);
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn budget_immutable_entries_rejects_every_unique_collision_and_preserves_original_invoices() {
    for collision in ["primary", "idempotency", "both", "identical"] {
        let (directory, db, root, _lease, _child) =
            immutable_unique_fixture("immutable-fee-replace");
        let entries: Vec<(String, String)> = db
            .prepare(
                "SELECT entry_id,idempotency_key FROM agent_budget_entries
            WHERE root_run_id=?1 AND dimension='wall_time_ms' ORDER BY rowid",
            )
            .unwrap()
            .query_map([&root], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            entries.len(),
            2,
            "actual first wall Reserve and Consume invoices"
        );
        let (id, key) = match collision {
            "primary" => (
                entries[1].0.clone(),
                format!("new-key:{}", uuid::Uuid::new_v4()),
            ),
            "idempotency" => (uuid::Uuid::new_v4().to_string(), entries[1].1.clone()),
            "both" => (entries[1].0.clone(), entries[0].1.clone()),
            _ => (entries[1].0.clone(), entries[1].1.clone()),
        };
        let physical = immutable_unique_physical(&db);
        let before = super::tests::application_table_snapshot(&db);
        let result=db.execute("INSERT OR REPLACE INTO agent_budget_entries(entry_id,root_run_id,assignment_id,
            lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at)
            SELECT ?2,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,?3,source_id,created_at
            FROM agent_budget_entries WHERE entry_id=?1",params![entries[1].0,id,key]);
        assert!(
            result.is_err(),
            "{collision}: original physical fee must survive: {result:?}"
        );
        assert!(super::tests::application_table_snapshot(&db) == before);
        assert_eq!(immutable_unique_physical(&db), physical, "{collision}");
        drop(db);
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn budget_immutable_collision_guard_cannot_be_ignored_by_outer_insert_policy() {
    let (directory, db, root, _lease, _child) =
        immutable_unique_fixture("immutable-ignore-collision");
    let physical = immutable_unique_physical(&db);
    let before = super::tests::application_table_snapshot(&db);
    for sql in [
        "INSERT OR IGNORE INTO agent_budget_clock_origins(root_run_id,started_at) SELECT root_run_id,started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
        "INSERT OR IGNORE INTO agent_budget_limits(root_run_id,dimension,hard_limit) SELECT root_run_id,dimension,hard_limit FROM agent_budget_limits WHERE root_run_id=?1",
        "INSERT OR IGNORE INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at) SELECT entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at FROM agent_budget_entries WHERE root_run_id=?1",
    ] {
        assert!(db.execute(sql,[&root]).is_err(),"forbidden duplicate must fail visibly");
        assert!(super::tests::application_table_snapshot(&db)==before);
        assert_eq!(immutable_unique_physical(&db),physical);
    }
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn budget_immutable_guards_preserve_real_scheduler_and_exact_original_fee_replay() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler},
    };
    let (directory, db, root, lease, child) = immutable_unique_fixture("immutable-valid-replay");
    let physical = immutable_unique_physical(&db);
    let replay = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "immutable-unique-owner",
        &json!({}),
        1,
        &["evidence.read".into()],
        10,
        1,
    )
    .unwrap();
    assert_eq!(replay.assignment_id, child.assignment_id);
    assert_eq!(replay.run_id, child.run_id);
    assert_eq!(
        immutable_unique_physical(&db),
        physical,
        "scheduler replay creates no new fees or owner"
    );
    let tx = db.unchecked_transaction().unwrap();
    budget::limits::initialize(&tx, &lease).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        immutable_unique_physical(&db),
        physical,
        "existing limits only verify"
    );
    let tx = db.unchecked_transaction().unwrap();
    budget::append(
        &tx,
        &lease,
        &child.assignment_id,
        "model_requests",
        budget::Kind::Forfeit,
        1,
        "immutable-original-unknown",
        "original-unknown-state-contract",
    )
    .unwrap();
    tx.commit().unwrap();
    let physical = immutable_unique_physical(&db);
    let tx = db.unchecked_transaction().unwrap();
    budget::append(
        &tx,
        &lease,
        &child.assignment_id,
        "model_requests",
        budget::Kind::Forfeit,
        1,
        "immutable-original-unknown",
        "original-unknown-state-contract",
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        immutable_unique_physical(&db),
        physical,
        "exact persisted journal replay remains read-only"
    );
    assert_eq!(
        budget::balance(&db, &root, None, "model_requests")
            .unwrap()
            .indeterminate,
        1
    );
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn budget_immutable_guards_add_to_existing_native_history_through_actual_startup_schema() {
    let (directory, db, root, _lease, _child) =
        immutable_unique_fixture("immutable-startup-upgrade");
    db.execute_batch(
        "DROP TRIGGER IF EXISTS budget_clock_no_replace;
        DROP TRIGGER IF EXISTS budget_limit_no_replace;
        DROP TRIGGER IF EXISTS budget_entry_no_replace;",
    )
    .unwrap();
    let physical = immutable_unique_physical(&db);
    drop(db);
    let path = db::initialize(&directory).unwrap();
    let db = db::open(&path).unwrap();
    db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
    assert_eq!(
        immutable_unique_physical(&db),
        physical,
        "actual schema update must not rewrite Native rows or fees"
    );
    let result=db.execute("INSERT OR REPLACE INTO agent_budget_clock_origins(root_run_id,started_at)
        SELECT root_run_id,datetime('now','localtime') FROM agent_budget_clock_origins WHERE root_run_id=?1",[&root]);
    assert!(
        result.is_err(),
        "existing database acquires guard without row migration: {result:?}"
    );
    assert_eq!(immutable_unique_physical(&db), physical);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
