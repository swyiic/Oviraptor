use super::*;

#[test]
fn cleanup_two_pauses_refund_only_unadopted_public_claims_and_keep_history_after_success() {
    let (mut c, mut k) = queue_fixture();
    queue_row(
        &c,
        &k,
        1,
        "paused.test",
        "public_ownership",
        "paused.test",
        0,
    );
    for cancelled in 1..=2 {
        let rows = claim_public_batch(&mut c, &k).unwrap();
        assert_eq!(rows.len(), 1);
        c.execute(
            "UPDATE asset_cleanup_runs SET enabled=0,phase='paused',generation=generation+1",
            [],
        )
        .unwrap();
        assert!(!finish_public(&mut c, &k, &rows[0], Ok(json!({"facts":[]}))).unwrap());
        // Duplicate late delivery is inert and cannot refund twice.
        settle_public_rows(&mut c, &k, &[1]).unwrap();
        let row:(String,i64,i64,i64)=c.query_row("SELECT state,attempts,json_extract(result_json,'$.cancelledRequests'),json_extract(result_json,'$.lastCancellationAt') FROM asset_cleanup_research WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!((row.0.as_str(), row.1, row.2), ("retry", 0, cancelled));
        assert!(row.3 > 0);
        c.execute("UPDATE asset_cleanup_runs SET enabled=1,generation=generation+1,claim_token='',owner_pid=0,lease_until=0",[]).unwrap();
        k = work::claim(&mut c).unwrap().unwrap();
        c.execute("UPDATE asset_cleanup_research SET next_at=0 WHERE id=1", [])
            .unwrap();
        assert!(!settle_exhausted(&mut c, &k).unwrap());
        assert_eq!(runnable_count(&c, &k).unwrap(), 1);
    }
    let rows = claim_public_batch(&mut c, &k).unwrap();
    assert!(finish_public(
        &mut c,
        &k,
        &rows[0],
        Ok(json!({"kind":"public_ownership","facts":[]}))
    )
    .unwrap());
    let row:(String,i64,i64)=c.query_row("SELECT state,attempts,json_extract(result_json,'$.cancelledRequests') FROM asset_cleanup_research WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(row, ("done".into(), 1, 2));
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_models", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn cleanup_two_actual_public_failures_still_exhaust_budget_without_cancellation_refund() {
    let (mut c, k) = queue_fixture();
    queue_row(
        &c,
        &k,
        1,
        "failed.test",
        "public_ownership",
        "failed.test",
        0,
    );
    for attempt in 1..=2 {
        c.execute("UPDATE asset_cleanup_research SET next_at=0 WHERE id=1", [])
            .unwrap();
        let rows = claim_public_batch(&mut c, &k).unwrap();
        let row = rows.iter().find(|r| r.0 == 1).unwrap();
        assert!(finish_public(&mut c, &k, row, Err("website HTTP 404".into())).unwrap());
        let state:(String,i64,Option<i64>)=c.query_row("SELECT state,attempts,json_extract(result_json,'$.cancelledRequests') FROM asset_cleanup_research WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(state.1, attempt);
        assert_eq!(state.2, None);
        assert_eq!(state.0, if attempt == 1 { "retry" } else { "exhausted" });
        // Isolate the website budget from new, independent CT fixture work.
        c.execute(
            "UPDATE asset_cleanup_research SET state='done' WHERE id!=1",
            [],
        )
        .unwrap();
    }
}

#[test]
fn cleanup_cold_public_claim_recovery_refunds_interrupts_but_never_replays_paid_unknown() {
    let (mut c, mut k) = queue_fixture();
    queue_row(&c, &k, 1, "cold.test", "public_ownership", "cold.test", 0);
    c.execute("INSERT INTO asset_cleanup_models(id,run_id,recipient,request_json,state,created_at) VALUES('paid-interrupted',?1,'fixture','original-paid-request','sent',?2)",params![k.id,now()]).unwrap();
    for cancelled in 1..=2 {
        let rows = claim_public_batch(&mut c, &k).unwrap();
        assert_eq!(rows.len(), 1);
        c.execute("UPDATE asset_cleanup_runs SET lease_until=0", [])
            .unwrap();
        k = work::claim(&mut c).unwrap().unwrap();
        let row:(String,i64,i64)=c.query_row("SELECT state,attempts,json_extract(result_json,'$.cancelledRequests') FROM asset_cleanup_research WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(row, ("retry".into(), 0, cancelled));
        assert!(!settle_exhausted(&mut c, &k).unwrap());
        c.execute("UPDATE asset_cleanup_research SET next_at=0 WHERE id=1", [])
            .unwrap();
    }
    assert_eq!(
        c.query_row(
            "SELECT state,request_json FROM asset_cleanup_models WHERE id='paid-interrupted'",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        )
        .unwrap(),
        ("unknown".into(), "original-paid-request".into())
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_models", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

fn answer(status: i64) -> Value {
    json!({"Status":status,"TC":false,"Question":[{"name":"app.example.test.","type":5}]})
}

fn queue_fixture() -> (Connection, work::Claim) {
    super::super::tests::fixture()
}
fn queue_row(
    c: &Connection,
    k: &work::Claim,
    id: i64,
    group: &str,
    kind: &str,
    target: &str,
    attempts: i64,
) {
    c.execute("INSERT OR IGNORE INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,?2,?2,'evidence')",params![k.id,group]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(id,run_id,group_key,kind,target,state,attempts) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,k.id,group,kind,target,if attempts==0{"pending"}else{"retry"},attempts]).unwrap();
}

#[test]
fn cleanup_fair_batch_prioritizes_first_attempts_rotates_sources_and_claims_before_io() {
    let (mut c, k) = queue_fixture();
    queue_row(&c, &k, 1, "old.test", "public_ownership", "old.test", 1);
    for id in 2..=25 {
        let kind = KINDS[(id as usize - 2) % 4];
        queue_row(
            &c,
            &k,
            id,
            &format!("group{id}.test"),
            kind,
            &format!("target{id}.test"),
            0,
        );
    }
    let rows = claim_public_batch(&mut c, &k).unwrap();
    assert_eq!(rows.len(), 12);
    assert!(rows.iter().all(|r| r.4 == 0 && r.0 != 1));
    for kind in KINDS {
        assert_eq!(rows.iter().filter(|r| r.2 == kind).count(), 3, "{kind}");
    }
    assert_eq!(rows.iter().map(|r| &r.1).collect::<BTreeSet<_>>().len(), 12);
    for row in &rows {
        assert_eq!(
            c.query_row(
                "SELECT state,attempts,claim_token FROM asset_cleanup_research WHERE id=?1",
                [row.0],
                |r| Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?
                ))
            )
            .unwrap(),
            ("running".into(), 1, k.token.clone())
        );
    }
    let second = claim_public_batch(&mut c, &k).unwrap();
    assert_eq!(second.len(), 12);
    assert_eq!(second[0].2, "ct_certificate");
    assert!(second
        .iter()
        .all(|r| rows.iter().all(|first| first.0 != r.0)));
    assert_eq!(next_research(&c, &k).unwrap().unwrap().0, 1);
}

#[test]
fn cleanup_shared_root_and_family_redundancy_finish_without_losing_original_index() {
    let (mut c, k) = queue_fixture();
    queue_row(&c, &k, 1, "163.com", "ct_search", "163.com", 0);
    for id in 2..=22 {
        queue_row(&c, &k, id, "163.com", "ct_certificate", &id.to_string(), 0);
    }
    queue_row(&c, &k, 30, "owner.test", "ct_search", "www.owner.test", 0);
    queue_row(&c, &k, 31, "owner.test", "ct_search", "mail.owner.test", 0);
    queue_row(&c, &k, 32, "owner.test", "dns_cname", "mail.owner.test", 0);
    let old = json!({"kind":"ct_search","certificateIds":[1,2,3],"checkedAt":now()});
    c.execute(
        "UPDATE asset_cleanup_research SET state='done',result_json=?1 WHERE id=1",
        [old.to_string()],
    )
    .unwrap();
    assert!(settle_exhausted(&mut c, &k).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT result_json FROM asset_cleanup_research WHERE id=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        old.to_string()
    );
    assert_eq!(c.query_row("SELECT count(*) FROM asset_cleanup_research WHERE group_key='163.com' AND state='exhausted'",[],|r|r.get::<_,i64>(0)).unwrap(),21);
    assert_eq!(c.query_row("SELECT count(*) FROM asset_cleanup_research WHERE group_key='owner.test' AND kind='ct_search' AND state='pending' AND target='owner.test'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(c.query_row("SELECT count(*) FROM asset_cleanup_research WHERE id IN (30,31) AND error LIKE '%duplicate_family_index%'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert!(pattern_only(&c, &k, "owner.test", "owner.test").unwrap());
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=32",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    assert!(
        !settle_exhausted(&mut c, &k).unwrap(),
        "terminal migration must be idempotent"
    );
}

#[test]
fn cleanup_leaf_samples_and_attempts_are_finite_preserve_errors_and_never_exclude() {
    let (mut c, k) = queue_fixture();
    for id in 1..=20 {
        queue_row(
            &c,
            &k,
            id,
            "owner.test",
            "ct_certificate",
            &id.to_string(),
            0,
        );
    }
    queue_row(
        &c,
        &k,
        30,
        "failed.test",
        "public_ownership",
        "failed.test",
        2,
    );
    c.execute(
        "UPDATE asset_cleanup_research SET error='original 404 with receipt' WHERE id=30",
        [],
    )
    .unwrap();
    assert!(settle_exhausted(&mut c, &k).unwrap());
    let remaining:Vec<i64>=c.prepare("SELECT CAST(target AS INTEGER) FROM asset_cleanup_research WHERE group_key='owner.test' AND state='pending' ORDER BY CAST(target AS INTEGER)").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(remaining, vec![16, 17, 18, 19, 20]);
    let row: (String, String, i64) = c
        .query_row(
            "SELECT state,error,attempts FROM asset_cleanup_research WHERE id=30",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, "exhausted");
    assert!(row.1.contains("original 404"));
    assert_eq!(row.2, 2);
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(runnable_count(&c, &k).unwrap(), 5);
}

#[test]
fn cleanup_generic_late_result_cannot_overwrite_pause_or_replacement_claim() {
    let (mut c, k) = queue_fixture();
    queue_row(&c, &k, 1, "owner.test", "public_ownership", "owner.test", 0);
    let rows = claim_public_batch(&mut c, &k).unwrap();
    c.execute(
        "UPDATE asset_cleanup_runs SET enabled=0,phase='paused',generation=generation+1",
        [],
    )
    .unwrap();
    assert!(!finish_public(&mut c, &k, &rows[0], Ok(json!({"facts":[]}))).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "retry"
    );
    c.execute(
        "UPDATE asset_cleanup_research SET state='running',claim_token='new-owner' WHERE id=1",
        [],
    )
    .unwrap();
    assert!(!finish_public(&mut c, &k, &rows[0], Err("failed old request".into())).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT claim_token FROM asset_cleanup_research WHERE id=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "new-owner"
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn cleanup_valid_negative_dns_cache_binds_target_and_ttl() {
    let value = dns_with("app.example.test", |_| Ok((answer(3), "a".repeat(64)))).unwrap();
    assert!(reusable_public(
        &value,
        "dns_cname",
        "app.example.test",
        false
    ));
    assert!(!reusable_public(
        &value,
        "dns_cname",
        "unrelated.test",
        false
    ));
    let mut expired = value.clone();
    expired["checkedAt"] = json!(now() - 86401);
    assert!(!reusable_public(
        &expired,
        "dns_cname",
        "app.example.test",
        false
    ));
    let mut poisoned = value;
    poisoned["records"] = json!([{"name":"unrelated.test","cname":"another.test"}]);
    assert!(!reusable_public(
        &poisoned,
        "dns_cname",
        "app.example.test",
        false
    ));
}

#[test]
fn cleanup_source_failure_finishes_only_its_provider_not_independent_sites_or_dns() {
    let (mut c, k) = queue_fixture();
    ensure_provider_backoffs(&c).unwrap();
    queue_row(&c, &k, 1, "owner.test", "ct_search", "owner.test", 0);
    queue_row(&c, &k, 2, "site.test", "public_ownership", "site.test", 0);
    queue_row(&c, &k, 3, "dns.test", "dns_cname", "dns.test", 0);
    record_source_failure(&c, &k, "ct_search", "crt.sh HTTP 502").unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    record_source_failure(&c, &k, "ct_certificate", "crt.sh HTTP 502").unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "exhausted"
    );
    assert_eq!(runnable_count(&c, &k).unwrap(), 2);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_research WHERE id IN (2,3) AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    c.execute(
        "UPDATE asset_cleanup_runs SET enabled=0,generation=generation+1",
        [],
    )
    .unwrap();
    assert!(!settle_exhausted(&mut c, &k).unwrap());
}

#[test]
fn cleanup_groups_rotate_across_batches_instead_of_first_twelve_families_monopolizing() {
    let (mut c, k) = queue_fixture();
    for group in 1..=25 {
        for host in 1..=5 {
            queue_row(
                &c,
                &k,
                group * 100 + host,
                &format!("family{group}.test"),
                "dns_cname",
                &format!("host{host}.family{group}.test"),
                0,
            );
        }
    }
    let first = claim_public_batch(&mut c, &k).unwrap();
    let second = claim_public_batch(&mut c, &k).unwrap();
    assert_eq!(first.len(), 12);
    assert_eq!(second.len(), 12);
    assert!(
        second
            .iter()
            .all(|row| first.iter().all(|previous| previous.1 != row.1)),
        "unseen groups must precede remaining hostnames of a dispatched group"
    );
    let third = claim_public_batch(&mut c, &k).unwrap();
    assert_eq!(third[0].1, "family25.test");
}

#[test]
fn cleanup_invalid_public_material_does_not_pause_batch_or_broadcast_owner_facts() {
    let (mut c, k) = queue_fixture();
    queue_row(&c, &k, 1, "owner.test", "ct_certificate", "42", 0);
    queue_row(&c, &k, 2, "other.test", "dns_cname", "app.example.test", 0);
    let rows = claim_public_batch(&mut c, &k).unwrap();
    let invalid = rows.iter().find(|r| r.2 == "ct_certificate").unwrap();
    assert!(finish_public(&mut c,&k,invalid,Ok(json!({"certificateId":43,"sourceUrl":"https://crt.sh/?d=43","sha256":"a".repeat(64),"organizations":["Wrong Owner"]}))).unwrap());
    let state: String = c
        .query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "retry");
    let dns = rows.iter().find(|r| r.2 == "dns_cname").unwrap();
    let valid = dns_with("app.example.test", |_| Ok((answer(3), "a".repeat(64)))).unwrap();
    assert!(finish_public(&mut c, &k, dns, Ok(valid)).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=2",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "done"
    );
    assert_eq!(
        c.query_row("SELECT enabled FROM asset_cleanup_runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn cleanup_success_resets_consecutive_source_failures_without_erasing_prior_request_errors() {
    let (c, k) = queue_fixture();
    ensure_provider_backoffs(&c).unwrap();
    queue_row(&c, &k, 1, "owner.test", "ct_search", "owner.test", 0);
    record_source_failure(&c, &k, "ct_search", "HTTP 502").unwrap();
    record_source_success(&c, &k, "ct_certificate").unwrap();
    record_source_failure(&c, &k, "ct_search", "HTTP 502").unwrap();
    assert_eq!(
        c.query_row(
            "SELECT failures FROM asset_cleanup_source_failures",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
}

#[test]
fn cleanup_identical_target_failure_is_one_request_shared_with_all_waiting_groups() {
    let (mut c, k) = queue_fixture();
    queue_row(&c, &k, 1, "one.test", "ct_certificate", "42", 0);
    queue_row(&c, &k, 2, "two.test", "ct_certificate", "42", 0);
    let rows = claim_public_batch(&mut c, &k).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(finish_public(&mut c, &k, &rows[0], Err("crt.sh HTTP 502".into())).unwrap());
    assert_eq!(c.query_row("SELECT count(*) FROM asset_cleanup_research WHERE id IN (1,2) AND state='retry' AND attempts=1 AND error='crt.sh HTTP 502'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(
        c.query_row(
            "SELECT failures FROM asset_cleanup_source_failures",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(batch_rows(&c, &k)
        .unwrap()
        .iter()
        .all(|r| r.2 != "ct_certificate"));
}

#[test]
fn cleanup_organization_exhaustion_preserves_accepted_names_and_never_reopens_on_cold_tick() {
    let (mut c, k) = queue_fixture();
    let original =
        json!({"acceptedNames":["Target Subsidiary"],"retryAt":now()+86400,"complete":false});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,attempts,next_at,result_json) VALUES(?1,'__organization','organization','Target Group','retry',2,?2,?3)",params![k.id,now()+86400,original.to_string()]).unwrap();
    let names: String = c
        .query_row(
            "SELECT target_names_json FROM asset_cleanup_runs",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(settle_exhausted(&mut c, &k).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT state,result_json FROM asset_cleanup_research WHERE kind='organization'",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        )
        .unwrap(),
        ("exhausted".into(), original.to_string())
    );
    assert_eq!(
        c.query_row(
            "SELECT target_names_json FROM asset_cleanup_runs",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        names
    );
    assert!(!organization(Path::new("/no-live-data-or-network"), &mut c, &k).unwrap());
    assert!(!settle_exhausted(&mut c, &k).unwrap());
}

#[test]
fn cleanup_same_official_catalog_never_reopens_exhausted_relationship_sources() {
    let (mut c, k) = queue_fixture();
    c.execute(
        "UPDATE asset_cleanup_runs SET organization='中国移动集团及其下属单位' WHERE id=?1",
        [&k.id],
    )
    .unwrap();
    let receipt = json!({"sourceCatalogRevision":organization_evidence::source_catalog_revision(),"supported":true,"names":["中国移动有限公司"],"retryAt":now()+86400,"error":"HTTP 403 saved"});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,attempts,next_at,result_json,error) VALUES(?1,'__organization','organization','中国移动集团及其下属单位','exhausted',2,?2,?3,'HTTP 403 saved')",params![k.id,now()+86400,receipt.to_string()]).unwrap();
    assert!(!organization(Path::new("/must-not-open-data-or-network"), &mut c, &k).unwrap());
    assert_eq!(c.query_row("SELECT attempts,result_json,error,state FROM asset_cleanup_research WHERE kind='organization'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).unwrap(),(2,receipt.to_string(),"HTTP 403 saved".into(),"exhausted".into()));
}

#[test]
fn cleanup_same_batch_provider_recovery_restores_attempted_rows_with_remaining_budget() {
    let (mut c, k) = queue_fixture();
    for id in 1..=3 {
        queue_row(
            &c,
            &k,
            id,
            &format!("family{id}.test"),
            "ct_certificate",
            &id.to_string(),
            0,
        );
    }
    let rows = claim_public_batch(&mut c, &k).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(finish_public(&mut c, &k, &rows[0], Err("crt.sh HTTP 502".into())).unwrap());
    assert!(finish_public(&mut c, &k, &rows[1], Err("crt.sh HTTP 502".into())).unwrap());
    for row in rows.iter().take(2) {
        let state: String = c
            .query_row(
                "SELECT state FROM asset_cleanup_research WHERE id=?1",
                [row.0],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "exhausted");
    }
    let deadlines: Vec<(i64, i64)> = c
        .prepare("SELECT id,next_at FROM asset_cleanup_research WHERE id IN (1,2) ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let third = &rows[2];
    let id = third.3.parse::<i64>().unwrap();
    let value = json!({"certificateId":id,"sourceUrl":format!("https://crt.sh/?d={id}"),"sha256":"a".repeat(64),"names":[],"organizations":[],"notBefore":now()-86400,"notAfter":now()+86400});
    assert!(finish_public(&mut c, &k, third, Ok(value)).unwrap());
    for (id, deadline) in deadlines {
        let row: (String, i64, i64, String) = c
            .query_row(
                "SELECT state,attempts,next_at,error FROM asset_cleanup_research WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "retry");
        assert_eq!(row.1, 1);
        assert_eq!(row.2, deadline);
        assert!(row.3.contains("HTTP 502"));
    }
    assert_eq!(
        c.query_row(
            "SELECT failures FROM asset_cleanup_source_failures WHERE provider='ct'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn cleanup_target_relation_change_replays_unresolved_locally_without_touching_paid_or_protected_states(
) {
    let (mut c, k) = queue_fixture();
    super::super::tests::add(&c, &k, 1, "www.subsidiary.test", "New Subsidiary");
    super::super::tests::prepared(&mut c, &k);
    work::reclassify(&mut c, &k, "domain:subsidiary.test").unwrap();
    c.execute(
        "UPDATE asset_cleanup_groups SET state='deferred' WHERE group_key='domain:subsidiary.test'",
        [],
    )
    .unwrap();
    for (state, domain) in [
        ("held", "manual.test"),
        ("excluded", "old.test"),
        ("restored", "restored.test"),
        ("conflict", "conflict.test"),
    ] {
        c.execute(
            "INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,?2,?3,?4)",
            params![k.id, format!("domain:{domain}"), domain, state],
        )
        .unwrap();
    }
    c.execute("INSERT INTO asset_cleanup_models(id,run_id,recipient,request_json,state,response_text,created_at) VALUES('paid-history',?1,'fixture','original-request','applied','original-response',?2)",params![k.id,now()]).unwrap();
    let mut names: BTreeSet<String> = serde_json::from_str(
        &c.query_row(
            "SELECT target_names_json FROM asset_cleanup_runs",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap();
    names.insert(review::scope::organization_key("New Subsidiary"));
    {
        let tx = c.transaction().unwrap();
        assert!(apply_target_names(&tx, &k, &names).unwrap());
        tx.commit().unwrap();
    }
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_groups WHERE group_key='domain:subsidiary.test'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    work::reclassify(&mut c, &k, "domain:subsidiary.test").unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_groups WHERE group_key='domain:subsidiary.test'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "retained"
    );
    for (state, domain) in [
        ("held", "manual.test"),
        ("excluded", "old.test"),
        ("restored", "restored.test"),
        ("conflict", "conflict.test"),
    ] {
        assert_eq!(
            c.query_row(
                "SELECT state FROM asset_cleanup_groups WHERE group_key=?1",
                [format!("domain:{domain}")],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            state
        );
    }
    assert_eq!(c.query_row("SELECT state,request_json,response_text FROM asset_cleanup_models WHERE id='paid-history'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).unwrap(),("applied".into(),"original-request".into(),"original-response".into()));
    let before: i64 = c
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    assert!(!apply_target_names(&c, &k, &names).unwrap());
    assert_eq!(
        c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn cleanup_dns_servfail_falls_back_and_keeps_source_failure_receipt() {
    let mut calls = Vec::new();
    let result = dns_with("app.example.test", |url| {
        calls.push(url.clone());
        let value = if calls.len() == 1 {
            answer(2)
        } else {
            let mut value = answer(0);
            value["Answer"] =
                json!([{"type":5,"name":"app.example.test.","data":"service.other.test."}]);
            value
        };
        Ok((value, format!("sha{}", calls.len())))
    })
    .unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].host_str(), Some("dns.google"));
    assert_eq!(calls[1].host_str(), Some("cloudflare-dns.com"));
    assert!(calls[0]
        .query_pairs()
        .any(|(key, value)| key == "edns_client_subnet" && value == "0.0.0.0/0"));
    assert!(!calls[1]
        .query_pairs()
        .any(|(key, _)| key == "edns_client_subnet" || key == "cd"));
    assert!(result["sourceAttempts"][0]["error"]
        .as_str()
        .unwrap()
        .contains("SERVFAIL"));
    assert_eq!(result["sourceAttempts"][0]["bodySha256"], "sha1");
    assert_eq!(result["sourceAttempts"][1]["state"], "done");
    assert_eq!(result["records"][0]["cname"], "service.other.test");
    assert_eq!(result["facts"], json!([]));
    assert_eq!(result["ownershipConclusion"], false);
    assert_eq!(result["hunterRequests"], 0);
}

#[test]
fn cleanup_dns_valid_negative_is_not_a_provider_failure_or_ownership_decision() {
    for status in [0, 3] {
        let mut calls = 0;
        let value = dns_with("app.example.test", |_| {
            calls += 1;
            Ok((answer(status), "negative-response-sha".into()))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(value["dnsStatus"], status);
        assert_eq!(value["records"], json!([]));
        assert_eq!(value["facts"], json!([]));
        assert_eq!(value["ownershipConclusion"], false);
    }
}

#[test]
fn cleanup_dns_error_names_rcode_and_truncation_instead_of_classifying_hosts() {
    for (status, description) in [
        (1, "FORMERR"),
        (2, "SERVFAIL"),
        (4, "NOTIMP"),
        (5, "REFUSED"),
    ] {
        let error = parse_dns(&answer(status), "app.example.test").unwrap_err();
        assert!(error.contains(description), "{error}");
        assert!(error.contains(&format!("Status={status}")), "{error}");
    }
    assert!(parse_dns(&json!({"Status":2}), "app.example.test")
        .unwrap_err()
        .contains("SERVFAIL"));
    let mut truncated = answer(0);
    truncated["TC"] = json!(true);
    assert!(parse_dns(&truncated, "app.example.test")
        .unwrap_err()
        .contains("TC=true"));
    let error = dns_with("app.example.test", |_| {
        Ok((truncated.clone(), "truncated-sha".into()))
    })
    .unwrap_err();
    assert!(error.contains("dns.google"));
    assert!(error.contains("cloudflare-dns.com"));
    assert!(error.contains("保留候选"));
    let mut malformed = answer(0);
    malformed["TC"] = json!("false");
    assert!(parse_dns(&malformed, "app.example.test")
        .unwrap_err()
        .contains("TC字段无效"));
    malformed["TC"] = json!(false);
    malformed["Answer"] = json!({"invalid":"not-an-array"});
    assert!(parse_dns(&malformed, "app.example.test")
        .unwrap_err()
        .contains("Answer不是记录数组"));
}

#[test]
fn cleanup_dns_bad_primary_binding_falls_back_but_never_accepts_unrelated_answers() {
    let mut calls = 0;
    let error = dns_with("app.example.test", |_| {
        calls += 1;
        let mut value = answer(0);
        if calls == 1 {
            value["Question"][0]["name"] = json!("other.test.");
        } else {
            value["Answer"] = json!([{"type":5,"name":"unrelated.test.","data":"target.test."}]);
        }
        Ok((value, "mismatched-sha".into()))
    })
    .unwrap_err();
    assert_eq!(calls, 2);
    assert!(error.contains("Question"));
    assert!(error.contains("无关的CNAME"));
}

#[test]
fn cleanup_dns_duplicate_identical_records_are_not_conflicts_but_different_targets_are() {
    let mut value = answer(0);
    value["Answer"] = json!([
        {"type":5,"name":"app.example.test.","data":"service.other.test."},
        {"type":5,"name":"app.example.test.","data":"service.other.test."}
    ]);
    assert_eq!(parse_dns(&value, "app.example.test").unwrap().len(), 1);
    value["Answer"][1]["data"] = json!("different.test.");
    assert!(parse_dns(&value, "app.example.test")
        .unwrap_err()
        .contains("多个目标"));
}

#[test]
fn cleanup_dns_invalid_candidate_sends_no_request_and_transport_failure_uses_bounded_pair() {
    let invalid = dns_with("https://app.example.test", |_| {
        panic!("invalid candidate requested")
    });
    assert!(invalid.unwrap_err().contains("规范域名"));
    let mut calls = 0;
    let error = dns_with("app.example.test", |_| {
        calls += 1;
        Err("公开研究 HTTP 502；不能据此判断目标不存在或无关".into())
    })
    .unwrap_err();
    assert_eq!(calls, 2);
    assert_eq!(error.matches("HTTP 502").count(), 2);
    assert!(error.contains("解析失败不代表资产不存在或无关"));
}

fn certificate_fixture() -> (Connection, work::Claim, Value) {
    let c = Connection::open_in_memory().unwrap();
    super::super::ensure(&c).unwrap();
    let k = work::Claim {
        id: "run-one".into(),
        p: 1,
        generation: 1,
        token: "owner".into(),
    };
    for (id, name, key) in [
        (1, "app.example.test", "domain:example.test"),
        (2, "app.other.test", "domain:other.test"),
        (3, "unrelated.test", "domain:unrelated.test"),
    ] {
        c.execute("INSERT INTO asset_cleanup_seeds(run_id,source_kind,source_id,host,source,source_ref,wildcard,group_key) VALUES(?1,'raw',?2,?3,'observed_host','fixture',0,?4)",params![k.id,id,name,key]).unwrap();
    }
    let v = json!({"certificateId":42,"sha256":"a".repeat(64),"names":["app.example.test","app.other.test"],"organizations":["Other Owner"],"notBefore":now()-86400,"notAfter":now()+86400,"sourceUrl":"https://crt.sh/?d=42","precertificate":false});
    (c, k, v)
}

#[test]
fn cleanup_certificate_success_is_reused_only_within_run_and_rebound_to_each_group() {
    let (c, k, v) = certificate_fixture();
    let mut first = certificate_result(&c, &k, "domain:example.test", "42", &v, false).unwrap();
    assert_eq!(first["facts"].as_array().unwrap().len(), 1);
    assert_eq!(first["facts"][0]["host"], "app.example.test");
    // A previous group's derived conclusion must never leak into another one.
    first["facts"][0]["organization"] = json!("DO NOT COPY DERIVED FACTS");
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,'domain:example.test','ct_certificate','42','done',?2)",params![k.id,first.to_string()]).unwrap();
    let cached = cached_certificate(&c, &k.id, "42").unwrap().unwrap();
    let second = certificate_result(&c, &k, "domain:other.test", "42", &cached, true).unwrap();
    assert_eq!(second["cacheHit"], true);
    assert_eq!(second["networkRequested"], false);
    assert_eq!(second["facts"].as_array().unwrap().len(), 1);
    assert_eq!(second["facts"][0]["host"], "app.other.test");
    assert_eq!(second["facts"][0]["organization"], "Other Owner");
    assert_eq!(
        second["certificateFetchedAt"],
        first["certificateFetchedAt"]
    );
    let unrelated =
        certificate_result(&c, &k, "domain:unrelated.test", "42", &cached, true).unwrap();
    assert_eq!(unrelated["facts"], json!([]));
    assert!(cached_certificate(&c, "another-run", "42")
        .unwrap()
        .is_none());
    assert!(cached_certificate(&c, &k.id, "43").unwrap().is_none());
}

#[test]
fn cleanup_certificate_partial_failed_or_unbound_receipts_are_not_reusable() {
    let (c, k, v) = certificate_fixture();
    let receipt = certificate_result(&c, &k, "domain:example.test", "42", &v, false).unwrap();
    for field in [
        "organizations",
        "notBefore",
        "notAfter",
        "precertificate",
        "certificateFetchedAt",
        "certificateId",
    ] {
        let mut invalid = receipt.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(reusable_certificate(&invalid, "42").is_none(), "{field}");
    }
    let mut bad_source = receipt.clone();
    bad_source["sourceUrl"] = json!("https://untrusted.test/?d=42");
    assert!(reusable_certificate(&bad_source, "42").is_none());
    assert!(reusable_certificate(&receipt, "43").is_none());
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json,error) VALUES(?1,'domain:example.test','ct_certificate','42','retry',?2,'HTTP 502')",params![k.id,receipt.to_string()]).unwrap();
    assert!(cached_certificate(&c, &k.id, "42").unwrap().is_none());
}

#[test]
fn cleanup_cached_certificate_rechecks_validity_and_precertificates_do_not_generate_facts() {
    let (c, k, mut v) = certificate_fixture();
    v["notAfter"] = json!(now() - 1);
    let expired = certificate_result(&c, &k, "domain:example.test", "42", &v, true).unwrap();
    assert_eq!(expired["current"], false);
    assert_eq!(expired["facts"], json!([]));
    v["notAfter"] = json!(now() + 86400);
    v["precertificate"] = json!(true);
    let precertificate = certificate_result(&c, &k, "domain:example.test", "42", &v, true).unwrap();
    assert_eq!(precertificate["current"], false);
    assert_eq!(precertificate["facts"], json!([]));
}

fn cache_drain_fixture() -> (Connection, work::Claim) {
    let (c, k, v) = certificate_fixture();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active');").unwrap();
    c.execute("INSERT INTO asset_cleanup_runs(id,project_id,organization,recipient,model,target_names_json,roots_json,auto_exclude,phase,enabled,raw_count,ledger_count,created_at,updated_at,claim_token,owner_pid,lease_until) VALUES(?1,1,'Target Group','fixture','fixture','[]','[]',1,'researching',1,0,0,?2,?2,?3,?4,?5)",params![k.id,now(),k.token,std::process::id(),now()+180]).unwrap();
    for key in ["domain:example.test", "domain:other.test"] {
        c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,?2,'fixture.test','evidence')",params![k.id,key]).unwrap();
    }
    let first = certificate_result(&c, &k, "domain:example.test", "42", &v, false).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(id,run_id,group_key,kind,target,state,result_json) VALUES(1,?1,'domain:example.test','ct_certificate','42','done',?2)",params![k.id,first.to_string()]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(id,run_id,group_key,kind,target,state,attempts,next_at,error) VALUES(2,?1,'domain:other.test','ct_certificate','42','retry',4,?2,'HTTP 502')",params![k.id,now()+86400]).unwrap();
    provider_backoff(&c, &k.id, "ct_certificate", "HTTP 502", now() + 86400).unwrap();
    (c, k)
}

#[test]
fn cleanup_cache_only_drain_ignores_provider_deadline_without_allowing_network_or_counting_attempts(
) {
    let (c, k) = cache_drain_fixture();
    assert!(next_research(&c, &k).unwrap().is_none());
    let ((id, key, kind, target, attempts), certificate) =
        cached_research(&c, &k).unwrap().unwrap();
    assert_eq!(
        (id, kind.as_str(), target.as_str(), attempts),
        (2, "ct_certificate", "42", 4)
    );
    let result = certificate_result(&c, &k, &key, &target, &certificate, true).unwrap();
    assert_eq!(result["networkRequested"], false);
    assert_eq!(result["facts"][0]["host"], "app.other.test");
    assert!(next_research(&c, &k).unwrap().is_none());
    assert_eq!(
        c.query_row(
            "SELECT attempts FROM asset_cleanup_research WHERE id=2",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        4
    );
}

#[test]
fn cleanup_invalid_cache_cannot_bypass_provider_backoff_or_cross_generation() {
    let (c, k) = cache_drain_fixture();
    c.execute("UPDATE asset_cleanup_research SET result_json=json_set(result_json,'$.sourceUrl','https://untrusted.test/?d=42') WHERE id=1",[]).unwrap();
    assert!(cached_research(&c, &k).unwrap().is_none());
    assert!(next_research(&c, &k).unwrap().is_none());
    let (c, k) = cache_drain_fixture();
    c.execute("UPDATE asset_cleanup_runs SET generation=generation+1", [])
        .unwrap();
    assert!(cached_research(&c, &k).unwrap().is_none());
    assert!(next_research(&c, &k).unwrap().is_none());
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=2",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "retry"
    );
}

#[test]
fn cleanup_gateway_cooldown_does_not_park_every_certificate_for_failed_targets_full_day() {
    for status in [502, 503, 504] {
        let error = format!("crt.sh ct_certificate HTTP {status}；服务未成功");
        assert_eq!(provider_retry_delay(&error, 60), 300);
        assert_eq!(provider_retry_delay(&error, 1800), 900);
        assert_eq!(provider_retry_delay(&error, 86400), 900);
    }
    for error in [
        "HTTP 403",
        "HTTP 429",
        "HTTP 403；HTTP 502",
        "HTTP 429；HTTP 503",
    ] {
        assert_eq!(provider_retry_delay(error, 86400), 86400, "{error}");
    }
    let (c, k, _) = certificate_fixture();
    c.execute("INSERT INTO asset_cleanup_research(id,run_id,group_key,kind,target,state,attempts,next_at,error) VALUES(1,?1,'domain:example.test','ct_certificate','42','retry',4,?2,'HTTP 502')",params![k.id,now()+86400]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(id,run_id,group_key,kind,target) VALUES(2,?1,'domain:other.test','ct_certificate','43')",[&k.id]).unwrap();
    let until = now() + provider_retry_delay("HTTP 502", 86400);
    provider_backoff(&c, &k.id, "ct_certificate", "HTTP 502", until).unwrap();
    let failed_target: i64 = c
        .query_row(
            "SELECT next_at FROM asset_cleanup_research WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let sibling: i64 = c
        .query_row(
            "SELECT next_at FROM asset_cleanup_research WHERE id=2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(failed_target >= now() + 86399);
    assert_eq!(sibling, until);
    assert!(sibling <= now() + 900);
    assert_eq!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_provider_backoffs WHERE run_id=?1 AND provider='ct'",
            [&k.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        until
    );
}

#[test]
fn cleanup_transport_outage_uses_short_provider_wait_without_changing_target_retry() {
    for error in ["公开研究请求失败", "request timed out", "响应不是有效JSON"] {
        assert_eq!(provider_retry_delay(error, 86400), 900);
        assert_eq!(provider_retry_delay(error, 60), 300);
    }
}
#[test]
fn cleanup_dns_target_servfail_and_individual_site_refusal_do_not_pause_independent_sources() {
    let (c, k, _) = certificate_fixture();
    for (kind, error) in [
        (
            "dns_cname",
            "dns.google: DNS Status=2；cloudflare-dns.com: 公开研究请求失败",
        ),
        ("public_ownership", "官网 HTTP 403"),
        ("public_ownership", "官网请求失败 timeout"),
    ] {
        provider_backoff(&c, &k.id, kind, error, now() + 86400).unwrap();
    }
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_provider_backoffs",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    provider_backoff(
        &c,
        &k.id,
        "dns_cname",
        "HTTP 429；DNS Status=2",
        now() + 86400,
    )
    .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_provider_backoffs WHERE provider='dns'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
