// Temporary creator/SQLite and actual original SDK transport; no real target/database.
struct BudgetTriggerFixture {
    h: AgentHarness,
    session: Option<MultiAgentSession>,
    boundary: ClientRootBoundary,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for BudgetTriggerFixture {
    fn drop(&mut self) {
        drop(self.session.take());
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = fs::remove_dir_all(&self.h.root);
    }
}
fn budget_trigger_fixture(reply: u8) -> BudgetTriggerFixture {
    let mut h = fresh_multi_production_harness("budget-trigger");
    let path = h.db_path.clone();
    let boundary: ClientRootBoundary = std::sync::Arc::new(std::sync::Mutex::new(None));
    let fault = boundary.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        if wire.contains("You are the Root Coordinator") && wire.contains("budget-allocation") {
            if let Some(apply) = fault.lock().unwrap().take() {
                let db = rusqlite::Connection::open_with_flags(
                    &path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
                )
                .unwrap();
                apply(&db);
            }
            if reply == 3 {
                return (
                    503,
                    "application/json",
                    json!({"error":{"message":"temporary budget assessment failure"}}).to_string(),
                );
            }
            return (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":["original allocation"],"missing":[],
                "suggestions":match reply {1=>vec![],2=>vec!["dispatch:web_executor"],_=>vec!["assess:budget_allocation"]},"costNotes":[],"risks":[]}).to_string()));
        }
        let response = fresh_multi_root_wire_response(&wire).unwrap_or_else(|| if wire.contains("SPA/API Mapper") {
            proposal_model_response(r#"{"summary":"actual original closed Mapper","priorityContracts":[],"risks":[]}"#)
        } else {model_round(&[],10)});
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = 20;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = 8;
    freeze_fresh_multi_production_harness(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    assert_eq!(h.model_seen.lock().unwrap().len(), 3);
    BudgetTriggerFixture {
        h,
        session: Some(session),
        boundary,
        stop,
    }
}
