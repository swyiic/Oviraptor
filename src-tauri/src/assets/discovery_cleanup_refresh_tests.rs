use super::*;

fn deferred(c: &mut Connection, k: &work::Claim, key: &str) {
    work::reclassify(c, k, key).unwrap();
    c.execute("UPDATE asset_cleanup_groups SET state='deferred',evidence_json=json_set(evidence_json,'$.completionReason','independent_identity_missing') WHERE run_id=?1 AND group_key=?2",params![k.id,key]).unwrap();
}
fn state(c: &Connection, k: &work::Claim, key: &str) -> String {
    c.query_row(
        "SELECT state FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
        params![k.id, key],
        |r| r.get(0),
    )
    .unwrap()
}
fn drain(c: &mut Connection, k: &work::Claim) {
    for _ in 0..1000 {
        if !refresh::tick(c, k).unwrap() {
            return;
        }
    }
    panic!("explicit local refresh did not terminate")
}
fn snapshot(
    c: &Connection,
    k: &work::Claim,
) -> Vec<(String, i64, String, String, String, bool, String)> {
    c.prepare("SELECT source_kind,source_id,host,source,source_ref,wildcard,group_key FROM asset_cleanup_seeds WHERE run_id=?1 ORDER BY source_kind,source_id").unwrap().query_map([&k.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap().collect::<Result<_,_>>().unwrap()
}
fn model(c: &Connection, k: &work::Claim, key: &str) {
    c.execute(r#"INSERT INTO asset_cleanup_models(id,run_id,recipient,request_json,state,response_text,usage_json,error,created_at) VALUES('old-paid',?1,'private','{"neverReplay":true}','applied','accepted original receipt','{"inputTokens":17}','','1')"#,[&k.id]).unwrap();
    c.execute("INSERT INTO asset_cleanup_model_items SELECT 'old-paid',run_id,group_key,basis_hash FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",params![k.id,key]).unwrap();
}

#[test]
fn cleanup_explicit_refresh_no_new_identity_keeps_deferred_paid_receipts_and_source_budget() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "a.example.test", "Other Limited");
    tests::prepared(&mut c, &k);
    let key = "domain:example.test";
    deferred(&mut c, &k, key);
    model(&c, &k, key);
    c.execute(r#"UPDATE asset_cleanup_research SET state='exhausted',attempts=2,error='original HTTP 403',result_json='{"oldReceipt":true}' WHERE run_id=?1"#,[&k.id]).unwrap();
    let before = snapshot(&c, &k);
    let changes = c.total_changes();
    assert!(!refresh::tick(&mut c, &k).unwrap());
    assert_eq!(
        c.total_changes(),
        changes,
        "no explicit request means no scan or DDL"
    );
    refresh::request_on_resume(&c, k.p, &k.id).unwrap();
    drain(&mut c, &k);
    assert_eq!(state(&c, &k, key), "deferred");
    assert_eq!(snapshot(&c, &k), before);
    assert_eq!(c.query_row("SELECT state||request_json||response_text||usage_json FROM asset_cleanup_models WHERE id='old-paid'",[],|r|r.get::<_,String>(0)).unwrap(),"applied{\"neverReplay\":true}accepted original receipt{\"inputTokens\":17}");
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_model_items", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_fact_refresh_history",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(c.query_row(r#"SELECT count(*) FROM asset_cleanup_research WHERE state!='exhausted' OR attempts!=2 OR error!='original HTTP 403' OR result_json!='{"oldReceipt":true}'"#,[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn cleanup_explicit_refresh_replaces_stale_cache_and_wakes_new_bound_identity_only() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "a.example.test", "Other Limited");
    tests::prepared(&mut c, &k);
    let key = "domain:example.test";
    deferred(&mut c, &k, key);
    model(&c, &k, key);
    let before = snapshot(&c, &k);
    c.execute(
        r#"UPDATE assets SET extra_json='{"icp.name":"Target"}' WHERE id=1"#,
        [],
    )
    .unwrap();
    refresh::request_on_resume(&c, k.p, &k.id).unwrap();
    drain(&mut c, &k);
    assert_eq!(state(&c, &k, key), "pending");
    let e = work::evidence_for(&c, &k, key).unwrap();
    assert_eq!(e["domainEvidence"][0]["organization"], "Target");
    assert_eq!(e["staleSnapshot"], false);
    assert_eq!(snapshot(&c, &k), before);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_fact_refresh_history",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(c.query_row("SELECT observations_json LIKE '%Other Limited%' FROM asset_cleanup_fact_refresh_history",[],|r|r.get::<_,bool>(0)).unwrap(),true);
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "retained");
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_models", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
}

#[test]
fn cleanup_explicit_refresh_unlinked_asset_cannot_keep_old_subject_or_wake_from_removed_evidence() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "a.example.test", "Other Limited");
    tests::prepared(&mut c, &k);
    let key = "domain:example.test";
    deferred(&mut c, &k, key);
    c.execute(
        "DELETE FROM project_assets WHERE project_id=1 AND asset_id=1",
        [],
    )
    .unwrap();
    refresh::request_on_resume(&c, 1, &k.id).unwrap();
    drain(&mut c, &k);
    assert_eq!(state(&c, &k, key), "deferred");
    assert_eq!(
        work::evidence_for(&c, &k, key).unwrap()["domainEvidence"],
        json!([])
    );
    assert_eq!(
        c.query_row(
            "SELECT observations_json FROM asset_cleanup_facts",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "[]"
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_fact_refresh_history",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn cleanup_explicit_refresh_timestamp_and_duplicate_source_are_not_new_material() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "a.example.test", "Other Limited");
    tests::prepared(&mut c, &k);
    let key = "domain:example.test";
    let fact = json!({"host":"a.example.test","organization":"Other Limited","source":"public_ct_subject","notAfter":now()+86400,"checkedAt":now()-100,"sourceUrl":"https://crt.sh/?d=1"});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate','1','done',?3)",params![k.id,key,json!({"facts":[fact.clone()]}).to_string()]).unwrap();
    deferred(&mut c, &k, key);
    model(&c, &k, key);
    let mut newer = fact;
    newer["checkedAt"] = json!(now());
    newer["bodySha256"] = json!("b".repeat(64));
    newer["sourceUrl"] = json!("https://crt.sh/?d=2");
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate','2','done',?3)",params![k.id,key,json!({"facts":[newer]}).to_string()]).unwrap();
    refresh::request_on_resume(&c, 1, &k.id).unwrap();
    drain(&mut c, &k);
    assert_eq!(state(&c, &k, key), "deferred");
    assert_eq!(work::pending_model_groups(&c, &k.id).unwrap(), 0);
}

#[test]
fn cleanup_explicit_refresh_new_public_identity_wakes_without_network_or_source_reset() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "a.example.test", "");
    tests::prepared(&mut c, &k);
    let key = "domain:example.test";
    deferred(&mut c, &k, key);
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate','42','done',?3)",params![k.id,key,json!({"facts":[{"host":"a.example.test","organization":"Target","source":"public_ct_subject","notAfter":now()+86400}]}).to_string()]).unwrap();
    refresh::request_on_resume(&c, 1, &k.id).unwrap();
    drain(&mut c, &k);
    assert_eq!(state(&c, &k, key), "pending");
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "retained");
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_models", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn cleanup_explicit_refresh_leaves_protected_states_and_old_shared_subject_untouched() {
    let (mut c, k) = tests::fixture();
    for (id, domain, final_state) in [
        (1, "a.example.test", "excluded"),
        (2, "a.other.test", "restored"),
        (3, "a.third.test", "held"),
        (4, "tenant.eos-beijing-1.cmecloud.cn", "deferred"),
    ] {
        tests::add(&c, &k, id, domain, "Other Limited");
        tests::prepared(&mut c, &k);
        let key: String = c
            .query_row(
                "SELECT group_key FROM asset_cleanup_links WHERE run_id=?1 AND asset_id=?2",
                params![k.id, id],
                |r| r.get(0),
            )
            .unwrap();
        deferred(&mut c, &k, &key);
        c.execute(
            "UPDATE asset_cleanup_groups SET state=?3 WHERE run_id=?1 AND group_key=?2",
            params![k.id, key, final_state],
        )
        .unwrap();
    }
    refresh::request_on_resume(&c, 1, &k.id).unwrap();
    drain(&mut c, &k);
    assert_eq!(state(&c, &k, "domain:example.test"), "excluded");
    assert_eq!(state(&c, &k, "domain:other.test"), "restored");
    assert_eq!(state(&c, &k, "domain:third.test"), "held");
    assert_eq!(
        state(&c, &k, "domain:tenant.eos-beijing-1.cmecloud.cn"),
        "deferred"
    );
    assert_eq!(
        c.query_row(
            "SELECT awakened_groups FROM asset_cleanup_local_refresh",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn cleanup_explicit_refresh_large_group_is_bounded_and_continues_its_asset_cursor() {
    let (mut c, k) = tests::fixture();
    for id in 1..=205 {
        tests::add(&c, &k, id, &format!("n{id}.example.test"), "Other Limited")
    }
    tests::prepared(&mut c, &k);
    let key = "domain:example.test";
    deferred(&mut c, &k, key);
    c.execute(
        r#"UPDATE assets SET extra_json='{"icp.name":"Target"}'"#,
        [],
    )
    .unwrap();
    refresh::request_on_resume(&c, 1, &k.id).unwrap();
    assert!(refresh::tick(&mut c, &k).unwrap());
    let first: (i64, i64) = c
        .query_row(
            "SELECT asset_cursor,checked_groups FROM asset_cleanup_local_refresh",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(first, (100, 0));
    assert_eq!(state(&c, &k, key), "deferred");
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_fact_refresh_history",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        100
    );
    drain(&mut c, &k);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_fact_refresh_history",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        205
    );
    assert_eq!(state(&c, &k, key), "pending");
}

#[test]
fn cleanup_explicit_refresh_generation_project_and_unit_directory_fences() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "a.example.test", "Other Limited");
    tests::prepared(&mut c, &k);
    let key = "domain:example.test";
    deferred(&mut c, &k, key);
    assert!(refresh::request_on_resume(&c, 2, &k.id).is_err());
    assert!(!has(&c, "asset_cleanup_local_refresh").unwrap());
    assert!(!refresh::request_targets_changed(
        &c,
        1,
        &k.id,
        "[\"Target\",\"Other Limited\"]",
        "[\"Other Limited\",\"Target\"]"
    )
    .unwrap());
    assert!(refresh::request_targets_changed(
        &c,
        1,
        &k.id,
        "[\"Target\"]",
        "[\"Target\",\"Other Limited\"]"
    )
    .unwrap());
    drain(&mut c, &k);
    assert_eq!(state(&c, &k, key), "pending");
    c.execute("UPDATE asset_cleanup_groups SET state='deferred'", [])
        .unwrap();
    refresh::request_on_resume(&c, 1, &k.id).unwrap();
    c.execute("UPDATE asset_cleanup_runs SET generation=generation+1", [])
        .unwrap();
    let changes = c.total_changes();
    assert!(!refresh::tick(&mut c, &k).unwrap());
    assert_eq!(c.total_changes(), changes);
}
