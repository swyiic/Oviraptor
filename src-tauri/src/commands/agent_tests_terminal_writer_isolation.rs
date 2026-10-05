// Actual shared terminal entry must preserve the caller's existing authority.
#[test]
fn terminal_writer_actual_root_closure_preserves_original_caller_authorizer() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (f, _) = ordered_exec_fixture("terminal-private-hook", "http://127.0.0.1:9/v1");
    let job = ordered_closure_prepared(&f);
    let db = db::open(&f.context.db_path).unwrap();
    db.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Update { table_name: "projects", .. } => Authorization::Deny,
        _ => Authorization::Allow,
    })).unwrap();
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("original local terminal authority")).unwrap();
    ordered_closure_assert_cancelled(&db, &job);
    let before = super::tests::application_table_snapshot(&db);
    assert!(db.execute("UPDATE projects SET name='caller-hook-must-still-deny'", []).is_err(),
        "terminal transaction removed the caller's existing authorizer");
    assert!(super::tests::application_table_snapshot(&db)==before);
}

#[test]
fn terminal_writer_failed_original_closure_preserves_caller_hook_and_every_prior_row() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (f, _) = ordered_exec_fixture("terminal-private-failure", "http://127.0.0.1:9/v1");
    let _job = ordered_closure_prepared(&f);
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER terminal_writer_failure BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='terminal' BEGIN SELECT RAISE(ABORT,'fixture closure failure'); END;").unwrap();
    db.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Update { table_name: "projects", .. } => Authorization::Deny,
        _ => Authorization::Allow,
    })).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("original closure rollback")).is_err());
    assert!(super::tests::application_table_snapshot(&db)==before);
    assert!(db.execute("UPDATE projects SET name='caller-failure-hook-must-deny'", []).is_err());
    assert!(super::tests::application_table_snapshot(&db)==before);
}
#[test]
fn terminal_writer_exceptional_fact_and_terminal_keep_caller_hook_and_same_original_cutoff() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use crate::agent_runtime::multi_agent::budget::clock::elapsed_fact;
    let (directory, db, root, lease) = exceptional_short_root();
    db.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Update { table_name: "projects", .. } => Authorization::Deny,
        _ => Authorization::Allow,
    })).unwrap();
    let fact = elapsed_fact::record_if_exceptional(&db, &lease).unwrap().unwrap();
    assert!(db.execute("UPDATE projects SET name='exceptional-fact-hook-must-deny'", []).is_err());
    finish_coordinator_run(&db, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    assert_eq!(db.query_row("SELECT finished_at FROM agent_runs WHERE id=?1", [&root], |r|r.get::<_,String>(0)).unwrap(), fact.cutoff);
    assert_eq!(elapsed_fact::read(&db, &root).unwrap().unwrap(), fact);
    assert!(db.execute("UPDATE projects SET name='exceptional-terminal-hook-must-deny'", []).is_err());
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
#[test]
fn terminal_writer_actual_entry_refuses_caller_transaction_without_enlisting_or_removing_hook() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (f, _) = ordered_exec_fixture("terminal-private-caller-tx", "http://127.0.0.1:9/v1");
    let db = db::open(&f.context.db_path).unwrap();
    db.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Update { table_name: "projects", .. } => Authorization::Deny,
        _ => Authorization::Allow,
    })).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("INSERT INTO projects(name) VALUES('original-uncommitted-caller-row')", []).unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let error = finish_coordinator_run(&tx, &f.actor, &AgentTargetOutcome::incomplete("original caller transaction")).unwrap_err();
    assert_eq!(error, "budget_elapsed_fact_private_transaction_required");
    assert!(super::tests::application_table_snapshot(&tx)==before);
    assert!(!tx.is_autocommit());
    tx.rollback().unwrap();
    assert!(db.execute("UPDATE projects SET name='caller-tx-hook-must-deny'", []).is_err());
}
#[test]
fn terminal_writer_both_private_boundaries_reject_memory_database_without_mutating_caller() {
    use crate::agent_runtime::multi_agent::budget::clock;
    let (f, _) = ordered_exec_fixture("terminal-private-memory", "http://127.0.0.1:9/v1");
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE prior_rows(value TEXT); INSERT INTO prior_rows VALUES('original');").unwrap();
    assert_eq!(clock::closure_write(&db, &f.actor, false, |_| Ok(())).unwrap_err(), "budget_closure_database_missing");
    assert_eq!(clock::elapsed_fact::record_if_exceptional(&db, &f.actor).unwrap_err(), "budget_elapsed_fact_database_missing");
    assert_eq!(db.query_row("SELECT value FROM prior_rows", [], |r|r.get::<_,String>(0)).unwrap(), "original");
}

#[test]
fn terminal_writer_caller_temp_emitter_introduced_during_callback_rolls_back_original_publication() {
    use crate::agent_runtime::multi_agent::budget::clock;
    let (f, _) = ordered_exec_fixture("terminal-private-late-temp", "http://127.0.0.1:9/v1");
    let _job = ordered_closure_prepared(&f);
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let result = clock::closure_write(&db, &f.actor, false, |tx| {
        finish_coordinator_run_in_transaction(tx, &f.actor, &AgentTargetOutcome::incomplete("original private callback"))?;
        db.execute_batch("CREATE TEMP TRIGGER agent_collaboration_run_update AFTER UPDATE ON main.agent_runs BEGIN INSERT INTO projects(name) VALUES('late-temp-emitter-business'); END;").unwrap();
        Ok(())
    });
    assert_eq!(result.unwrap_err(), "budget_closure_emitter_contract_conflict");
    assert!(super::tests::application_table_snapshot(&db)==before);
    db.execute_batch("DROP TRIGGER temp.agent_collaboration_run_update").unwrap();
}
