use super::*;
fn state(c: &Connection, k: &work::Claim, key: &str) -> String {
    c.query_row(
        "SELECT state FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
        params![k.id, key],
        |r| r.get(0),
    )
    .unwrap()
}
fn provider(c: &Connection, id: i64, name: &str, owner: &str) {
    c.execute(
        "UPDATE assets SET extra_json=?2 WHERE id=?1",
        params![
            id,
            json!({"cert.subject.cn":name,"cert.subject.org":owner}).to_string()
        ],
    )
    .unwrap();
}
fn website(c: &Connection, k: &work::Claim, key: &str, name: &str, owner: &str, mutation: &str) {
    let mut facts = json!([
        {"host":name,"organization":owner,"source":"official_site_operator","evidenceTier":"declared","sourceUrl":format!("https://{name}/"),"bodySha256":"a".repeat(64),"notAfter":now()+86400},
        {"host":name,"organization":owner,"source":"website_icp_declared","registrationNumber":"蜀ICP备12345678号","evidenceTier":"declared","sourceUrl":format!("https://{name}/"),"bodySha256":"a".repeat(64),"notAfter":now()+86400}
    ]);
    match mutation {
        "wrong_origin" => {
            facts[0]["sourceUrl"] = json!("https://unrelated.test/");
            facts[1]["sourceUrl"] = json!("https://unrelated.test/");
        }
        "body_mention" => facts[0]["source"] = json!("website_body_mention"),
        "no_registration" => facts.as_array_mut().unwrap().truncate(1),
        "no_receipt" => facts[0]["bodySha256"] = Value::Null,
        "expired" => {
            facts[0]["notAfter"] = json!(now() - 1);
            facts[1]["notAfter"] = json!(now() - 1);
        }
        _ => {}
    }
    let v = json!({"kind":"public_ownership","sourceUrl":format!("https://{name}/"),"bodySha256":"a".repeat(64),"checkedAt":now(),"facts":facts});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'public_ownership',?3,'done',?4)",params![k.id,key,name,v.to_string()]).unwrap();
}
fn advice(c: &Connection, k: &work::Claim, key: &str) {
    let e = work::evidence_for(c, k, key).unwrap();
    c.execute("UPDATE asset_cleanup_groups SET advice='exclude',ai_basis_hash=?3 WHERE run_id=?1 AND group_key=?2",params![k.id,key,work::basis(&e)]).unwrap();
}
#[test]
fn cleanup_website_registration_certificate_crosscheck_excludes_and_undoes_without_deletion() {
    let (mut c, k) = tests::fixture();
    for (id, name) in [(1, "bank.test"), (2, "login.bank.test")] {
        tests::add(&c, &k, id, name, "");
        provider(&c, id, name, "Example University");
    }
    tests::prepared(&mut c, &k);
    let key = "domain:bank.test";
    website(&c, &k, key, "bank.test", "Example University", "");
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(
        state(&c, &k, key),
        "evidence",
        "new proof needs fresh model judgment"
    );
    assert_eq!(work::pending_model_groups(&c, &k.id).unwrap(), 1);
    advice(&c, &k, key);
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "excluded");
    assert_eq!(exclusions::rules(&c, 1).unwrap(), vec!["bank.test"]);
    for table in ["assets", "asset_discovery_seeds"] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let tx = c.unchecked_transaction().unwrap();
    work::restore_on(&tx, 1, &k.id, None).unwrap();
    tx.commit().unwrap();
    assert_eq!(state(&c, &k, key), "restored");
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
}
#[test]
fn cleanup_website_wildcard_covers_one_label_but_never_arbitrary_sibling_depth() {
    for deep in [false, true] {
        let (mut c, k) = tests::fixture();
        tests::add(&c, &k, 1, "college.test", "");
        provider(&c, 1, "*.college.test", "Example College");
        tests::add(
            &c,
            &k,
            2,
            if deep {
                "new.zx.college.test"
            } else {
                "zx.college.test"
            },
            "",
        );
        tests::prepared(&mut c, &k);
        let key = "domain:college.test";
        website(&c, &k, key, "college.test", "Example College", "");
        advice(&c, &k, key);
        work::reclassify(&mut c, &k, key).unwrap();
        assert_eq!(
            state(&c, &k, key),
            if deep { "evidence" } else { "excluded" },
            "deep={deep}"
        );
    }
}
#[test]
fn cleanup_website_declarations_alone_foreign_mentions_and_invalid_receipts_cannot_exclude() {
    for case in [
        "wrong_origin",
        "body_mention",
        "no_registration",
        "no_receipt",
        "expired",
        "other_owner",
        "unproven_sibling",
        "scope",
        "target_hint",
    ] {
        let (mut c, k) = tests::fixture();
        tests::add(&c, &k, 1, "bank.test", "");
        provider(
            &c,
            1,
            "bank.test",
            if case == "target_hint" {
                "Target"
            } else {
                "Example University"
            },
        );
        if case == "unproven_sibling" {
            tests::add(&c, &k, 2, "new.bank.test", "");
        }
        tests::prepared(&mut c, &k);
        let key = "domain:bank.test";
        website(
            &c,
            &k,
            key,
            "bank.test",
            if case == "other_owner" {
                "Different Company Limited"
            } else {
                "Example University"
            },
            case,
        );
        if case == "scope" {
            c.execute(
                "INSERT INTO asset_discovery_scopes VALUES(1,'[\"bank.test\"]')",
                [],
            )
            .unwrap();
        }
        advice(&c, &k, key);
        work::reclassify(&mut c, &k, key).unwrap();
        assert_ne!(state(&c, &k, key), "excluded", "{case}");
        assert!(exclusions::rules(&c, 1).unwrap().is_empty(), "{case}");
    }
}
#[test]
fn cleanup_website_current_certificate_is_independent_of_same_page_registration() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "www.bank.test", "");
    tests::prepared(&mut c, &k);
    let key = "domain:bank.test";
    website(
        &c,
        &k,
        key,
        "www.bank.test",
        "Example University",
        "no_registration",
    );
    let v = json!({"facts":[{"host":"www.bank.test","organization":"Example University","source":"public_site_certificate","notAfter":now()+86400}]});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate','current-site-cert','done',?3)",params![k.id,key,v.to_string()]).unwrap();
    advice(&c, &k, key);
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "excluded");
}
#[test]
fn cleanup_website_queue_is_family_bounded_durable_and_deduplicated() {
    let (mut c, k) = tests::fixture();
    for id in 1..=150 {
        tests::add(&c, &k, id, &format!("n{id}.bank.test"), "");
    }
    tests::prepared(&mut c, &k);
    for _ in 0..3 {
        work::reclassify(&mut c, &k, "domain:bank.test").unwrap();
    }
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_research WHERE kind='public_ownership'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT target FROM asset_cleanup_research WHERE kind='public_ownership'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "bank.test"
    );
}
#[test]
fn cleanup_v3_rechecks_only_unresolved_preserves_paid_history_and_repairs_only_shared_transient_deadline(
) {
    let (c, k) = tests::fixture();
    c.execute(
        "UPDATE asset_cleanup_runs SET claim_token='',owner_pid=0,lease_until=0",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO asset_cleanup_policy_revisions VALUES(?1,2)",
        [&k.id],
    )
    .unwrap();
    for (key, state) in [
        ("pending-owner", "evidence"),
        ("manual", "held"),
        ("done", "excluded"),
        ("undo", "restored"),
    ] {
        c.execute(
            "INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,?2,?2,?3)",
            params![k.id, key, state],
        )
        .unwrap();
    }
    c.execute("INSERT INTO asset_cleanup_models(id,run_id,recipient,request_json,state,created_at) VALUES('never-repeat',?1,'fixture','{}','unknown',?2)",params![k.id,now()]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,'__organization','organization','Target Group','done','{\"prior\":true}')",[&k.id]).unwrap();
    let deadline = now() + 86400;
    c.execute("INSERT INTO asset_cleanup_provider_backoffs VALUES(?1,'dns',?2,'DNS SERVFAIL; fallback error sending request',?3)",params![k.id,deadline,now()]).unwrap();
    c.execute(
        "INSERT INTO asset_cleanup_provider_backoffs VALUES(?1,'ct',?2,'HTTP 429 quota',?3)",
        params![k.id, deadline, now()],
    )
    .unwrap();
    for (host, attempts, next) in [
        ("shared-untried", 0, deadline),
        ("already-tried", 4, deadline),
        ("independent", 0, deadline + 3600),
    ] {
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,attempts,next_at) VALUES(?1,'pending-owner','dns_cname',?2,'pending',?3,?4)",params![k.id,host,attempts,next]).unwrap();
    }
    work::ensure_runtime_indexes(&c).unwrap();
    assert_eq!(state(&c, &k, "pending-owner"), "pending");
    for (key, expected) in [
        ("manual", "held"),
        ("done", "excluded"),
        ("undo", "restored"),
    ] {
        assert_eq!(state(&c, &k, key), expected);
    }
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_models WHERE id='never-repeat'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown"
    );
    assert_eq!(
        c.query_row(
            "SELECT result_json FROM asset_cleanup_research WHERE kind='organization'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "{\"prior\":true}"
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE kind='organization'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    assert!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_research WHERE target='shared-untried'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
            <= now() + 300
    );
    assert_eq!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_research WHERE target='already-tried'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        deadline
    );
    assert_eq!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_research WHERE target='independent'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        deadline + 3600
    );
    assert_eq!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_provider_backoffs WHERE provider='ct'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        deadline
    );
    let writes: i64 = c
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    work::ensure_runtime_indexes(&c).unwrap();
    assert_eq!(
        c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        writes
    );
}
#[test]
fn cleanup_public_website_batch_is_bounded_durable_no_duplicate_and_pause_discards_late_results() {
    let (mut c, k) = tests::fixture();
    for id in 1..=6 {
        let key = format!("domain:owner{id}.test");
        c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,?2,?3,'evidence')",params![k.id,key,format!("owner{id}.test")]).unwrap();
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target) VALUES(?1,?2,'public_ownership',?3)",params![k.id,key,format!("owner{id}.test")]).unwrap();
    }
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target) VALUES(?1,'domain:owner1.test','ct_certificate','123')",[&k.id]).unwrap();
    let next = network::next_research(&c, &k).unwrap().unwrap();
    assert_eq!(next.2, "public_ownership");
    let rows = network::claim_website_batch_for_test(&mut c, &k).unwrap();
    assert_eq!(rows.len(), 4);
    for (id, _, _, _) in &rows {
        let record: (String, i64, String) = c
            .query_row(
                "SELECT state,attempts,claim_token FROM asset_cleanup_research WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(record, ("running".into(), 1, k.token.clone()));
    }
    let other = network::claim_website_batch_for_test(&mut c, &k).unwrap();
    assert_eq!(other.len(), 2);
    assert!(other
        .iter()
        .all(|row| rows.iter().all(|first| row.0 != first.0)));
    let (id, key, _, attempts) = &rows[0];
    assert!(network::finish_website_for_test(
        &mut c,
        &k,
        *id,
        key,
        *attempts,
        Err("owner website HTTP403".into())
    )
    .unwrap());
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_provider_backoffs",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "one site must never block independent websites or CT"
    );
    c.execute(
        "UPDATE asset_cleanup_runs SET enabled=0,generation=generation+1,phase='paused'",
        [],
    )
    .unwrap();
    let (id, key, _, attempts) = &rows[1];
    assert!(!network::finish_website_for_test(
        &mut c,
        &k,
        *id,
        key,
        *attempts,
        Ok(json!({"facts":[]}))
    )
    .unwrap());
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=?1",
            [id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "retry"
    );
    let protected = rows[2].0;
    c.execute(
        "UPDATE asset_cleanup_research SET claim_token='replacement-worker' WHERE id=?1",
        [protected],
    )
    .unwrap();
    network::settle_public_for_test(
        &mut c,
        &k,
        &rows.iter().map(|row| row.0).collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE id=?1",
            [protected],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "running",
        "late old batch must never release a replacement worker"
    );
    assert_eq!(c.query_row("SELECT count(*) FROM asset_cleanup_research WHERE id IN (?1,?2,?3) AND state='running' AND claim_token=?4",params![rows[0].0,rows[1].0,rows[3].0,k.token],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(network::claim_website_batch_for_test(&mut c, &k)
        .unwrap()
        .is_empty());
}
#[test]
fn cleanup_repeated_subjects_do_not_fake_truncation_and_large_family_checks_every_name() {
    for deep_last in [false, true] {
        let (mut c, k) = tests::fixture();
        for id in 1..=160 {
            let name = if id == 1 {
                "college.test".to_owned()
            } else {
                format!("repeat{id}.college.test")
            };
            tests::add(&c, &k, id, &name, "");
            provider(&c, id, "*.college.test", "Example College");
        }
        for id in 161..=201 {
            let name = if deep_last && id == 201 {
                "hidden.zx.college.test".to_owned()
            } else {
                format!("n{id}.college.test")
            };
            tests::add(&c, &k, id, &name, "");
        }
        tests::prepared(&mut c, &k);
        let key = "domain:college.test";
        website(&c, &k, key, "college.test", "Example College", "");
        let e = work::evidence_for(&c, &k, key).unwrap();
        assert_eq!(e["evidenceTruncated"], false);
        assert_eq!(e["namesTruncated"], true);
        let fact = e["domainEvidence"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["source"] == "provider_subject_cn")
            .unwrap();
        assert_eq!(fact["assetCount"], 160);
        assert_eq!(fact["assetIds"].as_array().unwrap().len(), 16);
        advice(&c, &k, key);
        work::reclassify(&mut c, &k, key).unwrap();
        assert_eq!(state(&c,&k,key),if deep_last{"evidence"}else{"excluded"},"last name outside one-label certificate must block family exclusion even after the UI/AI sample cutoff");
    }
}
#[test]
fn cleanup_bound_official_unit_relationship_preserves_subsidiary_without_granting_new_scope() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "unit.test", "");
    provider(&c, 1, "unit.test", "Subsidiary Company Limited");
    tests::prepared(&mut c, &k);
    let key = "domain:unit.test";
    website(&c, &k, key, "unit.test", "Subsidiary Company Limited", "");
    let initial = work::basis(&work::evidence_for(&c, &k, key).unwrap());
    let fact = json!({"facts":[{"host":"unit.test","organization":"Subsidiary Company Limited","parentOrganization":"Target","source":"official_unit_relation","evidenceTier":"declared","sourceUrl":"https://unit.test/about","bodySha256":"b".repeat(64),"notAfter":now()+86400}]});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'public_ownership','www.unit.test','done',?3)",params![k.id,key,fact.to_string()]).unwrap();
    assert_ne!(
        initial,
        work::basis(&work::evidence_for(&c, &k, key).unwrap())
    );
    advice(&c, &k, key);
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "retained");
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
    assert!(roots(&c, 1).unwrap().is_empty());
}
#[test]
fn cleanup_only_contradictory_bound_operators_enter_conflict_not_absent_evidence() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "bank.test", "");
    tests::prepared(&mut c, &k);
    let key = "domain:bank.test";
    website(
        &c,
        &k,
        key,
        "bank.test",
        "Example University",
        "no_registration",
    );
    let extra = json!({"facts":[
        {"host":"bank.test","organization":"Example University","source":"public_site_certificate","notAfter":now()+86400},
        {"host":"bank.test","organization":"Different Bank Limited","source":"public_ct_subject","notAfter":now()+86400},
        {"host":"bank.test","organization":"Different Bank Limited","source":"official_site_operator","evidenceTier":"declared","sourceUrl":"https://bank.test/about","bodySha256":"b".repeat(64),"notAfter":now()+86400}
    ]});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate','second-independent-owner','done',?3)",params![k.id,key,extra.to_string()]).unwrap();
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "conflict");
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
}
#[test]
fn cleanup_http_operator_requires_registration_and_current_independent_certificate() {
    for case in [
        "old_provider_only",
        "current_certificate",
        "no_registration",
        "unmarked_http",
    ] {
        let (mut c, k) = tests::fixture();
        tests::add(&c, &k, 1, "bank.test", "");
        if case == "old_provider_only" {
            provider(&c, 1, "bank.test", "Example University");
        }
        tests::prepared(&mut c, &k);
        let key = "domain:bank.test";
        website(
            &c,
            &k,
            key,
            "bank.test",
            "Example University",
            if case == "no_registration" {
                "no_registration"
            } else {
                ""
            },
        );
        let raw: String = c
            .query_row(
                "SELECT result_json FROM asset_cleanup_research WHERE kind='public_ownership'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut result: Value = serde_json::from_str(&raw).unwrap();
        for fact in result["facts"].as_array_mut().unwrap() {
            fact["sourceUrl"] = json!("http://bank.test/");
            if case != "unmarked_http" {
                fact["transport"] = json!("http");
            }
        }
        c.execute(
            "UPDATE asset_cleanup_research SET result_json=?1 WHERE kind='public_ownership'",
            [result.to_string()],
        )
        .unwrap();
        if case != "old_provider_only" {
            let current = json!({"facts":[{"host":"bank.test","organization":"Example University","source":"public_ct_subject","notAfter":now()+86400}]});
            c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate','http-independent-current-cert','done',?3)",params![k.id,key,current.to_string()]).unwrap();
        }
        advice(&c, &k, key);
        work::reclassify(&mut c, &k, key).unwrap();
        assert_eq!(
            state(&c, &k, key),
            if case == "current_certificate" {
                "excluded"
            } else {
                "evidence"
            },
            "{case}"
        );
    }
}
#[test]
fn cleanup_expired_website_results_refresh_in_bounded_owner_passes_and_preserve_receipts() {
    let (mut c, k) = tests::fixture();
    for id in 1..=101 {
        let key = format!("domain:old{id}.test");
        c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,?2,?3,'evidence')",params![k.id,key,format!("old{id}.test")]).unwrap();
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'public_ownership',?3,'done',?4)",params![k.id,key,format!("old{id}.test"),json!({"checkedAt":now()-8*86400,"facts":[],"receiptId":id}).to_string()]).unwrap();
    }
    for (key, state, raw) in [
        (
            "kept",
            "retained",
            json!({"checkedAt":now()-9*86400}).to_string(),
        ),
        ("fresh", "evidence", json!({"checkedAt":now()}).to_string()),
        ("corrupt", "evidence", "not-json".into()),
    ] {
        c.execute(
            "INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,?2,?2,?3)",
            params![k.id, key, state],
        )
        .unwrap();
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'public_ownership',?2,'done',?3)",params![k.id,key,raw]).unwrap();
    }
    let receipts: Vec<String> = c
        .prepare("SELECT result_json FROM asset_cleanup_research ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(work::refresh_expired_for_test(&mut c, &k).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_research WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        100
    );
    assert!(work::refresh_expired_for_test(&mut c, &k).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_research WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        101
    );
    let writes: i64 = c
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    assert!(!work::refresh_expired_for_test(&mut c, &k).unwrap());
    assert_eq!(
        c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        writes
    );
    let after: Vec<String> = c
        .prepare("SELECT result_json FROM asset_cleanup_research ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(receipts, after);
    for key in ["kept", "fresh", "corrupt"] {
        assert_eq!(
            c.query_row(
                "SELECT state FROM asset_cleanup_research WHERE target=?1",
                [key],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "done"
        );
    }
    c.execute(
        "UPDATE asset_cleanup_research SET state='done' WHERE target='old1.test'",
        [],
    )
    .unwrap();
    c.execute(
        "UPDATE asset_cleanup_groups SET state='evidence' WHERE group_key='domain:old1.test'",
        [],
    )
    .unwrap();
    c.execute(
        "UPDATE asset_cleanup_runs SET enabled=0,generation=generation+1",
        [],
    )
    .unwrap();
    assert!(!work::refresh_expired_for_test(&mut c, &k).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT state FROM asset_cleanup_research WHERE target='old1.test'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "done"
    );
}
#[test]
fn cleanup_shared_platform_owner_never_excludes_independent_customer_tenants() {
    for (root, tenant, operator) in [
        ("51job.com", "cmdi.51job.com", "前锦网络信息技术有限公司"),
        (
            "csdn.net",
            "10086id.csdn.net",
            "北京创新乐知网络技术有限公司",
        ),
        ("163.com", "eggy-10086.163.com", "网易计算机系统有限公司"),
        ("139site.com", "tenant.139site.com", "独立平台运营有限公司"),
    ] {
        for legacy in [false, true] {
            let (mut c, k) = tests::fixture();
            tests::add(&c, &k, 1, tenant, "");
            provider(&c, 1, &format!("*.{root}"), operator);
            tests::prepared(&mut c, &k);
            let current = format!("domain:{tenant}");
            assert_eq!(
                c.query_row(
                    "SELECT domain FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
                    params![k.id, current],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                tenant,
                "new shared snapshots isolate the actual tenant host"
            );
            let key = if legacy {
                // Explicitly simulate the old root-family snapshot. Updating
                // all three fixture projections keeps the provenance graph
                // consistent; production snapshots are never rewritten here.
                let key = format!("domain:{root}");
                c.execute("UPDATE asset_cleanup_groups SET group_key=?3,domain=?4,shared=0 WHERE run_id=?1 AND group_key=?2",params![k.id,current,key,root]).unwrap();
                for table in ["asset_cleanup_seeds", "asset_cleanup_links"] {
                    c.execute(
                        &format!(
                            "UPDATE {table} SET group_key=?3 WHERE run_id=?1 AND group_key=?2"
                        ),
                        params![k.id, current, key],
                    )
                    .unwrap();
                }
                key
            } else {
                current
            };
            let page = if legacy { root } else { tenant };
            website(&c, &k, &key, page, operator, "");
            // A current tenant leaf with the platform operator's legal name
            // still does not establish which customer controls the tenant.
            let ca = json!({"facts":[{"host":tenant,"organization":operator,"source":"public_site_certificate","notAfter":now()+86400}]});
            c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate','tenant-platform-ca','done',?3)",params![k.id,key,ca.to_string()]).unwrap();
            assert_eq!(
                work::evidence_for(&c, &k, &key).unwrap()["shared"],
                true,
                "legacy={legacy}"
            );
            advice(&c, &k, &key);
            work::reclassify(&mut c, &k, &key).unwrap();
            assert_eq!(state(&c, &k, &key), "deferred", "legacy={legacy}");
            assert!(exclusions::rules(&c, 1).unwrap().is_empty());
        }
    }
}
#[test]
fn cleanup_http_footer_cannot_upgrade_https_operator_with_only_historical_provider_subject() {
    for root_name in ["bank.test", "www.bank.test"] {
        let (mut c, k) = tests::fixture();
        tests::add(&c, &k, 1, "bank.test", "");
        provider(&c, 1, "bank.test", "Example University");
        tests::prepared(&mut c, &k);
        let key = "domain:bank.test";
        website(&c, &k, key, root_name, "Example University", "");
        let raw: String = c
            .query_row(
                "SELECT result_json FROM asset_cleanup_research WHERE kind='public_ownership'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut receipt: Value = serde_json::from_str(&raw).unwrap();
        receipt["facts"][1]["sourceUrl"] = json!(format!("http://{root_name}/"));
        receipt["facts"][1]["transport"] = json!("http");
        c.execute(
            "UPDATE asset_cleanup_research SET result_json=?1 WHERE kind='public_ownership'",
            [receipt.to_string()],
        )
        .unwrap();
        advice(&c, &k, key);
        work::reclassify(&mut c, &k, key).unwrap();
        assert_eq!(state(&c, &k, key), "evidence");
        assert!(exclusions::rules(&c, 1).unwrap().is_empty());
    }
}
#[test]
fn cleanup_website_only_declarations_wait_for_independent_evidence_before_spending_model_calls() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "bank.test", "");
    tests::prepared(&mut c, &k);
    let key = "domain:bank.test";
    website(&c, &k, key, "bank.test", "Example University", "");
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(work::pending_model_groups(&c, &k.id).unwrap(), 0);
    assert_eq!(c.query_row("SELECT json_extract(evidence_json,'$.websiteCrosscheckReady') FROM asset_cleanup_groups",[],|r|r.get::<_,bool>(0)).unwrap(),false);
    let ca = json!({"facts":[{"host":"bank.test","organization":"Example University","source":"public_site_certificate","notAfter":now()+86400}]});
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'ct_certificate','independent-ca-identity','done',?3)",params![k.id,key,ca.to_string()]).unwrap();
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(work::pending_model_groups(&c, &k.id).unwrap(), 1);
    assert_eq!(c.query_row("SELECT json_extract(evidence_json,'$.websiteCrosscheckReady') FROM asset_cleanup_groups",[],|r|r.get::<_,bool>(0)).unwrap(),true);
}

fn controller(
    c: &Connection,
    k: &work::Claim,
    key: &str,
    name: &str,
    owner: &str,
    parent: &str,
    mutation: &str,
) {
    let mut fact = json!({"host":name,"organization":owner,"parentOrganization":parent,"source":"official_unit_relation","relationship":"ultimate_controller_declared","evidenceTier":"declared","sourceUrl":format!("https://{name}/about"),"bodySha256":"c".repeat(64),"notAfter":now()+86400});
    if mutation == "expired" {
        fact["notAfter"] = json!(now() - 1);
    }
    if mutation == "wrong_origin" {
        fact["sourceUrl"] = json!("https://elsewhere.test/about");
    }
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,result_json) VALUES(?1,?2,'public_ownership',?3,'done',?4)",params![k.id,key,format!("{name}/about"),json!({"kind":"public_ownership","facts":[fact]}).to_string()]).unwrap();
}
fn legacy_owned_exclusion(
    c: &mut Connection,
    k: &work::Claim,
    key: &str,
    domain: &str,
    batch: &str,
) {
    let tx = c.transaction().unwrap();
    let receipt = exclusions::cleanup_apply_on(&tx, k.p, &[domain.into()], batch).unwrap();
    tx.execute(
        "INSERT INTO asset_cleanup_batches VALUES(?1,?2,?3,?4,?5,0,?6)",
        params![
            batch,
            k.id,
            k.p,
            json!([domain]).to_string(),
            receipt.to_string(),
            now()
        ],
    )
    .unwrap();
    tx.execute(
        "UPDATE asset_cleanup_groups SET state='excluded' WHERE run_id=?1 AND group_key=?2",
        params![k.id, key],
    )
    .unwrap();
    tx.commit().unwrap();
}
#[test]
fn cleanup_corporate_identity_never_proves_unrelated_group_even_with_ai_and_unknown_controller() {
    for (owner, parent, mutation) in [
        ("启明星辰信息技术集团股份有限公司", "", ""),
        ("独立运营子公司有限公司", "未知控股集团有限公司", ""),
        (
            "Example Corporation Limited",
            "Unknown Parent Corporation",
            "",
        ),
        ("Example Company LLC", "Target", "expired"),
        ("Example Corp Inc.", "Target", "wrong_origin"),
    ] {
        let (mut c, k) = tests::fixture();
        tests::add(&c, &k, 1, "subsidiary.test", owner);
        provider(&c, 1, "subsidiary.test", owner);
        tests::prepared(&mut c, &k);
        let key = "domain:subsidiary.test";
        website(&c, &k, key, "subsidiary.test", owner, "");
        if !parent.is_empty() {
            controller(&c, &k, key, "subsidiary.test", owner, parent, mutation);
        }
        advice(&c, &k, key);
        work::reclassify(&mut c, &k, key).unwrap();
        assert_eq!(
            state(&c, &k, key),
            "evidence",
            "{owner} / {parent} / {mutation}"
        );
        let evidence: String = c
            .query_row(
                "SELECT evidence_json FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",
                params![k.id, key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&evidence).unwrap()["corporateRelationshipUnverified"],
            true
        );
        assert!(exclusions::rules(&c, k.p).unwrap().is_empty());
        assert!(
            !work::settle_completed_groups(&mut c, &k).unwrap(),
            "untried CT/DNS must still be collected"
        );
        c.execute("UPDATE asset_cleanup_research SET state='exhausted',attempts=2,error='HTTP 502 bounded failure' WHERE run_id=?1 AND state IN ('pending','retry')",[&k.id]).unwrap();
        assert!(work::settle_completed_groups(&mut c, &k).unwrap());
        let (state,reason):(String,String)=c.query_row("SELECT state,json_extract(evidence_json,'$.completionReason') FROM asset_cleanup_groups WHERE run_id=?1 AND group_key=?2",params![k.id,key],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(
            (state.as_str(), reason.as_str()),
            ("deferred", "relationship_unverified")
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM asset_cleanup_models", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "policy correction must not issue a paid request"
        );
    }
}
#[test]
fn cleanup_new_positive_relation_restores_only_its_owned_batch_and_retains_subsidiary() {
    let (mut c, k) = tests::fixture();
    let owner = "启明星辰信息技术集团股份有限公司";
    tests::add(&c, &k, 1, "venus-shaped.test", owner);
    provider(&c, 1, "venus-shaped.test", owner);
    tests::prepared(&mut c, &k);
    let key = "domain:venus-shaped.test";
    website(&c, &k, key, "venus-shaped.test", owner, "");
    advice(&c, &k, key);
    legacy_owned_exclusion(
        &mut c,
        &k,
        key,
        "venus-shaped.test",
        "legacy-company-exclusion",
    );
    controller(&c, &k, key, "venus-shaped.test", owner, "Target", "");
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "retained");
    assert!(exclusions::rules(&c, k.p).unwrap().is_empty());
    let (restored,raw):(bool,String)=c.query_row("SELECT restored,receipt_json FROM asset_cleanup_batches WHERE id='legacy-company-exclusion'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let receipt: Value = serde_json::from_str(&raw).unwrap();
    assert!(restored);
    assert_eq!(receipt["restoreResult"]["restored"], 1);
    assert_eq!(
        receipt["restoreReason"],
        "corporate_relationship_policy_recheck"
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_discovery_exclusion_events WHERE action='restore'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "retained");
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_discovery_exclusion_events WHERE action='restore'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "policy recovery is idempotent"
    );
    assert!(
        roots(&c, k.p).unwrap().is_empty(),
        "relevance does not grant probe scope"
    );
    assert_eq!(
        c.query_row("SELECT auto_exclude FROM asset_cleanup_runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
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
}
#[test]
fn cleanup_unknown_corporation_recovers_legacy_rule_but_never_a_later_external_decision() {
    for later_manual in [false, true] {
        let (mut c, k) = tests::fixture();
        let owner = "Unknown Subsidiary Company Limited";
        tests::add(&c, &k, 1, "unknown-company.test", owner);
        provider(&c, 1, "unknown-company.test", owner);
        tests::prepared(&mut c, &k);
        let key = "domain:unknown-company.test";
        website(&c, &k, key, "unknown-company.test", owner, "");
        advice(&c, &k, key);
        legacy_owned_exclusion(&mut c, &k, key, "unknown-company.test", "legacy-owned");
        if later_manual {
            c.execute("INSERT INTO asset_discovery_exclusion_events(project_id,domain,action,receipt_json) VALUES(1,'unknown-company.test','apply','{\"manual\":true}')",[]).unwrap();
        }
        work::reclassify(&mut c, &k, key).unwrap();
        assert_eq!(
            state(&c, &k, key),
            if later_manual { "excluded" } else { "evidence" }
        );
        assert_eq!(
            exclusions::rules(&c, k.p).unwrap().is_empty(),
            !later_manual
        );
        assert_eq!(
            c.query_row(
                "SELECT restored FROM asset_cleanup_batches WHERE id='legacy-owned'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap(),
            !later_manual
        );
    }
}
#[test]
fn cleanup_manual_review_and_nonowned_exclusion_are_not_overwritten_by_corporate_recheck() {
    for manual_review in [false, true] {
        let (mut c, k) = tests::fixture();
        let owner = "Unknown Subsidiary Company Limited";
        tests::add(&c, &k, 1, "manual-company.test", owner);
        provider(&c, 1, "manual-company.test", owner);
        tests::prepared(&mut c, &k);
        let key = "domain:manual-company.test";
        website(&c, &k, key, "manual-company.test", owner, "");
        advice(&c, &k, key);
        legacy_owned_exclusion(&mut c, &k, key, "manual-company.test", "legacy-owned");
        if manual_review {
            c.execute_batch("CREATE TABLE asset_seed_reviews(id TEXT PRIMARY KEY,project_id INTEGER,organization TEXT);CREATE TABLE asset_seed_review_items(run_id TEXT,item_key TEXT,decision TEXT,evidence_json TEXT);INSERT INTO asset_seed_reviews VALUES('manual-review',1,'Target Group');INSERT INTO asset_seed_review_items VALUES('manual-review','one','rejected','{\"domains\":[\"manual-company.test\"]}');").unwrap();
        } else {
            c.execute(
                "DELETE FROM asset_cleanup_batches WHERE id='legacy-owned'",
                [],
            )
            .unwrap();
        }
        controller(&c, &k, key, "manual-company.test", owner, "Target", "");
        work::reclassify(&mut c, &k, key).unwrap();
        assert_eq!(state(&c, &k, key), "excluded");
        assert_eq!(
            exclusions::rules(&c, k.p).unwrap(),
            vec!["manual-company.test"]
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM asset_discovery_exclusion_events WHERE action='restore'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
#[test]
fn cleanup_restored_group_can_accept_positive_relationship_without_reexcluding_unknown_company() {
    let (mut c, k) = tests::fixture();
    let owner = "Subsidiary Company Limited";
    tests::add(&c, &k, 1, "restored-company.test", owner);
    provider(&c, 1, "restored-company.test", owner);
    tests::prepared(&mut c, &k);
    let key = "domain:restored-company.test";
    website(&c, &k, key, "restored-company.test", owner, "");
    advice(&c, &k, key);
    c.execute(
        "UPDATE asset_cleanup_groups SET state='restored' WHERE run_id=?1",
        [&k.id],
    )
    .unwrap();
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "restored");
    controller(&c, &k, key, "restored-company.test", owner, "Target", "");
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(state(&c, &k, key), "retained");
    assert!(exclusions::rules(&c, k.p).unwrap().is_empty());
}

#[test]
fn cleanup_expired_or_missing_old_corporate_proof_restores_owned_rule_without_inventing_identity() {
    for expiry in ["expired", "absent"] {
        let (mut c, k) = tests::fixture();
        tests::add(&c, &k, 1, "expired-company.test", "");
        tests::prepared(&mut c, &k);
        let key = "domain:expired-company.test";
        if expiry == "expired" {
            website(
                &c,
                &k,
                key,
                "expired-company.test",
                "Corporate Operator Limited",
                "expired",
            );
        }
        advice(&c, &k, key);
        legacy_owned_exclusion(
            &mut c,
            &k,
            key,
            "expired-company.test",
            "legacy-stale-company",
        );
        work::reclassify(&mut c, &k, key).unwrap();
        assert_eq!(state(&c, &k, key), "evidence", "{expiry}");
        assert!(
            exclusions::rules(&c, k.p).unwrap().is_empty(),
            "old receipt must satisfy current exclusion gate: {expiry}"
        );
        assert_eq!(
            c.query_row(
                "SELECT restored FROM asset_cleanup_batches WHERE id='legacy-stale-company'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap(),
            true
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
