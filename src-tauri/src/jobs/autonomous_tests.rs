use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
struct Sink;
impl AssetJobEventSink for Sink {
    fn publish(&self, _: &str, _: &Value) -> Result<(), String> {
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    path: PathBuf,
    context: AssetJobContext,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("oviraptor-agent-scope-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("configuration")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for p in [&root, &root.join("configuration")] {
                fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let path = db::initialize(&root).unwrap();
        let c = db::open(&path).unwrap();
        let instance = Uuid::new_v4().to_string();
        c.execute_batch("CREATE TABLE asset_service_store(singleton INTEGER PRIMARY KEY,instance_id TEXT NOT NULL UNIQUE,schema_version INTEGER NOT NULL);").unwrap();
        c.execute(
            "INSERT INTO asset_service_store VALUES(1,?1,1)",
            [&instance],
        )
        .unwrap();
        c.execute_batch(
            "INSERT INTO projects(id,name) VALUES(9001,'Synthetic Agent'),(9002,'Other project');",
        )
        .unwrap();
        c.execute_batch(SCHEMA).unwrap();
        crate::asset_discovery::action_on(
            &path,
            &root,
            crate::asset_discovery::DiscoveryInput {
                project_id: 9001,
                action: "scopes".into(),
                roots: vec!["example.com".into()],
                host: String::new(),
                cursor: 0,
                request_id: String::new(),
                hunter_kind: "domain".into(),
            },
        )
        .unwrap();
        let context = AssetJobContext::service(&path, &root, instance, Arc::new(Sink)).unwrap();
        Self {
            root,
            path,
            context,
        }
    }
    fn asset(&self, id: i64, project: i64, host: &str, category: &str) {
        let c = db::open(&self.path).unwrap();
        c.execute("INSERT INTO assets(id,asset_key,host,link,title,content_category) VALUES(?1,?2,?3,?3,?4,?5)",params![id,format!("synthetic-{id}"),format!("https://{host}/"),format!("title {id}"),category]).unwrap();
        c.execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(?1,?2)",
            params![project, id],
        )
        .unwrap();
    }
    fn run<'a>(&'a self, cancel: CancelToken) -> Run<'a> {
        let c = db::open(&self.path).unwrap();
        let limits = Limits {
            max_rounds: 5,
            max_assets: 3,
            max_minutes: 1,
            hunter_requests: 0,
            hunter_enabled: false,
        };
        c.execute("INSERT OR IGNORE INTO asset_autonomous_runs(id,project_id,status,goal,limits_json,roots_json,model) VALUES('synthetic',9001,'running','synthetic',?1,'[\"example.com\"]','fixture')",[json!(limits).to_string()]).unwrap();
        Run {
            context: &self.context,
            id: "synthetic",
            project: 9001,
            roots: vec!["example.com".into()],
            limits,
            cancel,
            deadline: Instant::now() + Duration::from_secs(60),
            seen: HashSet::new(),
            probed: 0,
            hunter: 0,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn decision(action: &str) -> Decision {
    Decision {
        action: action.into(),
        reason: "根据真实线索选择下一步".into(),
        host: "example.com".into(),
        hosts: vec![],
        asset_ids: vec![],
        cursor: 0,
        hunter_kind: "domain".into(),
    }
}
#[test]
fn selection_never_expands_and_rejects_cross_project_stale_or_deleted() {
    let f = Fixture::new();
    f.asset(1, 9001, "one.example.com", "");
    f.asset(2, 9001, "two.example.com", "");
    f.asset(3, 9002, "other.example.com", "");
    let c = db::open(&f.path).unwrap();
    let scope = |ids: Vec<i64>, count: i64| ReprobeScope::Selection {
        selections: ids
            .into_iter()
            .map(|asset_id| AssetSelection {
                project_id: 9001,
                asset_id,
            })
            .collect(),
        expected_count: count,
    };
    assert_eq!(
        crate::commands::assets::reprobe_members_on(&c, 9001, &scope(vec![2], 1)).unwrap(),
        vec![2]
    );
    assert!(crate::commands::assets::reprobe_members_on(&c, 9001, &scope(vec![3], 1)).is_err());
    assert!(crate::commands::assets::reprobe_members_on(&c, 9001, &scope(vec![1, 2], 1)).is_err());
    c.execute(
        "UPDATE project_assets SET is_deleted=1 WHERE asset_id=2",
        [],
    )
    .unwrap();
    assert!(crate::commands::assets::reprobe_members_on(&c, 9001, &scope(vec![2], 1)).is_err());
}
#[test]
fn filtered_reprobe_preserves_conditions_and_counts_members_across_pages() {
    let f = Fixture::new();
    f.asset(1, 9001, "one.example.com", "");
    f.asset(2, 9001, "two.example.com", "");
    f.asset(3, 9002, "two.example.com", "");
    let c = db::open(&f.path).unwrap();
    let mut query = crate::asset_models::AssetQuery {
        project_id: Some(9001),
        search: "two.example.com".into(),
        page: 99,
        page_size: 10,
        collapse_identical_responses: true,
        ..Default::default()
    };
    let resolve = |q| {
        crate::commands::assets::reprobe_members_on(
            &c,
            9001,
            &ReprobeScope::Query {
                query: q,
                expected_count: 1,
            },
        )
    };
    assert_eq!(resolve(query.clone()).unwrap(), vec![2]);
    query.search = "no-match.example.com".into();
    assert!(resolve(query.clone()).is_err());
    query.project_id = Some(9002);
    assert!(resolve(query).is_err());
}
#[test]
fn reprobe_frozen_membership_survives_resume_and_new_project_assets() {
    let f = Fixture::new();
    f.asset(1, 9001, "one.example.com", "");
    f.asset(2, 9001, "two.example.com", "");
    let refined = f.root.join("frozen");
    let ids = json!([2]);
    assert_eq!(
        write_scoped_reprobe_sources(&f.path, 9001, &refined, Some(&ids)).unwrap(),
        1
    );
    let bytes = fs::read(refined.join("P1_active_strong.csv")).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("two.example.com"));
    assert!(!String::from_utf8_lossy(&bytes).contains("one.example.com"));
    f.asset(4, 9001, "new.example.com", "");
    assert_eq!(
        write_scoped_reprobe_sources(&f.path, 9001, &refined, Some(&ids)).unwrap(),
        1
    );
    assert_eq!(
        fs::read(refined.join("P1_active_strong.csv")).unwrap(),
        bytes
    );
    assert!(
        write_scoped_reprobe_sources(&f.path, 9001, &f.root.join("bad"), Some(&json!({}))).is_err()
    );
    assert!(!f.root.join("bad").exists());
}
#[test]
fn agent_enforces_project_domain_budget_and_cancel_before_effect() {
    let f = Fixture::new();
    f.asset(1, 9001, "one.example.com", "");
    f.asset(2, 9001, "example.com.evil.test", "");
    f.asset(3, 9002, "other.example.com", "");
    let c = db::open(&f.path).unwrap();
    assert_eq!(
        validate_assets(&c, 9001, &["example.com".into()], &[1]).unwrap(),
        vec![1]
    );
    assert!(validate_assets(&c, 9001, &["example.com".into()], &[2]).is_err());
    assert!(validate_assets(&c, 9001, &["example.com".into()], &[3]).is_err());
    let mut run = f.run(CancelToken::new());
    assert!(run
        .effect(&decision("hunter"))
        .unwrap_err()
        .contains("Hunter"));
    let mut d = decision("probe");
    d.asset_ids = vec![1, 2, 3, 4];
    assert!(run.effect(&d).unwrap_err().contains("预算"));
    assert_eq!(run.probed, 0);
    run.cancel.cancel();
    assert!(run.effect(&decision("dns")).unwrap_err().contains("停止"));
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM runs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn seed_probe_requires_observed_concrete_in_scope_host_and_preserves_trash() {
    let f = Fixture::new();
    let mut c = db::open(&f.path).unwrap();
    c.execute("INSERT INTO asset_discovery_seeds(project_id,host,source,source_ref,wildcard) VALUES(9001,'new.example.com','certificate_transparency','example.com',0),(9001,'example.com','certificate_cn','asset:1',1),(9001,'evil.test','dns_cname','example.com',0)",[]).unwrap();
    for host in ["guess.example.com", "example.com", "evil.test"] {
        assert!(seed_candidates(&mut c, 9001, &["example.com".into()], &[host.into()]).is_err());
    }
    let ids = seed_candidates(
        &mut c,
        9001,
        &["example.com".into()],
        &["new.example.com".into()],
    )
    .unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(
        seed_candidates(
            &mut c,
            9001,
            &["example.com".into()],
            &["new.example.com".into()]
        )
        .unwrap(),
        ids
    );
    let (tier,score,decision):(String,String,String)=c.query_row("SELECT a.review_tier,a.score,pa.decision FROM project_assets pa JOIN assets a ON a.id=pa.asset_id",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        (tier, score, decision),
        ("P3".into(), "0".into(), "pending".into())
    );
    c.execute("UPDATE project_assets SET is_deleted=1", [])
        .unwrap();
    assert!(seed_candidates(
        &mut c,
        9001,
        &["example.com".into()],
        &["new.example.com".into()]
    )
    .is_err());
}
#[test]
fn malformed_and_unknown_model_actions_have_no_execution_path() {
    for text in [
        "do a scan",
        r#"{"action":"shell","reason":"do it"}"#,
        r#"{"action":"dns","reason":"x","projectId":9002}"#,
        r#"{"action":"dns","reason":""}"#,
    ] {
        assert!(parse_decision(text).is_err());
    }
    let mut a = decision("probe");
    a.asset_ids = vec![1, 2];
    let mut b = a.clone();
    b.asset_ids = vec![2, 1, 1];
    assert_eq!(step_key(&a), step_key(&b));
}
#[test]
fn actual_gateway_replans_from_tool_result_and_does_not_repeat_action() {
    let f = Fixture::new();
    for id in 1..=251 {f.asset(id,9001,&format!("host-{id}.example.com"),"");}
    let _ = f.run(CancelToken::new());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let capture = Arc::new(Mutex::new(Vec::<Value>::new()));
    let output = capture.clone();
    let server = thread::spawn(move || {
        for action in ["inventory", "inventory", "stop"] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = vec![];
            let mut b = [0u8; 4096];
            let header_end = loop {
                let n = stream.read(&mut b).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&b[..n]);
                if let Some(i) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let header = String::from_utf8_lossy(&bytes[..header_end]);
            let len = header
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            while bytes.len() < header_end + len {
                let n = stream.read(&mut b).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&b[..n]);
            }
            output
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&bytes[header_end..header_end + len]).unwrap());
            let body=json!({"choices":[{"message":{"role":"assistant","content":json!({"action":action,"reason":"依据工具结果继续规划"}).to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
    });
    let mut profile = crate::agent_runtime::model::GatewayProfile::resolve(
        &crate::agent_runtime::model::ResolvedModelProfile {
            model: "synthetic-model".into(),
            api_base: format!("http://{address}/v1"),
            api_key: String::new(),
            deployment: "local".into(),
        },
        &Default::default(),
        None,
    )
    .unwrap();
    profile.max_output_tokens = Some(1200);
    let input = StartInput {
        project_id: 9001,
        goal: "synthetic".into(),
        limits: Limits {
            max_rounds: 5,
            max_assets: 3,
            max_minutes: 1,
            hunter_requests: 0,
            hunter_enabled: false,
        },
    };
    assert!(execute(
        &f.context,
        "synthetic",
        input,
        vec!["example.com".into()],
        OpenAiCompatibleGateway::new(profile, &[]),
        CancelToken::new()
    )
    .is_ok());
    server.join().unwrap();
    let requests = capture.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let state: Value =
        serde_json::from_str(requests[1]["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(state["recentReceipts"][0]["result"]["items"][0]["id"], 1);
    let c = db::open(&f.path).unwrap();
    let states: Vec<String> = c
        .prepare("SELECT state FROM asset_autonomous_steps ORDER BY round")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(states, ["completed", "completed", "completed"]);
    let third: Value=serde_json::from_str(requests[2]["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(third["recentReceipts"][1]["result"]["items"][0]["id"],251);
    assert_eq!(third["pagination"]["inventory"]["complete"],true);
}

#[test]
fn agent_probe_action_waits_for_real_native_http_job_and_reads_persisted_result() {
    let f = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://localhost:{}/",
        listener.local_addr().unwrap().port()
    );
    let c = db::open(&f.path).unwrap();
    c.execute(
        "UPDATE asset_discovery_scopes SET roots_json='[\"localhost\"]' WHERE project_id=9001",
        [],
    )
    .unwrap();
    c.execute("UPDATE config_profiles SET settings_json=?1 WHERE is_default=1", [json!({"fofaKey":"","configPath":"","allowPrivate":true,"workers":1,"probeTimeout":2,"probeTotalTimeout":3,"probeRetries":0,"schemeFallback":false,"perHostInterval":0,"priorityRate":1000,"otherRate":1000,"proxyUrl":""}).to_string()]).unwrap();
    c.execute("INSERT INTO assets(id,asset_key,host,link,port,protocol) VALUES(1,'local-agent-candidate',?1,?1,?2,'http')",params![url,listener.local_addr().unwrap().port().to_string()]).unwrap();
    c.execute(
        "INSERT INTO project_assets(project_id,asset_id) VALUES(9001,1)",
        [],
    )
    .unwrap();
    let served = Arc::new(AtomicBool::new(false));
    let output = served.clone();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut b = [0; 8192];
                    let _ = stream.read(&mut b);
                    let body="<html><title>synthetic native agent probe</title><body>Local fixture only</body></html>";
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
                    output.store(true, Ordering::SeqCst);
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("fixture listener: {e}"),
            }
        }
    });
    let mut run = f.run(CancelToken::new());
    run.roots = vec!["localhost".into()];
    let mut d = decision("probe");
    d.asset_ids = vec![1];
    let result = run.effect(&d).unwrap();
    server.join().unwrap();
    assert!(served.load(Ordering::SeqCst));
    assert_eq!(result["status"], "completed");
    assert_eq!(result["results"][0]["assetId"], 1);
    assert_eq!(result["results"][0]["statusCode"], "200");
    assert_eq!(run.probed, 1);
    let until = Instant::now() + Duration::from_secs(5);
    while f.context.busy().unwrap() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn inventory_prioritizes_confirmed_scope_without_losing_late_ids() {
    let f=Fixture::new();
    for id in 1..=300 { f.asset(id,9001,&format!("noise-{id}.unrelated.net"),""); }
    f.asset(10001,9001,"dns.cdn.example.com","");
    f.asset(10002,9001,"example.com.attacker.test","");
    f.asset(10003,9002,"other.example.com","");
    let c=db::open(&f.path).unwrap();let page=inventory(&c,9001,&["example.com".into()],0).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(),1);
    assert_eq!(page["items"][0]["id"],10001);assert_eq!(page["hasMore"],false);
}
#[test]
fn runtime_pagination_replaces_omitted_or_replayed_model_cursors() {
    let mut pages=Pagination::default();let mut d=decision("inventory");pages.prepare(&mut d).unwrap();let first=step_key(&d);
    pages.record("inventory",&json!({"nextCursor":1375,"hasMore":true,"items":[]}));
    d.cursor=0;pages.prepare(&mut d).unwrap();assert_eq!(d.cursor,1375);assert_ne!(first,step_key(&d));
    pages.record("inventory",&json!({"nextCursor":2000,"hasMore":false}));assert!(pages.prepare(&mut d).is_err());
    let mut harvest=decision("harvest");pages.prepare(&mut harvest).unwrap();assert_eq!(harvest.cursor,0);
    pages.record("harvest",&json!({"nextCursor":8000,"batchRows":1000}));pages.prepare(&mut harvest).unwrap();assert_eq!(harvest.cursor,8000);
}
#[test]
fn hunter_zero_is_unlimited_only_with_explicit_permission() {
    let f=Fixture::new();let mut run=f.run(CancelToken::new());run.limits.hunter_requests=3;
    assert_eq!(hunter_capability(&run.limits)["enabled"],false);
    assert!(run.effect(&decision("hunter")).unwrap_err().contains("未授权"));assert_eq!(run.hunter,0);
    run.limits.hunter_enabled=true;run.limits.hunter_requests=0;
    assert_eq!(hunter_capability(&run.limits)["enabled"],true);assert!(hunter_capability(&run.limits)["queryLimit"].is_null());
}
