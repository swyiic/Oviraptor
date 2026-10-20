use super::*;
struct Fixture {
    root: std::path::PathBuf,
    m: Manager,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("ovi-certificate-task-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("test.sqlite");
        let c = db::open(&path).unwrap();
        c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,asset_key TEXT,host TEXT,link TEXT,extra_json TEXT);CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,PRIMARY KEY(project_id,asset_id));").unwrap();
        super::super::ensure(&c).unwrap();
        c.execute(
            "INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.test\"]')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO asset_discovery_scopes VALUES(2,'[\"other.test\"]')",
            [],
        )
        .unwrap();
        for i in 1..=5 {
            let host = if i == 4 {
                "https://outside.invalid".into()
            } else {
                format!("https://h{i}.example.test")
            };
            let e = if i == 5 {
                json!({"probe_cert_sha256":"a".repeat(64)})
            } else {
                json!({"cert.sn":"provider-kept","cert.subject.cn":"original-cn","company":"original","probe_outcome":"web_alive"})
            };
            c.execute(
                "INSERT INTO assets VALUES(?1,?2,?3,?3,?4)",
                params![i, format!("key-{i}"), host, e.to_string()],
            )
            .unwrap();
            c.execute("INSERT INTO project_assets VALUES(1,?1,0)", [i])
                .unwrap();
        }
        Self {
            m: Manager::new(&path).unwrap(),
            root,
        }
    }
    fn create(&self) -> String {
        self.m
            .dispatch(
                1,
                "start",
                json!({"startPaused":true,"workers":1}),
                "operation-a",
            )
            .unwrap()["tasks"][0]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn status(&self, id: &str) -> Value {
        self.m.status(1, Some(id)).unwrap()["tasks"][0].clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.m.shutdown();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn cert() -> crate::certificates::Certificate {
    crate::certificates::from_der(include_bytes!("../../test-fixtures/tls/leaf.der")).unwrap()
}
#[test]
fn snapshot_is_project_bound_scoped_missing_only_and_idempotent() {
    let f = Fixture::new();
    let id = f.create();
    assert_eq!(f.status(&id)["progress"]["total"], 3);
    assert_eq!(f.create(), id);
    assert!(f.m.status(2, Some(&id)).is_err());
    assert!(
        f.m.dispatch(
            1,
            "start",
            json!({"assetIds":[999],"startPaused":true}),
            "x"
        )
        .is_ok(),
        "existing unfinished queue is returned rather than creating a second snapshot"
    );
    assert!(f
        .m
        .dispatch(1, "start", json!({"workers":99}), "x")
        .is_err());
}
#[test]
fn completed_item_commits_real_der_and_seed_without_overwriting_provider_or_manual_state() {
    let f = Fixture::new();
    let id = f.create();
    let c = db::open(&f.m.path).unwrap();
    c.execute(
        "UPDATE asset_certificate_tasks SET status='running' WHERE id=?1",
        [&id],
    )
    .unwrap();
    let (aid, key, url, _) = f.m.claim(1, &id).unwrap().unwrap();
    f.m.finish(1, &id, aid, &key, &url, Ok(cert())).unwrap();
    let e: Value = serde_json::from_str(
        &c.query_row("SELECT extra_json FROM assets WHERE id=?1", [aid], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(e["cert.sn"], "provider-kept");
    assert_eq!(e["cert.subject.cn"], "original-cn");
    assert_eq!(e["probe_outcome"], "web_alive");
    assert_eq!(e["probe_cert_sha256"], cert().sha256);
    assert_eq!(f.status(&id)["progress"]["captured"], 1);
    assert_eq!(f.status(&id)["progress"]["newFingerprints"], 1);
    assert_eq!(c.query_row("SELECT count(*) FROM asset_discovery_seeds WHERE project_id=1 AND source='certificate_sha256'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}
#[test]
fn pause_and_dead_owner_recovery_keep_completed_work_and_requeue_only_inflight() {
    let f = Fixture::new();
    let id = f.create();
    let c = db::open(&f.m.path).unwrap();
    c.execute(
        "UPDATE asset_certificate_tasks SET status='running',owner_pid=2147483647 WHERE id=?1",
        [&id],
    )
    .unwrap();
    let (aid, key, url, _) = f.m.claim(1, &id).unwrap().unwrap();
    f.m.finish(1, &id, aid, &key, &url, Ok(cert())).unwrap();
    let second = f.m.claim(1, &id).unwrap().unwrap();
    let recovered = Manager::new(&f.m.path).unwrap();
    let state = recovered.status(1, Some(&id)).unwrap();
    assert_eq!(state["tasks"][0]["status"], "interrupted");
    assert_eq!(state["tasks"][0]["progress"]["captured"], 1);
    assert_eq!(state["tasks"][0]["progress"]["pending"], 2);
    assert_eq!(state["tasks"][0]["progress"]["running"], 0);
    recovered.pause(1, &id).unwrap();
    assert!(recovered.claim(1, &id).unwrap().is_none());
    assert_eq!(
        c.query_row(
            "SELECT status FROM asset_certificate_task_items WHERE task_id=?1 AND asset_id=?2",
            params![id, second.0],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
}
#[test]
fn scope_revocation_or_recycle_before_write_never_commits_certificate() {
    for revoke in [true, false] {
        let f = Fixture::new();
        let id = f.create();
        let c = db::open(&f.m.path).unwrap();
        c.execute(
            "UPDATE asset_certificate_tasks SET status='running' WHERE id=?1",
            [&id],
        )
        .unwrap();
        let (aid, key, url, _) = f.m.claim(1, &id).unwrap().unwrap();
        if revoke {
            c.execute(
                "UPDATE asset_discovery_scopes SET roots_json='[]' WHERE project_id=1",
                [],
            )
            .unwrap();
        } else {
            c.execute(
                "UPDATE project_assets SET is_deleted=1 WHERE asset_id=?1",
                [aid],
            )
            .unwrap();
        }
        f.m.finish(1, &id, aid, &key, &url, Ok(cert())).unwrap();
        assert_eq!(f.status(&id)["progress"]["captured"], 0);
        assert_eq!(f.status(&id)["progress"]["skipped"], 1);
    }
}
#[test]
fn failed_seed_index_rolls_back_asset_and_progress_as_one_transaction() {
    let f = Fixture::new();
    let id = f.create();
    let c = db::open(&f.m.path).unwrap();
    c.execute(
        "UPDATE asset_certificate_tasks SET status='running' WHERE id=?1",
        [&id],
    )
    .unwrap();
    let (aid, key, url, _) = f.m.claim(1, &id).unwrap().unwrap();
    let before: String = c
        .query_row("SELECT extra_json FROM assets WHERE id=?1", [aid], |r| {
            r.get(0)
        })
        .unwrap();
    c.execute_batch("CREATE TRIGGER fail_certificate_index BEFORE INSERT ON asset_discovery_seeds BEGIN SELECT RAISE(ABORT,'test index unavailable'); END;").unwrap();
    assert!(f.m.finish(1, &id, aid, &key, &url, Ok(cert())).is_err());
    assert_eq!(
        before,
        c.query_row("SELECT extra_json FROM assets WHERE id=?1", [aid], |r| {
            r.get::<_, String>(0)
        })
        .unwrap()
    );
    assert_eq!(f.status(&id)["progress"]["captured"], 0);
}
#[test]
fn multiple_assets_sharing_a_leaf_count_one_unique_fingerprint_and_preserve_each_evidence() {
    let f = Fixture::new();
    let id = f.create();
    let c = db::open(&f.m.path).unwrap();
    c.execute(
        "UPDATE asset_certificate_tasks SET status='running' WHERE id=?1",
        [&id],
    )
    .unwrap();
    for _ in 0..2 {
        let (aid, key, url, _) = f.m.claim(1, &id).unwrap().unwrap();
        f.m.finish(1, &id, aid, &key, &url, Ok(cert())).unwrap();
    }
    let p = f.status(&id)["progress"].clone();
    assert_eq!(p["captured"], 2);
    assert_eq!(p["uniqueFingerprints"], 1);
    assert_eq!(p["newFingerprints"], 1);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_discovery_seed_evidence WHERE source='certificate_sha256'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
#[test]
fn explicit_retry_only_retries_failed_items_not_already_completed_assets() {
    let f = Fixture::new();
    let id = f.create();
    let c = db::open(&f.m.path).unwrap();
    c.execute(
        "UPDATE asset_certificate_tasks SET status='running' WHERE id=?1",
        [&id],
    )
    .unwrap();
    for i in 0..3 {
        let (aid, key, url, _) = f.m.claim(1, &id).unwrap().unwrap();
        f.m.finish(
            1,
            &id,
            aid,
            &key,
            &url,
            if i == 0 {
                Err("timeout".into())
            } else {
                Ok(cert())
            },
        )
        .unwrap();
    }
    c.execute(
        "UPDATE asset_certificate_tasks SET status='completed' WHERE id=?1",
        [&id],
    )
    .unwrap();
    let mut e: Value = serde_json::from_str(
        &c.query_row("SELECT extra_json FROM assets WHERE id=1", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
    )
    .unwrap();
    e["certificate_pem"] = json!(include_str!("../../test-fixtures/tls/leaf.pem"));
    c.execute(
        "UPDATE assets SET extra_json=?1 WHERE id=1",
        [e.to_string()],
    )
    .unwrap();
    f.m.resume(1, &id, true).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while f.status(&id)["status"] == "running" && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20))
    }
    assert_eq!(f.status(&id)["status"], "completed");
    let p = f.status(&id)["progress"].clone();
    assert_eq!(p["captured"], 3);
    assert_eq!(p["failed"], 0);
    assert_eq!(p["attempts"], 4);
}
#[test]
fn actual_tls_handshake_reads_leaf_without_sending_an_http_request() {
    use std::io::Read;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(
                include_bytes!("../../test-fixtures/tls/leaf.der").to_vec(),
            )],
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                include_bytes!("../../test-fixtures/tls/test-only-key.der").to_vec(),
            )),
        )
        .unwrap();
        let (socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut stream = rustls::StreamOwned::new(
            rustls::ServerConnection::new(Arc::new(cfg)).unwrap(),
            socket,
        );
        let mut data = [0; 4096];
        assert!(
            !matches!(stream.read(&mut data),Ok(n)if n>0),
            "certificate collector must never transmit HTTP or application data"
        );
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let captured = runtime
        .block_on(tls::capture(&format!("https://{addr}/"), 2, true))
        .unwrap();
    assert_eq!(captured.sha256, cert().sha256);
    server.join().unwrap();
    assert!(
        runtime
            .block_on(tls::capture(&format!("https://{addr}/"), 2, false))
            .is_err(),
        "production collector must refuse loopback"
    );
}
#[test]
fn private_bridge_rejects_missing_bearer_browser_origin_and_cross_project_task_reads() {
    let f = Fixture::new();
    let id = f.create();
    let bridge = crate::certificate_bridge::Bridge::start(f.m.clone()).unwrap();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let url = format!("{}/certificate-task", bridge.origin);
    let request =
        json!({"projectId":1,"action":"status","args":{"taskId":id},"operationId":"test-status"});
    assert!(!client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(request.to_string())
        .send()
        .unwrap()
        .text()
        .map(|s| serde_json::from_str::<Value>(&s).unwrap())
        .unwrap()["ok"]
        .as_bool()
        .unwrap());
    let body = serde_json::to_vec(&request).unwrap();
    let allowed = client
        .post(&url)
        .bearer_auth(&bridge.token)
        .header("Content-Type", "application/json")
        .body(body.clone())
        .send()
        .unwrap();
    assert_eq!(
        allowed
            .text()
            .map(|s| serde_json::from_str::<Value>(&s).unwrap())
            .unwrap()["ok"],
        true
    );
    let denied = client
        .post(&url)
        .bearer_auth(&bridge.token)
        .header("Origin", "http://untrusted.invalid")
        .body(body)
        .send()
        .unwrap();
    assert_eq!(
        denied
            .text()
            .map(|s| serde_json::from_str::<Value>(&s).unwrap())
            .unwrap()["ok"],
        false
    );
    let mut other = request;
    other["projectId"] = json!(2);
    let denied = client
        .post(&url)
        .bearer_auth(&bridge.token)
        .header("Content-Type", "application/json")
        .body(other.to_string())
        .send()
        .unwrap();
    assert_eq!(
        denied
            .text()
            .map(|s| serde_json::from_str::<Value>(&s).unwrap())
            .unwrap()["ok"],
        false
    );
}
#[test]
fn snapshot_includes_the_tail_beyond_one_thousand_and_excludes_later_inserts() {
    let f = Fixture::new();
    let c = db::open(&f.m.path).unwrap();
    c.execute_batch("WITH RECURSIVE ids(n) AS(VALUES(6) UNION ALL SELECT n+1 FROM ids WHERE n<5006) INSERT INTO assets SELECT n,'key-'||n,'https://h'||n||'.example.test','https://h'||n||'.example.test','{}' FROM ids;INSERT INTO project_assets SELECT 1,id,0 FROM assets WHERE id>=6;").unwrap();
    let id = f.create();
    assert_eq!(f.status(&id)["progress"]["total"], 5004);
    c.execute("INSERT INTO assets VALUES(99999,'late','https://late.example.test','https://late.example.test','{}')",[]).unwrap();
    c.execute("INSERT INTO project_assets VALUES(1,99999,0)", [])
        .unwrap();
    assert_eq!(f.status(&id)["progress"]["total"], 5004);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_certificate_task_items WHERE task_id=?1 AND asset_id=5006",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn live_pause_then_reopen_and_resume_retains_each_saved_item_without_repeating_attempts() {
    let f = Fixture::new();
    let c = db::open(&f.m.path).unwrap();
    let extra=json!({"certificate_pem":include_str!("../../test-fixtures/tls/leaf.pem"),"cert.sn":"kept"}).to_string();
    for i in 6..=205 {
        let url = format!("https://h{i}.example.test");
        c.execute(
            "INSERT INTO assets VALUES(?1,?2,?3,?3,?4)",
            params![i, format!("k{i}"), url, extra],
        )
        .unwrap();
        c.execute("INSERT INTO project_assets VALUES(1,?1,0)", [i])
            .unwrap();
    }
    c.execute("UPDATE assets SET extra_json=?1 WHERE id<=3", [extra])
        .unwrap();
    let id = f.create();
    f.m.resume(1, &id, false).unwrap();
    f.m.pause(1, &id).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while f.status(&id)["status"] == "pausing" && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10))
    }
    assert_eq!(f.status(&id)["status"], "paused");
    assert!(f.status(&id)["progress"]["pending"].as_i64().unwrap() > 0);
    let reopened = Manager::new(&f.m.path).unwrap();
    reopened.resume(1, &id, false).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while reopened.status(1, Some(&id)).unwrap()["tasks"][0]["status"] == "running"
        && std::time::Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(20))
    }
    let state = reopened.status(1, Some(&id)).unwrap()["tasks"][0].clone();
    assert_eq!(state["status"], "completed");
    assert_eq!(state["progress"]["captured"], 203);
    assert_eq!(state["progress"]["attempts"], 203);
    assert_eq!(state["progress"]["uniqueFingerprints"], 1);
}
