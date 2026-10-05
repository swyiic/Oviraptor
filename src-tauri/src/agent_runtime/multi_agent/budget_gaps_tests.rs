use super::*;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE agent_runs (
            id TEXT PRIMARY KEY,scan_id TEXT,attempt_number INTEGER,target_url TEXT,root_run_id TEXT
         );
         CREATE TABLE agent_budget_ledger (
            root_run_id TEXT PRIMARY KEY,reserved_tokens INTEGER,spent_tokens INTEGER,
            reserved_requests INTEGER,spent_requests INTEGER,lease_epoch INTEGER,fencing_token TEXT
         );
         CREATE TABLE agent_budget_limits(root_run_id TEXT,dimension TEXT,hard_limit INTEGER);
         CREATE TABLE agent_budget_entries(root_run_id TEXT,assignment_id TEXT,dimension TEXT,kind TEXT,amount INTEGER);
         CREATE TABLE agent_assignments (
            id TEXT PRIMARY KEY,coordinator_run_id TEXT,reserved_tokens INTEGER,
            reserved_requests INTEGER,budget_settled_at TEXT
         );
         CREATE TABLE tool_invocations (
            id INTEGER PRIMARY KEY,run_id TEXT,contract_key TEXT,finished_at TEXT
         );
         CREATE TABLE agent_coordinator_leases (
            scan_id TEXT,attempt_number INTEGER,target_key TEXT,root_run_id TEXT,
            lease_epoch INTEGER,fencing_token TEXT
         );
         INSERT INTO agent_runs VALUES('root','scan',1,'https://example.test','');
         INSERT INTO agent_runs VALUES('child','scan',1,'https://example.test','root');
         INSERT INTO agent_runs VALUES('other','scan',2,'https://example.test','');",
    )
    .unwrap();
    db.execute_batch(include_str!("budget/web_model_schema.sql"))
        .unwrap();
    db
}

#[test]
fn matches_unsettled_reservations_and_counts_root_and_child_tools_only() {
    let db = fixture();
    db.execute_batch(
        "INSERT INTO agent_budget_ledger VALUES('root',11,7,2,1,3,'fence');
         INSERT INTO agent_assignments VALUES('a','root',10,1,'');
         INSERT INTO agent_assignments VALUES('b','root',1,1,'');
         INSERT INTO agent_assignments VALUES('settled','root',90,90,'done');
         INSERT INTO agent_assignments VALUES('other','other',90,90,'');
         INSERT INTO tool_invocations VALUES(1,'root','','');
         INSERT INTO tool_invocations VALUES(2,'child','key','done');
         INSERT INTO tool_invocations VALUES(3,'other','','');
         INSERT INTO agent_coordinator_leases VALUES('scan',1,'https://example.test','root',3,'fence');",
    )
    .unwrap();

    let report = budget_ledger_gaps(&db, "root").unwrap();
    assert!(report.ledger_exists);
    assert_eq!((report.reserved_tokens, report.spent_tokens), (11, 7));
    assert_eq!((report.reserved_requests, report.spent_requests), (2, 1));
    assert_eq!(report.unsettled_assignment_tokens, 11);
    assert_eq!(report.unsettled_assignment_requests, 2);
    assert_eq!(report.reservation_token_delta, Some(0));
    assert_eq!(report.reservation_request_delta, Some(0));
    assert_eq!(report.indeterminate_invocations, 1);
    assert_eq!(report.unclosed_web_model_calls, 0);
    assert_eq!(report.unkeyed_invocations, 1);
    assert!(report.coordinator_lease_found);
    assert!(report.fencing_matches_coordinator);
    assert_eq!(report.missing_dimensions.len(), 9);
    assert!(report.append_journal.is_none());
    assert!(report
        .missing_dimensions
        .contains(&"target_requests".into()));
}

#[test]
fn mismatch_and_missing_ledger_are_reported_without_mutation() {
    let db = fixture();
    db.execute_batch(
        "INSERT INTO agent_budget_ledger VALUES('root',3,0,4,0,1,'old');
         INSERT INTO agent_assignments VALUES('a','root',5,2,'');
         INSERT INTO agent_coordinator_leases VALUES('scan',1,'https://example.test','root',2,'new');",
    )
    .unwrap();

    let report = budget_ledger_gaps(&db, "root").unwrap();
    assert_eq!(report.reservation_token_delta, Some(-2));
    assert_eq!(report.reservation_request_delta, Some(2));
    assert!(!report.fencing_matches_coordinator);
    let ledger: (i64, i64) = db
        .query_row(
            "SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id='root'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(ledger, (3, 4));

    db.execute(
        "DELETE FROM agent_budget_ledger WHERE root_run_id='root'",
        [],
    )
    .unwrap();
    let missing = budget_ledger_gaps(&db, "root").unwrap();
    assert!(!missing.ledger_exists);
    assert_eq!(missing.unsettled_assignment_tokens, 5);
    assert_eq!(missing.reservation_token_delta, None);
    assert!(!missing.fencing_matches_coordinator);
}

#[test]
fn refuses_to_report_a_child_as_a_root() {
    let db = fixture();
    assert!(budget_ledger_gaps(&db, "child").is_err());
    assert!(budget_ledger_gaps(&db, "missing").is_err());
}
