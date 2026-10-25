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

#[test]
fn cleanup_manual_guard_large_review_uses_partial_index_preserves_decisions_and_is_idempotent() {
    let (mut c, k) = tests::fixture();
    c.execute_batch("CREATE TABLE asset_seed_reviews(id TEXT PRIMARY KEY,project_id INTEGER NOT NULL,organization TEXT NOT NULL);CREATE INDEX asset_seed_reviews_project ON asset_seed_reviews(project_id);CREATE TABLE asset_seed_review_items(run_id TEXT NOT NULL,item_key TEXT NOT NULL,state TEXT NOT NULL,decision TEXT NOT NULL,evidence_json TEXT NOT NULL,PRIMARY KEY(run_id,item_key));CREATE INDEX asset_seed_review_queue ON asset_seed_review_items(run_id,state,item_key);").unwrap();
    c.execute_batch("INSERT INTO asset_seed_reviews VALUES('large-review',1,'Target Group'),('other-project-review',2,'Target Group'),('other-organization-review',1,'Other Group');").unwrap();
    let tx = c.transaction().unwrap();
    for n in 0..12_500 {
        tx.execute("INSERT INTO asset_seed_review_items VALUES('large-review',?1,'pending','pending','{\"domains\":[\"www.unrelated.test\"]}')",[format!("pending-{n:05}")]).unwrap();
    }
    tx.execute_batch("INSERT INTO asset_seed_review_items VALUES('large-review','manual-accepted','done','accepted','{\"domains\":[\"www.protected.test\"]}'),('large-review','manual-rejected','done','rejected','{\"domains\":[\"www.rejected.test\"]}'),('other-project-review','foreign-project','done','accepted','{\"domains\":[\"www.unrelated.test\"]}'),('other-organization-review','foreign-organization','done','accepted','{\"domains\":[\"www.unrelated.test\"]}');").unwrap();
    tx.commit().unwrap();
    let manual_before:Vec<(String,String,String,String)>=c.prepare("SELECT run_id,item_key,decision,evidence_json FROM asset_seed_review_items WHERE decision IN ('accepted','rejected') ORDER BY run_id,item_key").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<_,_>>().unwrap();
    for (id, name) in [
        (1, "www.protected.test"),
        (2, "www.unrelated.test"),
        (3, "www.rejected.test"),
    ] {
        tests::add(&c, &k, id, name, "Other Owner");
    }
    tests::prepared(&mut c, &k);
    let manual_query="SELECT EXISTS(SELECT 1 FROM asset_seed_review_items i JOIN asset_seed_reviews r ON r.id=i.run_id JOIN asset_cleanup_runs current ON current.id=?1 AND current.project_id=r.project_id AND current.organization=r.organization JOIN json_each(i.evidence_json,'$.domains') n WHERE i.decision IN ('accepted','rejected') AND (n.value=?2 OR n.value LIKE '%.'||?2))";
    let plan_before: Vec<String> = c
        .prepare(&format!("EXPLAIN QUERY PLAN {manual_query}"))
        .unwrap()
        .query_map(params![k.id, "no-match.invalid"], |r| r.get(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!plan_before
        .iter()
        .any(|p| p.contains("asset_cleanup_manual_decisions")));
    work::ensure_runtime_indexes(&c).unwrap();
    let plan_after: Vec<String> = c
        .prepare(&format!("EXPLAIN QUERY PLAN {manual_query}"))
        .unwrap()
        .query_map(params![k.id, "no-match.invalid"], |r| r.get(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        plan_after
            .iter()
            .any(|p| p.contains("asset_cleanup_manual_decisions")
                && p.contains("run_id=?")
                && p.contains("decision=?")),
        "manual guard must filter actual decisions before reading evidence JSON: {plan_after:?}"
    );
    let sql: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='asset_cleanup_manual_decisions'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sql.contains("WHERE decision IN ('accepted','rejected')"));
    let schema_version: i64 = c
        .query_row("PRAGMA schema_version", [], |r| r.get(0))
        .unwrap();
    let changes: i64 = c
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    work::ensure_runtime_indexes(&c).unwrap();
    assert_eq!(
        c.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        schema_version
    );
    assert_eq!(
        c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        changes
    );
    for (id, name, domain, expected) in [
        (1, "www.protected.test", "protected.test", "held"),
        (2, "www.unrelated.test", "unrelated.test", "excluded"),
        (3, "www.rejected.test", "rejected.test", "held"),
    ] {
        let key = format!("domain:{domain}");
        let certificate = json!({"kind":"ct_certificate","facts":[{"host":name,"organization":"Other Owner","source":"public_ct_subject","certificateSha256":"a".repeat(64),"sourceUrl":format!("https://crt.sh/?d={id}"),"notAfter":now()+86400}],"sourceUrl":format!("https://crt.sh/?d={id}"),"bodySha256":"a".repeat(64),"checkedAt":now()});
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate',?3,'done',?4)",params![k.id,key,id.to_string(),certificate.to_string()]).unwrap();
        let evidence = work::evidence_for(&c, &k, &key).unwrap();
        c.execute("UPDATE asset_cleanup_groups SET advice='exclude',ai_basis_hash=?3 WHERE run_id=?1 AND group_key=?2",params![k.id,key,work::basis(&evidence)]).unwrap();
        work::reclassify(&mut c, &k, &key).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT state FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
                params![k.id, key],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            expected,
            "manual accepted/rejected must protect this project and organization only: {domain}"
        );
    }
    assert_eq!(exclusions::rules(&c, 1).unwrap(), vec!["unrelated.test"]);
    let manual_after:Vec<(String,String,String,String)>=c.prepare("SELECT run_id,item_key,decision,evidence_json FROM asset_seed_review_items WHERE decision IN ('accepted','rejected') ORDER BY run_id,item_key").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(manual_after, manual_before);
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_models", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn cleanup_expired_fact_returning_to_paid_basis_converges_without_model_replay_or_infinite_wait() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "www.returned.test", "Other Owner");
    tests::prepared(&mut c, &k);
    let key = "domain:returned.test";
    work::reclassify(&mut c, &k, key).unwrap();
    let original_basis = work::basis(&work::evidence_for(&c, &k, key).unwrap());
    let paid_request = r#"{"targetOrganization":"Target Group","originalPaid":true}"#;
    let paid_response = r#"[{"key":"r0","label":"evidence","reason":"original accepted result"}]"#;
    let paid_usage = r#"{"inputTokens":17,"outputTokens":9}"#;
    c.execute("INSERT INTO asset_cleanup_models(id,run_id,recipient,request_json,state,response_text,usage_json,error,created_at) VALUES('prior-paid-basis',?1,'fixture',?2,'applied',?3,?4,'',?5)",params![k.id,paid_request,paid_response,paid_usage,now()]).unwrap();
    c.execute(
        "INSERT INTO asset_cleanup_model_items VALUES('prior-paid-basis',?1,?2,?3)",
        params![k.id, key, original_basis],
    )
    .unwrap();
    c.execute("UPDATE asset_cleanup_groups SET advice='evidence',ai_basis_hash=?3 WHERE run_id=?1 AND group_key=?2",params![k.id,key,original_basis]).unwrap();
    let fact = json!({"kind":"ct_certificate","checkedAt":now(),"sourceUrl":"https://crt.sh/?d=42","bodySha256":"a".repeat(64),"facts":[{"host":"www.returned.test","organization":"Other Owner","source":"public_ct_subject","notAfter":now()+86400}]});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate','42','done',?3)",params![k.id,key,fact.to_string()]).unwrap();
    work::reclassify(&mut c, &k, key).unwrap();
    let saved_basis: String = c
        .query_row(
            "SELECT basis_hash FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
            params![k.id, key],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(saved_basis, original_basis);
    assert_eq!(work::pending_model_groups(&c, &k.id).unwrap(), 1);
    // The collector's saved receipt remains intact, but its identity expires
    // between candidate projection and the real current evidence read.
    c.execute("UPDATE asset_cleanup_research SET result_json=json_set(result_json,'$.facts[0].notAfter',?3) WHERE run_id=?1 AND group_key=?2 AND kind='ct_certificate'",params![k.id,key,now()-1]).unwrap();
    assert_eq!(
        work::basis(&work::evidence_for(&c, &k, key).unwrap()),
        original_basis
    );
    c.execute("UPDATE asset_cleanup_research SET state='exhausted',attempts=2,error='source unavailable after finite attempts' WHERE run_id=?1 AND kind!='ct_certificate'",[&k.id]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state) VALUES(?1,'__organization','organization','Target Group','done')",[&k.id]).unwrap();
    assert!(
        !work::settle_completed_groups(&mut c, &k).unwrap(),
        "the stale candidate reproduces the terminal blocker before its cached basis is reconciled"
    );
    assert!(work::analyze(&mut c, &k).unwrap(), "a used current basis must count its local convergence as progress, without entering transport");
    assert_eq!(work::pending_model_groups(&c, &k.id).unwrap(), 0);
    assert_eq!(
        c.query_row(
            "SELECT basis_hash FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
            params![k.id, key],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        original_basis
    );
    assert!(work::settle_completed_groups(&mut c, &k).unwrap());
    assert!(work::scheduler_step(Path::new("unused-no-network"), &mut c, &k).unwrap());
    assert!(work::finish_step(&c, &k, Ok(())).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT phase,enabled,claim_token FROM asset_cleanup_runs WHERE id=?1",
            [&k.id],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, String>(2)?
            ))
        )
        .unwrap(),
        ("completed_with_gaps".into(), false, "".into())
    );
    assert!(work::claim(&mut c).unwrap().is_none());
    let paid:(String,String,String,String,String)=c.query_row("SELECT state,request_json,response_text,usage_json,error FROM asset_cleanup_models WHERE id='prior-paid-basis'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(
        paid,
        (
            "applied".into(),
            paid_request.into(),
            paid_response.into(),
            paid_usage.into(),
            "".into()
        )
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_models", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_model_items WHERE basis_hash=?1",
            [original_basis],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(c.query_row("SELECT count(*) FROM asset_cleanup_research WHERE kind!='ct_certificate' AND kind!='organization' AND (state!='exhausted' OR attempts!=2)",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
    assert_eq!(
        c.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
