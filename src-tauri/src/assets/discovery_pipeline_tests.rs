use super::*;
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> (Temporary, Connection, Manager) {
    let dir = Temporary(
        std::env::temp_dir().join(format!("ovi-discovery-pipeline-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = db::initialize(&dir.0).unwrap();
    let c = db::open(&path).unwrap();
    c.execute("INSERT OR IGNORE INTO projects(id,name,status) VALUES(91,'Target','active'),(92,'Other','active')",[]).unwrap();
    crate::asset_discovery::ensure(&c).unwrap();
    let free = enrichment::Manager::new(&path).unwrap();
    let manager = Manager::new(&path, free).unwrap();
    manager.shutdown();
    (dir, c, manager)
}
fn input(extra: Value) -> Input {
    let mut value = json!({"projectId":91,"action":"start","organization":"中国移动集团及其下属单位","includeIsolated":true});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}
fn op(c: &Connection, p: i64) -> String {
    c.query_row(
        "SELECT operation_id FROM asset_discovery_pipelines WHERE project_id=?1",
        [p],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn first_collection_canonicalizes_company_and_double_click_but_new_intent_is_not_lost() {
    let (_dir, c, _m) = fixture();
    let first = input(json!({"collectFirst":true}));
    start_on(&c, &first, first.organization.as_deref().unwrap()).unwrap();
    let original = op(&c, 91);
    assert_eq!(
        c.query_row(
            "SELECT targets_json FROM asset_discovery_pipelines WHERE project_id=91",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "[\"中国移动\"]"
    );
    start_on(&c, &first, first.organization.as_deref().unwrap()).unwrap();
    assert_eq!(op(&c, 91), original);
    let changed = input(json!({"collectFirst":true,"collectionTargets":["10086.cn"]}));
    start_on(&c, &changed, changed.organization.as_deref().unwrap()).unwrap();
    assert_ne!(op(&c, 91), original);
    let second = op(&c, 91);
    phase(&c, 91, "steady", "").unwrap();
    start_on(&c, &changed, changed.organization.as_deref().unwrap()).unwrap();
    assert_ne!(op(&c, 91), second);
}
#[test]
fn enabling_hunter_after_free_settlement_reopens_work_without_replaying_collection() {
    let (_dir,c,_m)=fixture();
    let first=input(json!({"allowHunter":false}));
    start_on(&c,&first,first.organization.as_deref().unwrap()).unwrap();
    let original=op(&c,91);
    crate::asset_discovery::rounds::complete(&c,91,"free settled").unwrap();
    c.execute("UPDATE asset_discovery_pipelines SET phase='steady',last_signature='unchanged-free-evidence' WHERE project_id=91",[]).unwrap();
    let paid=input(json!({"allowHunter":true}));
    start_on(&c,&paid,paid.organization.as_deref().unwrap()).unwrap();
    assert_eq!(op(&c,91),original);
    let state:(bool,String,String,Option<i64>)=c.query_row("SELECT allow_hunter,phase,last_signature,collection_run_id FROM asset_discovery_pipelines WHERE project_id=91",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(state,(true,"local_analysis".into(),"".into(),None));
    assert_eq!(crate::asset_discovery::rounds::new_evidence(&c,91,"paid wave ready").unwrap(),2);
    assert_eq!(crate::asset_discovery::rounds::new_evidence(&c,91,"same wave").unwrap(),2);
}
#[test]
fn explicit_continue_keeps_original_collection_and_paid_flag_without_new_intent() {
    let (_dir, c, _m) = fixture();
    let initial = input(json!({"collectFirst":true,"allowHunter":true}));
    start_on(&c, &initial, initial.organization.as_deref().unwrap()).unwrap();
    let original = op(&c, 91);
    c.execute(
        "UPDATE asset_discovery_pipelines SET phase='paused',enabled=0 WHERE project_id=91",
        [],
    )
    .unwrap();
    let resume = input(json!({}));
    start_on(&c, &resume, resume.organization.as_deref().unwrap()).unwrap();
    assert_eq!(op(&c, 91), original);
    assert_eq!(status(&c, 91).unwrap()["allowHunter"], true);
    let cancel_paid = input(json!({"allowHunter":false}));
    start_on(
        &c,
        &cancel_paid,
        cancel_paid.organization.as_deref().unwrap(),
    )
    .unwrap();
    assert_eq!(status(&c, 91).unwrap()["allowHunter"], false);
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM runs WHERE project_id=91", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}
#[test]
fn interrupted_collection_is_held_without_background_replay_and_one_project_cannot_starve_another()
{
    let (_dir, mut c, m) = fixture();
    let initial = input(json!({"collectFirst":true}));
    start_on(&c, &initial, initial.organization.as_deref().unwrap()).unwrap();
    c.execute("INSERT INTO runs(id,project_id,name,pipeline,status,error) VALUES(701,91,'original','collect','interrupted','network unknown')",[]).unwrap();
    c.execute(
        "UPDATE asset_discovery_pipelines SET collection_run_id=701 WHERE project_id=91",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO asset_seed_assists(project_id,organization,include_isolated,phase,enabled) VALUES(91,'中国移动集团及其下属单位',1,'collecting',1)",[]).unwrap();
    assert!(
        m.start_collection(&c, 91, false).is_err(),
        "a saved integer alone cannot adopt an original native job"
    );
    m.continue_pipelines(&mut c).unwrap();
    assert_eq!(status(&c, 91).unwrap()["phase"], "blocked");
    assert_eq!(
        c.query_row("SELECT status FROM runs WHERE id=701", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "interrupted"
    );
    // A stale cross-project reference is rejected locally, while another
    // project's coordinator still receives its turn.
    c.execute("INSERT INTO runs(id,project_id,name,pipeline,status) VALUES(702,92,'other','collect','completed')",[]).unwrap();
    c.execute(
        "UPDATE asset_discovery_pipelines SET collection_run_id=702 WHERE project_id=91",
        [],
    )
    .unwrap();
    let mut other = input(json!({}));
    other.project_id = 92;
    start_on(&c, &other, other.organization.as_deref().unwrap()).unwrap();
    c.execute("INSERT INTO asset_seed_assists(project_id,organization,include_isolated,phase,enabled) VALUES(92,'中国移动集团及其下属单位',1,'collecting',1)",[]).unwrap();
    m.continue_pipelines(&mut c).unwrap();
    assert_eq!(status(&c, 91).unwrap()["phase"], "blocked");
    assert_eq!(status(&c, 92).unwrap()["phase"], "steady");
}
#[test]
fn hunter_receipts_show_incremental_cost_and_ambiguous_page_without_replay() {
    let (_dir, c, m) = fixture();
    let start = input(json!({"allowHunter":true}));
    start_on(&c, &start, start.organization.as_deref().unwrap()).unwrap();
    let operation = op(&c, 91);
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(91,'[\"example.com\"]')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO asset_discovery_settings VALUES(1,1,'synthetic-never-sent',3,10)",
        [],
    )
    .unwrap();
    let i = hunter_expansion::ExpansionInput {
        project_id: 91,
        kind: "domain_tree".into(),
        target: "example.com".into(),
    };
    let query = hunter_expansion::prepare_inline(&m.path, &i).unwrap();
    c.execute("INSERT INTO asset_discovery_pipeline_queries(operation_id,project_id,kind,target,query_id,baseline_spent,baseline_pages,baseline_unique) VALUES(?1,91,'domain_tree','example.com',?2,5,1,2)",params![operation,query]).unwrap();
    c.execute(
        "UPDATE hunter_expansions SET spent=12,unique_rows=8,status='unknown' WHERE id=?1",
        [&query],
    )
    .unwrap();
    c.execute("INSERT INTO hunter_expansion_pages(query_id,page,state,request_id,accounted) VALUES(?1,1,'imported','known',1),(?1,2,'sending','unknown',0)",[&query]).unwrap();
    let before = status(&c, 91).unwrap();
    assert_eq!(before["hunter"]["spentPoints"], 7);
    assert_eq!(before["hunter"]["pages"], 1);
    assert_eq!(before["hunter"]["costUnknown"], true);
    assert_eq!(before["counts"]["hunterAdded"], 6);
    assert_eq!(
        hunter_expansion::status_on(&c, 91, &query).unwrap()["status"],
        "unknown"
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM hunter_expansion_pages WHERE query_id=?1",
            [&query],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn hunter_wave_omits_excluded_only_identities_but_keeps_independent_shared_support() {
    let (_dir,mut c,_m)=fixture();
    let start=input(json!({"allowHunter":true}));
    start_on(&c,&start,start.organization.as_deref().unwrap()).unwrap();
    c.execute("INSERT INTO asset_discovery_scopes VALUES(91,'[\"example.com\"]')",[]).unwrap();
    for (id,endpoint,ch) in [(7101,"noise.other.net",'a'),(7102,"noise.other.net",'b'),(7103,"good.example.com",'b')] {
        let extra=json!({"cert_sha256":ch.to_string().repeat(64),"cert.domain":"app.example.com"});
        let link=format!("https://{endpoint}");
        c.execute("INSERT INTO assets(id,asset_key,host,link,extra_json) VALUES(?1,?2,?3,?4,?5)",params![id,format!("pipeline-identity-{id}"),endpoint,link,extra.to_string()]).unwrap();
        c.execute("INSERT INTO project_assets(project_id,asset_id) VALUES(91,?1)",[id]).unwrap();
        crate::asset_discovery::ingest_observations(&c,91,&[(id,endpoint.into(),link,extra.to_string())]).unwrap();
    }
    let before=candidates(&c,91,&op(&c,91)).unwrap();
    assert!(before.iter().any(|q|q.kind=="fingerprint" && q.target=="a".repeat(64)));
    let preview=exclusions::manage(&mut c,exclusions::ExclusionInput{project_id:91,domain:"noise.other.net".into(),action:"preview".into(),expected_digest:None}).unwrap();
    exclusions::manage(&mut c,exclusions::ExclusionInput{project_id:91,domain:"noise.other.net".into(),action:"apply".into(),expected_digest:preview["digest"].as_str().map(str::to_owned)}).unwrap();
    let clean=candidates(&c,91,&op(&c,91)).unwrap();
    assert!(!clean.iter().any(|q|q.kind=="fingerprint" && q.target=="a".repeat(64)));
    assert!(clean.iter().any(|q|q.kind=="fingerprint" && q.target=="b".repeat(64)));
    assert_eq!(c.query_row("SELECT COUNT(*) FROM assets WHERE id IN (7101,7102,7103)",[],|r|r.get::<_,i64>(0)).unwrap(),3);
}
#[test]
fn pause_never_claims_or_stops_an_unrelated_manual_free_owner() {
    let (_dir, c, m) = fixture();
    let start = input(json!({}));
    start_on(&c, &start, start.organization.as_deref().unwrap()).unwrap();
    c.execute("INSERT INTO asset_enrichment_runs(id,project_id,operation_id,roots_json,status,enabled,continuous,interval_seconds,workers,created_at,updated_at) VALUES('manual',91,'operator-click','[]','waiting',1,1,3600,4,1,1)",[]).unwrap();
    enrichment(&c, 91, &json!({"run":{"id":"manual"}})).unwrap();
    assert!(status(&c, 91).unwrap()["enrichmentRunId"].is_null());
    m.pause_pipeline(&c, 91).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT enabled FROM asset_enrichment_runs WHERE id='manual'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap(),
        true
    );
}
