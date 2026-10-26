use super::*;

fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT);INSERT INTO projects VALUES(1,'主项目','active'),(2,'其他项目','active');CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,company TEXT,extra_json TEXT,content_category TEXT,probe_outcome TEXT DEFAULT '');CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,decision TEXT DEFAULT 'pending');CREATE TABLE asset_events(id INTEGER PRIMARY KEY,project_id INTEGER,asset_id INTEGER,event_type TEXT,summary TEXT);CREATE TABLE asset_ownership_profiles(project_id INTEGER,legal_name TEXT,aliases_json TEXT,approved_domains_json TEXT,shared_domains_json TEXT);").unwrap();
    super::super::ensure(&c).unwrap();
    c
}
fn query(p: i64) -> Query {
    serde_json::from_value(json!({"projectId":p})).unwrap()
}
fn baseline(c: &Connection, p: i64) {
    for _ in 0..100 {
        let r = initialize(c, p).unwrap();
        assert!(r["processed"].as_u64().unwrap() <= BATCH as u64);
        if r["initialized"] == true {
            return;
        }
    }
    panic!("baseline did not converge")
}
fn drain(c: &Connection, p: i64, round: i64) {
    for _ in 0..100 {
        let r = sync(c, p, "operation-fixture", round).unwrap();
        assert!(r["processed"].as_u64().unwrap() <= BATCH as u64);
        if r["hasMore"] == false {
            return;
        }
    }
    panic!("sync did not converge")
}
fn insert(c: &Connection, p: i64, h: &str, source: &str, reference: &str) {
    super::super::insert_seed(c, p, h, source, reference, false).unwrap();
}
fn item(c: &Connection, p: i64, h: &str) -> Value {
    let v = read(c, &query(p)).unwrap();
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["value"] == h)
        .unwrap()
        .clone()
}

#[test]
fn read_is_pure_and_uninitialized_is_unknown_not_zero() {
    let c = fixture();
    let before: i64 = c
        .query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r.get(0))
        .unwrap();
    let v = read(&c, &query(1)).unwrap();
    assert_eq!(v["ready"], false);
    assert!(v["total"].is_null());
    assert!(v["summary"].is_null());
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert!(detail(&c, 1, 1, 1, 20)
        .unwrap_err()
        .contains("not_initialized"));
}
#[test]
fn historical_baseline_and_real_round_are_not_stolen_by_initializer() {
    let c = fixture();
    insert(&c, 1, "old.example.com", "certificate_cn", "asset:1");
    baseline(&c, 1);
    let old = item(&c, 1, "old.example.com");
    assert!(old["firstSeenRound"].is_null());
    assert!(old["firstSeenAt"].is_null());
    assert_eq!(old["historicalBaseline"], true);
    assert_eq!(old["observationCount"], 1);
    insert(&c, 1, "new.example.com", "certificate_cn", "asset:2");
    let init = initialize(&c, 1).unwrap();
    assert_eq!(init["processed"], 0);
    assert_eq!(init["hasMore"], false);
    let result = sync(&c, 1, "operation-2", 2).unwrap();
    assert_eq!(result["newSeeds"], 1);
    assert_eq!(result["newObservations"], 1);
    let new = item(&c, 1, "new.example.com");
    assert_eq!(new["firstSeenRound"], 2);
    assert_eq!(new["historicalBaseline"], false);
    assert!(new["firstSeenAt"].is_string());
    assert_eq!(sync(&c, 1, "operation-2", 2).unwrap()["newObservations"], 0);
    assert_eq!(
        item(&c, 1, "old.example.com")["firstSeenRound"],
        Value::Null
    );
}
#[test]
fn same_value_deduplicates_but_source_edges_and_projects_are_preserved() {
    let c = fixture();
    insert(&c, 1, "same.example.com", "certificate_cn", "asset:1");
    insert(&c, 1, "same.example.com", "certificate_cn", "asset:2");
    insert(&c, 2, "same.example.com", "certificate_cn", "asset:9");
    baseline(&c, 1);
    baseline(&c, 2);
    let a = item(&c, 1, "same.example.com");
    let b = item(&c, 2, "same.example.com");
    assert_eq!(read(&c, &query(1)).unwrap()["total"], 1);
    assert_eq!(a["observationCount"], 2);
    assert_eq!(b["observationCount"], 1);
    assert!(detail(&c, 2, a["id"].as_i64().unwrap(), 1, 20)
        .unwrap_err()
        .contains("unavailable"));
    assert_eq!(
        detail(&c, 1, a["id"].as_i64().unwrap(), 1, 20).unwrap()["observationsTotal"],
        2
    );
}
#[test]
fn bounded_batches_keep_global_round_and_repeated_sync_has_no_discoveries() {
    let c = fixture();
    baseline(&c, 1);
    let tx = c.unchecked_transaction().unwrap();
    for n in 0..1201 {
        insert(
            &tx,
            1,
            &format!("n{n}.example.com"),
            "certificate_cn",
            &format!("asset:{}", n + 1),
        );
    }
    tx.commit().unwrap();
    let first = sync(&c, 1, "one-wave", 7).unwrap();
    assert_eq!(first["processed"], 500);
    assert_eq!(first["hasMore"], true);
    drain(&c, 1, 7);
    assert_eq!(read(&c, &query(1)).unwrap()["total"], 1201);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(DISTINCT first_round) FROM asset_project_seeds WHERE project_id=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(sync(&c, 1, "later", 8).unwrap()["newObservations"], 0);
}

fn hunter(c: &Connection) {
    c.execute_batch("CREATE TABLE hunter_expansions(id TEXT PRIMARY KEY,project_id INTEGER,kind TEXT,target TEXT,status TEXT,created_at TEXT,spent INTEGER,cost_unknown INTEGER,unique_rows INTEGER,received INTEGER,account_json TEXT);CREATE TABLE hunter_expansion_pages(query_id TEXT,page INTEGER,state TEXT,cost INTEGER,accounted INTEGER,raw_count INTEGER,accepted INTEGER,rejected INTEGER,import_json TEXT,payload_json TEXT);CREATE TABLE hunter_expansion_links(project_id INTEGER,endpoint TEXT,query_id TEXT,page INTEGER,evidence_json TEXT);CREATE TABLE asset_discovery_pipeline_queries(operation_id TEXT,project_id INTEGER,query_id TEXT,round_number INTEGER);").unwrap();
}
fn query_row(c: &Connection, id: &str) {
    c.execute("INSERT INTO hunter_expansions VALUES(?1,1,'domain','query.example.com','running','2026-10-23T01:00:00Z',0,1,0,0,?2)",params![id,json!({"apiKey":"DO-NOT-LEAK-KEY","phone":"private-account"}).to_string()]).unwrap();
}
#[test]
fn material_query_receipts_append_and_late_pages_keep_original_round() {
    let c = fixture();
    hunter(&c);
    baseline(&c, 1);
    query_row(&c, "q1");
    c.execute(
        "INSERT INTO asset_discovery_pipeline_queries VALUES('original-op',1,'q1',4)",
        [],
    )
    .unwrap();
    drain(&c, 1, 9);
    c.execute(
        "INSERT INTO hunter_expansion_pages VALUES('q1',1,'sending',NULL,0,0,0,0,'{}',?1)",
        [json!({"body":"DO-NOT-LEAK-BODY","ssl_certificate":"DO-NOT-LEAK-DER"}).to_string()],
    )
    .unwrap();
    drain(&c, 1, 9);
    c.execute("UPDATE hunter_expansion_pages SET state='received',cost=2,accounted=1,raw_count=10,accepted=8,rejected=2",[]).unwrap();
    drain(&c, 1, 10);
    c.execute("UPDATE hunter_expansion_pages SET state='imported',import_json='{\"result\":{\"inserted\":3,\"updated\":5}}'",[]).unwrap();
    // Import receipt itself is a material receipt transition, even if the page
    // accounting numbers stay equal to those already received.
    drain(&c, 1, 10);
    c.execute("UPDATE hunter_expansions SET status='completed',spent=2,cost_unknown=0,unique_rows=8,received=10",[]).unwrap();
    drain(&c, 1, 11);
    let seed = item(&c, 1, "query.example.com");
    assert_eq!(seed["firstSeenRound"], 4);
    let d = detail(&c, 1, seed["id"].as_i64().unwrap(), 1, 20).unwrap();
    let obs = d["observations"].as_array().unwrap();
    assert!(obs
        .iter()
        .all(|o| o["round"] == 4 && o["operationId"] == "original-op"));
    assert!(obs.iter().any(|o| o["sourceType"] == "hunter_page"
        && o["costUnknown"] == true
        && o["spentPoints"].is_null()));
    assert!(obs.iter().any(|o| o["sourceType"] == "hunter_page"
        && o["spentPoints"] == 2
        && o["costUnknown"] == false));
    assert!(obs.iter().any(|o| o["evidence"]["queryState"] == "sending"));
    assert!(obs
        .iter()
        .any(|o| o["evidence"]["queryState"] == "received"));
    assert!(obs
        .iter()
        .any(|o| o["evidence"]["queryState"] == "imported"));
    for secret in [
        "DO-NOT-LEAK-KEY",
        "private-account",
        "DO-NOT-LEAK-BODY",
        "DO-NOT-LEAK-DER",
    ] {
        assert!(!d.to_string().contains(secret));
    }
    let count = d["observationsTotal"].clone();
    assert_eq!(sync(&c, 1, "poll-op", 12).unwrap()["newObservations"], 0);
    assert_eq!(
        detail(&c, 1, seed["id"].as_i64().unwrap(), 1, 20).unwrap()["observationsTotal"],
        count
    );
}
#[test]
fn duplicate_hunter_assets_remain_a_discovery_trail_not_new_seed() {
    let c = fixture();
    hunter(&c);
    baseline(&c, 1);
    for (q, round) in [("q1", 2), ("q2", 2)] {
        query_row(&c, q);
        c.execute(
            "INSERT INTO asset_discovery_pipeline_queries VALUES(?1,1,?2,?3)",
            params!["op", q, round],
        )
        .unwrap();
        c.execute("INSERT INTO hunter_expansion_links VALUES(1,'https://same.example.com',?1,1,?2)",params![q,json!({"domain":"same.example.com","cert_sha256":"a".repeat(64),"ssl_certificate":"PRIVATE"}).to_string()]).unwrap();
    }
    drain(&c, 1, 2);
    let seed = item(&c, 1, "same.example.com");
    assert_eq!(seed["observationCount"], 2);
    let d = detail(&c, 1, seed["id"].as_i64().unwrap(), 1, 20).unwrap();
    assert_eq!(d["observationsTotal"], 2);
    assert!(d["observations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["queryId"] == "q1"));
    assert!(d["observations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["queryId"] == "q2"));
    assert!(!d.to_string().contains("PRIVATE"));
    assert_eq!(item(&c, 1, &"a".repeat(64))["kind"], "fingerprint");
}
#[test]
fn only_real_free_observations_generate_cross_round_rediscovery() {
    let c = fixture();
    c.execute_batch("CREATE TABLE asset_enrichment_observations(id INTEGER PRIMARY KEY,project_id INTEGER,kind TEXT,target TEXT,content_hash TEXT,payload_json TEXT,first_seen INTEGER,last_seen INTEGER);").unwrap();
    baseline(&c, 1);
    c.execute("INSERT INTO asset_enrichment_observations VALUES(1,1,'dns','fresh.example.com','hash','{}',100,100)",[]).unwrap();
    drain(&c, 1, 7);
    let a = item(&c, 1, "fresh.example.com");
    assert_eq!(a["firstSeenRound"], 7);
    assert_eq!(a["observationCount"], 1);
    assert_eq!(sync(&c, 1, "poll", 8).unwrap()["newObservations"], 0);
    c.execute("UPDATE asset_enrichment_observations SET last_seen=101", [])
        .unwrap();
    drain(&c, 1, 8);
    let a = item(&c, 1, "fresh.example.com");
    assert_eq!(a["firstSeenRound"], 7);
    assert_eq!(a["lastSeenRound"], 8);
    assert_eq!(a["observationCount"], 2);
    c.execute("UPDATE asset_enrichment_observations SET last_seen=101", [])
        .unwrap();
    assert_eq!(sync(&c, 1, "poll", 8).unwrap()["newObservations"], 0);
}
fn review_fixture(c: &Connection) {
    c.execute_batch("CREATE TABLE asset_seed_reviews(id TEXT PRIMARY KEY,project_id INTEGER,organization TEXT,include_isolated INTEGER,cursor INTEGER,max_id INTEGER);CREATE TABLE asset_seed_review_items(run_id TEXT,item_key TEXT,kind TEXT,target TEXT,evidence_json TEXT,asset_count INTEGER,state TEXT,advice TEXT,confidence REAL,reason TEXT,decision TEXT,revision INTEGER);CREATE TABLE asset_seed_review_refs(run_id TEXT,item_key TEXT,asset_id INTEGER);CREATE TABLE asset_seed_assists(project_id INTEGER PRIMARY KEY,run_id TEXT,organization TEXT,include_isolated INTEGER,proposal_json TEXT);INSERT INTO asset_seed_reviews VALUES('review',1,'目标单位',0,1,1);INSERT INTO assets VALUES(1,'https://service.example.com','https://service.example.com','采集公司标签','{\"icp.name\":\"目标单位有限公司\",\"collection_provider\":\"fofa\"}','','');INSERT INTO project_assets VALUES(1,1,0,'pending');INSERT INTO asset_seed_review_items VALUES('review','key','certificate_metadata','service.example.com','{\"domains\":[\"service.example.com\"],\"icpOrganizations\":[\"目标单位有限公司\"]}',1,'done','retain',.99,'模型建议','pending',0);INSERT INTO asset_seed_review_refs VALUES('review','key',1);").unwrap();
    c.execute("INSERT INTO asset_seed_assists VALUES(1,'review','目标单位',0,?1)",[json!({"groups":[{"verification":{"status":"bound_local_evidence","sources":[{"kind":"icp","host":"unrelated-bank.example","organization":"目标单位有限公司"}]}}]}).to_string()]).unwrap();
}
#[test]
fn trust_rechecks_current_independent_facts_never_ai_or_cached_proposal() {
    for update in [
        "UPDATE assets SET extra_json='{\"icp.name\":\"无关银行有限公司\"}'",
        "UPDATE project_assets SET is_deleted=1",
        "UPDATE assets SET content_category='custom_rule'",
        "UPDATE project_assets SET decision='rejected'",
    ] {
        let c = fixture();
        review_fixture(&c);
        insert(&c, 1, "service.example.com", "observed_host", "asset:1");
        insert(
            &c,
            1,
            "unrelated-bank.example",
            "certificate_domain",
            "asset:1",
        );
        baseline(&c, 1);
        assert_eq!(
            item(&c, 1, "service.example.com")["trustStatus"],
            "verified"
        );
        assert_eq!(
            item(&c, 1, "unrelated-bank.example")["trustStatus"],
            "needs_evidence"
        );
        c.execute(update, []).unwrap();
        let stale = item(&c, 1, "service.example.com");
        assert_eq!(stale["trustStatus"], "needs_evidence");
        assert_eq!(stale["trustPending"], true);
        drain(&c, 1, 3);
        assert_eq!(
            item(&c, 1, "service.example.com")["trustStatus"],
            "needs_evidence"
        );
        assert_eq!(
            item(&c, 1, "service.example.com")["verificationCoverage"]["verifiedEvidence"],
            0
        );
    }
}
#[test]
fn manual_sample_rates_require_actual_decision_events_and_history_survives_exclusion() {
    let c = fixture();
    c.execute_batch("INSERT INTO assets VALUES(1,'https://one.example.com','https://one.example.com','','{}','','');INSERT INTO project_assets VALUES(1,1,0,'confirmed');CREATE TABLE asset_discovery_domain_exclusions(project_id INTEGER,domain TEXT,enabled INTEGER);CREATE TABLE asset_discovery_excluded_evidence(project_id INTEGER,host TEXT,source TEXT,source_ref TEXT,domain TEXT);INSERT INTO asset_discovery_domain_exclusions VALUES(1,'one.example.com',1);").unwrap();
    insert(&c, 1, "one.example.com", "observed_host", "asset:1");
    baseline(&c, 1);
    let a = item(&c, 1, "one.example.com");
    assert_eq!(a["trustStatus"], "excluded");
    assert!(a["manualConclusion"]["acceptanceRate"].is_null());
    assert_eq!(a["manualConclusion"]["reviewed"], 0);
    c.execute(
        "INSERT INTO asset_events VALUES(1,1,1,'decision','confirmed: 人工已审')",
        [],
    )
    .unwrap();
    assert_eq!(
        item(&c, 1, "one.example.com")["manualConclusion"]["acceptanceRate"],
        1.0
    );
    c.execute(
        "INSERT INTO asset_events VALUES(2,1,1,'decision','rejected: 更新人工结论')",
        [],
    )
    .unwrap();
    assert_eq!(
        item(&c, 1, "one.example.com")["manualConclusion"]["acceptanceRate"],
        0.0
    );
    c.execute("UPDATE asset_discovery_domain_exclusions SET enabled=0", [])
        .unwrap();
    let changes = c.total_changes();
    assert_eq!(item(&c, 1, "one.example.com")["trustStatus"], "needs_evidence");
    assert_eq!(read(&c, &query(1)).unwrap()["summary"]["excluded"], 0);
    assert_eq!(c.total_changes(), changes, "undo must project before any sync");
    assert_eq!(c.query_row("SELECT trust_status FROM asset_project_seeds WHERE id=?1",[a["id"].as_i64().unwrap()],|r|r.get::<_,String>(0)).unwrap(), "excluded", "the old derived row remains untouched by reads");
    drain(&c, 1, 4);
    let b = item(&c, 1, "one.example.com");
    assert_eq!(b["trustStatus"], "needs_evidence");
    assert_eq!(b["observationCount"], a["observationCount"]);
    assert_eq!(b["firstSeenRound"], Value::Null);
}
#[test]
fn repeated_fofa_membership_receipts_record_a_real_new_run_only() {
    let c = fixture();
    c.execute_batch("ALTER TABLE project_assets ADD COLUMN last_run_id INTEGER;INSERT INTO assets VALUES(1,'https://one.example.com','https://one.example.com','','{\"collection_provider\":\"fofa\"}','','');INSERT INTO project_assets VALUES(1,1,0,'pending',12);").unwrap();
    insert(&c, 1, "one.example.com", "observed_host", "asset:1");
    baseline(&c, 1);
    let before = item(&c, 1, "one.example.com");
    c.execute("UPDATE project_assets SET last_run_id=13", [])
        .unwrap();
    drain(&c, 1, 9);
    let after = item(&c, 1, "one.example.com");
    assert_eq!(
        after["observationCount"].as_i64().unwrap(),
        before["observationCount"].as_i64().unwrap() + 1
    );
    assert!(after["firstSeenRound"].is_null());
    assert_eq!(after["lastSeenRound"], 9);
    let detail = detail(&c, 1, after["id"].as_i64().unwrap(), 1, 20).unwrap();
    assert!(detail["observations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["sourceType"] == "fofa"
            && o["queryId"] == "collection-run:13"
            && o["evidence"]["collectionRunId"] == 13));
    c.execute("UPDATE project_assets SET last_run_id=13", [])
        .unwrap();
    assert_eq!(sync(&c, 1, "same-run", 10).unwrap()["newObservations"], 0);
}
#[test]
fn paginated_read_only_history_is_complete_and_bounded() {
    let path = std::env::temp_dir().join(format!("ovi-seed-ledger-{}.db", uuid::Uuid::new_v4()));
    {
        let c = fixture();
        c.execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
    }
    {
        let c = Connection::open(&path).unwrap();
        for n in 0..61 {
            insert(
                &c,
                1,
                &format!(
                    "{}.{}.{}.n{n}.example.com",
                    "a".repeat(63),
                    "b".repeat(63),
                    "c".repeat(63)
                ),
                "certificate_cn",
                &format!("asset:{n}"),
            );
        }
        for n in 100..145 {
            insert(
                &c,
                1,
                "many.example.com",
                "certificate_cn",
                &format!("asset:{n}"),
            );
        }
        baseline(&c, 1);
    }
    let c = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut q = query(1);
    let a = read(&c, &q).unwrap();
    assert_eq!(a["items"].as_array().unwrap().len(), 50);
    assert_eq!(a["summary"]["total"], 62);
    assert!(
        a.to_string().len() < 48 * 1024,
        "list is {} bytes",
        a.to_string().len()
    );
    q.page = 2;
    assert_eq!(read(&c, &q).unwrap()["items"].as_array().unwrap().len(), 12);
    q.search = "many".into();
    q.page = 1;
    let seed = read(&c, &q).unwrap()["items"][0].clone();
    let id = seed["id"].as_i64().unwrap();
    let mut ids = HashSet::new();
    for page in 1..=3 {
        let d = detail(&c, 1, id, page, 20).unwrap();
        assert!(d.to_string().len() < 48 * 1024);
        assert_eq!(d["observationsTotal"], 45);
        for o in d["observations"].as_array().unwrap() {
            ids.insert(o["id"].as_i64().unwrap());
        }
    }
    assert_eq!(ids.len(), 45);
    q.page_size = 51;
    assert!(read(&c, &q).is_err());
    assert!(detail(&c, 1, id, 1, 21).is_err());
    drop(c);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn mismatched_hunter_link_project_is_never_projected_by_delta_or_baseline() {
    let c = fixture();
    hunter(&c);
    baseline(&c, 1);
    query_row(&c, "owner-one");
    c.execute("INSERT INTO hunter_expansion_links VALUES(2,'https://private.other-project.example','owner-one',1,'{\"domain\":\"private.other-project.example\"}')",[]).unwrap();
    drain(&c, 1, 2);
    baseline(&c, 2);
    assert!(!read(&c, &query(1))
        .unwrap()
        .to_string()
        .contains("private.other-project"));
    assert_eq!(read(&c, &query(2)).unwrap()["total"], 0);
}

#[test]
fn context_triggers_are_project_scoped_or_only_invalidate_shared_directory_states() {
    let c = fixture();
    c.execute_batch("CREATE TABLE project_org_bindings(project_id INTEGER,flag TEXT);CREATE TABLE asset_discovery_organization_evidence(project_id INTEGER,flag TEXT);CREATE TABLE org_entities(id INTEGER,flag TEXT);CREATE TABLE org_entity_identifiers(id INTEGER,flag TEXT);").unwrap();
    insert(&c, 1, "one.example.com", "observed_host", "asset:1");
    insert(&c, 2, "two.example.com", "observed_host", "asset:2");
    baseline(&c, 1);
    baseline(&c, 2);
    let revision = |p| {
        c.query_row(
            "SELECT proof_revision FROM asset_project_seed_state WHERE project_id=?1",
            [p],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let (a, b) = (revision(1), revision(2));
    c.execute("INSERT INTO project_org_bindings VALUES(1,'original')", [])
        .unwrap();
    assert_eq!(revision(1), a + 1);
    assert_eq!(revision(2), b);
    c.execute(
        "UPDATE project_org_bindings SET flag='changed' WHERE project_id=1",
        [],
    )
    .unwrap();
    assert_eq!(revision(1), a + 2);
    assert_eq!(revision(2), b);
    c.execute("DELETE FROM project_org_bindings", []).unwrap();
    assert_eq!(revision(1), a + 3);
    assert_eq!(revision(2), b);
    c.execute(
        "INSERT INTO asset_discovery_organization_evidence VALUES(2,'official change')",
        [],
    )
    .unwrap();
    assert_eq!(revision(1), a + 3);
    assert_eq!(revision(2), b + 1);
    c.execute("INSERT INTO org_entities VALUES(1,'shared directory')", [])
        .unwrap();
    assert_eq!(revision(1), a + 4);
    assert_eq!(revision(2), b + 2);
    c.execute(
        "INSERT INTO org_entity_identifiers VALUES(1,'official-domain')",
        [],
    )
    .unwrap();
    assert_eq!(revision(1), a + 5);
    assert_eq!(revision(2), b + 3);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM asset_project_seed_observations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM asset_project_seed_changes", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn changed_organization_aliases_invalidate_and_reproject_current_facts_locally() {
    let c = fixture();
    review_fixture(&c);
    c.execute("INSERT INTO asset_ownership_profiles VALUES(1,'目标单位有限公司','[\"子公司有限公司\"]','[]','[]')",[]).unwrap();
    c.execute(
        "UPDATE assets SET extra_json='{\"icp.name\":\"子公司有限公司\"}'",
        [],
    )
    .unwrap();
    insert(&c, 1, "service.example.com", "observed_host", "asset:1");
    baseline(&c, 1);
    assert_eq!(
        item(&c, 1, "service.example.com")["trustStatus"],
        "verified"
    );
    c.execute(
        "UPDATE asset_ownership_profiles SET aliases_json='[]' WHERE project_id=1",
        [],
    )
    .unwrap();
    let pending = item(&c, 1, "service.example.com");
    assert_eq!(pending["trustPending"], true);
    assert_eq!(pending["trustStatus"], "needs_evidence");
    drain(&c, 1, 2);
    assert_eq!(
        item(&c, 1, "service.example.com")["trustStatus"],
        "needs_evidence"
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM asset_project_seed_observations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn proof_ttl_is_read_only_stale_then_one_owner_projection_per_refresh_window() {
    let c = fixture();
    review_fixture(&c);
    insert(&c, 1, "service.example.com", "observed_host", "asset:1");
    baseline(&c, 1);
    let original = item(&c, 1, "service.example.com");
    let id = original["id"].as_i64().unwrap();
    assert_eq!(original["trustStatus"], "verified");
    c.execute("UPDATE asset_project_seed_state SET proof_cache_at=CAST(strftime('%s','now') AS INTEGER)-901 WHERE project_id=1",[]).unwrap();
    let before = c.total_changes();
    let stale = detail(&c, 1, id, 1, 20).unwrap();
    assert_eq!(stale["seed"]["trustPending"], true);
    assert_eq!(stale["seed"]["trustStatus"], "needs_evidence");
    assert_eq!(read(&c, &query(1)).unwrap()["summary"]["verified"], 0);
    let mut verified = query(1);
    verified.trust = "verified".into();
    assert_eq!(read(&c, &verified).unwrap()["total"], 0);
    assert_eq!(c.total_changes(), before);
    let result = sync(&c, 1, "local-recheck", 3).unwrap();
    assert_eq!(result["newObservations"], 0);
    let refreshed = detail(&c, 1, id, 1, 20).unwrap();
    assert_eq!(refreshed["seed"]["trustPending"], false);
    assert_eq!(refreshed["seed"]["trustStatus"], "verified");
    c.execute("UPDATE asset_project_seed_state SET proof_cache_at=CAST(strftime('%s','now') AS INTEGER)-100 WHERE project_id=1",[]).unwrap();
    let cached: i64 = c
        .query_row(
            "SELECT proof_cache_at FROM asset_project_seed_state WHERE project_id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for n in 0..601 {
        insert(
            &c,
            1,
            &format!("new{n}.example.com"),
            "observed_host",
            &format!("asset:{}", n + 10),
        );
    }
    let first = sync(&c, 1, "same-proof-window", 4).unwrap();
    assert_eq!(first["processed"], 500);
    assert_eq!(first["hasMore"], true);
    assert_eq!(
        c.query_row(
            "SELECT proof_cache_at FROM asset_project_seed_state WHERE project_id=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        cached
    );
    drain(&c, 1, 4);
    assert_eq!(
        c.query_row(
            "SELECT proof_cache_at FROM asset_project_seed_state WHERE project_id=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        cached
    );
}
#[test]
fn historical_gate_cache_preserves_refusals_but_live_delta_and_policy_revalidate() {
    let c = fixture();
    let hash = "d".repeat(64);
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.com\"]')",
        [],
    )
    .unwrap();
    for n in 0..1201 {
        insert(
            &c,
            1,
            &hash,
            "certificate_sha256",
            &format!("asset:{}", n + 1),
        );
    }
    let key = format!("1:fingerprint:{hash}");
    GATE_CALLS.with(|calls| {
        calls.borrow_mut().remove(&key);
    });
    baseline(&c, 1);
    assert_eq!(GATE_CALLS.with(|calls| calls.borrow()[&key]), 1);
    assert_eq!(item(&c, 1, &hash)["hunterEligible"], false);
    insert(&c, 1, &hash, "certificate_sha256", "asset:99999");
    drain(&c, 1, 2);
    assert_eq!(GATE_CALLS.with(|calls| calls.borrow()[&key]), 2);
    assert_eq!(item(&c, 1, &hash)["hunterEligible"], false);
    c.execute(
        "UPDATE asset_discovery_scopes SET roots_json='[]' WHERE project_id=1",
        [],
    )
    .unwrap();
    drain(&c, 1, 3);
    assert_eq!(GATE_CALLS.with(|calls| calls.borrow()[&key]), 3);
}
#[test]
fn batched_list_sources_counts_and_manual_samples_match_read_only_detail() {
    let c = fixture();
    c.execute_batch("INSERT INTO assets VALUES(1,'https://same.example.com','https://same.example.com','','{\"collection_provider\":\"fofa\",\"discovery_sources_json\":[{\"source\":\"hunter\"}]}','','');INSERT INTO assets VALUES(2,'https://same.example.com','https://same.example.com','','{}','','');INSERT INTO project_assets VALUES(1,1,0,'pending'),(1,2,0,'pending');INSERT INTO asset_events VALUES(1,1,1,'decision','confirmed: yes'),(2,1,2,'decision','rejected: no');").unwrap();
    insert(&c, 1, "same.example.com", "observed_host", "asset:1");
    insert(&c, 1, "same.example.com", "certificate_cn", "asset:2");
    baseline(&c, 1);
    let list = item(&c, 1, "same.example.com");
    let detail = detail(&c, 1, list["id"].as_i64().unwrap(), 1, 20).unwrap();
    assert_eq!(list, detail["seed"]);
    assert_eq!(list["manualConclusion"]["acceptanceRate"], 0.5);
    assert_eq!(list["assetCount"], 2);
    assert_eq!(list["sourceTypes"], json!(["fofa", "history", "hunter"]));
}
#[test]
fn malformed_pending_conflict_never_silently_drops_a_current_conflict() {
    let c = fixture();
    c.execute_batch("CREATE TABLE asset_enrichment_conflicts(project_id INTEGER,identity TEXT,hosts_json TEXT,decision TEXT);INSERT INTO asset_enrichment_conflicts VALUES(1,'other','invalid-json','pending');").unwrap();
    insert(&c, 1, "one.example.com", "observed_host", "asset:1");
    assert!(initialize(&c, 1).is_err());
    assert_eq!(read(&c, &query(1)).unwrap()["ready"], false);
    c.execute(
        "UPDATE asset_enrichment_conflicts SET hosts_json='{\"name\":\"one.example.com\"}'",
        [],
    )
    .unwrap();
    baseline(&c, 1);
    assert_eq!(item(&c, 1, "one.example.com")["trustStatus"], "conflict");
}

fn live_rule(c: &mut Connection, p: i64, domain: &str, action: &str) {
    let preview = exclusions::manage(c, exclusions::ExclusionInput {
        project_id:p, domain:domain.into(), action:"preview".into(), expected_digest:None,
    }).unwrap();
    exclusions::manage(c, exclusions::ExclusionInput {
        project_id:p, domain:domain.into(), action:action.into(), expected_digest:preview["digest"].as_str().map(str::to_owned),
    }).unwrap();
}

#[test]
fn ready_ledger_reads_live_exclusions_and_undo_without_sync_or_cache_writes() {
    let mut c = fixture();
    for (host, source) in [("bad.example.net","observed_host"),("login.bad.example.net","certificate_cn"),("alias.bad.example.net","cname"),("bad.example.net.evil.test","observed_host")] {
        insert(&c,1,host,source,&format!("https://{host}"));
    }
    insert(&c,2,"login.bad.example.net","observed_host","https://login.bad.example.net");
    baseline(&c,1);baseline(&c,2);
    let original=read(&c,&query(1)).unwrap();
    let id=item(&c,1,"login.bad.example.net")["id"].as_i64().unwrap();
    live_rule(&mut c,1,"bad.example.net","apply");
    c.execute("UPDATE asset_project_seed_state SET policy_cursor=0,proof_cache_at=0 WHERE project_id=1",[]).unwrap();
    let before=c.total_changes();
    c.execute_batch("PRAGMA query_only=ON").unwrap();
    let active=read(&c,&query(1)).unwrap();
    assert_eq!(active["summary"]["excluded"],3,"new rules must project without an owner sync");
    assert_eq!(active["summary"]["needsEvidence"],1);
    let mut excluded=query(1);excluded.trust="excluded".into();excluded.page_size=1;
    assert_eq!(read(&c,&excluded).unwrap()["total"],3);
    excluded.kind="cname".into();assert_eq!(read(&c,&excluded).unwrap()["total"],1);
    let blocked=detail(&c,1,id,1,20).unwrap();
    assert_eq!(blocked["seed"]["trustStatus"],"excluded");
    assert_eq!(blocked["seed"]["hunterEligible"],false);
    assert_eq!(blocked["seed"]["verificationCoverage"]["verifiedEvidence"],0);
    assert!(blocked["seed"]["trustReason"].as_str().unwrap().contains("排除"));
    assert_eq!(read(&c,&query(2)).unwrap()["summary"]["excluded"],0);
    assert_eq!(c.total_changes(),before,"list, filters, summary and detail must be pure reads");
    c.execute_batch("PRAGMA query_only=OFF").unwrap();
    assert_eq!(c.query_row("SELECT count(*) FROM asset_project_seeds WHERE project_id=1 AND trust_status='excluded'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"projection must not rewrite the stored cache");
    live_rule(&mut c,1,"bad.example.net","restore");
    let before=c.total_changes();c.execute_batch("PRAGMA query_only=ON").unwrap();
    let restored=read(&c,&query(1)).unwrap();
    assert_eq!(restored["summary"],original["summary"]);
    assert_eq!(detail(&c,1,id,1,20).unwrap()["seed"]["trustStatus"],"needs_evidence");
    assert_eq!(c.total_changes(),before);
}

#[test]
fn live_rules_override_a_fresh_verified_cache_and_undo_preserves_ttl_downgrade() {
    let mut c=fixture();review_fixture(&c);
    insert(&c,1,"service.example.com","observed_host","asset:1");
    baseline(&c,1);
    let original=item(&c,1,"service.example.com");
    let id=original["id"].as_i64().unwrap();
    assert_eq!(original["trustStatus"],"verified");
    live_rule(&mut c,1,"service.example.com","apply");
    let changes=c.total_changes();c.execute_batch("PRAGMA query_only=ON").unwrap();
    let blocked=detail(&c,1,id,1,20).unwrap()["seed"].clone();
    assert_eq!(blocked["trustStatus"],"excluded");assert_eq!(blocked["hunterEligible"],false);
    assert_eq!(blocked["verificationCoverage"]["verifiedEvidence"],0);
    assert_eq!(read(&c,&query(1)).unwrap()["summary"]["verified"],0);
    assert_eq!(read(&c,&query(1)).unwrap()["summary"]["excluded"],1);
    assert_eq!(c.total_changes(),changes);
    c.execute_batch("PRAGMA query_only=OFF").unwrap();
    live_rule(&mut c,1,"service.example.com","restore");
    assert_eq!(item(&c,1,"service.example.com")["trustStatus"],"verified");
    assert_eq!(detail(&c,1,id,1,20).unwrap()["seed"],item(&c,1,"service.example.com"));
    c.execute("UPDATE asset_project_seed_state SET proof_cache_at=CAST(strftime('%s','now') AS INTEGER)-901 WHERE project_id=1",[]).unwrap();
    let changes=c.total_changes();c.execute_batch("PRAGMA query_only=ON").unwrap();
    let expired=detail(&c,1,id,1,20).unwrap()["seed"].clone();
    assert_eq!(expired["trustStatus"],"needs_evidence");assert_eq!(expired["trustPending"],true);
    assert_eq!(expired["hunterEligible"],false);assert_eq!(expired["verificationCoverage"]["verifiedEvidence"],0);
    let mut verified=query(1);verified.trust="verified".into();
    assert_eq!(read(&c,&verified).unwrap()["total"],0);
    assert_eq!(read(&c,&query(1)).unwrap()["summary"]["needsEvidence"],1);
    assert_eq!(c.total_changes(),changes);
}

#[test]
fn live_domain_projection_never_excludes_fingerprints_or_overrides_independent_provenance() {
    let mut c=fixture();let shared="a".repeat(64);let dependent="b".repeat(64);
    insert(&c,1,"bad.example.net","observed_host","https://bad.example.net");
    insert(&c,1,"good.example.org","observed_host","https://good.example.org");
    insert(&c,1,&shared,"certificate_sha256","https://bad.example.net");
    insert(&c,1,&shared,"certificate_sha256","https://good.example.org");
    insert(&c,1,&dependent,"certificate_sha256","https://bad.example.net");
    baseline(&c,1);
    let original=item(&c,1,&shared);assert_eq!(original["observationCount"],2);
    live_rule(&mut c,1,"bad.example.net","apply");
    c.execute_batch("PRAGMA query_only=ON").unwrap();let changes=c.total_changes();
    let mut q=query(1);q.kind="fingerprint".into();
    let fingerprints=read(&c,&q).unwrap();assert_eq!(fingerprints["total"],2);
    assert!(fingerprints["items"].as_array().unwrap().iter().all(|s|s["trustStatus"]!="excluded" && s["hunterEligible"]==false));
    q.trust="excluded".into();assert_eq!(read(&c,&q).unwrap()["total"],0);
    assert_eq!(read(&c,&query(1)).unwrap()["summary"]["excluded"],1);
    assert_eq!(item(&c,1,&shared)["observationCount"],original["observationCount"]);
    let raw_hidden=|hash:&str|c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM asset_discovery_seeds WHERE project_id=1 AND host=?1 AND {})",exclusions::hidden_sql()),[hash],|r|r.get::<_,bool>(0)).unwrap();
    assert!(!raw_hidden(&shared),"another independent source protects the shared raw fingerprint");
    assert!(raw_hidden(&dependent),"the dependent raw reference is still blocked by the existing gate");
    assert_eq!(c.total_changes(),changes);
}

#[test]
fn explicit_seed_exclusion_is_exact_reversible_and_shared_by_hunter_and_list(){
 let mut c=fixture();insert(&c,1,"one.example.com","observed_host","asset:1");insert(&c,1,"two.example.com","observed_host","asset:2");insert(&c,2,"one.example.com","observed_host","asset:3");baseline(&c,1);baseline(&c,2);
 let before=c.query_row("SELECT count(*) FROM asset_discovery_seeds",[],|r|r.get::<_,i64>(0)).unwrap();
 let a=item(&c,1,"one.example.com");let id=a["id"].as_i64().unwrap();let op=uuid::Uuid::new_v4().to_string();
 let input=|action:&str,digest:Option<&str>|serde_json::from_value::<seed_policy::Input>(json!({"projectId":1,"action":action,"seedIds":[id],"expectedDigest":digest,"operationId":op})).unwrap();
 let preview=seed_policy::manage(&mut c,input("preview",None)).unwrap();let result=seed_policy::manage(&mut c,input("apply",preview["digest"].as_str())).unwrap();
 assert_eq!(result["count"],1);assert_eq!(seed_policy::manage(&mut c,input("apply",preview["digest"].as_str())).unwrap(),result,"unknown response uses same operation receipt");
 assert_eq!(item(&c,1,"one.example.com")["trustStatus"],"excluded");assert_eq!(item(&c,1,"one.example.com")["subjects"],json!([]));assert_eq!(item(&c,1,"one.example.com")["hunterEligible"],false);
 assert_ne!(item(&c,1,"two.example.com")["trustStatus"],"excluded");assert_ne!(item(&c,2,"one.example.com")["trustStatus"],"excluded");
 assert!(hunter_expansion::evidence_anchor(&c,1,"one.example.com","domain").unwrap_err().contains("精确排除"));
 assert!(!seed_policy::blocked(&c,1,"domain","child.one.example.com").unwrap());
 let restore=serde_json::from_value(json!({"projectId":1,"action":"restore","batchId":result["batchId"],"operationId":uuid::Uuid::new_v4().to_string()})).unwrap();seed_policy::manage(&mut c,restore).unwrap();
 assert_ne!(item(&c,1,"one.example.com")["trustStatus"],"excluded");assert_eq!(before,c.query_row("SELECT count(*) FROM asset_discovery_seeds",[],|r|r.get::<_,i64>(0)).unwrap());
}
#[test]
fn full_filter_exclusion_crosses_pages_and_rejects_cross_project_ids(){
 let mut c=fixture();for i in 0..61{insert(&c,1,&format!("n{i}.example.com"),"observed_host","")}insert(&c,2,"outside.test","observed_host","");baseline(&c,1);baseline(&c,2);
 let selected_query=query(1);let p=serde_json::from_value(json!({"projectId":1,"action":"preview","query":selected_query})).unwrap();let preview=seed_policy::manage(&mut c,p).unwrap();assert_eq!(preview["count"],61);assert!(preview.get("identities").is_none());assert_eq!(preview["samples"].as_array().unwrap().len(),8);
 let input=serde_json::from_value(json!({"projectId":1,"action":"apply","query":selected_query,"expectedDigest":preview["digest"],"operationId":uuid::Uuid::new_v4().to_string()})).unwrap();seed_policy::manage(&mut c,input).unwrap();assert_eq!(read(&c,&selected_query).unwrap()["summary"]["excluded"],61);
 let other=item(&c,2,"outside.test")["id"].as_i64().unwrap();let bad=serde_json::from_value(json!({"projectId":1,"action":"preview","seedIds":[other]})).unwrap();assert!(seed_policy::manage(&mut c,bad).unwrap_err().contains("project_mismatch"));
}
#[test]
fn confirmed_domain_reuses_human_scope_without_fabricating_independent_ownership(){
 let c=fixture();insert(&c,1,"cmsrict.com","observed_host","");baseline(&c,1);
 c.execute("INSERT INTO asset_discovery_scopes(project_id,roots_json) VALUES(1,'[\"cmsrict.com\"]')",[]).unwrap();
 let row=item(&c,1,"cmsrict.com");assert_eq!(row["trustStatus"],"accepted");assert!(row["trustReason"].as_str().unwrap().contains("人工"));assert_eq!(row["verificationCoverage"]["verifiedEvidence"],0);
 assert_eq!(read(&c,&query(1)).unwrap()["summary"]["accepted"],1);
}
#[test]
fn explicit_legacy_human_decisions_are_shared_without_suffix_expansion(){
 let c=fixture();insert(&c,1,"reject.test","observed_host","");insert(&c,1,"child.reject.test","observed_host","");insert(&c,1,"accept.test","observed_host","");baseline(&c,1);
 c.execute_batch("CREATE TABLE asset_seed_reviews(id TEXT PRIMARY KEY,project_id INTEGER);CREATE TABLE asset_seed_review_items(run_id TEXT,evidence_json TEXT,decision TEXT,kind TEXT,target TEXT);INSERT INTO asset_seed_reviews VALUES('r',1);INSERT INTO asset_seed_review_items VALUES('r','{\"domains\":[\"reject.test\"]}','rejected','domain','reject.test'),('r','{\"domains\":[\"accept.test\"]}','accepted','domain','accept.test');").unwrap();
 assert_eq!(item(&c,1,"reject.test")["trustStatus"],"excluded");assert_ne!(item(&c,1,"child.reject.test")["trustStatus"],"excluded");assert_eq!(item(&c,1,"accept.test")["trustStatus"],"accepted");assert!(seed_policy::blocked(&c,1,"domain","reject.test").unwrap());assert!(!seed_policy::blocked(&c,2,"domain","reject.test").unwrap());
}
