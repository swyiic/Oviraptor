use super::*;

fn group(c: &Connection, k: &work::Claim, key: &str) {
    c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,?2,?2,'evidence')",params![k.id,key]).unwrap();
}

#[test]
fn cleanup_shared_tenant_is_not_excluded_or_left_in_unproductive_public_loop() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "tenant.eos-beijing-1.cmecloud.cn", "");
    tests::prepared(&mut c, &k);
    let key: String = c
        .query_row("SELECT group_key FROM asset_cleanup_groups", [], |r| {
            r.get(0)
        })
        .unwrap();
    work::reclassify(&mut c, &k, &key).unwrap();
    let (state, text): (String, String) = c
        .query_row(
            "SELECT state,evidence_json FROM asset_cleanup_groups",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "deferred");
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["completionReason"],
        "shared_tenant"
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_research", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_discovery_seeds", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
}

#[test]
fn cleanup_finite_source_failure_becomes_explicit_gap_not_conflict_or_exclusion() {
    let (mut c, k) = tests::fixture();
    group(&c, &k, "unreachable.test");
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,attempts,error,result_json) VALUES(?1,'unreachable.test','public_ownership','unreachable.test','exhausted',2,'HTTP 502 prior attempts','{\"prior\":true}')",[&k.id]).unwrap();
    assert!(work::settle_completed_groups(&mut c, &k).unwrap());
    let (state,reason):(String,String)=c.query_row("SELECT state,json_extract(evidence_json,'$.completionReason') FROM asset_cleanup_groups",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        (state.as_str(), reason.as_str()),
        ("deferred", "source_unavailable")
    );
    assert_eq!(
        c.query_row(
            "SELECT error||result_json FROM asset_cleanup_research",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "HTTP 502 prior attempts{\"prior\":true}"
    );
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
    assert!(
        !work::settle_completed_groups(&mut c, &k).unwrap(),
        "settled groups must not busy-loop"
    );
}

#[test]
fn cleanup_future_or_inflight_work_cannot_be_reported_as_completed() {
    let (mut c, k) = tests::fixture();
    for (key, state) in [("future.test", "retry"), ("running.test", "running")] {
        group(&c, &k, key);
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,next_at) VALUES(?1,?2,'dns_cname',?2,?3,?4)",params![k.id,key,state,now()+3600]).unwrap();
    }
    assert!(!work::settle_completed_groups(&mut c, &k).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_groups WHERE state='evidence'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn cleanup_unknown_paid_result_is_preserved_with_its_own_gap_reason() {
    let (mut c, k) = tests::fixture();
    group(&c, &k, "unknown.test");
    c.execute("INSERT INTO asset_cleanup_models(id,run_id,recipient,request_json,state,response_text,usage_json,error,created_at) VALUES('paid',?1,'private','{\"neverReplay\":true}','unknown','','{}','connection lost',?2)",params![k.id,now()]).unwrap();
    c.execute(
        "INSERT INTO asset_cleanup_model_items VALUES('paid',?1,'unknown.test','old-basis')",
        [&k.id],
    )
    .unwrap();
    assert!(work::settle_completed_groups(&mut c, &k).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT json_extract(evidence_json,'$.completionReason') FROM asset_cleanup_groups",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "model_result_unknown"
    );
    assert_eq!(
        c.query_row(
            "SELECT state||request_json||error FROM asset_cleanup_models",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown{\"neverReplay\":true}connection lost"
    );
}

#[test]
fn cleanup_completed_round_with_gaps_releases_claim_and_stops_instead_of_repeating() {
    let (mut c, k) = tests::fixture();
    c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state,evidence_json) VALUES(?1,'gap','gap.test','deferred','{\"completionReason\":\"independent_identity_missing\"}')",[&k.id]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state) VALUES(?1,'__organization','organization','Target Group','done')",[&k.id]).unwrap();
    assert!(work::scheduler_step(Path::new("unused-no-network"), &mut c, &k).unwrap());
    assert!(work::finish_step(&c, &k, Ok(())).unwrap());
    let row: (String, bool, String, i64) = c
        .query_row(
            "SELECT phase,enabled,claim_token,owner_pid FROM asset_cleanup_runs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(row, ("completed_with_gaps".into(), false, "".into(), 0));
    assert!(work::claim(&mut c).unwrap().is_none());
}

#[test]
fn cleanup_finite_gap_status_is_read_only_and_counts_groups_separately() {
    let (c, k) = tests::fixture();
    for key in ["a", "b"] {
        c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state,evidence_json) VALUES(?1,?2,?2,'deferred','{\"completionReason\":\"shared_tenant\"}')",params![k.id,key]).unwrap();
    }
    let before = c.total_changes();
    let value = status_on(
        &c,
        1,
        &Read {
            run_id: Some(k.id.clone()),
            filter: Some("deferred".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(value["total"], 2);
    assert_eq!(value["remainingReasons"]["shared_tenant"], 2);
    assert_eq!(value["counts"]["needs_evidence"], 0);
    assert_eq!(c.total_changes(), before);
}

#[test]
fn cleanup_finite_round_waits_for_remaining_organization_attempt_before_completion() {
    let (mut c, k) = tests::fixture();
    c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,'gap','gap.test','deferred')",[&k.id]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,attempts,next_at) VALUES(?1,'__organization','organization','Target Group','retry',1,?2)",params![k.id,now()+3600]).unwrap();
    assert!(work::scheduler_step(Path::new("unused-no-network"), &mut c, &k).unwrap());
    assert_eq!(
        c.query_row("SELECT enabled FROM asset_cleanup_runs", [], |r| r
            .get::<_, bool>(0))
            .unwrap(),
        true
    );
    assert_eq!(
        c.query_row("SELECT phase FROM asset_cleanup_runs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "waiting_evidence"
    );
    assert!(!work::scheduler_step(Path::new("unused-no-network"), &mut c, &k).unwrap());
}

#[test]
#[ignore = "explicit local replay on a disposable real-project snapshot; no network or model"]
fn cleanup_real_finite_policy_preserves_original_data_and_paid_receipts() {
    let path =
        std::env::var("OVI_FINITE_REPLAY_DB").expect("disposable finite-replay database required");
    let canonical = std::fs::canonicalize(&path).unwrap();
    assert!(canonical.starts_with("/private/tmp") && path.contains("finite-replay"));
    let run = std::env::var("OVI_FINITE_REPLAY_RUN").unwrap();
    let mut c =
        Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    fn fingerprint(c: &Connection, table: &str) -> String {
        let mut q = c
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = q.column_count();
        let mut rows = q.query([]).unwrap();
        let mut digest = Sha256::new();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..columns {
                digest.update(format!("{:?}", row.get_ref(i).unwrap()).as_bytes());
                digest.update([0]);
            }
        }
        format!("{:x}", digest.finalize())
    }
    let tables = [
        "assets",
        "project_assets",
        "asset_discovery_seeds",
        "asset_project_seeds",
        "asset_cleanup_seeds",
        "asset_cleanup_models",
    ];
    let original = tables
        .iter()
        .map(|t| fingerprint(&c, t))
        .collect::<Vec<_>>();
    let project: i64 = c
        .query_row(
            "SELECT project_id FROM asset_cleanup_runs WHERE id=?1",
            [&run],
            |r| r.get(0),
        )
        .unwrap();
    let roots_before = roots(&c, project).unwrap();
    c.execute("UPDATE asset_cleanup_runs SET enabled=0,claim_token='',owner_pid=0,lease_until=0 WHERE id!=?1",[&run]).unwrap();
    c.execute("UPDATE asset_cleanup_runs SET enabled=1,claim_token='',owner_pid=0,lease_until=0 WHERE id=?1",[&run]).unwrap();
    work::ensure_runtime_indexes(&c).unwrap();
    let k = work::claim(&mut c).unwrap().unwrap();
    assert_eq!(k.id, run);
    let started = std::time::Instant::now();
    let keys: Vec<String> = c
        .prepare("SELECT group_key FROM asset_cleanup_groups WHERE run_id=?1 AND state='pending'")
        .unwrap()
        .query_map([&run], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for key in &keys {
        work::reclassify(&mut c, &k, key).unwrap();
    }
    network::settle_exhausted(&mut c, &k).unwrap();
    let deferred:i64=c.query_row("SELECT count(*) FROM asset_cleanup_groups WHERE run_id=?1 AND state='deferred' AND json_extract(evidence_json,'$.completionReason')='shared_tenant'",[&run],|r|r.get(0)).unwrap();
    assert!(
        deferred > 5000,
        "real shared tenants must leave the unproductive evidence loop"
    );
    assert_eq!(roots(&c, project).unwrap(), roots_before);
    for (table, before) in tables.iter().zip(original) {
        assert_eq!(
            fingerprint(&c, table),
            before,
            "protected original table changed: {table}"
        );
    }
    println!(
        "{}",
        json!({"projectId":project,"runId":run,"localGroupsReviewed":keys.len(),"sharedTenantsWithExplicitGap":deferred,"elapsedMs":started.elapsed().as_millis(),"protectedTablesPreserved":tables,"networkRequests":0,"modelRequests":0})
    );
}
