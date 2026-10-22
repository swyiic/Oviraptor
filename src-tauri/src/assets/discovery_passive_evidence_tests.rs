use super::*;
const PEM: &str = include_str!("../../test-fixtures/ct/public-leaf.pem");
fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE asset_seed_reviews(id TEXT PRIMARY KEY,project_id INTEGER,cursor INTEGER,max_id INTEGER,organization TEXT DEFAULT 'Target Group',include_isolated INTEGER DEFAULT 1);INSERT INTO asset_seed_reviews(id,project_id,cursor,max_id) VALUES('r',1,1,1),('s',2,1,1);CREATE TABLE asset_seed_review_items(run_id TEXT,item_key TEXT,evidence_json TEXT,decision TEXT,state TEXT,advice TEXT,confidence REAL,revision INTEGER DEFAULT 0);INSERT INTO asset_seed_review_items(run_id,item_key,evidence_json,decision,state,advice,confidence) VALUES('r','k','{\"domains\":[\"example.test\",\"mail.example.test\",\"unrelated.test\"]}','pending','done','retain',0.9);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,extra_json TEXT,content_category TEXT,probe_outcome TEXT);CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER,decision TEXT);CREATE TABLE asset_seed_review_refs(run_id TEXT,item_key TEXT,asset_id INTEGER);").unwrap();
    ensure(&c).unwrap();
    c
}
fn item(kind: &str, target: &str, host: &str) -> Item {
    Item {
        id: 1,
        kind: kind.into(),
        target: target.into(),
        host: host.into(),
        token: "token".into(),
        attempts: 1,
    }
}
fn save(c: &mut Connection, id: i64, run: &str, host: &str) {
    c.execute(
        "INSERT INTO asset_passive_candidates VALUES(1,?1,'k',?2)",
        params![run, host],
    )
    .unwrap();
    c.execute(
        "INSERT INTO asset_passive_certificate_refs VALUES(1,?1,?2,?3)",
        params![run, host, id],
    )
    .unwrap();
    c.execute("INSERT INTO asset_passive_tasks(id,project_id,run_id,kind,target,host,state,claim_token,attempts) VALUES(?1,1,?2,'ct_certificate',?3,?4,'running','token',1)",params![id,run,id.to_string(),host]).unwrap();
    let mut i = item("ct_certificate", &id.to_string(), host);
    i.id = id;
    let mut value = execute_with(&i, |url, _| {
        assert_eq!(url.host_str(), Some("crt.sh"));
        Ok(PEM.as_bytes().to_vec())
    })
    .unwrap();
    value["names"] = json!(["bank.test"]);
    value["organizations"] = json!(["Injected Organization"]);
    assert!(finish(c, 1, run, &i, Ok(value)).unwrap());
}
#[test]
fn exact_index_ids_deduplicate_but_unrelated_association_and_bad_id_do_not_admit() {
    let v=parse_search(br#"[{"id":12,"name_value":"*.example.test\nexample.test"},{"id":12,"name_value":"example.test"},{"id":15,"name_value":"bank.test"}]"#,"example.test").unwrap();
    assert_eq!(v["certificateIds"], json!([12]));
    assert_eq!(v["complete"], false);
    assert!(parse_search(br#"[{"name_value":"example.test"}]"#, "example.test").is_err());
    assert!(!name_covers("*.example.test", "deep.mail.example.test"));
    assert!(name_covers("*.example.test", "mail.example.test"));
    assert!(!name_covers("example.test", "example.test.evil.test"));
}
#[test]
fn fixed_transport_and_der_are_authoritative_not_injected_names_or_subject() {
    let mut c = fixture();
    save(&mut c, 77, "r", "mail.example.test");
    let v = observations(&c, 1, "r", &BTreeSet::from(["mail.example.test".into()])).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0]["organization"], "Target Group");
    assert_eq!(v[0]["source"], "public_ct_subject");
    assert_eq!(v[0]["host"], "example.test");
    assert_eq!(v[0]["wildcard"], true);
    assert_eq!(v[0]["certificateId"], 77);
    assert_eq!(v[0]["sourceUrl"], "https://crt.sh/?d=77");
    assert!(v[0]["certificateSha256"].as_str().unwrap().len() == 64);
    assert!(v[0]["validUntil"].as_i64().unwrap() > now());
    // Same public cache cannot cross project/run or cover a provider co-location.
    assert!(
        observations(&c, 2, "s", &BTreeSet::from(["mail.example.test".into()]))
            .unwrap()
            .is_empty()
    );
    c.execute(
        "INSERT INTO asset_passive_certificate_refs VALUES(1,'r','bank.test',77)",
        [],
    )
    .unwrap();
    assert!(
        observations(&c, 1, "r", &BTreeSet::from(["bank.test".into()]))
            .unwrap()
            .is_empty()
    );
    c.execute("UPDATE asset_passive_certificates SET not_after=1", [])
        .unwrap();
    assert!(
        observations(&c, 1, "r", &BTreeSet::from(["mail.example.test".into()]))
            .unwrap()
            .is_empty()
    );
    assert!(request_url(&item("ct_search", "https://evil.test/a", "evil.test")).is_err());
    assert!(request_url(&item("ct_certificate", "https://evil.test/a", "evil.test")).is_err());
}
#[test]
fn bounded_transport_records_403_timeout_retry_and_never_turns_failure_into_absence() {
    let mut c = fixture();
    for (id, message) in [
        (1, "crt.sh ct_search HTTP 403；服务拒绝访问"),
        (2, "crt.sh ct_search 请求超时"),
    ] {
        c.execute("INSERT INTO asset_passive_tasks(id,project_id,run_id,kind,target,host,state,claim_token,attempts) VALUES(?1,1,'r','ct_search',?2,?2,'running','token',1)",params![id,format!("x{id}.example.test")]).unwrap();
        let mut i = item(
            "ct_search",
            &format!("x{id}.example.test"),
            &format!("x{id}.example.test"),
        );
        i.id = id;
        let result = execute_with(&i, |url, _| {
            assert_eq!(url.host_str(), Some("crt.sh"));
            Err(message.into())
        });
        assert!(finish(&mut c, 1, "r", &i, result).unwrap());
        assert!(!finish(&mut c, 1, "r", &i, Err("late receipt".into())).unwrap());
    }
    let v = summary(&c, 1, "r").unwrap();
    assert_eq!(v["retry"], 2);
    assert_eq!(v["hunterRequests"], 0);
    assert_eq!(v["targetConnections"], 0);
    assert!(v.to_string().contains("403"));
    assert!(v.to_string().contains("超时"));
    assert!(
        execute_with(&item("ct_certificate", "7", "example.test"), |_, max| Ok(
            vec![0; max + 1]
        ))
        .is_err()
    );
    assert!(
        execute_with(&item("ct_certificate", "7", "example.test"), |_, _| Ok(
            b"<html>access denied</html>".to_vec()
        ))
        .is_err()
    );
}
#[test]
fn lease_recovery_and_claim_token_prevent_stale_receipt_and_duplicate_network() {
    let mut c = fixture();
    c.execute(
        "INSERT INTO asset_passive_candidates VALUES(1,'r','k','example.test')",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO asset_passive_tasks(project_id,run_id,kind,target,host) VALUES(1,'r','ct_search','example.test','example.test')",[]).unwrap();
    let first = claim(&mut c, 1, "r").unwrap().unwrap();
    assert!(claim(&mut c, 1, "r").unwrap().is_none());
    assert_eq!(recover(&c, 1, "r").unwrap(), 0);
    c.execute(
        "UPDATE asset_passive_tasks SET lease_until=0,updated_at=datetime('now','-1 hour')",
        [],
    )
    .unwrap();
    assert_eq!(recover(&c, 1, "r").unwrap(), 1);
    let second = claim(&mut c, 1, "r").unwrap().unwrap();
    assert_ne!(first.token, second.token);
    assert!(!finish(&mut c, 1, "r", &first, Ok(json!({"certificateIds":[]}))).unwrap());
    assert!(finish(&mut c, 1, "r", &second, Ok(json!({"certificateIds":[8,8]}))).unwrap());
    let n: i64 = c
        .query_row(
            "SELECT count(*) FROM asset_passive_tasks WHERE kind='ct_certificate'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn evidence_only_provider_cn_can_research_without_enumerating_associated_banks() {
    let c = fixture();
    c.execute("UPDATE asset_seed_review_items SET state='deferred',advice='evidence',confidence=0,evidence_json=?1 WHERE run_id='r'",[json!({"domains":["10086.cn","bank.test"]}).to_string()]).unwrap();
    c.execute("INSERT INTO assets VALUES(1,'https://mail.10086.cn','https://mail.10086.cn',?1,'','')",[json!({"cert.domain":"10086.cn bank.test","cert.subject.cn":"*.10086.cn","cert.subject.org":"Target Group"}).to_string()]).unwrap();
    c.execute_batch("INSERT INTO project_assets VALUES(1,1,0,'pending');INSERT INTO asset_seed_review_refs VALUES('r','k',1);").unwrap();
    assert_eq!(enqueue_review_candidates(&c, 1, "r").unwrap(), 1);
    let hosts: Vec<String> = c
        .prepare("SELECT host FROM asset_passive_tasks ORDER BY host")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(hosts, vec!["10086.cn"]);
    assert_eq!(enqueue_review_candidates(&c, 1, "r").unwrap(), 0);
    assert_eq!(summary(&c, 1, "r").unwrap()["pending"], 1);
}
#[test]
fn public_fingerprint_anchor_reparses_der_and_all_names_must_be_confirmed() {
    let mut c = fixture();
    save(&mut c, 88, "r", "mail.example.test");
    let cert = crate::certificates::from_text(PEM).unwrap();
    assert_eq!(
        fingerprint_candidates(&c, 1).unwrap(),
        vec![cert.sha256.clone()]
    );
    assert_eq!(
        fingerprint_anchor(&c, 1, &cert.sha256, &["example.test".into()]).unwrap(),
        Some("mail.example.test".into())
    );
    assert_eq!(
        fingerprint_anchor(&c, 1, &cert.sha256, &["mail.example.test".into()]).unwrap(),
        None
    );
    assert_eq!(
        fingerprint_anchor(&c, 2, &cert.sha256, &["example.test".into()]).unwrap(),
        None
    );
    c.execute(
        "UPDATE asset_passive_certificates SET sha256=?1",
        ["b".repeat(64)],
    )
    .unwrap();
    assert_eq!(
        fingerprint_anchor(&c, 1, &"b".repeat(64), &["example.test".into()]).unwrap(),
        None
    );
    c.execute("UPDATE asset_seed_review_items SET decision='rejected'", [])
        .unwrap();
    assert!(fingerprint_candidates(&c, 1).unwrap().is_empty());
}
#[test]
fn precertificate_poison_is_not_a_paid_fingerprint_seed() {
    use rustls::pki_types::{pem::PemObject, CertificateDer};
    let pem = include_str!("../../test-fixtures/ct/precertificate.pem");
    let der = CertificateDer::from_pem_slice(pem.as_bytes()).unwrap();
    assert!(is_precertificate(der.as_ref()));
    let final_leaf = crate::certificates::from_text(PEM).unwrap();
    let final_der = base64::engine::general_purpose::STANDARD
        .decode(final_leaf.der_base64)
        .unwrap();
    assert!(!is_precertificate(&final_der));
    let mut c = fixture();
    save(&mut c, 88, "r", "mail.example.test");
    let raw = base64::engine::general_purpose::STANDARD.encode(der.as_ref());
    let sha = format!("{:x}", Sha256::digest(der.as_ref()));
    c.execute(
        "UPDATE asset_passive_certificates SET der_base64=?1,sha256=?2",
        params![raw, sha],
    )
    .unwrap();
    assert!(fingerprint_candidates(&c, 1).unwrap().is_empty());
    assert!(fingerprint_anchor(&c, 1, &sha, &["example.test".into()])
        .unwrap()
        .is_none());
}

#[test]
fn ten_thousand_pending_hosts_claim_with_bounded_sql_work_without_quadratic_writer_lock() {
    let mut c=fixture();
    // Mirror the production review item's primary-key index (the minimal
    // standalone fixture intentionally does not otherwise need this table key).
    c.execute_batch("CREATE INDEX review_item_lookup ON asset_seed_review_items(run_id,item_key); BEGIN IMMEDIATE; WITH RECURSIVE n(v) AS (VALUES(1) UNION ALL SELECT v+1 FROM n WHERE v<10000) INSERT INTO asset_passive_candidates SELECT 1,'r','k','h'||v||'.example.test' FROM n; INSERT INTO asset_passive_tasks(project_id,run_id,kind,target,host) SELECT project_id,run_id,'ct_search',host,host FROM asset_passive_candidates; COMMIT;").unwrap();
    let work=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let measure=work.clone();
    // A VM-work limit detects the former N*N correlated scan without relying
    // on host speed or an external network timeout.
    c.progress_handler(1000,Some(move||measure.fetch_add(1,std::sync::atomic::Ordering::Relaxed)>5000));
    let claimed=claim(&mut c,1,"r").expect("10k candidate claim must stay within five million SQLite VM operations").unwrap();
    c.progress_handler(0,None::<fn()->bool>);
    assert_eq!(claimed.kind,"ct_search");
    assert!(work.load(std::sync::atomic::Ordering::Relaxed)<5000);
    assert_eq!(c.query_row("SELECT count(*) FROM asset_passive_tasks WHERE state='running'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}
