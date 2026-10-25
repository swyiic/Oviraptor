use super::*;
fn fixture() -> (Connection, work::Claim) {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,extra_json TEXT);CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER);CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);").unwrap();
    super::super::ensure(&c).unwrap();
    ensure(&c).unwrap();
    let k = work::Claim {
        id: "cleanup-one".into(),
        p: 1,
        generation: 1,
        token: "claim-one".into(),
    };
    c.execute("INSERT INTO asset_cleanup_runs(id,project_id,organization,recipient,model,target_names_json,roots_json,auto_exclude,phase,enabled,raw_count,ledger_count,created_at,updated_at,claim_token,owner_pid,lease_until) VALUES(?1,1,'Target Group','fixture','fixture','[\"target\"]','[]',1,'preparing',1,0,0,?2,?2,?3,?4,?5)",params![k.id,now(),k.token,std::process::id(),now()+180]).unwrap();
    (c, k)
}
fn add(c: &Connection, k: &work::Claim, id: i64, h: &str, owner: &str) {
    let extra = json!({"icp.name":owner});
    c.execute(
        "INSERT INTO assets VALUES(?1,?2,?2,?3)",
        params![id, format!("https://{h}"), extra.to_string()],
    )
    .unwrap();
    c.execute("INSERT INTO project_assets VALUES(1,?1)", [id])
        .unwrap();
    c.execute("INSERT INTO asset_discovery_seeds(id,project_id,host,source,source_ref,wildcard) VALUES(?1,1,?2,'observed_host',?3,0)",params![id,h,format!("asset:{id}")]).unwrap();
    c.execute(
        "INSERT INTO asset_cleanup_seeds VALUES(?1,'raw',?2,?3,'observed_host',?4,0,'')",
        params![k.id, id, h, format!("asset:{id}")],
    )
    .unwrap();
}
fn prepared(c: &mut Connection, k: &work::Claim) {
    while work::prepare(c, k).unwrap() {}
}
fn public(c: &Connection, k: &work::Claim, key: &str, h: &str, owner: &str) {
    let v = json!({"facts":[{"host":h,"organization":owner,"source":"public_ct_subject","certificateSha256":"a".repeat(64),"sourceUrl":"https://crt.sh/?d=42","notAfter":now()+86400}],"sourceUrl":"https://crt.sh/?d=42","bodySha256":"a".repeat(64),"checkedAt":now(),"kind":"ct_certificate"});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate',?3,'done',?4)",params![k.id,key,h,v.to_string()]).unwrap();
}
fn advice(c: &Connection, k: &work::Claim, key: &str) {
    let e = work::evidence_for(c, k, key).unwrap();
    c.execute("UPDATE asset_cleanup_groups SET advice='exclude',ai_basis_hash=?3 WHERE run_id=?1 AND group_key=?2",params![k.id,key,work::basis(&e)]).unwrap();
}
fn state(c: &Connection, k: &work::Claim, key: &str) -> String {
    c.query_row(
        "SELECT state FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
        params![k.id, key],
        |r| r.get(0),
    )
    .unwrap()
}
#[test]
fn cleanup_pages_all_snapshot_rows_and_preserves_source_indices() {
    let (mut c, k) = fixture();
    for id in 1..=1205 {
        add(&c, &k, id, &format!("n{id}.example.test"), "Other Bank");
    }
    assert!(work::prepare(&mut c, &k).unwrap());
    assert_eq!(
        c.query_row("SELECT cursor FROM asset_cleanup_runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        500
    );
    // A paused generation invalidates its old worker without losing the page.
    c.execute("UPDATE asset_cleanup_runs SET enabled=0,generation=2", [])
        .unwrap();
    assert!(!work::prepare(&mut c, &k).unwrap());
    c.execute("UPDATE asset_cleanup_runs SET enabled=1,generation=1", [])
        .unwrap();
    prepared(&mut c, &k);
    assert_eq!(
        c.query_row("SELECT cursor FROM asset_cleanup_runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1205
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_discovery_seeds", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1205
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_groups", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn cleanup_one_host_witness_never_excludes_unproven_sibling_family() {
    let (mut c, k) = fixture();
    add(&c, &k, 1, "www.bank.test", "Other Bank");
    add(&c, &k, 2, "new.bank.test", "");
    prepared(&mut c, &k);
    let key = "domain:bank.test";
    public(&c, &k, key, "www.bank.test", "Other Bank");
    advice(&c, &k, key);
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "evidence");
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
}
#[test]
fn cleanup_independent_witnesses_apply_and_undo_without_deleting_any_original() {
    let (mut c, k) = fixture();
    add(&c, &k, 1, "bank.test", "Other Bank");
    prepared(&mut c, &k);
    let key = "domain:bank.test";
    public(&c, &k, key, "bank.test", "Other Bank");
    advice(&c, &k, key);
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "excluded");
    assert_eq!(exclusions::rules(&c, 1).unwrap(), vec!["bank.test"]);
    assert_eq!(
        c.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_discovery_seeds", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let tx = c.unchecked_transaction().unwrap();
    work::restore_on(&tx, 1, &k.id, None).unwrap();
    tx.commit().unwrap();
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
    assert_eq!(state(&c, &k, key), "restored");
    assert_eq!(
        c.query_row("SELECT auto_exclude FROM asset_cleanup_runs", [], |r| r
            .get::<_, bool>(0))
            .unwrap(),
        false
    );
    assert!(!work::claim_valid(&c, &k).unwrap());
}
#[test]
fn cleanup_current_target_scope_shared_mixed_and_new_evidence_block_exclusion() {
    for case in [
        "scope",
        "shared",
        "mixed",
        "new_host",
        "changed_owner",
        "archived",
    ] {
        let (mut c, k) = fixture();
        add(&c, &k, 1, "bank.test", "Other Bank");
        prepared(&mut c, &k);
        let key = "domain:bank.test";
        public(&c, &k, key, "bank.test", "Other Bank");
        advice(&c, &k, key);
        match case {
            "scope" => {
                c.execute(
                    "INSERT INTO asset_discovery_scopes VALUES(1,'[\"bank.test\"]')",
                    [],
                )
                .unwrap();
            }
            "shared" => {
                c.execute("UPDATE asset_cleanup_groups SET shared=1", [])
                    .unwrap();
            }
            "mixed" => {
                c.execute("UPDATE asset_cleanup_facts SET mixed=1", [])
                    .unwrap();
            }
            "new_host" => {
                c.execute("INSERT INTO asset_discovery_seeds(project_id,host,source,source_ref,wildcard) VALUES(1,'new.bank.test','observed_host','asset:2',0)",[]).unwrap();
            }
            "changed_owner" => {
                c.execute(
                    "UPDATE assets SET extra_json='{\"icp.name\":\"Target Group\"}'",
                    [],
                )
                .unwrap();
            }
            _ => {
                c.execute("UPDATE projects SET status='archived' WHERE id=1", [])
                    .unwrap();
            }
        }
        work::reclassify(&mut c, &k, key).unwrap();
        assert_ne!(state(&c, &k, key), "excluded", "{case}");
        assert!(exclusions::rules(&c, 1).unwrap().is_empty(), "{case}");
    }
}
#[test]
fn cleanup_dns_response_requires_matching_question_and_connected_cname_chain() {
    let good = json!({"Status":0,"Question":[{"name":"a.example.test.","type":5}],"Answer":[{"name":"a.example.test.","type":5,"data":"b.example.test."},{"name":"b.example.test.","type":5,"data":"outside.net."}]});
    assert_eq!(
        network::parse_dns(&good, "a.example.test").unwrap().len(),
        2
    );
    let mut wrong = good.clone();
    wrong["Question"][0]["name"] = json!("unrelated.test");
    assert!(network::parse_dns(&wrong, "a.example.test").is_err());
    let mut unrelated = good.clone();
    unrelated["Answer"][1]["name"] = json!("unrelated.test");
    assert!(network::parse_dns(&unrelated, "a.example.test").is_err());
    let mut cycle = good.clone();
    cycle["Answer"][1]["data"] = json!("a.example.test");
    assert!(network::parse_dns(&cycle, "a.example.test").is_err());
    assert!(network::parse_dns(
        &json!({"Status":3,"Question":[{"name":"a.example.test","type":5}]}),
        "a.example.test"
    )
    .unwrap()
    .is_empty());
}
#[test]
fn cleanup_operation_reconcile_is_exact_and_keeps_current_run_status() {
    let (c, k) = fixture();
    c.execute("INSERT INTO asset_cleanup_operations VALUES(1,'trusted-one','start','hash',?1,'{\"runId\":\"cleanup-one\"}')",[&k.id]).unwrap();
    let v = status_on(
        &c,
        1,
        &Read {
            operation_id: Some("trusted-one".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(v["runId"], k.id);
    assert_eq!(v["operation"]["state"], "completed");
    let v = status_on(
        &c,
        1,
        &Read {
            operation_id: Some("different".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(v["operation"]["state"], "unknown");
    assert!(status_on(
        &c,
        2,
        &Read {
            run_id: Some(k.id),
            ..Default::default()
        }
    )
    .is_err());
}

// A real HTTP exchange through the configured production gateway. The public
// owner facts remain fixtures; no external service or real project is touched.
fn cleanup_provider(
    handle: impl FnOnce(Value) -> (u16, String) + Send + 'static,
) -> (
    String,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::thread::JoinHandle<()>,
) {
    use std::io::{Read as _, Write as _};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let server = std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        start.elapsed() < Duration::from_secs(15),
                        "cleanup did not send its real model request"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("fixture accept: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut raw = Vec::new();
        let mut buf = [0u8; 8192];
        let (boundary, length) = loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            raw.extend_from_slice(&buf[..n]);
            assert!(raw.len() < 2 * 1024 * 1024);
            if let Some(boundary) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..boundary]);
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (k, v) = line.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                if raw.len() >= boundary + 4 + length {
                    break (boundary + 4, length);
                }
            }
        };
        seen.fetch_add(1, Ordering::SeqCst);
        let body: Value = serde_json::from_slice(&raw[boundary..boundary + length]).unwrap();
        let (status, body) = handle(body);
        write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        stream.flush().unwrap();
    });
    (base, calls, server)
}
fn configure_cleanup_provider(c: &Connection, base: &str) {
    c.execute_batch("CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,is_default INTEGER NOT NULL,settings_json TEXT NOT NULL,updated_at TEXT NOT NULL)").unwrap();
    c.execute("INSERT INTO config_profiles VALUES(1,1,?1,'2000-01-01')",[json!({"activeModelProfileId":"selected","modelProfiles":[{"id":"selected","llm":"openai/synthetic-test-model","deployment":"local","apiBase":base,"localApiKey":"synthetic-key"}]}).to_string()]).unwrap();
    let (_, recipient, _) = profile(c).unwrap();
    c.execute("UPDATE asset_cleanup_runs SET recipient=?1", [recipient])
        .unwrap();
}
fn cleanup_answer(label: &str) -> String {
    json!({"choices":[{"message":{"role":"assistant","content":json!([{"key":"r0","label":label,"reason":"same-domain independent identity evidence; fixture relation judgement"}]).to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":17,"completion_tokens":9,"total_tokens":26}}).to_string()
}
#[test]
fn cleanup_real_gateway_paused_response_is_journaled_then_cold_resume_consumes_once() {
    use std::sync::atomic::Ordering;
    let (mut original, k) = fixture();
    add(&original, &k, 1, "bank.test", "Other Bank");
    prepared(&mut original, &k);
    let key = "domain:bank.test";
    public(&original, &k, key, "bank.test", "Other Bank");
    work::reclassify(&mut original, &k, key).unwrap();
    let dir = std::env::temp_dir().join(format!("ovi-cleanup-gateway-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("cleanup.sqlite");
    let server_path = path.clone();
    let id = k.id.clone();
    let (base, calls, server) = cleanup_provider(move |request| {
        assert_eq!(request["model"], "synthetic-test-model");
        let input: Value =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(input["items"].as_array().unwrap().len(), 1);
        // The actual pause handler runs while the paid HTTP request is in
        // flight. Its generation must prevent late publication/exclusion.
        dispatch(
            &server_path,
            1,
            "pause",
            json!({"runId":id}),
            "pause-during-gateway",
        )
        .unwrap();
        (200, cleanup_answer("exclude"))
    });
    configure_cleanup_provider(&original, &base);
    original
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    drop(original);
    let mut c = db::open(&path).unwrap();
    assert!(work::analyze(&mut c, &k).unwrap());
    server.join().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(state(&c, &k, key), "evidence");
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
    let receipt:(String,i64,i64)=c.query_row("SELECT state,json_extract(usage_json,'$.inputTokens'),json_extract(usage_json,'$.outputTokens') FROM asset_cleanup_models",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(receipt, ("received".into(), 17, 9));
    drop(c);
    // Cold reopen and the real resume + claim + consumer path. Only the
    // discardable worker lease is expired, as happens after the old app exits.
    let mut c = db::open(&path).unwrap();
    dispatch(
        &path,
        1,
        "resume",
        json!({"runId":k.id}),
        "cold-resume-gateway",
    )
    .unwrap();
    c.execute(
        "UPDATE asset_cleanup_runs SET lease_until=0,claim_token='',owner_pid=0",
        [],
    )
    .unwrap();
    let resumed = work::claim(&mut c).unwrap().unwrap();
    assert!(work::apply_received(&mut c, &resumed).unwrap());
    assert_eq!(state(&c, &resumed, key), "excluded");
    assert!(!work::apply_received(&mut c, &resumed).unwrap());
    assert!(!work::analyze(&mut c, &resumed).unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let totals:(i64,i64)=c.query_row("SELECT count(*),sum(json_extract(usage_json,'$.inputTokens')) FROM asset_cleanup_models",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(totals, (1, 17));
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(c);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn cleanup_eight_unknown_batches_do_not_starve_the_ninth_real_gateway_item() {
    use std::sync::atomic::Ordering;
    let (mut c, k) = fixture();
    for n in 1..=9 {
        add(&c, &k, n, &format!("n{n:02}.test"), "Other Bank");
    }
    prepared(&mut c, &k);
    for n in 1..=9 {
        let key = format!("domain:n{n:02}.test");
        work::reclassify(&mut c, &k, &key).unwrap();
        if n <= 8 {
            let basis = work::basis(&work::evidence_for(&c, &k, &key).unwrap());
            let request = format!("old-unknown-{n}");
            c.execute("INSERT INTO asset_cleanup_models(id,run_id,recipient,request_json,state,created_at) VALUES(?1,?2,'fixture','{}','unknown',?3)",params![request,k.id,now()]).unwrap();
            c.execute(
                "INSERT INTO asset_cleanup_model_items VALUES(?1,?2,?3,?4)",
                params![request, k.id, key, basis],
            )
            .unwrap();
        }
    }
    let (base, calls, server) = cleanup_provider(|request| {
        let input: Value =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(input["items"].as_array().unwrap().len(), 1);
        assert_eq!(input["items"][0]["evidence"]["domain"], "n09.test");
        (200, cleanup_answer("evidence"))
    });
    configure_cleanup_provider(&c, &base);
    assert!(work::analyze(&mut c, &k).unwrap());
    server.join().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!work::analyze(&mut c, &k).unwrap());
    let rows: (i64, i64) = c
        .query_row(
            "SELECT count(*),sum(state='unknown') FROM asset_cleanup_models",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(rows, (9, 8));
}
#[test]
fn cleanup_real_gateway_unknown_transport_is_not_reissued() {
    use std::sync::atomic::Ordering;
    let (mut c, k) = fixture();
    add(&c, &k, 1, "bank.test", "Other Bank");
    prepared(&mut c, &k);
    work::reclassify(&mut c, &k, "domain:bank.test").unwrap();
    let (base, calls, server) =
        cleanup_provider(|_| (503, json!({"error":"synthetic unavailable"}).to_string()));
    configure_cleanup_provider(&c, &base);
    assert!(work::analyze(&mut c, &k).unwrap());
    server.join().unwrap();
    assert!(!work::analyze(&mut c, &k).unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        c.query_row("SELECT state FROM asset_cleanup_models", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "unknown"
    );
    assert_eq!(state(&c, &k, "domain:bank.test"), "evidence");
}

#[test]
fn cleanup_operation_receipt_never_reads_another_newer_run() {
    let (c, k) = fixture();
    c.execute("INSERT INTO asset_cleanup_operations VALUES(1,'old-start','start','hash',?1,'{\"runId\":\"cleanup-one\"}')",[&k.id]).unwrap();
    c.execute(
        "UPDATE asset_cleanup_runs SET enabled=0 WHERE id=?1",
        [&k.id],
    )
    .unwrap();
    c.execute("INSERT INTO asset_cleanup_runs SELECT 'newer-run',project_id,'Different target',recipient,model,target_names_json,roots_json,auto_exclude,phase,0,generation,cursor,9999,8888,reason,0,'',0,created_at+1,updated_at FROM asset_cleanup_runs WHERE id=?1",[&k.id]).unwrap();
    let value = status_on(
        &c,
        1,
        &Read {
            operation_id: Some("old-start".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(value["runId"], k.id);
    assert_eq!(value["organization"], "Target Group");
    assert!(status_on(
        &c,
        1,
        &Read {
            operation_id: Some("old-start".into()),
            run_id: Some("newer-run".into()),
            ..Default::default()
        }
    )
    .is_err());
}

#[test]
fn cleanup_archiving_project_parks_owner_without_network_or_losing_checkpoint() {
    let (mut c, k) = fixture();
    c.execute("UPDATE projects SET status='archived' WHERE id=1", [])
        .unwrap();
    assert!(work::claim(&mut c).unwrap().is_none());
    let (enabled, phase): (bool, String) = c
        .query_row(
            "SELECT enabled,phase FROM asset_cleanup_runs WHERE id=?1",
            [&k.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(!enabled);
    assert_eq!(phase, "paused");
}
