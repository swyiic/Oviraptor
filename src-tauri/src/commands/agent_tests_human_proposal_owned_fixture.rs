// Actual command entry and localhost provider. No specialist role test double.
// Prerequisite: g2 test_fixture.rs plus the final SDK v2 reader/schema.
fn human_owned_fixture(tag: &str, endpoint: &str) -> (RootTickFixture, String) {
    let f = root_tick_fixture(tag, endpoint);
    let db = db::open(&f.context.db_path).unwrap();
    let id = confirm_queue_directive(&db, &f.actor, "@mapper 请分析已有冻结证据");
    (f, id)
}
fn human_owned_acked(db: &rusqlite::Connection) -> i64 {
    db.query_row("SELECT count(*) FROM agent_messages WHERE kind='human_assessment_result' AND acknowledged_at<>''", [], |r| r.get(0)).unwrap()
}
fn human_owned_calls(db: &rusqlite::Connection) -> i64 {
    db.query_row("SELECT count(*) FROM agent_specialist_calls", [], |r| {
        r.get(0)
    })
    .unwrap()
}
fn human_owned_wait_seen(seen: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while seen.lock().unwrap().is_empty() {
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
}

// Keep the automatic fixture cleanup alive as long as its supervised context.
struct LiveProposalGuard {
    _parent: Option<crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor>,
    _fixture: WebModeFixture,
}

// Current live entries need an original Root and a parent; legacy saved fixtures
// remain separate for their read-only Native receipt contracts.
fn live_proposal_fixture(
    name: &str,
    text: &str,
) -> (PathBuf, AgentRunContext, String, LiveProposalGuard) {
    live_proposal_fixture_limits(name, text, (60_000, 20))
}
fn live_proposal_fixture_limits(
    name: &str,
    text: &str,
    limits: (i64, i64),
) -> (PathBuf, AgentRunContext, String, LiveProposalGuard) {
    let f = root_tick_fixture_protocol_limits(name, "http://127.0.0.1:9/v1", true, limits);
    let db = db::open(&f.context.db_path).unwrap();
    let id = confirm_queue_directive(&db, &f.actor, text);
    (
        f.f.root.clone(),
        f.context,
        id,
        LiveProposalGuard {
            _parent: f.parent,
            _fixture: f.f,
        },
    )
}
