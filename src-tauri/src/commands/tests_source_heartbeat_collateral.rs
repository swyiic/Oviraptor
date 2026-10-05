#[test]
fn source_heartbeat_tool_collateral_business_or_foreign_root_rolls_back_all_rows() {
    use crate::agent_runtime::multi_agent::{source, source_rounds};
    for fault in ["business", "foreign_root"] {
        let (root, db, record, actor, child, seen, stop) = source_timer_fixture(4);
        let path = root.join("oviraptor.sqlite3");
        let work = root.join("attempt-0001");
        let (model, runtime, _) =
            verify_source_runtime_contract(&db, &record.scan_id, 1, &work).unwrap();
        let context = SpecialistTransportContext {
            supervision: None,
            db_path: &path,
            scan_id: &record.scan_id,
            attempt_number: 1,
            target_key: &actor.target_key,
            run_id: &actor.root_run_id,
            environment: &model,
            proxy: source_runtime_proxy(&runtime).unwrap(),
            usage_dir: &work,
            deadline: None,
        };
        let revision: i64 = db
            .query_row(
                "SELECT evidence_revision FROM agent_assignments WHERE id=?1",
                [&child.assignment_id],
                |r| r.get(0),
            )
            .unwrap();
        let slice = source::tool_task_slice(&db, &actor, child.role, revision).unwrap();
        let caps = source::tool_capabilities(child.role).unwrap();
        let request = json!({"messages":[{"role":"user","content":json!({"sourceTask":slice}).to_string()}],
            "tools":caps.iter().map(|name|json!({"type":"function","function":{"name":name}})).collect::<Vec<_>>()});
        assert!(matches!(
            source_rounds::start_authorized(&db, &actor, &child, 1, &request, 800, |tx| {
                authorize_source_tool_phase(tx, &context, &actor, &child)
            })
            .unwrap(),
            source_rounds::Start::Dispatch(_)
        ));
        source_heartbeat_collateral_fixture(&db, &child, fault);
        let before = application_table_snapshot(&db);
        let tx =
            rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        let result = heartbeat_source_tool_phase(&tx, &context, &actor, &child);
        let denied = result.is_err();
        if denied {
            drop(tx);
        } else {
            tx.commit().unwrap();
        }
        let unchanged = application_table_snapshot(&db) == before;
        let paid = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            None,
            "model_requests",
        )
        .unwrap()
        .consumed;
        let calls = seen.lock().unwrap().len();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(db);
        fs::remove_dir_all(root).unwrap();
        assert!(
            denied,
            "{fault}: actual Source tool heartbeat accepted collateral writes"
        );
        assert!(
            unchanged,
            "{fault}: every business/foreign/original paid/grant row must roll back"
        );
        assert_eq!((calls, paid), (2, 2));
    }
}
#[test]
fn source_heartbeat_specialist_collateral_business_or_foreign_root_rolls_back_all_rows() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{scheduler, source, specialist},
    };
    for fault in ["business", "foreign_root"] {
        let (root, db, record, actor) = source_tool_true_born_fixture_model(None);
        let path = root.join("oviraptor.sqlite3");
        let work = root.join("attempt-0001");
        let (model, runtime, _) =
            verify_source_runtime_contract(&db, &record.scan_id, 1, &work).unwrap();
        let context = SpecialistTransportContext {
            supervision: None,
            db_path: &path,
            scan_id: &record.scan_id,
            attempt_number: 1,
            target_key: &actor.target_key,
            run_id: &actor.root_run_id,
            environment: &model,
            proxy: source_runtime_proxy(&runtime).unwrap(),
            usage_dir: &work,
            deadline: None,
        };
        let slice = source::task_slice(&db, &actor, AgentRole::RepoMapper).unwrap();
        let child = scheduler::prepare_readonly_child(
            &db,
            &actor,
            AgentRole::RepoMapper,
            "source_results_ready",
            &slice,
            8000,
        )
        .unwrap();
        let request = source_specialist_request(&db, &actor, AgentRole::RepoMapper);
        assert!(matches!(
            specialist::start_authorized(&db, &actor, &child, &request, |tx| {
                authorize_source_specialist(tx, &context, &actor)
            })
            .unwrap(),
            specialist::Start::Dispatch(_)
        ));
        source_heartbeat_collateral_fixture(&db, &child, fault);
        let before = application_table_snapshot(&db);
        let tx =
            rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        let result = heartbeat_source_specialist(&tx, &actor, &child);
        let denied = result.is_err();
        if denied {
            drop(tx);
        } else {
            tx.commit().unwrap();
        }
        let unchanged = application_table_snapshot(&db) == before;
        drop(db);
        fs::remove_dir_all(root).unwrap();
        assert!(
            denied,
            "{fault}: actual executing Source specialist heartbeat accepted collateral writes"
        );
        assert!(
            unchanged,
            "{fault}: every business/foreign/original grant row must roll back"
        );
    }
}
fn source_heartbeat_collateral_fixture(
    db: &rusqlite::Connection,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    fault: &str,
) {
    db.execute("UPDATE agent_assignments SET lease_expires_at=datetime('now','+5 seconds','localtime') WHERE id=?1",[&child.assignment_id]).unwrap();
    db.execute(
        "INSERT INTO projects(id,name) VALUES(9029,'isolated heartbeat business')",
        [],
    )
    .unwrap();
    // A foreign diagnostic sentinel is never an executable/funded grant.
    db.execute("INSERT INTO agent_runs(id,root_run_id,scan_id,attempt_number,target_url,backend,role,status)
        SELECT 'heartbeat-foreign-root','heartbeat-foreign-root',scan_id,attempt_number,
          'https://foreign.invalid','native','coordinator','prepared' FROM agent_runs WHERE id=?1",
        [&child.run_id]).unwrap();
    let mutation = match fault {
        "business" => "UPDATE projects SET name='escaped heartbeat' WHERE id=9029;",
        _ => "UPDATE agent_runs SET hard_token_budget=999 WHERE id='heartbeat-foreign-root';",
    };
    db.execute_batch(&format!(
        "CREATE TRIGGER source_heartbeat_collateral AFTER UPDATE ON agent_assignments
        BEGIN {mutation} END;"
    ))
    .unwrap();
}
#[test]
fn source_heartbeat_preserves_already_installed_outer_authorizer() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{scheduler, source, specialist},
    };
    use rusqlite::hooks::{AuthAction, Authorization};
    let (root, db, _, actor) = source_tool_true_born_fixture_model(None);
    let slice = source::task_slice(&db, &actor, AgentRole::RepoMapper).unwrap();
    let child = scheduler::prepare_readonly_child(
        &db,
        &actor,
        AgentRole::RepoMapper,
        "source_results_ready",
        &slice,
        8000,
    )
    .unwrap();
    let request = source_specialist_request(&db, &actor, AgentRole::RepoMapper);
    assert!(matches!(
        specialist::start(&db, &actor, &child, &request).unwrap(),
        specialist::Start::Dispatch(_)
    ));
    db.execute("UPDATE agent_assignments SET lease_expires_at=datetime('now','+5 seconds','localtime') WHERE id=?1",[&child.assignment_id]).unwrap();
    db.execute(
        "INSERT INTO projects(id,name) VALUES(9030,'outer protection')",
        [],
    )
    .unwrap();
    db.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| {
        match ctx.action {
            AuthAction::Update {
                table_name: "projects",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }
    }))
    .unwrap();
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    heartbeat_source_specialist(&tx, &actor, &child).unwrap();
    assert!(
        tx.execute("UPDATE projects SET name='outer removed' WHERE id=9030", [])
            .is_err(),
        "the heartbeat may neither replace nor clear another writer's authorizer"
    );
    tx.rollback().unwrap();
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>)
        .unwrap();
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
