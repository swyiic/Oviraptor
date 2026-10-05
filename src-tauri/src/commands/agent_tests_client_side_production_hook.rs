// Existing production feature-red: current actual finish creates no ClientSide.
#[test]
fn client_side_actual_finish_execution_triggers_dedicated_readonly_sdk_from_original_http_facts() {
    let mut fixture = client_hook_fixture("client-actual-hook", 60000, 20);
    let original_claims = client_hook_rows(&fixture.db, "agent_http_request_claims");
    let original_target_costs = client_hook_target_costs(&fixture.db);
    fixture.finish().unwrap();
    let db = &fixture.db;
    let root = &fixture.root;
    let site_seen = &fixture.site_seen;
    let model_seen = &fixture.model_seen;
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side' AND state='completed'",[&root],|r|r.get::<_,i64>(0)).unwrap(),1,
        "actual production finish path must trigger the independent ClientSide worker");
    assert_eq!(
        site_seen.lock().unwrap().len(),
        1,
        "ClientSide cannot replay supplier target I/O"
    );
    let calls = model_seen.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let body: JsonValue = serde_json::from_str(calls[0].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body.get("max_tokens")
            .or_else(|| body.get("max_completion_tokens"))
            .and_then(JsonValue::as_u64),
        Some(256)
    );
    assert!(body.get("tools").is_none_or(|v| v == &json!([])));
    drop(calls);
    assert_eq!(
        client_hook_rows(db, "agent_http_request_claims"),
        original_claims
    );
    assert_eq!(client_hook_target_costs(db), original_target_costs);
    assert_eq!(
        db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let task:String=db.query_row("SELECT task_slice_json FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side'",[&root],|r|r.get(0)).unwrap();
    let task: JsonValue = serde_json::from_str(&task).unwrap();
    assert_eq!(task["observations"][0]["kind"], "http_csp_absent");
    assert_eq!(task["observations"][0]["classification"], "source-derived");
    assert_eq!(
        task["observations"][0]["sourceProof"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    fixture
        .session
        .as_ref()
        .unwrap()
        .supervisor
        .check()
        .unwrap();
}
fn client_hook_rows(db: &rusqlite::Connection, table: &str) -> String {
    assert!(matches!(
        table,
        "agent_http_request_claims" | "agent_root_budget_attempts"
    ));
    let mut q = db
        .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
        .unwrap();
    let n = q.column_count();
    let rows = q
        .query_map([], |r| {
            (0..n)
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    format!("{rows:?}")
}
fn client_hook_target_costs(db: &rusqlite::Connection) -> String {
    let mut q=db.prepare("SELECT rowid,* FROM agent_budget_entries WHERE dimension='target_requests' ORDER BY rowid").unwrap();
    let n = q.column_count();
    let rows = q
        .query_map([], |r| {
            (0..n)
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    format!("{rows:?}")
}
