use super::*;

#[test]
fn local_filter_diagnostics_distinguish_missing_relation_from_invalid_endpoint() {
    let payload=json!({"code":200,"data":{"arr":[
        {"domain":"other.example.com","port":443,"protocol":"https"},
        {"domain":"app.example.com","port":0,"protocol":"https"},
        {"domain":"app.example.com","port":443,"protocol":"https","url":"https://app.example.com/"}
    ]}});
    let (rows,rejected,reasons)=rows_report(&payload,"domain","app.example.com").unwrap();
    assert_eq!(rows.len(),1);assert_eq!(rejected,2);
    assert_eq!(reasons["relation_mismatch_or_missing"],1);
    assert_eq!(reasons["invalid_port"],1);
}
fn fixture() -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0);INSERT INTO project_assets VALUES(1,7,0);CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);").unwrap();
    ensure(&c).unwrap();
    c.execute_batch("CREATE TABLE assets(id INTEGER PRIMARY KEY,extra_json TEXT NOT NULL,host TEXT NOT NULL DEFAULT 'app.example.com',link TEXT NOT NULL DEFAULT 'https://app.example.com');").unwrap();
    c.execute("INSERT INTO assets(id,extra_json) VALUES(7,?1)",[json!({"cert_sha256":"a".repeat(64),"cert.domain":"app.example.com"}).to_string()]).unwrap();
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.com\",\"tenant.cloud.test\"]')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO asset_discovery_settings VALUES(1,1,'synthetic-key',3,10)",
        [],
    )
    .unwrap();
    for (h, s, r) in [
        ("app.example.com", "observed_host", "asset:7"),
        ("tenant.cloud.test", "cname", "asset:7"),
        (&"a".repeat(64), "certificate_sha256", "asset:7"),
    ] {
        insert_seed(&c, 1, h, s, r, false).unwrap();
    }
    c.execute_batch(TABLES).unwrap();
    c
}
fn input(kind: &str, target: &str) -> ExpansionInput {
    ExpansionInput {
        project_id: 1,
        kind: kind.into(),
        target: target.into(),
    }
}
fn item(id: u32) -> Value {
    json!({"ip":format!("192.0.2.{}",id%250+1),"port":443,"url":format!("https://192.0.2.{}:{}/",id%250+1,443),"protocol":"https","web_title":format!("asset {id}"),"cert_sha256":"a".repeat(64)})
}
fn page(total: usize, ids: &[u32]) -> Value {
    json!({"code":200,"data":{"total":total,"arr":ids.iter().map(|id|item(*id)).collect::<Vec<_>>(),"consume_quota":format!("消耗积分：{}",ids.len()),"rest_quota":"今日剩余积分：100"}})
}
struct Fake {
    pages: HashMap<i64, Value>,
    calls: Vec<i64>,
    free: i64,
    equity: i64,
    fail: Option<i64>,
}
impl Provider for Fake {
    fn account(&mut self) -> Result<Value, String> {
        Ok(
            json!({"rest_free_point":self.free,"rest_equity_point":self.equity,"once_export_quota":10000,"rest_export_quota":-1}),
        )
    }
    fn search(
        &mut self,
        _: &str,
        p: i64,
        _: i64,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Value, String> {
        self.calls.push(p);
        if self.fail == Some(p) {
            return Err("synthetic timeout".into());
        }
        let v = self
            .pages
            .get(&p)
            .expect("only planned pages may be called")
            .clone();
        self.equity -= points(&v["data"]["consume_quota"]).unwrap_or(0);
        Ok(v)
    }
}
fn fake(pages: Vec<Value>) -> Fake {
    Fake {
        pages: pages
            .into_iter()
            .enumerate()
            .map(|(i, p)| (i as i64 + 1, p))
            .collect(),
        calls: vec![],
        free: 0,
        equity: 10000,
        fail: None,
    }
}
#[test]
fn exact_queries_require_real_project_evidence_and_use_official_fingerprint_field() {
    let mut c = fixture();
    let hash = "a".repeat(64);
    assert_eq!(
        exact_query("fingerprint", &hash).unwrap(),
        format!("cert.sha-256=\"{hash}\"")
    );
    assert!(prepare(
        &mut c,
        &input("fingerprint", &"b".repeat(64)),
        "synthetic-key"
    )
    .is_err());
    assert!(prepare(&mut c, &input("cname", "other.cloud.test"), "synthetic-key").is_err());
    assert!(prepare(
        &mut c,
        &input("domain", "guessed.example.com"),
        "synthetic-key"
    )
    .is_err());
    assert!(prepare(
        &mut c,
        &input("certificate", "example.com"),
        "synthetic-key"
    )
    .is_err());
    assert!(prepare(&mut c, &input("fingerprint", &hash), "synthetic-key").is_ok());
    assert!(prepare(
        &mut c,
        &input("cname", "tenant.cloud.test"),
        "synthetic-key"
    )
    .is_ok());
    let mut other = input("fingerprint", &hash);
    other.project_id = 2;
    assert!(prepare(&mut c, &other, "synthetic-key").is_err());
}

#[test]
fn resumed_query_rechecks_certificate_before_any_provider_request() {
    let mut c=fixture();
    let id=prepare(&mut c,&input("fingerprint",&"a".repeat(64)),"synthetic-key").unwrap();
    c.execute("UPDATE assets SET extra_json=?1 WHERE id=7",[json!({"cert_sha256":"a".repeat(64),"cert.domain":"unrelated.test"}).to_string()]).unwrap();
    let mut provider=fake(vec![]);
    let result=scan(&mut c,1,&id,&mut provider,||false,|_,_|panic!("inadmissible evidence must never import paid results"));
    assert!(result.is_err());
    assert!(provider.calls.is_empty(),"pause/resume must not bypass the current certificate gate");
}

#[test]
fn excluding_a_domain_stops_an_already_prepared_query_before_provider_calls() {
    let mut c=fixture();
    let id=prepare(&mut c,&input("fingerprint",&"a".repeat(64)),"synthetic-key").unwrap();
    let preview=exclusions::manage(&mut c,exclusions::ExclusionInput{project_id:1,domain:"example.com".into(),action:"preview".into(),expected_digest:None}).unwrap();
    exclusions::manage(&mut c,exclusions::ExclusionInput{project_id:1,domain:"example.com".into(),action:"apply".into(),expected_digest:preview["digest"].as_str().map(str::to_owned)}).unwrap();
    let mut provider=fake(vec![]);
    assert!(scan(&mut c,1,&id,&mut provider,||false,|_,_|panic!("excluded query cannot import paid results")).is_err());
    assert!(provider.calls.is_empty());
}
#[test]
fn automatically_reads_later_pages_keeps_high_value_tail_and_deduplicates() {
    let mut c = fixture();
    let id = prepare(
        &mut c,
        &input("fingerprint", &"a".repeat(64)),
        "synthetic-key",
    )
    .unwrap();
    let mut f = fake(vec![
        page(21, &(1..=10).collect::<Vec<_>>()),
        page(21, &(10..=19).collect::<Vec<_>>()),
        page(21, &[99]),
    ]);
    let mut imported = vec![];
    let result = scan(
        &mut c,
        1,
        &id,
        &mut f,
        || false,
        |_, rows| {
            imported.extend_from_slice(rows);
            Ok(json!({}))
        },
    )
    .unwrap();
    assert_eq!(f.calls, vec![1, 2, 3]);
    assert_eq!(result["status"], "completed");
    assert_eq!(result["spentPoints"], 21);
    assert_eq!(result["received"], 21);
    assert_eq!(result["uniqueRows"], 20);
    assert!(imported.iter().any(|r| r["title"] == "asset 99"));
    assert_eq!(imported.len(), 20);
    assert_eq!(result["account"]["rest_free_point"], 0);
    assert!(result["account"]["rest_equity_point"].as_i64().unwrap() < 10000);
}
#[test]
fn pause_and_resume_keep_page_size_and_cursor_without_rebuying_first_page() {
    let mut c = fixture();
    let id = prepare(
        &mut c,
        &input("fingerprint", &"a".repeat(64)),
        "synthetic-key",
    )
    .unwrap();
    let mut f = fake(vec![
        page(11, &(1..=10).collect::<Vec<_>>()),
        page(11, &[99]),
    ]);
    let stop = Arc::new(AtomicBool::new(false));
    let output = stop.clone();
    let first = scan(
        &mut c,
        1,
        &id,
        &mut f,
        || stop.load(Ordering::SeqCst),
        |_, _| {
            output.store(true, Ordering::SeqCst);
            Ok(json!({}))
        },
    )
    .unwrap();
    assert_eq!(first["nextPage"], 2);
    assert_eq!(first["status"], "paused");
    assert_eq!(f.calls, vec![1]);
    let result = scan(&mut c, 1, &id, &mut f, || false, |_, _| Ok(json!({}))).unwrap();
    assert_eq!(f.calls, vec![1, 2]);
    assert_eq!(result["spentPoints"], 11);
    assert_eq!(result["status"], "completed");
}
#[test]
fn timeout_preserves_unknown_page_and_never_retries_it() {
    let mut c = fixture();
    let id = prepare(
        &mut c,
        &input("fingerprint", &"a".repeat(64)),
        "synthetic-key",
    )
    .unwrap();
    let mut f = fake(vec![page(20, &(1..=10).collect::<Vec<_>>())]);
    f.fail = Some(1);
    let first = scan(&mut c, 1, &id, &mut f, || false, |_, _| Ok(json!({}))).unwrap();
    assert_eq!(first["status"], "unknown");
    assert_eq!(first["costUnknown"], true);
    let again = scan(&mut c, 1, &id, &mut f, || false, |_, _| Ok(json!({}))).unwrap();
    assert_eq!(again["status"], "unknown");
    assert_eq!(f.calls, vec![1]);
}
#[test]
fn failed_local_import_reuses_received_page_and_stable_import_id_without_search() {
    let mut c = fixture();
    let id = prepare(
        &mut c,
        &input("fingerprint", &"a".repeat(64)),
        "synthetic-key",
    )
    .unwrap();
    let mut f = fake(vec![page(1, &[99])]);
    let mut request = String::new();
    assert!(scan(
        &mut c,
        1,
        &id,
        &mut f,
        || false,
        |id, _| {
            request = id.into();
            Err("local import unavailable".into())
        }
    )
    .is_err());
    let result = scan(
        &mut c,
        1,
        &id,
        &mut f,
        || false,
        |id, rows| {
            assert_eq!(id, request);
            assert_eq!(rows.len(), 1);
            Ok(json!({}))
        },
    )
    .unwrap();
    assert_eq!(f.calls, vec![1]);
    assert_eq!(result["spentPoints"], 1);
    assert_eq!(result["status"], "completed");
}
#[test]
fn crash_after_receipt_before_cost_accounting_recovers_cost_without_requery() {
    let mut c = fixture();
    let id = prepare(
        &mut c,
        &input("fingerprint", &"a".repeat(64)),
        "synthetic-key",
    )
    .unwrap();
    c.execute(
        "UPDATE hunter_expansions SET page_size=10 WHERE id=?1",
        [&id],
    )
    .unwrap();
    c.execute("INSERT INTO hunter_expansion_pages(query_id,page,state,request_id,payload_json) VALUES(?1,1,'received','synthetic-receipt',?2)",params![id,page(1,&[99]).to_string()]).unwrap();
    let mut f = fake(vec![]);
    let result = scan(&mut c, 1, &id, &mut f, || false, |_, _| Ok(json!({}))).unwrap();
    assert!(f.calls.is_empty());
    assert_eq!(result["spentPoints"], 1);
    assert_eq!(result["status"], "completed");
}
#[test]
fn repeated_page_and_early_empty_page_stop_with_explicit_gap_and_cost() {
    for last in [page(30, &(1..=10).collect::<Vec<_>>()), page(30, &[])] {
        let mut c = fixture();
        let id = prepare(
            &mut c,
            &input("fingerprint", &"a".repeat(64)),
            "synthetic-key",
        )
        .unwrap();
        let mut f = fake(vec![page(30, &(1..=10).collect::<Vec<_>>()), last]);
        let result = scan(&mut c, 1, &id, &mut f, || false, |_, _| Ok(json!({}))).unwrap();
        assert_eq!(result["status"], "partial");
        assert_eq!(result["nextPage"], 2);
        assert_eq!(f.calls, vec![1, 2]);
    }
}
#[test]
fn mismatched_certificate_is_retained_as_rejected_and_stops_further_charging() {
    let mut c = fixture();
    let id = prepare(
        &mut c,
        &input("fingerprint", &"a".repeat(64)),
        "synthetic-key",
    )
    .unwrap();
    let mut payload = page(50, &[1]);
    payload["data"]["arr"][0]["cert_sha256"] = json!("b".repeat(64));
    let mut f = fake(vec![payload]);
    let mut calls = 0;
    let result = scan(
        &mut c,
        1,
        &id,
        &mut f,
        || false,
        |_, rows| {
            calls += rows.len();
            Ok(json!({}))
        },
    )
    .unwrap();
    assert_eq!(calls, 0);
    assert_eq!(result["rejected"], 1);
    assert_eq!(result["status"], "partial");
    assert_eq!(result["spentPoints"], 1);
}
#[test]
fn all_asset_types_and_ip_only_tls_survive_exact_relation_validation() {
    let hash = "a".repeat(64);
    let v = json!({"code":200,"data":{"arr":[{"ip":"8.141.92.254","port":443,"protocol":"https","cert_sha256":hash},{"ip":"218.201.45.123","port":8883,"protocol":"mqtt","cert_sha256":hash}]}});
    let (r, bad) = rows(&v, "fingerprint", &hash).unwrap();
    assert_eq!(bad, 0);
    assert_eq!(r.len(), 2);
    assert_eq!(r[1]["protocol"], "mqtt");
    let u = page_url(
        "https://hunter.qianxin.com/openApi/search",
        "not-a-real-key",
        "cert.sha-256=\"abc\"",
        7,
        100,
        "2026-09-21",
        "2026-10-20",
        "fingerprint",
    )
    .unwrap();
    let q = u.query_pairs().collect::<HashMap<_, _>>();
    assert_eq!(q.get("page").unwrap(), "7");
    assert_eq!(q.get("is_web").unwrap(), "3");
    assert!(base64::engine::general_purpose::URL_SAFE
        .decode(q.get("search").unwrap().as_bytes())
        .is_ok());
}
#[test]
fn credentials_are_not_in_journal_and_query_restarts_reuse_same_frozen_identity() {
    let mut c = fixture();
    let i = input("fingerprint", &"a".repeat(64));
    let a = prepare(&mut c, &i, "synthetic-secret").unwrap();
    let b = prepare(&mut c, &i, "synthetic-secret").unwrap();
    assert_eq!(a, b);
    let j = list_on(&c, 1).unwrap();
    assert!(!json!(j).to_string().contains("synthetic-secret"));
    assert_eq!(points(&json!("消耗积分：12")), Some(12));
    assert_eq!(points(&json!("未返回")), None);
    assert_eq!(
        cost(
            &json!({"data":{"consume_quota":"消耗积分：2"}}),
            &json!({"rest_free_point":100,"rest_equity_point":0}),
            &json!({"rest_free_point":80,"rest_equity_point":0})
        ),
        Some(2)
    );
}
#[test]
fn real_importer_replays_one_page_without_duplicate_runs_and_merges_multiple_relations() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-hunter-import-{}", uuid::Uuid::new_v4()));
    let path = crate::db::initialize(&root).unwrap();
    let c = crate::db::open(&path).unwrap();
    c.execute(
        "INSERT INTO projects(id,name) VALUES(9001,'Synthetic Hunter merge')",
        [],
    )
    .unwrap();
    let payload = page(1, &[99]);
    let (a, _) = rows(&payload, "fingerprint", &"a".repeat(64)).unwrap();
    let first = uuid::Uuid::new_v4().to_string();
    let r = crate::jobs::import_hunter_discovery(&path, &root, 9001, "example.com", &first, &a)
        .unwrap();
    let again = crate::jobs::import_hunter_discovery(&path, &root, 9001, "example.com", &first, &a)
        .unwrap();
    assert_eq!(r["runId"], again["runId"]);
    let mut b = a.clone();
    b[0]["hunter_query_kind"] = json!("cname");
    b[0]["matched_queries"] = json!("Hunter cname: tenant.cloud.test");
    crate::jobs::import_hunter_discovery(
        &path,
        &root,
        9001,
        "example.com",
        &uuid::Uuid::new_v4().to_string(),
        &b,
    )
    .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM project_assets WHERE project_id=9001",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM runs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let extra: String = c
        .query_row("SELECT extra_json FROM assets", [], |r| r.get(0))
        .unwrap();
    assert!(extra.contains("fingerprint") && extra.contains("cname"));
    drop(c);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changes_in_total_are_not_reported_as_stable_complete_coverage() {
    let mut c = fixture();
    let id = prepare(
        &mut c,
        &input("fingerprint", &"a".repeat(64)),
        "synthetic-key",
    )
    .unwrap();
    let mut f = fake(vec![
        page(11, &(1..=10).collect::<Vec<_>>()),
        page(12, &[98, 99]),
    ]);
    let r = scan(&mut c, 1, &id, &mut f, || false, |_, _| Ok(json!({}))).unwrap();
    assert_eq!(r["status"], "partial");
    assert!(r["reason"].as_str().unwrap().contains("总量发生变化"));
    assert_eq!(f.calls, vec![1, 2]);
    let _ = scan(&mut c, 1, &id, &mut f, || false, |_, _| Ok(json!({}))).unwrap();
    assert_eq!(f.calls, vec![1, 2]);
}
#[test]
fn cname_and_domain_results_require_exact_returned_relation() {
    let v = json!({"code":200,"data":{"arr":[{"ip":"192.0.2.1","port":443,"domain":"app.example.com","protocol":"https","cname_target":"tenant.cloud.test"},{"ip":"192.0.2.2","port":443,"domain":"app.example.com.evil.test","protocol":"https","cname_target":"tenant.cloud.test.evil.test"}]}});
    let (a, b) = rows(&v, "cname", "tenant.cloud.test").unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(b, 1);
    let (a, b) = rows(&v, "domain", "app.example.com").unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(b, 1);
    let (a, _) = rows(&v, "domain_tree", "example.com").unwrap();
    assert_eq!(a.len(), 1);
}

#[test]
fn official_hunter_site_registration_is_retained_as_endpoint_evidence_not_project_label() {
    let payload = json!({"code":200,"data":{"arr":[{
        "ip":"192.0.2.10","port":443,"domain":"owned.example.test",
        "url":"https://owned.example.test/","protocol":"https",
        "company":"Exact Registered Legal Owner","number":"京ICP备-fixture-8",
        "cname_target":"other.example.test",
    }]}});
    let (accepted, rejected) = rows(&payload, "domain", "owned.example.test").unwrap();
    assert_eq!(rejected, 0);
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["icp_name"], "Exact Registered Legal Owner");
    assert_eq!(accepted[0]["icp_number"], "京ICP备-fixture-8");
    assert!(
        accepted[0].get("company").is_none(),
        "the importer owns the separate collection label"
    );
}
#[test]
fn zero_balance_stops_before_search_and_unknown_charge_is_not_zero() {
    let mut c = fixture();
    let id = prepare(
        &mut c,
        &input("fingerprint", &"a".repeat(64)),
        "synthetic-key",
    )
    .unwrap();
    let mut f = fake(vec![]);
    f.equity = 0;
    let r = scan(&mut c, 1, &id, &mut f, || false, |_, _| Ok(json!({}))).unwrap();
    assert_eq!(r["status"], "quota");
    assert!(f.calls.is_empty());
    assert_eq!(points(&json!("消耗积分：未知")), None);
    assert_eq!(
        balance(&json!({"rest_free_point":-1,"rest_equity_point":-1})),
        None
    );
}

#[test]
fn shared_fingerprint_keeps_multiple_real_references_when_first_asset_is_deleted() {
    let c = fixture();
    c.execute("INSERT INTO assets(id,extra_json) SELECT 8,extra_json FROM assets WHERE id=7",[]).unwrap();
    c.execute("INSERT INTO project_assets VALUES(1,8,0)", [])
        .unwrap();
    insert_seed(&c, 1, "app.example.com", "observed_host", "asset:8", false).unwrap();
    insert_seed(
        &c,
        1,
        &"a".repeat(64),
        "certificate_sha256",
        "asset:8",
        false,
    )
    .unwrap();
    c.execute(
        "UPDATE project_assets SET is_deleted=1 WHERE asset_id=7",
        [],
    )
    .unwrap();
    assert_eq!(
        evidence_anchor(&c, 1, &"a".repeat(64), "fingerprint").unwrap(),
        "app.example.com"
    );
    c.execute(
        "UPDATE project_assets SET is_deleted=1 WHERE asset_id=8",
        [],
    )
    .unwrap();
    assert!(evidence_anchor(&c, 1, &"a".repeat(64), "fingerprint").is_err());
}

#[test]
fn legacy_seed_reads_do_not_require_or_create_new_evidence_table() {
    let c = fixture();
    c.execute("DROP TABLE asset_discovery_seed_evidence", [])
        .unwrap();
    let before = c.total_changes();
    assert_eq!(
        evidence_anchor(&c, 1, "tenant.cloud.test", "cname").unwrap(),
        "app.example.com"
    );
    assert_eq!(c.total_changes(), before);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='asset_discovery_seed_evidence'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn account_delta_never_impersonates_confirmed_query_cost_when_receipt_is_missing() {
    assert_eq!(
        cost(
            &json!({"data":{}}),
            &json!({"rest_free_point":100,"rest_equity_point":100}),
            &json!({"rest_free_point":80,"rest_equity_point":100})
        ),
        None
    );
}

#[test]
fn imported_domain_without_certificate_names_and_external_cname_remain_pending() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-hunter-next-seeds-{}",
        uuid::Uuid::new_v4()
    ));
    let path = crate::db::initialize(&root).unwrap();
    let c = crate::db::open(&path).unwrap();
    c.execute(
        "INSERT INTO projects(id,name) VALUES(9001,'Synthetic continuous discovery')",
        [],
    )
    .unwrap();
    ensure(&c).unwrap();
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(9001,'[\"example.com\"]')",
        [],
    )
    .unwrap();
    let rows = vec![
        json!({"host":"app.example.com","link":"https://app.example.com/","ip":"192.0.2.9","port":"443","protocol":"https","domain":"app.example.com","title":"observed endpoint","score":"40","cert_sha256":"a".repeat(64),"cname_target":"tenant.cloud.test","hunter_query_kind":"domain","matched_queries":"Hunter domain: app.example.com"}),
    ];
    crate::jobs::import_hunter_discovery(
        &path,
        &root,
        9001,
        "example.com",
        &uuid::Uuid::new_v4().to_string(),
        &rows,
    )
    .unwrap();
    assert!(evidence_anchor(&c, 9001, &"a".repeat(64), "fingerprint").is_err());
    assert!(evidence_anchor(&c, 9001, "tenant.cloud.test", "cname").is_err());
    let view = crate::asset_discovery::view(&c, 9001).unwrap();
    assert!(view["seeds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["source"] == "certificate_sha256" && s["queryEligible"] == false));
    drop(c);
    std::fs::remove_dir_all(root).unwrap();
}
