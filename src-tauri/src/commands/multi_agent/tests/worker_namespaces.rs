// Storage fixtures only: copying a run never grants a worker execution rights.
pub(super) fn namespace_copy_child(db: &rusqlite::Connection, original: &str) -> String {
    let child = format!("run-{}", uuid::Uuid::new_v4());
    let mut query = db.prepare("SELECT * FROM agent_runs WHERE id=?1").unwrap();
    let count = query.column_count();
    let id = query.column_index("id").unwrap();
    let mut values = query
        .query_row([original], |r| {
            (0..count)
                .map(|n| r.get::<_, rusqlite::types::Value>(n))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap();
    values[id] = rusqlite::types::Value::Text(child.clone());
    let parameters = (1..=count)
        .map(|n| format!("?{n}"))
        .collect::<Vec<_>>()
        .join(",");
    db.execute(
        &format!("INSERT INTO agent_runs VALUES({parameters})"),
        rusqlite::params_from_iter(values),
    )
    .unwrap();
    child
}

#[test]
fn assignment_worker_namespace_specialist_preserves_two_original_dispatches() {
    use crate::agent_runtime::multi_agent::specialist;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(_) =
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    let second = namespace_copy_child(&db, &child.run_id);
    db.execute("INSERT INTO agent_specialist_calls(assignment_id,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,state)
        SELECT assignment_id,?2,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,state
        FROM agent_specialist_calls WHERE child_run_id=?1",params![child.run_id,second]).expect("another original worker must have its own durable dispatch");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert!(db
        .execute(
            "UPDATE agent_specialist_calls SET child_run_id=?1 WHERE child_run_id=?2",
            params![child.run_id, second]
        )
        .is_err());
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_worker_namespace_capabilities_preserve_revoked_original_grants() {
    let (root, context, _lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    db.execute(
        "UPDATE agent_capability_leases SET revoked_at='withdrawn' WHERE child_run_id=?1",
        [&child.run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let second = namespace_copy_child(&db, &child.run_id);
    db.execute("INSERT INTO agent_capability_leases(id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at)
        SELECT 'second-'||id,root_run_id,assignment_id,?2,capability,lease_epoch,fencing_token,lease_expires_at
        FROM agent_capability_leases WHERE child_run_id=?1",params![child.run_id,second]).expect("new worker grants must not replace old revoked evidence");
    let after = super::tests::application_table_snapshot(&db);
    for (table, rows) in before {
        if ![
            "agent_runs",
            "agent_capability_leases",
            "agent_collaboration_events",
        ]
        .contains(&table.as_str())
        {
            assert_eq!(
                after.iter().find(|(name, _)| name == &table).unwrap().1,
                rows,
                "{table}"
            );
        }
    }
    assert_eq!(db.query_row("SELECT count(*) FROM agent_collaboration_events WHERE entity_id=?1 AND event_type='agent_run'",[&second],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at='withdrawn'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert!(db.execute("INSERT INTO agent_capability_leases SELECT 'duplicate-'||id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at,revoked_at,created_at FROM agent_capability_leases WHERE child_run_id=?1",[&second]).is_err());
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_worker_namespace_scheduler_replay_ignores_retained_revoked_other_child() {
    use crate::agent_runtime::multi_agent::scheduler;
    let (root, path, _, lease) = multi_agent_test_root("namespace-replay", 1000, 10);
    let db = db::open(&path).unwrap();
    let child = schedule_authority_fixture(&db, &lease).unwrap();
    scheduler::mark_child_running(&db, &lease, &child).unwrap();
    let other = namespace_copy_child(&db, &child.run_id);
    db.execute("INSERT INTO agent_capability_leases(id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at,revoked_at)
        SELECT 'historic-'||id,root_run_id,assignment_id,?2,capability,lease_epoch,fencing_token,'2000-01-01','withdrawn'
        FROM agent_capability_leases WHERE child_run_id=?1",params![child.run_id,other]).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(schedule_authority_fixture(&db, &lease).unwrap(), child);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_worker_namespace_specialist_callbacks_mutate_only_original_child() {
    use crate::agent_runtime::multi_agent::specialist;
    for phase in ["received", "uncertain", "unsent"] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        let specialist::Start::Dispatch(call) =
            specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
        else {
            panic!()
        };
        let other = namespace_copy_child(&db, &child.run_id);
        db.execute("INSERT INTO agent_specialist_calls(assignment_id,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,state)
            SELECT assignment_id,?2,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,state
            FROM agent_specialist_calls WHERE child_run_id=?1",params![child.run_id,other]).unwrap();
        let read_other = || {
            db.query_row("SELECT json_array(assignment_id,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,state,response_json,usage_json,response_hash,event_sequence,failure_code,created_at,finished_at)
            FROM agent_specialist_calls WHERE child_run_id=?1",[&other],|r|r.get::<_,String>(0)).unwrap()
        };
        let before = read_other();
        match phase {
            "received" => {
                specialist::record_received(
                    &db,
                    &call,
                    "original response",
                    false,
                    &late_web_model_response().usage,
                )
                .unwrap();
            }
            "uncertain" => {
                specialist::record_uncertain(&db, &call, "original outcome unknown").unwrap()
            }
            _ => specialist::record_not_sent(&db, &call, "user_cancelled").unwrap(),
        }
        assert_eq!(read_other(), before, "{phase}");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&other],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(
            specialist::start(&db, &lease, &child, &json!({"messages":[]})).is_err()
                || phase == "received"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
