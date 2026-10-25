use super::*;

#[test]
fn cleanup_queue_status_separates_runnable_retries_from_parked_history() {
    let c = Connection::open_in_memory().unwrap();
    ensure(&c).unwrap();
    for (key, state) in [
        ("active", "evidence"),
        ("kept", "retained"),
        ("excluded", "excluded"),
    ] {
        c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES('one',?1,'example.test',?2)",params![key,state]).unwrap();
    }
    for (key, kind, state, next, error) in [
        ("active", "ct_search", "done", 0, ""),
        ("kept", "dns_cname", "done", 0, ""),
        ("active", "ct_search", "pending", 900, ""),
        ("active", "dns_cname", "pending", 950, ""),
        ("active", "dns_cname", "retry", 800, "temporary DNS failure"),
        ("active", "ct_certificate", "running", 0, ""),
        ("__organization", "organization", "pending", 950, ""),
        ("kept", "dns_cname", "pending", 0, "obsolete error"),
        ("excluded", "ct_certificate", "retry", 0, "old CT error"),
    ] {
        let count: i64 = c
            .query_row("SELECT count(*) FROM asset_cleanup_research", [], |r| {
                r.get(0)
            })
            .unwrap();
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,next_at,error) VALUES('one',?1,?2,?3,?4,?5,?6)",params![key,kind,format!("target-{count}"),state,next,error]).unwrap();
    }
    c.execute("INSERT INTO asset_cleanup_provider_backoffs VALUES('one','dns',2000,'resolver temporarily unavailable',1000)",[]).unwrap();
    c.execute("INSERT INTO asset_cleanup_provider_backoffs VALUES('other-run','ct',3000,'other run',1000)",[]).unwrap();
    let before: i64 = c
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    let v = research_status(&c, "one", 1000).unwrap();
    assert_eq!(v["completed"], 2);
    assert_eq!(v["pending"], 5);
    assert_eq!(v["pendingQueued"], 3);
    assert_eq!(v["retrying"], 1);
    assert_eq!(v["running"], 1);
    assert_eq!(v["parked"], 2);
    assert_eq!(v["errors"], 1);
    assert_eq!(v["due"], 2);
    assert_eq!(v["nextAt"], 1000);
    assert_eq!(v["providerWaits"].as_array().unwrap().len(), 1);
    assert_eq!(v["providerWaits"][0]["provider"], "dns");
    assert_eq!(v["recentErrors"].as_array().unwrap().len(), 1);
    assert_eq!(v["recentErrors"][0]["nextAt"], 2000);
    assert_eq!(v["recentErrors"][0]["taskNextAt"], 800);
    assert_eq!(
        c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn cleanup_queue_status_legacy_read_never_creates_provider_table() {
    let c = Connection::open_in_memory().unwrap();
    ensure(&c).unwrap();
    c.execute_batch("DROP TABLE asset_cleanup_provider_backoffs")
        .unwrap();
    c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES('one','active','example.test','evidence')",[]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,next_at,error) VALUES('one','active','ct_search','example.test','retry',2000,'HTTP 502')",[]).unwrap();
    let v = research_status(&c, "one", 1000).unwrap();
    assert_eq!(v["pending"], 1);
    assert_eq!(v["retrying"], 1);
    assert_eq!(v["due"], 0);
    assert_eq!(v["nextAt"], 2000);
    assert_eq!(v["providerWaits"], json!([]));
    assert!(!has(&c, "asset_cleanup_provider_backoffs").unwrap());
}

#[test]
fn cleanup_model_queue_reports_local_evidence_without_any_public_source() {
    let (mut c, k) = tests::fixture();
    tests::add(&c, &k, 1, "www.bank.test", "Other Bank");
    tests::prepared(&mut c, &k);
    work::reclassify(&mut c, &k, "domain:bank.test").unwrap();
    assert_eq!(work::pending_model_groups(&c, &k.id).unwrap(), 1);
    c.execute(
        "INSERT INTO asset_cleanup_provider_backoffs VALUES(?1,'ct',?2,'HTTP 502',?3)",
        params![k.id, now() + 86400, now()],
    )
    .unwrap();
    assert_eq!(work::pending_model_groups(&c, &k.id).unwrap(), 1);
    let basis: String = c
        .query_row("SELECT basis_hash FROM asset_cleanup_groups", [], |r| {
            r.get(0)
        })
        .unwrap();
    c.execute(
        "INSERT INTO asset_cleanup_model_items VALUES('unknown-request',?1,'domain:bank.test',?2)",
        params![k.id, basis],
    )
    .unwrap();
    assert_eq!(
        work::pending_model_groups(&c, &k.id).unwrap(),
        0,
        "unknown paid request must not be replayed"
    );
}

#[test]
fn cleanup_waiting_and_analyzing_phases_consume_local_batch_before_due_public_request() {
    use std::sync::atomic::Ordering;
    for phase in ["waiting_evidence", "analyzing"] {
        let (mut c, k) = tests::fixture();
        tests::add(&c, &k, 1, "www.bank.test", "Other Bank");
        tests::prepared(&mut c, &k);
        work::reclassify(&mut c, &k, "domain:bank.test").unwrap();
        c.execute("UPDATE asset_cleanup_research SET state='done'", [])
            .unwrap();
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state) VALUES(?1,'__organization','organization','Target Group','done')",[&k.id]).unwrap();
        // If public work runs first, this deliberately unsupported fixture kind
        // fails locally before transport and becomes retry. It can never make
        // an external request, even when this regression is reintroduced.
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target) VALUES(?1,'domain:bank.test','fixture_invalid_no_transport','www.bank.test')",[&k.id]).unwrap();
        c.execute("UPDATE asset_cleanup_runs SET phase=?1", [phase])
            .unwrap();
        let (base, calls, server) = tests::cleanup_provider(|request| {
            assert!(request["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("Other Bank"));
            (200, tests::cleanup_answer("evidence"))
        });
        tests::configure_cleanup_provider(&c, &base);
        assert!(work::scheduler_step(Path::new("unused-fixture-path"), &mut c, &k).unwrap());
        server.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "{phase}");
        assert_eq!(c.query_row("SELECT state FROM asset_cleanup_research WHERE kind='fixture_invalid_no_transport'",[],|r|r.get::<_,String>(0)).unwrap(),"pending","{phase}");
        assert_eq!(
            c.query_row("SELECT state FROM asset_cleanup_models", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "applied",
            "{phase}"
        );
    }
}

fn local_leaf(c: &Connection, id: i64, leaf: &[u8], icp: &str) {
    use base64::Engine as _;
    let extra = json!({"probe_cert_der_base64":base64::engine::general_purpose::STANDARD.encode(leaf),"icp.name":icp});
    c.execute(
        "UPDATE assets SET extra_json=?2 WHERE id=?1",
        params![id, extra.to_string()],
    )
    .unwrap();
}
fn local_advice(c: &Connection, k: &work::Claim, key: &str) {
    let e = work::evidence_for(c, k, key).unwrap();
    c.execute("UPDATE asset_cleanup_groups SET advice='exclude',ai_basis_hash=?3 WHERE run_id=?1 AND group_key=?2",params![k.id,key,work::basis(&e)]).unwrap();
}
fn local_state(c: &Connection, key: &str) -> String {
    c.query_row(
        "SELECT state FROM asset_cleanup_groups WHERE group_key=?1",
        [key],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn cleanup_current_local_der_and_exact_endpoint_icp_can_exclude_without_public_request() {
    let (mut c, k) = tests::fixture();
    tests::add(
        &c,
        &k,
        1,
        "owned.cleanup-fixture.test",
        "Other Bank Limited",
    );
    local_leaf(
        &c,
        1,
        include_bytes!("../../test-fixtures/tls/cleanup-organization.der"),
        "Other Bank Limited",
    );
    tests::prepared(&mut c, &k);
    let key = "domain:cleanup-fixture.test";
    let e = work::evidence_for(&c, &k, key).unwrap();
    assert!(e["domainEvidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["source"] == "leaf_subject" && f["currentLocalLeaf"] == true));
    local_advice(&c, &k, key);
    work::reclassify(&mut c, &k, key).unwrap();
    assert_eq!(local_state(&c, key), "excluded");
    assert_eq!(
        exclusions::rules(&c, 1).unwrap(),
        vec!["cleanup-fixture.test"]
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_research", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
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
    let tx = c.unchecked_transaction().unwrap();
    work::restore_on(&tx, 1, &k.id, None).unwrap();
    tx.commit().unwrap();
    assert!(exclusions::rules(&c, 1).unwrap().is_empty());
    assert_eq!(local_state(&c, key), "restored");
}

#[test]
fn cleanup_provider_metadata_and_expired_local_der_remain_evidence_even_with_matching_icp() {
    for case in ["metadata", "expired"] {
        let (mut c, k) = tests::fixture();
        let (h, owner) = if case == "expired" {
            ("owned.example.test", "Synthetic Mobile Subsidiary")
        } else {
            ("owned.cleanup-fixture.test", "Other Bank Limited")
        };
        tests::add(&c, &k, 1, h, owner);
        if case == "expired" {
            local_leaf(
                &c,
                1,
                include_bytes!("../../test-fixtures/tls/subject-organization.der"),
                owner,
            );
        } else {
            c.execute("UPDATE assets SET extra_json=?1",[json!({"icp.name":owner,"cert.subject.cn":h,"cert.subject.org":owner,"probe_cert_captured_at":chrono::Utc::now().to_rfc3339(),"cert.not_after":"2099-01-01"}).to_string()]).unwrap();
        }
        tests::prepared(&mut c, &k);
        let key = if case == "expired" {
            "domain:example.test"
        } else {
            "domain:cleanup-fixture.test"
        };
        let e = work::evidence_for(&c, &k, key).unwrap();
        assert!(
            !e["domainEvidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["currentLocalLeaf"] == true),
            "{case}"
        );
        local_advice(&c, &k, key);
        work::reclassify(&mut c, &k, key).unwrap();
        assert_eq!(local_state(&c, key), "evidence", "{case}");
        assert!(exclusions::rules(&c, 1).unwrap().is_empty(), "{case}");
    }
}

#[test]
fn cleanup_local_der_cross_check_cannot_expand_to_other_hosts_or_override_protections() {
    for case in [
        "wrong_endpoint",
        "sibling",
        "wildcard",
        "wildcard_apex",
        "target_hint",
        "scope",
        "shared",
        "mixed",
        "changed_snapshot",
    ] {
        let (mut c, k) = tests::fixture();
        let h = if case == "wildcard" {
            "child.tenant.cleanup-fixture.test"
        } else if case == "wildcard_apex" {
            "tenant.cleanup-fixture.test"
        } else {
            "owned.cleanup-fixture.test"
        };
        tests::add(&c, &k, 1, h, "Other Bank Limited");
        local_leaf(
            &c,
            1,
            include_bytes!("../../test-fixtures/tls/cleanup-organization.der"),
            "Other Bank Limited",
        );
        if case == "wrong_endpoint" {
            c.execute("UPDATE assets SET host='https://other.cleanup-fixture.test',link='https://other.cleanup-fixture.test'",[]).unwrap();
        }
        if case == "sibling" {
            tests::add(&c, &k, 2, "unproven.cleanup-fixture.test", "");
        }
        if case == "target_hint" {
            tests::add(&c, &k, 2, "second.cleanup-fixture.test", "");
            c.execute(
                "UPDATE assets SET host=?1,link=?1 WHERE id=2",
                [format!("https://{h}")],
            )
            .unwrap();
            c.execute(
                "UPDATE asset_discovery_seeds SET host=?1,source='certificate_domain' WHERE id=2",
                [h],
            )
            .unwrap();
            c.execute("UPDATE asset_cleanup_seeds SET host=?1,source='certificate_domain' WHERE source_id=2",[h]).unwrap();
            c.execute(
                "UPDATE assets SET extra_json=?1 WHERE id=2",
                [json!({"cert.subject.cn":h,"cert.subject.org":"Target"}).to_string()],
            )
            .unwrap();
        }
        tests::prepared(&mut c, &k);
        let key = "domain:cleanup-fixture.test";
        if case == "scope" {
            c.execute(
                "INSERT INTO asset_discovery_scopes VALUES(1,'[\"cleanup-fixture.test\"]')",
                [],
            )
            .unwrap();
        }
        if case == "shared" {
            c.execute("UPDATE asset_cleanup_groups SET shared=1", [])
                .unwrap();
        }
        if case == "mixed" {
            c.execute("UPDATE asset_cleanup_facts SET mixed=1", [])
                .unwrap();
        }
        if case == "changed_snapshot" {
            c.execute("UPDATE assets SET extra_json='{}'", []).unwrap();
        }
        local_advice(&c, &k, key);
        work::reclassify(&mut c, &k, key).unwrap();
        assert_ne!(local_state(&c, key), "excluded", "{case}");
        assert!(exclusions::rules(&c, 1).unwrap().is_empty(), "{case}");
    }
}

#[test]
fn cleanup_owner_policy_migration_reclassifies_only_unresolved_groups_once() {
    let (c, k) = tests::fixture();
    for state in [
        "evidence", "conflict", "held", "retained", "excluded", "restored",
    ] {
        c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state,basis_hash,ai_basis_hash) VALUES(?1,?2,?2,?2,'unchanged-basis','unchanged-basis')",params![k.id,state]).unwrap();
    }
    c.execute("INSERT INTO asset_cleanup_models VALUES('old-request',?1,'fixture','{}','unknown','','{}','unknown charge',0)",[&k.id]).unwrap();
    c.execute("INSERT INTO asset_cleanup_model_items VALUES('old-request',?1,'evidence','unchanged-basis')",[&k.id]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,next_at,error,attempts) VALUES(?1,'evidence','dns_cname','legacy.example.test','retry',?2,'DNS返回错误或截断，未采纳为完整结果',4)",params![k.id,now()+86400]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,next_at,error,attempts) VALUES(?1,'evidence','dns_cname','limited.example.test','retry',?2,'HTTP 429',4)",params![k.id,now()+86400]).unwrap();
    c.execute(
        "INSERT INTO asset_cleanup_provider_backoffs VALUES(?1,'dns',?2,'HTTP 429',?3)",
        params![k.id, now() + 86400, now()],
    )
    .unwrap();
    let old_outage = now() + 86400;
    c.execute(
        "INSERT INTO asset_cleanup_provider_backoffs VALUES(?1,'ct',?2,'crt.sh HTTP 502',?3)",
        params![k.id, old_outage, now()],
    )
    .unwrap();
    for (target, state, attempts, next) in [
        ("untouched.example.test", "pending", 0, old_outage),
        ("already-tried.example.test", "retry", 4, old_outage),
        ("independent.example.test", "pending", 0, old_outage + 3600),
    ] {
        c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state,next_at,error,attempts) VALUES(?1,'evidence','ct_search',?2,?3,?4,'',?5)",params![k.id,target,state,next,attempts]).unwrap();
    }
    work::ensure_runtime_indexes(&c).unwrap();
    assert_eq!(
        local_state(&c, "evidence"),
        "evidence",
        "do not migrate an in-flight owner's publication state"
    );
    c.execute(
        "UPDATE asset_cleanup_runs SET claim_token='',lease_until=0,owner_pid=0",
        [],
    )
    .unwrap();
    work::ensure_runtime_indexes(&c).unwrap();
    assert_eq!(local_state(&c, "evidence"), "pending");
    for state in ["conflict", "held", "retained", "excluded", "restored"] {
        assert_eq!(local_state(&c, state), state);
    }
    assert_eq!(
        c.query_row("SELECT state FROM asset_cleanup_models", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "unknown"
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_model_items", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    let (next,attempts,error):(i64,i64,String)=c.query_row("SELECT next_at,attempts,error FROM asset_cleanup_research WHERE target='legacy.example.test'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(next <= now());
    assert_eq!(attempts, 4);
    assert_eq!(error, "DNS返回错误或截断，未采纳为完整结果");
    assert!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_research WHERE target='limited.example.test'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
            > now()
    );
    assert!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_provider_backoffs WHERE provider='dns'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
            > now() + 900,
        "429 provider circuit breaker remains authoritative"
    );
    let capped: i64 = c
        .query_row(
            "SELECT next_at FROM asset_cleanup_provider_backoffs WHERE provider='ct'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(capped <= now() + 900);
    assert_eq!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_research WHERE target='untouched.example.test'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        capped
    );
    assert_eq!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_research WHERE target='already-tried.example.test'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        old_outage
    );
    assert_eq!(
        c.query_row(
            "SELECT next_at FROM asset_cleanup_research WHERE target='independent.example.test'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        old_outage + 3600
    );
    let changes: i64 = c
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    work::ensure_runtime_indexes(&c).unwrap();
    assert_eq!(
        c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        changes,
        "policy migration must not keep resetting completed local work"
    );
}

#[test]
fn cleanup_owner_policy_migration_handles_legacy_missing_provider_table() {
    let (c, _k) = tests::fixture();
    c.execute_batch("DROP TABLE asset_cleanup_provider_backoffs; UPDATE asset_cleanup_runs SET claim_token='',owner_pid=0,lease_until=0;").unwrap();
    work::ensure_runtime_indexes(&c).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT version FROM asset_cleanup_policy_revisions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert!(!has(&c, "asset_cleanup_provider_backoffs").unwrap());
}

#[test]
fn cleanup_idle_provider_wait_is_not_progress_but_local_classification_is() {
    let (mut c, k) = tests::fixture();
    c.execute("INSERT INTO asset_cleanup_groups(run_id,group_key,domain,state) VALUES(?1,'domain:example.test','example.test','evidence')",[&k.id]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state) VALUES(?1,'__organization','organization','Target Group','done')",[&k.id]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,next_at) VALUES(?1,'domain:example.test','ct_search','example.test',?2)",params![k.id,now()+86400]).unwrap();
    c.execute("UPDATE asset_cleanup_runs SET phase='waiting_evidence',reason='已完成当前判断；缺证项保留并在公开来源退避到期后继续，冲突已集中汇总'",[]).unwrap();
    let before: i64 = c
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    assert!(!work::scheduler_step(Path::new("unused-fixture-path"), &mut c, &k).unwrap());
    assert_eq!(
        c.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before,
        "an unchanged wait must not rewrite its phase/reason"
    );
    c.execute("UPDATE asset_cleanup_groups SET state='pending'", [])
        .unwrap();
    assert!(work::scheduler_step(Path::new("unused-fixture-path"), &mut c, &k).unwrap());
    assert_eq!(local_state(&c, "domain:example.test"), "evidence");
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_cleanup_models", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn cleanup_full_local_reclassification_drains_pages_before_new_model_or_public_work() {
    let (mut c, k) = tests::fixture();
    for n in 1..=151 {
        tests::add(&c, &k, n, &format!("owned.unit{n}.test"), "Other Bank");
    }
    tests::prepared(&mut c, &k);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_cleanup_groups WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        151
    );
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target,state) VALUES(?1,'__organization','organization','Target Group','done')",[&k.id]).unwrap();
    c.execute("INSERT INTO asset_cleanup_research(run_id,group_key,kind,target) VALUES(?1,'domain:unit1.test','fixture_invalid_no_transport','owned.unit1.test')",[&k.id]).unwrap();
    // No model profile is configured. The first 151 local classifications must
    // not need one or reach a public provider, even though every completed page
    // creates eligible model work and due CT/DNS follow-up rows.
    for remaining in [101, 51, 1, 0] {
        assert!(work::scheduler_step(Path::new("unused-fixture-path"), &mut c, &k).unwrap());
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM asset_cleanup_groups WHERE state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            remaining
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM asset_cleanup_models", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(c.query_row("SELECT count(*) FROM asset_cleanup_research WHERE attempts>0 OR state IN ('running','retry')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    assert_eq!(work::pending_model_groups(&c,&k.id).unwrap(),151,"all local evidence is now available to model batches, independently of provider availability");
}
