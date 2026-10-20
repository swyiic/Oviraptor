use super::*;
fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);").unwrap();
    ensure(&c).unwrap();
    c
}
#[test]
fn seed_extraction_does_not_turn_wildcards_or_email_into_owned_hosts() {
    assert_eq!(
        host("https://Login.Example.com:443/path?q=x").as_deref(),
        Some("login.example.com")
    );
    for bad in [
        "user@example.com",
        "http://user:password@example.com",
        "127.0.0.1",
        "localhost",
        "foo..com",
        "com",
    ] {
        assert!(host(bad).is_none(), "{bad}");
    }
    assert!(!in_scope("example.com.attacker.test", "example.com"));
    assert!(!in_scope("badexample.com", "example.com"));
    assert!(in_scope("login.example.com", "example.com"));
    assert_eq!(
        field_hosts(&json!("*.example.com\nlogin.example.com")),
        vec![
            ("example.com".into(), true),
            ("login.example.com".into(), false)
        ]
    );
}
#[test]
fn actual_page_and_inline_script_names_are_extracted_but_not_executed() {
    let names = body_hosts(
        r#"<script>fetch('https://api.example.com/v1');const x='//cdn.other.test/a.js';</script><a href='https://login.example.com/'>login</a>"#,
    );
    assert_eq!(
        names,
        vec!["api.example.com", "cdn.other.test", "login.example.com"]
    );
    assert!(body_hosts("contact user@example.com, ordinary prose").is_empty());
    assert!(body_hosts(&format!(
        "{}https://hidden.example.com/",
        "x".repeat(524288)
    ))
    .is_empty());
}
#[test]
fn hunter_key_is_independent_and_never_returned_by_settings() {
    let mut c = fixture();
    let input = HunterSettingsInput {
        expected_revision: 0,
        key: Some("synthetic-hunter-secret".into()),
        clear_key: false,
        max_requests: 2,
        max_results: 10,
        network_mode: None,
    };
    let result = save_settings_on(&mut c, input).unwrap();
    assert_eq!(result["keyConfigured"], true);
    assert!(!result.to_string().contains("synthetic-hunter-secret"));
    assert!(save_settings_on(
        &mut c,
        HunterSettingsInput {
            expected_revision: 0,
            key: None,
            clear_key: true,
            max_requests: 2,
            max_results: 10,
            network_mode: None
        }
    )
    .is_err());
    save_settings_on(
        &mut c,
        HunterSettingsInput {
            expected_revision: 1,
            key: None,
            clear_key: false,
            max_requests: 2,
            max_results: 10,
            network_mode: None,
        },
    )
    .unwrap();
    assert!(setting(&c).unwrap().1.contains("synthetic"));
    save_settings_on(
        &mut c,
        HunterSettingsInput {
            expected_revision: 2,
            key: None,
            clear_key: true,
            max_requests: 2,
            max_results: 10,
            network_mode: None,
        },
    )
    .unwrap();
    assert!(!settings_on(&c).unwrap()["keyConfigured"].as_bool().unwrap());
}
#[test]
fn harvested_external_names_stay_unapproved_and_keep_their_original_sources() {
    let c = fixture();
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.com\"]')",
        [],
    )
    .unwrap();
    let rows=vec![(12,"login.example.com".into(),"https://login.example.com/".into(),json!({"cert.domain":"*.example.com\nother.test","cname":"shared.cloud.test","discovery_hosts_json":"[\"api.example.com\",\"tracker.other.test\"]"}).to_string())];
    ingest_observations(&c, 1, &rows).unwrap();
    ingest_observations(&c, 1, &rows).unwrap();
    let v = view(&c, 1).unwrap();
    assert_eq!(v["seeds"].as_array().unwrap().len(), 6);
    assert!(view(&c, 2).unwrap()["seeds"].as_array().unwrap().is_empty());
    for seed in v["seeds"].as_array().unwrap() {
        if seed["wildcard"] == true || seed["host"].as_str().unwrap().ends_with(".test") {
            assert_eq!(seed["eligible"], false);
        }
        assert_eq!(seed["reference"], "asset:12");
    }
}
#[test]
fn reservation_is_durable_before_network_and_never_retries_unknown_billing() {
    let mut c = fixture();
    let id = uuid::Uuid::new_v4().to_string();
    assert!(hunter_reserve(&mut c, 1, &id, "query-a", 1)
        .unwrap()
        .is_none());
    assert!(hunter_reserve(&mut c, 1, &id, "query-a", 1)
        .unwrap_err()
        .contains("不自动重放"));
    assert!(hunter_reserve(&mut c, 1, &id, "different", 1).is_err());
    assert!(
        hunter_reserve(&mut c, 1, &uuid::Uuid::new_v4().to_string(), "query-a", 1)
            .unwrap_err()
            .contains("未确认")
    );
    assert!(
        hunter_reserve(&mut c, 1, &uuid::Uuid::new_v4().to_string(), "query-b", 1)
            .unwrap_err()
            .contains("预算")
    );
    assert!(hunter_reserve(&mut c, 2, &id, "query-a", 1).is_err());
    assert!(hunter_reserve(&mut c, 2, &uuid::Uuid::new_v4().to_string(), "query-a", 1).is_ok());
}
#[test]
fn completed_queries_are_cached_and_do_not_reserve_another_paid_request() {
    let mut c = fixture();
    let id = uuid::Uuid::new_v4().to_string();
    hunter_reserve(&mut c, 1, &id, "q", 2).unwrap();
    c.execute("UPDATE asset_discovery_requests SET state='completed',result_json='{\"accepted\":3}' WHERE request_id=?1",[&id]).unwrap();
    assert_eq!(
        hunter_reserve(&mut c, 1, &id, "q", 2).unwrap().unwrap()["accepted"],
        3
    );
    assert_eq!(
        hunter_reserve(&mut c, 1, &uuid::Uuid::new_v4().to_string(), "q", 2)
            .unwrap()
            .unwrap()["accepted"],
        3
    );
    let n: i64 = c
        .query_row("SELECT count(*) FROM asset_discovery_requests", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 1);
}
#[test]
fn hunter_response_scope_is_checked_independently_of_the_remote_query() {
    let v = json!({"code":200,"data":{"arr":[{"url":"https://login.example.com/","ip":"1.2.3.4","port":443,"protocol":"https","web_title":"fixture"},{"url":"https://example.com.attacker.test/"},{"url":"https://tracker.test/"},{"url":"https://user:password@login.example.com/"},{"url":"https://login.example.com/?token=private"}]}});
    let rows = hunter_rows(&v, "example.com", 20, "domain").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["host"], "login.example.com");
    assert!(hunter_rows(
        &json!({"code":401,"data":{"arr":[]}}),
        "example.com",
        20,
        "domain"
    )
    .is_err());
    assert!(hunter_rows(&json!({"code":200}), "example.com", 20, "domain").is_err());
}
#[test]
fn reading_an_unconfigured_owner_does_not_create_settings_or_copy_a_fofa_key() {
    let c = Connection::open_in_memory().unwrap();
    assert_eq!(settings_on(&c).unwrap()["keyConfigured"], false);
    assert!(!exists(&c).unwrap());
}
#[test]
fn documented_hunter_queries_require_actual_project_certificate_and_cname_evidence() {
    let c = fixture();
    assert_eq!(
        hunter_query(&c, 1, "example.com", "domain").unwrap(),
        "domain.suffix=\"example.com\""
    );
    assert!(!hunter_query(&c, 1, "login.example.com", "domain")
        .unwrap()
        .contains("domain.suffix"));
    assert!(hunter_query(&c, 1, "example.com", "certificate").is_err());
    insert_seed(&c, 1, "example.com", "certificate_cn", "asset:3", true).unwrap();
    assert_eq!(
        hunter_query(&c, 1, "example.com", "certificate").unwrap(),
        "cert.subject.suffix=\"example.com\""
    );
    assert!(hunter_query(&c, 2, "example.com", "certificate").is_err());
    assert!(hunter_query(&c, 1, "example.com", "cname").is_err());
    insert_seed(&c, 1, "example.com", "dns_cname", "asset:4", false).unwrap();
    assert_eq!(
        hunter_query(&c, 1, "example.com", "cname").unwrap(),
        "cname_target==\"example.com\""
    );
}
#[test]
fn documented_hunter_request_is_base64url_first_page_and_only_recent_thirty_days() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    let url = hunter_search_url(
        "https://hunter.qianxin.com/openApi/search",
        "synthetic-secret",
        "cert.subject.suffix=\"example.com\"",
        10,
        date,
    )
    .unwrap();
    let p = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(p["page"], "1");
    assert_eq!(p["page_size"], "10");
    assert_eq!(p["start_time"], "2026-09-21");
    assert_eq!(p["end_time"], "2026-10-20");
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE
            .decode(p["search"].as_bytes())
            .unwrap(),
        b"cert.subject.suffix=\"example.com\""
    );
}
#[test]
fn hunter_account_redacts_personal_info_and_stops_before_queries_when_free_quota_is_exhausted() {
    let p = json!({"code":200,"data":{"type":"个人账号","rest_free_point":0,"rest_equity_point":1000,"once_export_quota":10000,"personal_info":{"phone":"synthetic-private-phone"}}});
    let a = hunter_account(&p).unwrap();
    assert!(!a.to_string().contains("synthetic-private-phone"));
    assert!(hunter_page_size(&a, 10).is_err());
    assert_eq!(
        hunter_page_size(
            &json!({"rest_free_point":15,"rest_export_quota":-1,"once_export_quota":-1}),
            20
        )
        .unwrap(),
        10
    );
    assert!(hunter_page_size(&json!({"rest_free_point":100,"rest_export_quota":0}), 10).is_err());
    assert_eq!(
        hunter_page_size(&json!({"rest_free_point":-1}), 10).unwrap(),
        10
    );
}
#[test]
fn hunter_certificate_and_cname_hits_keep_ip_endpoints_as_low_confidence_leads() {
    let p = json!({"code":200,"data":{"arr":[{"url":"https://8.141.92.254/","ip":"8.141.92.254","port":443,"protocol":"https","cert_sha256":"synthetic-fingerprint"}]}});
    assert!(hunter_rows(&p, "cmecloud.cn", 10, "domain")
        .unwrap()
        .is_empty());
    for kind in ["certificate", "cname"] {
        let r = hunter_rows(&p, "cmecloud.cn", 10, kind).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0]["score"], "40");
        assert!(!r[0]["evidence"].as_str().unwrap().contains("注册域:"));
    }
    assert!(hunter_rows(
        &json!({"code":200,"data":{"total":0,"arr":null}}),
        "example.com",
        10,
        "domain"
    )
    .unwrap()
    .is_empty());
}
#[test]
fn public_suffix_and_private_tenant_boundaries_are_not_confirmable_scopes() {
    for h in ["com", "com.cn", "github.io", "co.uk"] {
        assert!(crate::commands::ownership::ownership_domain_parts(h)
            .and_then(|p| p.registrable)
            .is_none());
    }
    assert_eq!(
        crate::commands::ownership::ownership_domain_parts("tenant.github.io")
            .unwrap()
            .registrable
            .as_deref(),
        Some("tenant.github.io")
    );
}

#[test]
fn documented_hunter_http_contract_round_trips_through_a_local_recording_server() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(vec![]));
    let recorded = calls.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let done = stop.clone();
    let worker = thread::spawn(move || {
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while !done.load(Ordering::SeqCst) && std::time::Instant::now() < until {
            let (mut stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(_) => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut bytes = vec![];
            let mut buf = [0; 4096];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if bytes.windows(4).any(|p| p == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&bytes);
            let target = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = reqwest::Url::parse(&format!("http://localhost{target}")).unwrap();
            recorded.lock().unwrap().push(url.clone());
            let body=if url.path()=="/openApi/userInfo" {json!({"code":200,"data":{"type":"个人账号","rest_free_point":499,"rest_export_quota":-1,"once_export_quota":10000,"personal_info":{"phone":"do-not-export"}}})}
                else{json!({"code":200,"data":{"total":1,"arr":[{"ip":"218.201.45.123","port":443,"domain":"","url":"https://218.201.45.123/","protocol":"https","web_title":"local contract fixture","cert_sha256":"synthetic-fingerprint"}],"consume_quota":"消耗积分：1","rest_quota":"今日剩余积分：498"}})}.to_string();
            let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
        }
    });
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let mut info = reqwest::Url::parse(&format!("{base}/openApi/userInfo")).unwrap();
    info.query_pairs_mut()
        .append_pair("api-key", "synthetic-contract-key");
    let account =
        hunter_account(&get_json(&client, info, "synthetic-contract-key").unwrap()).unwrap();
    let size = hunter_page_size(&account, 10).unwrap();
    let url = hunter_search_url(
        &format!("{base}/openApi/search"),
        "synthetic-contract-key",
        "cert.subject.suffix=\"iov10086.com\"",
        size,
        chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
    )
    .unwrap();
    let payload = get_json(&client, url, "synthetic-contract-key").unwrap();
    let rows = hunter_rows(&payload, "iov10086.com", size, "certificate").unwrap();
    stop.store(true, Ordering::SeqCst);
    worker.join().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["host"], "218.201.45.123");
    assert!(!account.to_string().contains("do-not-export"));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].path(), "/openApi/userInfo");
    assert_eq!(calls[1].path(), "/openApi/search");
    let params = calls[1]
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(params["page"], "1");
    assert_eq!(params["page_size"], "10");
    assert_eq!(params["start_time"], "2026-09-21");
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE
            .decode(params["search"].as_bytes())
            .unwrap(),
        b"cert.subject.suffix=\"iov10086.com\""
    );
}

#[test]
fn hunter_network_policy_roundtrips_with_revision_and_does_not_clear_key() {
    let mut c = fixture();
    let input = |revision, mode: Option<&str>| {
        serde_json::from_value::<HunterSettingsInput>(json!({"expectedRevision":revision,"key":null,"clearKey":false,"maxRequests":0,"maxResults":20,"networkMode":mode})).unwrap()
    };
    assert_eq!(settings_on(&c).unwrap()["networkMode"], "direct");
    let invalid = input(0, Some("unexpected"));
    assert!(save_settings_on(&mut c, invalid).is_err());
    assert_eq!(setting(&c).unwrap().0, 0);
    let mut first = input(0, Some("business"));
    first.key = Some("synthetic-hunter-secret".into());
    let result = save_settings_on(&mut c, first).unwrap();
    assert_eq!(result["networkMode"], "business");
    assert!(!result.to_string().contains("synthetic-hunter-secret"));
    assert_eq!(
        save_settings_on(&mut c, input(1, None)).unwrap()["networkMode"],
        "business"
    );
    assert_eq!(
        save_settings_on(&mut c, input(2, Some("direct"))).unwrap()["networkMode"],
        "direct"
    );
    assert_eq!(setting(&c).unwrap().1, "synthetic-hunter-secret");
}
