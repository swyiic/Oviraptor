use super::*;
use crate::agent_runtime::model::{LocalResourcePolicy, ResolvedModelProfile};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT);INSERT INTO projects VALUES(1,'Target','active'),(2,'Other','active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT,shared_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,company TEXT,extra_json TEXT,content_category TEXT,probe_outcome TEXT DEFAULT '');CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,decision TEXT DEFAULT 'pending');").unwrap();
    ensure(&c).unwrap();
    ensure_assist(&c).unwrap();
    c
}
fn add(c: &Connection, id: i64, name: &str, independent: bool) {
    let mut extra = json!({"cert.domain":name});
    if independent {
        extra["icp_name"] = json!("Target independent organization");
    }
    c.execute(
        "INSERT INTO assets VALUES(?1,?2,?2,'untrusted collection label',?3,'','')",
        params![id, format!("https://{name}"), extra.to_string()],
    )
    .unwrap();
    c.execute("INSERT INTO project_assets VALUES(1,?1,0,'pending')", [id])
        .unwrap();
}
fn profile(base: &str) -> GatewayProfile {
    GatewayProfile::resolve(
        &ResolvedModelProfile {
            model: "synthetic-test-model".into(),
            api_base: base.into(),
            api_key: "synthetic-key".into(),
            deployment: "local".into(),
        },
        &LocalResourcePolicy {
            max_output_tokens: Some(2048),
            max_context_tokens: 128000,
        },
        None,
    )
    .unwrap()
}
fn bind(c: &mut Connection, profile: &GatewayProfile) -> String {
    let identity = recipient(profile);
    let id = prepare_pass(c, 1, "Target group", true, &identity, &HashSet::new()).unwrap();
    c.execute("INSERT INTO asset_seed_assists(project_id,run_id,organization,include_isolated,recipient,phase,enabled) VALUES(1,?1,'Target group',1,?2,'preparing',1) ON CONFLICT(project_id) DO UPDATE SET run_id=excluded.run_id,recipient=excluded.recipient,phase='preparing',enabled=1",params![id,identity]).unwrap();
    id
}
fn provider() -> (GatewayProfile, Arc<AtomicUsize>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut raw = Vec::new();
        let mut buf = [0; 8192];
        let mut boundary = None;
        let mut length = 0;
        loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            raw.extend_from_slice(&buf[..n]);
            if boundary.is_none() {
                if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    boundary = Some(p + 4);
                    length = String::from_utf8_lossy(&raw[..p])
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                }
            }
            if boundary.is_some_and(|p| raw.len() >= p + length) {
                break;
            }
        }
        let request: Value =
            serde_json::from_slice(&raw[boundary.unwrap()..boundary.unwrap() + length]).unwrap();
        let data: Value =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        let rows = data["untrustedEvidence"].as_array().unwrap();
        let labels=rows.iter().map(|r|json!({"key":r["key"],"label":"retain","confidence":0.99,"reason":"explicit independent target organization"})).collect::<Vec<_>>();
        let body=json!({"choices":[{"message":{"content":json!(labels).to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":70,"completion_tokens":30,"total_tokens":100}}).to_string();
        seen.fetch_add(1, Ordering::SeqCst);
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).unwrap();
    });
    (profile(&base), calls, worker)
}
#[test]
fn one_pass_crosses_local_pages_analyzes_once_and_caches_whole_project_proposal() {
    let mut c = fixture();
    for id in 1..=501 {
        add(&c, id, "one.example.com", true);
    }
    let (profile, calls, server) = provider();
    let id = bind(&mut c, &profile);
    run_pass(&mut c, 1, &id, &CancelToken::new(), |_| Ok(profile)).unwrap();
    server.join().unwrap();
    let view = assist_status(&c, 1).unwrap();
    assert_eq!(view["phase"], "awaiting_scope");
    assert_eq!(view["review"]["run"]["processedAssets"], 501);
    assert_eq!(view["review"]["run"]["requests"], 1);
    assert_eq!(view["proposal"]["groups"][0]["root"], "example.com");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(roots(&c, 1).unwrap().is_empty());
    assert_eq!(view["proposal"]["hunterRequests"], 0);
    let previous = view["proposal"].clone();
    c.execute("DELETE FROM asset_seed_review_refs WHERE run_id=?1", [&id])
        .unwrap();
    // A poll uses the cached receipt; confirmation performs the fresh validation.
    assert_eq!(assist_status(&c, 1).unwrap()["proposal"], previous);
}
#[test]
fn repeated_start_reuses_completed_model_receipts_and_run_instead_of_paying_again() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    let (profile, calls, server) = provider();
    let id = bind(&mut c, &profile);
    run_pass(&mut c, 1, &id, &CancelToken::new(), |_| Ok(profile.clone())).unwrap();
    server.join().unwrap();
    let second = bind(&mut c, &profile);
    assert_eq!(second, id);
    run_pass(&mut c, 1, &id, &CancelToken::new(), |_| {
        panic!("completed rows must not resolve or call a model")
    })
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM asset_seed_reviews", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM asset_seed_review_model_receipts",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn missing_external_evidence_is_automatically_parked_and_not_a_human_task() {
    let mut c = fixture();
    for id in 1..=1001 {
        add(&c, id, &format!("host{id}.external.test"), false);
    }
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    run_pass(&mut c, 1, &id, &CancelToken::new(), |_| {
        panic!("names alone must not be sent to a model")
    })
    .unwrap();
    let view = assist_status(&c, 1).unwrap();
    assert_eq!(view["phase"], "ready");
    assert_eq!(view["review"]["total"], 0);
    assert_eq!(view["review"]["run"]["requests"], 0);
    assert!(view["proposal"].is_null());
    assert!(
        view["review"]["run"]["automationCounts"]["missing_evidence"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert!(roots(&c, 1).unwrap().is_empty());
}
#[test]
fn interrupted_paid_rows_become_unknown_without_replay() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    c.execute(
        "UPDATE asset_seed_review_items SET state='analyzing' WHERE run_id=?1",
        [&id],
    )
    .unwrap();
    run_pass(&mut c, 1, &id, &CancelToken::new(), |_| {
        panic!("interrupted requests must not be replayed")
    })
    .unwrap();
    assert_eq!(
        assist_status(&c, 1).unwrap()["review"]["run"]["unknownRequests"],
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM asset_seed_review_items WHERE state='unknown'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    run_pass(&mut c, 1, &id, &CancelToken::new(), |_| {
        panic!("unknown rows remain isolated")
    })
    .unwrap();
    assert_eq!(
        assist_status(&c, 1).unwrap()["review"]["run"]["unknownRequests"],
        1
    );
}
#[test]
fn explicit_pause_survives_reentry_and_does_not_advance_the_local_cursor() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    c.execute("UPDATE asset_seed_assists SET enabled=0,phase='paused'", [])
        .unwrap();
    run_pass(&mut c, 1, &id, &CancelToken::new(), |_| {
        panic!("paused task must not resolve a model")
    })
    .unwrap();
    let v = assist_status(&c, 1).unwrap();
    assert_eq!(v["phase"], "paused");
    assert_eq!(v["enabled"], false);
    assert_eq!(v["review"]["run"]["cursor"], 0);
}
#[test]
fn recipient_change_is_rejected_without_new_run_or_rebinding_old_receipts() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    let original = profile("http://127.0.0.1:9/v1");
    let id = bind(&mut c, &original);
    let changed = profile("http://127.0.0.1:10/v1");
    assert!(check_recipient(&c, 1, &id, &recipient(&changed)).is_err());
    assert!(prepare_pass(
        &mut c,
        1,
        "Target group",
        true,
        &recipient(&changed),
        &HashSet::new()
    )
    .is_err());
    assert_eq!(
        c.query_row(
            "SELECT recipient FROM asset_seed_reviews WHERE id=?1",
            [&id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        recipient(&original)
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM asset_seed_reviews", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(check_recipient(&c, 2, &id, &recipient(&original)).is_err());
}
#[test]
fn confirming_a_proposal_clears_cached_suggestions_and_records_durable_enrichment_intent() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    c.execute("UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=0.99 WHERE run_id=?1",[&id]).unwrap();
    refresh_proposal(&c, 1, &id).unwrap();
    let proposal = assist_status(&c, 1).unwrap()["proposal"].clone();
    let selection:scope::Selection=serde_json::from_value(json!({"projectId":1,"runId":id,"hosts":["example.com"],"expectedRoots":[],"proposalToken":proposal["token"],"userConfirmed":true})).unwrap();
    let receipt = scope::confirm_on(&mut c, &selection).unwrap();
    assert_eq!(receipt["scopeSaved"], true);
    let token: String = c
        .query_row(
            "SELECT enrichment_pending FROM asset_seed_assists WHERE project_id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(json!(token), proposal["token"]);
    triage::run(&mut c, 1, &id).unwrap();
    refresh_proposal(&c, 1, &id).unwrap();
    let after = assist_status(&c, 1).unwrap();
    assert_eq!(after["phase"], "ready");
    assert!(after["proposal"].is_null());
    enrichment_receipt(&c, 1, &id, &token, Some("synthetic launch failure")).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT enrichment_pending FROM asset_seed_assists",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        token
    );
    enrichment_receipt(&c, 1, &id, &token, None).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT enrichment_pending FROM asset_seed_assists",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        ""
    );
}
#[test]
fn stale_cached_proposal_is_replaceable_after_background_evidence_changes() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    c.execute("UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=.99 WHERE run_id=?1",[&id]).unwrap();
    refresh_proposal(&c, 1, &id).unwrap();
    let old = assist_status(&c, 1).unwrap()["proposal"].clone();
    c.execute(
        "UPDATE asset_seed_review_items SET revision=revision+1 WHERE run_id=?1",
        [&id],
    )
    .unwrap();
    let selection:scope::Selection=serde_json::from_value(json!({"projectId":1,"runId":id,"hosts":["example.com"],"expectedRoots":[],"proposalToken":old["token"],"userConfirmed":true})).unwrap();
    assert!(scope::confirm_on(&mut c, &selection).is_err());
    refresh_proposal(&c, 1, &id).unwrap();
    assert_ne!(
        assist_status(&c, 1).unwrap()["proposal"]["token"],
        old["token"]
    );
    assert!(roots(&c, 1).unwrap().is_empty());
}

fn disk_fixture(c: &Connection) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("ovi-assist-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.sqlite");
    c.execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    (dir, path)
}
fn wait_free_paused(manager: &Manager, c: &Connection) {
    manager.pause(1).unwrap();
    let start = std::time::Instant::now();
    loop {
        let value = manager.enrichment.dispatch(1,"status",json!({}),"").unwrap();
        if value["run"]["status"] == "paused" { break; }
        assert!(start.elapsed() < Duration::from_secs(5),"{value}");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_runs WHERE enabled=1",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}
fn saved_free_queue(c: &Connection, scope: &str, status: &str) -> String {
    // The production owner registry is process-global. Parallel temporary DBs
    // must use distinct run IDs, exactly as UUID-backed production runs do.
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    c.execute("INSERT INTO asset_enrichment_runs(id,project_id,operation_id,roots_json,status,enabled,continuous,interval_seconds,workers,asset_cursor,seed_cursor,last_sync,created_at,updated_at) VALUES(?1,1,'old-assist-operation',?2,?3,0,1,3600,1,501,702,?4,?4,?4)",params![id,json!([scope]).to_string(),status,now]).unwrap();
    for kind in ["ct","dns_a","dns_aaaa","dns_cname","tls"] {
        let target=if kind=="tls" {format!("https://{scope}/")}else{scope.into()};
        c.execute("INSERT INTO asset_enrichment_items(run_id,kind,host,target,status,attempts,failures,next_at,result_json,updated_at) VALUES(?1,?2,?3,?4,?5,3,1,?6,'{\"saved\":true}',?7)",params![id,kind,scope,target,if kind=="tls" {"retry"}else{"done"},now+86400,now]).unwrap();
    }
    id
}
#[test]
fn ai_pause_continue_and_restart_resume_the_same_free_queue_without_resetting_progress() {
    let c=fixture();c.execute("INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.test\"]')",[]).unwrap();
    let(dir,path)=disk_fixture(&c);drop(c);
    let manager=Manager::new(&path,enrichment::Manager::new(&path).unwrap()).unwrap();
    let c=db::open(&path).unwrap();let saved=saved_free_queue(&c,"example.test","paused");
    let snapshot=|c:&Connection| c.query_row("SELECT json_group_array(json_array(kind,status,attempts,failures,next_at,result_json)) FROM (SELECT * FROM asset_enrichment_items WHERE run_id=?1 ORDER BY id)",[&saved],|r|r.get::<_,String>(0)).unwrap();
    let before=snapshot(&c);
    for status in ["paused","completed"] {
        c.execute("UPDATE asset_enrichment_runs SET status=?1 WHERE id=?2",params![status,saved]).unwrap();
        manager.start_enrichment(&c,1).unwrap();
        manager.start_enrichment(&c,1).unwrap(); // Already enabled wins over a fresh operation ID.
        assert_eq!(manager.enrichment.dispatch(1,"status",json!({}),"").unwrap()["run"]["id"],saved);
        wait_free_paused(&manager,&c);
        assert_eq!(snapshot(&c),before);
        assert_eq!(c.query_row("SELECT asset_cursor FROM asset_enrichment_runs WHERE id=?1",[&saved],|r|r.get::<_,i64>(0)).unwrap(),501);
        assert_eq!(c.query_row("SELECT seed_cursor FROM asset_enrichment_runs WHERE id=?1",[&saved],|r|r.get::<_,i64>(0)).unwrap(),702);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_runs",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    c.execute("UPDATE asset_enrichment_runs SET status='interrupted',enabled=1,owner_pid=0",[]).unwrap();
    manager.shutdown();drop(c);
    let reopened=Manager::new(&path,enrichment::Manager::new(&path).unwrap()).unwrap();
    let c=db::open(&path).unwrap();
    assert_eq!(reopened.enrichment.dispatch(1,"status",json!({}),"").unwrap()["run"]["id"],saved);
    reopened.start_enrichment(&c,1).unwrap();wait_free_paused(&reopened,&c);
    assert_eq!(snapshot(&c),before);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_runs",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    reopened.shutdown();drop(c);std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn ai_continue_never_reuses_another_scope_or_revives_a_scope_changed_history() {
    for status in ["paused","scope_changed"] {
        let c=fixture();c.execute("INSERT INTO asset_discovery_scopes VALUES(1,'[\"current.test\"]')",[]).unwrap();
        // All executable names are excluded in this local-only test; no CT,
        // DNS or TLS network operation can be admitted by the real worker.
        c.execute_batch("CREATE TABLE asset_discovery_domain_exclusions(project_id INTEGER,domain TEXT,enabled INTEGER);INSERT INTO asset_discovery_domain_exclusions VALUES(1,'current.test',1);").unwrap();
        let(dir,path)=disk_fixture(&c);drop(c);
        let manager=Manager::new(&path,enrichment::Manager::new(&path).unwrap()).unwrap();
        let c=db::open(&path).unwrap();
        let saved=saved_free_queue(&c,if status=="paused" {"old.test"}else{"current.test"},status);
        manager.start_enrichment(&c,1).unwrap();
        let view=manager.enrichment.dispatch(1,"status",json!({}),"").unwrap();
        assert_ne!(view["run"]["id"],saved);
        wait_free_paused(&manager,&c);
        assert_eq!(c.query_row("SELECT status FROM asset_enrichment_runs WHERE id=?1",[&saved],|r|r.get::<_,String>(0)).unwrap(),status);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_runs",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        manager.shutdown();drop(c);std::fs::remove_dir_all(dir).unwrap();
    }
}
fn configured(c: &Connection, profile: &GatewayProfile) {
    c.execute_batch("CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,is_default INTEGER NOT NULL,settings_json TEXT NOT NULL,updated_at TEXT NOT NULL)").unwrap();
    let base = profile.endpoint.strip_suffix("/chat/completions").unwrap();
    c.execute("INSERT INTO config_profiles VALUES(1,1,?1,'2000-01-01')",[json!({"activeModelProfileId":"selected","modelProfiles":[{"id":"selected","llm":"openai/synthetic-test-model","deployment":"local","apiBase":base,"localApiKey":"synthetic-key"}]}).to_string()]).unwrap();
}
#[test]
fn startup_recovers_authorized_chain_from_disk_then_leaves_cached_proposal_waiting() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    let (profile, calls, server) = provider();
    bind(&mut c, &profile);
    configured(&c, &profile);
    let (dir, path) = disk_fixture(&c);
    drop(c);
    let enrichment = enrichment::Manager::new(&path).unwrap();
    let manager = Manager::new(&path, enrichment).unwrap();
    let c = db::open(&path).unwrap();
    let start = std::time::Instant::now();
    loop {
        let v = assist_status(&c, 1).unwrap();
        if v["phase"] == "awaiting_scope" {
            break;
        }
        assert_ne!(v["phase"], "blocked", "{v}");
        assert!(start.elapsed() < Duration::from_secs(15), "{v}");
        thread::sleep(Duration::from_millis(10));
    }
    server.join().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let before = assist_status(&c, 1).unwrap();
    let reopened = Manager::new(&path, enrichment::Manager::new(&path).unwrap()).unwrap();
    assert_eq!(
        assist_status(&c, 1).unwrap()["proposal"],
        before["proposal"]
    );
    assert_eq!(assist_status(&c, 1).unwrap()["phase"], "awaiting_scope");
    manager.shutdown();
    reopened.shutdown();
    drop(c);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn startup_does_not_resume_persisted_pause_or_send_to_a_changed_recipient() {
    for paused in [false, true] {
        let mut c = fixture();
        add(&c, 1, "one.example.com", true);
        let original = profile("http://127.0.0.1:9/v1");
        bind(&mut c, &original);
        configured(&c, &profile("http://127.0.0.1:10/v1"));
        if paused {
            c.execute("UPDATE asset_seed_assists SET enabled=0,phase='paused'", [])
                .unwrap();
        }
        let (dir, path) = disk_fixture(&c);
        drop(c);
        let manager = Manager::new(&path, enrichment::Manager::new(&path).unwrap()).unwrap();
        let c = db::open(&path).unwrap();
        let view = assist_status(&c, 1).unwrap();
        assert_eq!(view["phase"], if paused { "paused" } else { "blocked" });
        assert_eq!(view["enabled"], false);
        assert_eq!(view["review"]["run"]["requests"], 0);
        manager.shutdown();
        drop(c);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn enabled_workflow_automatically_analyzes_new_evidence_but_never_replays_completed_rows() {
    let mut c = fixture();
    add(&c, 1, "old.example.com", true);
    let (profile, calls, server) = provider();
    let id = bind(&mut c, &profile);
    configured(&c, &profile);
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    c.execute("UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=.99,reason='completed before new discovery' WHERE run_id=?1",[&id]).unwrap();
    refresh_proposal(&c, 1, &id).unwrap();
    add(&c, 2, "fresh.other.test", true);
    let (dir, path) = disk_fixture(&c);
    drop(c);
    let manager = Manager::new(&path, enrichment::Manager::new(&path).unwrap()).unwrap();
    manager.continue_pending().unwrap();
    let c = db::open(&path).unwrap();
    let start = std::time::Instant::now();
    loop {
        let pending: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM asset_seed_review_items WHERE state='pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let done: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM asset_seed_review_model_receipts",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if pending == 0 && done == 1 && !active(&id) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(15));
        thread::sleep(Duration::from_millis(10));
    }
    server.join().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let sent: String = c
        .query_row(
            "SELECT request_keys_json FROM asset_seed_review_model_receipts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!sent.contains("domain_group:example.com"));
    assert!(sent.contains("other.test"));
    let before = c
        .query_row(
            "SELECT requests FROM asset_seed_reviews WHERE id=?1",
            [&id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    manager.continue_pending().unwrap();
    assert!(!active(&id));
    assert_eq!(
        c.query_row(
            "SELECT requests FROM asset_seed_reviews WHERE id=?1",
            [&id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        before
    );
    manager.shutdown();
    drop(c);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn already_confirmed_unrelated_family_refresh_does_not_expire_new_family_consent_token() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    add(&c, 2, "confirmed.other.test", true);
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    c.execute(
        "UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=.99",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(1,'[\"other.test\"]')",
        [],
    )
    .unwrap();
    refresh_proposal(&c, 1, &id).unwrap();
    let old = assist_status(&c, 1).unwrap()["proposal"]["token"].clone();
    c.execute("UPDATE asset_seed_review_items SET revision=revision+1,evidence_json=json_set(evidence_json,'$.backgroundRefresh','new TLS evidence') WHERE evidence_json LIKE '%confirmed.other.test%'",[]).unwrap();
    refresh_proposal(&c, 1, &id).unwrap();
    assert_eq!(assist_status(&c, 1).unwrap()["proposal"]["token"], old);
    c.execute("UPDATE asset_seed_review_items SET revision=revision+1 WHERE evidence_json LIKE '%one.example.com%'",[]).unwrap();
    refresh_proposal(&c, 1, &id).unwrap();
    assert_ne!(assist_status(&c, 1).unwrap()["proposal"]["token"], old);
}
#[test]
fn manual_rejection_refreshes_cached_proposal_instead_of_leaving_a_selected_rejected_family() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    c.execute(
        "UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=.99",
        [],
    )
    .unwrap();
    refresh_proposal(&c, 1, &id).unwrap();
    let keys: Vec<(String, i64)> = c
        .prepare("SELECT item_key,revision FROM asset_seed_review_items WHERE run_id=?1")
        .unwrap()
        .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for (key, revision) in keys {
        decide_on(
            &mut c,
            ReviewDecision {
                project_id: 1,
                run_id: id.clone(),
                key,
                expected_revision: revision,
                decision: "rejected".into(),
            },
        )
        .unwrap();
    }
    assert!(assist_status(&c, 1).unwrap()["proposal"].is_null());
    assert_eq!(assist_status(&c, 1).unwrap()["phase"], "ready");
}

#[test]
fn legacy_collection_labels_and_mismatched_certificate_subjects_do_not_propose_external_roots() {
    let mut c = fixture();
    for (id, name) in [
        (1, "000511.cn"),
        (2, "007rcw.com"),
        (3, "xcx.bmcc.ytult.top"),
        (4, "vsp.cmitry.com"),
    ] {
        add(&c, id, name, false);
    }
    c.execute("UPDATE assets SET extra_json=?1 WHERE id=3",[json!({"cert.domain":"bmcc.com.cn","cert.subject.cn":"xcx.bmcc.com.cn","cert.subject.org":"Target group"}).to_string()]).unwrap();
    c.execute("UPDATE assets SET extra_json=?1 WHERE id=4",[json!({"cert.domain":"*.cmitry.com","cert.subject.org":"Target group","cname":"vsp.cmitry.com.e.aliyunwaf5.com"}).to_string()]).unwrap();
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    c.execute("UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=.9,reason='legacy raw advice preserved'",[]).unwrap();
    refresh_proposal(&c, 1, &id).unwrap();
    let view = assist_status(&c, 1).unwrap();
    let groups = view["proposal"]["groups"].as_array().unwrap();
    for bad in ["000511.cn", "007rcw.com", "ytult.top", "aliyunwaf5.com"] {
        assert!(
            !groups.iter().any(|g| g["root"] == bad),
            "{bad} was inherited from unrelated certificate evidence"
        );
    }
    assert!(groups.iter().any(|g| g["root"] == "bmcc.com.cn"));
    assert!(groups.iter().any(|g| g["root"] == "cmitry.com"));
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_seed_review_items WHERE advice!='retain' OR reason!='legacy raw advice preserved'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(roots(&c, 1).unwrap().is_empty());
    assert_eq!(view["review"]["run"]["requests"], 0);
}
#[test]
fn certificate_refresh_during_ai_is_durably_queued_then_indexed_when_the_owner_is_idle() {
    let mut c = fixture();
    add(&c, 1, "fixture.example.test", false);
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    let cert =
        crate::certificates::from_der(include_bytes!("../../test-fixtures/tls/leaf.der")).unwrap();
    c.execute(
        "UPDATE assets SET extra_json=?1 WHERE id=1",
        [
            json!({"cert.domain":"fixture.example.test","probe_cert_der_base64":cert.der_base64})
                .to_string(),
        ],
    )
    .unwrap();
    controls()
        .lock()
        .unwrap()
        .insert(id.clone(), CancelToken::new());
    refresh_enriched_asset(&mut c, 1, 1).unwrap();
    controls().lock().unwrap().remove(&id);
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM asset_seed_assist_refresh", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    drain_refresh(&mut c, 1, &id).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM asset_seed_assist_refresh", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM asset_seed_review_items WHERE kind='certificate' AND target=?1",
            [cert.sha256],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        assist_status(&c, 1).unwrap()["review"]["run"]["requests"],
        0
    );
}

#[test]
fn older_global_asset_attached_to_project_is_discovered_without_repaying_for_old_advice() {
    let mut c = fixture();
    add(&c, 1, "fresh.other.test", true);
    c.execute(
        "UPDATE project_assets SET project_id=2 WHERE asset_id=1",
        [],
    )
    .unwrap();
    add(&c, 100, "old.external.test", false);
    let (profile, calls, server) = provider();
    let id = bind(&mut c, &profile);
    configured(&c, &profile);
    run_pass(&mut c, 1, &id, &CancelToken::new(), |_| {
        panic!("old names-only evidence is not a model request")
    })
    .unwrap();
    let (dir, path) = disk_fixture(&c);
    drop(c);
    let manager = Manager::new(&path, enrichment::Manager::new(&path).unwrap()).unwrap();
    let c = db::open(&path).unwrap();
    c.execute("INSERT INTO project_assets VALUES(1,1,0,'pending')", [])
        .unwrap();
    manager.continue_pending().unwrap();
    let start = std::time::Instant::now();
    loop {
        if !active(&id) && assist_status(&c, 1).unwrap()["review"]["run"]["requests"] == 1 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(15));
        thread::sleep(Duration::from_millis(10));
    }
    server.join().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        assist_status(&c, 1).unwrap()["review"]["run"]["processedAssets"],
        2
    );
    assert!(assist_status(&c, 1).unwrap()["proposal"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g["root"] == "other.test"));
    manager.shutdown();
    drop(c);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn failed_free_enrichment_intent_retries_with_backoff_without_a_new_model_request() {
    let mut c = fixture();
    add(&c, 1, "old.external.test", false);
    let id = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    run_pass(&mut c, 1, &id, &CancelToken::new(), |_| {
        panic!("names-only evidence does not need AI")
    })
    .unwrap();
    record_enrichment_intent(&c, 1, &id, "synthetic-intent").unwrap();
    let (dir, path) = disk_fixture(&c);
    drop(c);
    // Empty roots make start fail before any network request. The durable intent
    // remains and the watcher backs off instead of repeatedly asking the user.
    let manager = Manager::new(&path, enrichment::Manager::new(&path).unwrap()).unwrap();
    let c = db::open(&path).unwrap();
    let retry: i64 = c
        .query_row(
            "SELECT enrichment_retry_at FROM asset_seed_assists",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(retry > chrono::Utc::now().timestamp() + 30);
    manager.continue_pending().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT enrichment_retry_at FROM asset_seed_assists",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        retry
    );
    assert_eq!(
        c.query_row(
            "SELECT enrichment_pending FROM asset_seed_assists",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "synthetic-intent"
    );
    assert_eq!(
        assist_status(&c, 1).unwrap()["review"]["run"]["requests"],
        0
    );
    manager.shutdown();
    drop(c);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn restoring_a_nonisolated_run_exposes_its_original_input_and_reuses_that_run() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    add(&c, 2, "isolated.external.test", false);
    c.execute("UPDATE project_assets SET is_deleted=1 WHERE asset_id=2", []).unwrap();
    let identity = recipient(&profile("http://127.0.0.1:9/v1"));
    let id = prepare_pass(&mut c, 1, "Target group", false, &identity, &HashSet::new()).unwrap();
    c.execute("INSERT INTO asset_seed_assists(project_id,run_id,organization,include_isolated,recipient,phase,enabled) VALUES(1,?1,'Target group',0,?2,'paused',0)",params![id,identity]).unwrap();
    let restored = assist_status(&c, 1).unwrap();
    assert_eq!(restored["includeIsolated"], false);
    assert_eq!(restored["review"]["run"]["includeIsolated"], false);
    let same = prepare_pass(&mut c, 1, "Target group", restored["review"]["run"]["includeIsolated"].as_bool().unwrap(), &identity, &HashSet::new()).unwrap();
    assert_eq!(same, id);
    assert_eq!(restored["review"]["run"]["totalAssets"], 1);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_seed_reviews", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
}

#[test]
fn evidence_status_can_follow_the_assist_run_without_switching_to_a_newer_target() {
    let mut c = fixture();
    add(&c, 1, "one.example.com", true);
    let original = bind(&mut c, &profile("http://127.0.0.1:9/v1"));
    while !harvest_page(&mut c, 1, &original).unwrap() {}
    let newer = prepare_on(&mut c, 1, "Different target", true, &HashSet::new()).unwrap();
    while !harvest_page(&mut c, 1, &newer).unwrap() {}
    let active = assist_status(&c, 1).unwrap();
    let exact = view_run_on(&c, 1, Some(&original), "all", 1, "").unwrap();
    assert_eq!(active["review"]["run"]["id"], original);
    assert_eq!(exact["run"]["id"], original);
    assert_eq!(exact["run"]["organization"], "Target group");
    assert_eq!(view_on(&c, 1, "all", 1, "").unwrap()["run"]["id"], newer);
    assert!(view_run_on(&c, 2, Some(&original), "all", 1, "").unwrap()["run"].is_null());
}
