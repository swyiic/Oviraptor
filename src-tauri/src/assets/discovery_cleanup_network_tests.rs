use super::*;

fn answer(status: i64) -> Value {
    json!({"Status":status,"TC":false,"Question":[{"name":"app.example.test.","type":5}]})
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
