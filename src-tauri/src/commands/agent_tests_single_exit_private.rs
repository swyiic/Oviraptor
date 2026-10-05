// Actual fresh Single creation, original owner, actual SDK; no finance issuer.
fn single_exit_private_fixture() -> (WebModeFixture, AgentRunContext) {
    let f = web_mode_fixture("single", "https://single-exit-private.example.test/app");
    let plan = web_mode_test_plan(&f);
    persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &f.target, &plan).unwrap();
    let mut context = test_context(&f.path, &f.target, vec![AgentIdentity::anonymous()]);
    context.scan_id = f.scan.clone();
    context.execution_plan = plan;
    context.run = runtime_open_run(&f.path, &f.scan, &context.route);
    assert!(context.run.is_some());
    (f, context)
}
#[test]
fn single_exit_private_actual_paid_sdk_close_retains_original_caller_hook_and_fee() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (f, mut context) = single_exit_private_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", model_round(&[], 20))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&context.environment, None).unwrap(),
        &[],
    );
    let response = native_model_transport(
        &context,
        &client,
        vec![json!({"role":"user","content":"actual original Single paid call"})],
        &[],
        1,
    )
    .unwrap_or_else(|_| panic!("actual original Single SDK must succeed before caller-hook boundary"));
    assert_eq!(response.usage.total_tokens, 60);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    let owner = RootOwner::load_single(&db, root).unwrap();
    let fees = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",
        [],
    )
    .unwrap();
    db.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Update {
            table_name: "projects",
            ..
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }))
    .unwrap();
    let fact = owner.close_single_finance(&db).unwrap();
    assert!(fact.elapsed_ms >= 0);
    let before = single_finally_physical(&db);
    assert!(
        db.execute("UPDATE projects SET name='caller-hook-must-deny'", [])
            .is_err(),
        "Single exit writer cleared the caller's original authorizer"
    );
    assert_eq!(single_finally_physical(&db), before);
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
    assert_eq!(owner.close_single_finance(&db).unwrap(), fact);
    assert_eq!(single_finally_physical(&db), before);
    assert!(owner.require_live(&db).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn single_exit_private_failed_receipt_retains_caller_hook_and_every_original_row() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    for fault in [
        "SELECT RAISE(IGNORE)",
        "SELECT RAISE(ABORT,'original exit fault')",
        "UPDATE projects SET name='forbidden-exit-collateral'",
    ] {
        let (f, context) = single_exit_private_fixture();
        let db = db::open(&f.path).unwrap();
        let owner = RootOwner::load_single(&db, &context.run.as_ref().unwrap().run_id).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER single_private_fault BEFORE INSERT ON agent_single_exit_receipts BEGIN {fault}; END;")).unwrap();
        db.authorizer(Some(|c: AuthContext<'_>| match c.action {
            AuthAction::Update {
                table_name: "projects",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();
        let before = single_finally_physical(&db);
        assert!(owner.close_single_finance(&db).is_err(), "{fault}");
        assert_eq!(single_finally_physical(&db), before, "{fault}");
        assert!(
            db.execute("UPDATE projects SET name='failure-hook-must-deny'", [])
                .is_err(),
            "{fault}"
        );
        assert_eq!(single_finally_physical(&db), before, "{fault}");
    }
}
#[test]
fn single_exit_private_refuses_original_caller_transaction_and_preserves_uncommitted_rows() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (f, context) = single_exit_private_fixture();
    let db = db::open(&f.path).unwrap();
    let owner = RootOwner::load_single(&db, &context.run.as_ref().unwrap().run_id).unwrap();
    db.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Update {
            table_name: "projects",
            ..
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }))
    .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute(
        "INSERT INTO projects(name) VALUES('caller uncommitted')",
        [],
    )
    .unwrap();
    let before = single_finally_physical(&tx);
    assert_eq!(
        owner.close_single_finance(&tx).unwrap_err(),
        "budget_single_exit_private_transaction_required"
    );
    assert!(!tx.is_autocommit());
    assert_eq!(single_finally_physical(&tx), before);
    tx.rollback().unwrap();
    assert!(db
        .execute("UPDATE projects SET name='transaction-hook-must-deny'", [])
        .is_err());
}
#[test]
fn single_exit_private_memory_database_is_refused_without_mutating_original_rows() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let (f, context) = single_exit_private_fixture();
    let real = db::open(&f.path).unwrap();
    let owner = RootOwner::load_single(&real, &context.run.as_ref().unwrap().run_id).unwrap();
    let memory = rusqlite::Connection::open_in_memory().unwrap();
    memory.execute_batch("CREATE TABLE prior_rows(value TEXT); INSERT INTO prior_rows VALUES('original memory row');").unwrap();
    let before = single_finally_physical(&memory);
    assert_eq!(
        owner.close_single_finance(&memory).unwrap_err(),
        "budget_single_exit_database_missing"
    );
    assert_eq!(single_finally_physical(&memory), before);
}
