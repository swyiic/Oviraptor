use super::*;
fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT);INSERT INTO projects VALUES(1,'移动真实结构测试','active'),(2,'其他项目','active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,company TEXT,extra_json TEXT,content_category TEXT,probe_outcome TEXT DEFAULT '');CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,decision TEXT DEFAULT 'pending');").unwrap();
    ensure(&c).unwrap();
    c.execute_batch(TABLES).unwrap();
    c
}
fn add(c: &Connection, id: i64, p: i64, name: &str, value: Value, deleted: bool) {
    c.execute(
        "INSERT INTO assets(id,host,link,company,extra_json,content_category) VALUES(?1,?2,?2,'中国移动（采集标签）',?3,'')",
        params![id, format!("https://{name}"), value.to_string()],
    )
    .unwrap();
    c.execute(
        "INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(?1,?2,?3)",
        params![p, id, deleted],
    )
    .unwrap();
}
fn prepare(c: &mut Connection) -> String {
    let id = prepare_on(c, 1, "中国移动集团及其下属单位", true, &HashSet::new()).unwrap();
    while !harvest_page(c, 1, &id).unwrap() {}
    id
}
#[test]
fn full_local_scan_deduplicates_same_leaf_across_ports_and_never_leaks_pages_keys_or_grants() {
    let mut c = fixture();
    let hash = "a".repeat(64);
    for id in 1..=1300 {
        add(
            &c,
            id,
            1,
            "one.example.com",
            json!({"cert_sha256":hash,"cert.domain":"one.example.com","apiKey":"synthetic-secret","body":"private-page-body","cert.subject.org":"unbound subject organization"}),
            false,
        );
    }
    add(&c, 9000, 2, "other.example.net", json!({}), false);
    let id = prepare(&mut c);
    let view = view_on(&c, 1, "all", 1, "").unwrap();
    assert_eq!(view["run"]["processedAssets"], 1300);
    assert_eq!(view["run"]["groups"], 2);
    assert_eq!(view["run"]["requests"], 0);
    let item = view["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "certificate")
        .unwrap();
    assert_eq!(item["assetCount"], 1300);
    assert!(item["evidence"]["subjectOrganizations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(item["evidence"]["organizationHints"]
        .to_string()
        .contains("unbound"));
    for forbidden in ["synthetic-secret", "private-page-body", "other.example.net"] {
        assert!(!view.to_string().contains(forbidden));
    }
    assert!(roots(&c, 1).unwrap().is_empty());
    assert!(
        c.query_row(
            "SELECT COUNT(*) FROM asset_seed_review_refs WHERE run_id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
            > 1300
    );
}
#[test]
fn pause_cursor_resumes_after_many_empty_or_repeated_assets_without_dropping_later_evidence() {
    let mut c = fixture();
    for id in 1..=501 {
        add(
            &c,
            id,
            1,
            &format!("n{id}.example.com"),
            json!({"cert_sha256":"b".repeat(64),"cert.domain":"example.com"}),
            false,
        );
    }
    let id = prepare_on(&mut c, 1, "明确单位", true, &HashSet::new()).unwrap();
    assert!(!harvest_page(&mut c, 1, &id).unwrap());
    status(&c, &id, "paused", "test checkpoint").unwrap();
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    let v = view_on(&c, 1, "all", 1, "").unwrap();
    assert_eq!(v["run"]["processedAssets"], 501);
    assert_eq!(v["run"]["status"], "ready");
    assert!(v["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["assetCount"] == 501));
}
#[test]
fn invalid_duplicate_invented_and_incomplete_ai_advice_is_never_accepted() {
    let items = vec![("known".into(), json!({}))];
    for text in [
        "[]",
        r#"[{"key":"invented","label":"retain","confidence":1,"reason":"invented"}]"#,
        r#"[{"key":"known","label":"execute_hunter","confidence":1,"reason":"bad"}]"#,
        r#"[{"key":"known","label":"retain","confidence":1.1,"reason":"bad"}]"#,
    ] {
        assert!(match parse_advice(text, &items) {
            Err(_) => true,
            Ok(v) => !v.diagnostic.is_empty() && v.valid.is_empty(),
        });
    }
    assert!(parse_advice(
        r#"[{"key":"known","label":"review","confidence":0.5,"reason":"缺少同证书单位证据"}]"#,
        &items
    )
    .is_ok());
}
#[test]
fn mixed_san_and_provider_names_cannot_be_misbound_to_a_fresh_leaf() {
    let v = json!({"probe_cert_sha256":"a".repeat(64),"probe_cert_dns_names_json":"[\"ours.example.com\",\"other.vendor.net\"]","cert_sha256":"b".repeat(64),"cert.domain":"misbound.example.org","cert.subject.org":"unbound business"});
    let values = units(&v, "https://our-host.example.com");
    let cert = values.iter().find(|u| u.kind == "certificate").unwrap();
    assert_eq!(cert.target, "a".repeat(64));
    assert!(cert.ambiguous);
    assert!(!cert.domains.contains("misbound.example.org"));
    assert!(cert.organizations.is_empty());
}
#[test]
fn approval_is_project_revision_bound_and_never_grants_scope_or_hunter() {
    let mut c = fixture();
    add(&c, 1, 1, "one.example.com", json!({}), false);
    let id = prepare(&mut c);
    let v = view_on(&c, 1, "all", 1, "").unwrap();
    let key = v["items"][0]["key"].as_str().unwrap().to_string();
    let revision = v["items"][0]["revision"].as_i64().unwrap();
    let input = |project, revision| ReviewDecision {
        project_id: project,
        run_id: id.clone(),
        key: key.clone(),
        expected_revision: revision,
        decision: "accepted".into(),
    };
    assert!(decide_on(&mut c, input(2, 0)).is_err());
    decide_on(&mut c, input(1, revision)).unwrap();
    assert!(decide_on(&mut c, input(1, revision)).is_err());
    assert_eq!(view_on(&c, 1, "accepted", 1, "").unwrap()["total"], 1);
    assert!(roots(&c, 1).unwrap().is_empty());
}
#[test]
fn scope_requires_explicit_confirmation_live_observation_and_exact_scope_revision() {
    let mut c = fixture();
    add(&c, 1, 1, "one.example.com", json!({}), false);
    add(&c, 2, 1, "trashed.example.net", json!({}), true);
    let id = prepare(&mut c);
    let input = |name: &str, confirmed, expected| Confirm {
        project_id: 1,
        run_id: id.clone(),
        hosts: vec![name.into()],
        expected_roots: expected,
        user_confirmed: confirmed,
    };
    assert!(confirm_on(&mut c, input("one.example.com", false, vec![])).is_err());
    assert!(confirm_on(&mut c, input("example.com", true, vec![])).is_err());
    assert!(confirm_on(&mut c, input("trashed.example.net", true, vec![])).is_err());
    let saved = confirm_on(&mut c, input("one.example.com", true, vec![])).unwrap();
    assert_eq!(saved["hunterStarted"], false);
    assert_eq!(saved["probeStarted"], false);
    assert_eq!(saved["ownershipConfirmed"], false);
    assert!(confirm_on(&mut c, input("one.example.com", true, vec![])).is_err());
}
#[test]
fn interrupted_model_batch_is_manually_reviewable_and_not_put_back_in_the_dispatch_queue() {
    let mut c = fixture();
    add(&c, 1, 1, "one.example.com", json!({}), false);
    let id = prepare(&mut c);
    c.execute(
        "UPDATE asset_seed_review_items SET state='analyzing' WHERE run_id=?1",
        [&id],
    )
    .unwrap();
    unknown_on(&c, &id, "unknown transport").unwrap();
    assert_eq!(view_on(&c, 1, "errors", 1, "").unwrap()["total"], 1);
    assert_eq!(view_on(&c, 1, "pending", 1, "").unwrap()["total"], 0);
}

fn provider(
    reply: impl Fn(Value) -> Option<String> + Send + 'static,
) -> (
    OpenAiCompatibleGateway,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::thread::JoinHandle<()>,
) {
    provider_many(1, reply)
}
fn provider_many(
    count: usize,
    reply: impl Fn(Value) -> Option<String> + Send + 'static,
) -> (
    OpenAiCompatibleGateway,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::thread::JoinHandle<()>,
) {
    use crate::agent_runtime::model::{GatewayProfile, LocalResourcePolicy, ResolvedModelProfile};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let thread = std::thread::spawn(move || {
        for _ in 0..count {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                if let Ok((socket, _)) = listener.accept() {
                    break socket;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "model request did not reach the test endpoint"
                );
                std::thread::sleep(Duration::from_millis(2));
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 8192];
            let request = loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(offset) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..offset]);
                    let length = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap();
                    if bytes.len() >= offset + 4 + length {
                        break serde_json::from_slice::<Value>(
                            &bytes[offset + 4..offset + 4 + length],
                        )
                        .unwrap();
                    }
                }
            };
            seen.fetch_add(1, Ordering::SeqCst);
            assert!(!request.to_string().contains("private-page-body"));
            assert!(!request.to_string().contains("synthetic-secret"));
            if let Some(body) = reply(request) {
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                socket.write_all(response.as_bytes()).unwrap();
            }
        }
    });
    let profile = GatewayProfile::resolve(
        &ResolvedModelProfile {
            model: "test-review-model".into(),
            api_base: format!("http://{address}/v1"),
            api_key: "".into(),
            deployment: "local".into(),
        },
        &LocalResourcePolicy {
            max_output_tokens: Some(2048),
            max_context_tokens: 32000,
        },
        None,
    )
    .unwrap();
    (OpenAiCompatibleGateway::new(profile, &[]), hits, thread)
}
#[test]
fn real_gateway_trial_saves_every_advice_and_usage_then_leaves_the_remaining_batches_pending() {
    use std::sync::atomic::Ordering;
    let mut c = fixture();
    for id in 1..=20 {
        add(
            &c,
            id,
            1,
            &format!("n{id}.example.com"),
            json!({"cert_sha256":format!("{id:064x}"),"cert.domain":format!("n{id}.example.com"),"body":"private-page-body","apiKey":"synthetic-secret","cert.subject.org":"目标单位的证书主体线索"}),
            false,
        );
    }
    let id = prepare(&mut c);
    let (gateway, hits, thread) = provider(|request| {
        assert!(request["tools"].as_array().is_none_or(|v| v.is_empty()));
        let input: Value =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        let advice=input["untrustedEvidence"].as_array().unwrap().iter().map(|row|json!({"key":row["key"],"label":"retain","confidence":0.9,"reason":"仅为关联建议，人工确认范围"})).collect::<Vec<_>>();
        Some(json!({"choices":[{"message":{"content":json!(advice).to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":123,"completion_tokens":45,"total_tokens":168}}).to_string())
    });
    analyze(&mut c, 1, &id, gateway, &CancelToken::new(), 1).unwrap();
    thread.join().unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let view = view_on(&c, 1, "all", 1, "").unwrap();
    assert_eq!(view["run"]["requests"], 1);
    assert_eq!(view["run"]["inputTokens"], 123);
    assert_eq!(view["run"]["outputTokens"], 45);
    assert_eq!(view["run"]["status"], "paused");
    assert_eq!(view["run"]["counts"]["retain"], 12);
    assert_eq!(view["run"]["counts"]["pending"], 9);
    assert!(roots(&c, 1).unwrap().is_empty());
}
#[test]
fn transport_outcome_unknown_is_not_replayed_by_a_resume_or_a_status_poll() {
    use std::sync::atomic::Ordering;
    let mut c = fixture();
    add(
        &c,
        1,
        1,
        "one.example.com",
        json!({"icp_name":"独立单位证据"}),
        false,
    );
    let id = prepare(&mut c);
    let (gateway, hits, thread) = provider(|_| None);
    analyze(&mut c, 1, &id, gateway, &CancelToken::new(), 0).unwrap();
    thread.join().unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let view = view_on(&c, 1, "errors", 1, "").unwrap();
    assert_eq!(view["total"], 1);
    assert_eq!(view["run"]["counts"]["unknown"], 1);
    assert_eq!(view["run"]["requests"], 1);
    let profile = crate::agent_runtime::model::GatewayProfile::resolve(
        &crate::agent_runtime::model::ResolvedModelProfile {
            model: "no-dispatch-expected".into(),
            api_base: "http://127.0.0.1:1/v1".into(),
            api_key: "".into(),
            deployment: "local".into(),
        },
        &Default::default(),
        None,
    )
    .unwrap();
    analyze(
        &mut c,
        1,
        &id,
        OpenAiCompatibleGateway::new(profile, &[]),
        &CancelToken::new(),
        0,
    )
    .unwrap();
    assert_eq!(
        view_on(&c, 1, "errors", 1, "").unwrap()["run"]["requests"],
        1
    );
}
#[test]
fn oversized_multilingual_evidence_is_bounded_and_cannot_receive_an_unqualified_keep_label() {
    let evidence = json!({"domains":(0..64).map(|i|format!("n{i}.example.com")).collect::<Vec<_>>(),"companies":vec!["观测标签".repeat(200);8],"subjectOrganizations":vec!["集团组织".repeat(200);8],"organizationHints":[],"icpOrganizations":[],"sources":[],"exampleAssetIds":[1,2],"evidenceTruncated":false});
    let bounded = bounded_ai_evidence(evidence);
    assert!(bounded.to_string().len() <= 6000);
    assert_eq!(bounded["evidenceTruncated"], true);
}

#[test]
fn quarantine_switch_covers_content_rules_rejection_and_trash_without_restoring_any_row() {
    let mut c = fixture();
    for id in 1..=4 {
        add(&c, id, 1, "same.example.com", json!({}), id == 4);
    }
    c.execute(
        "UPDATE assets SET content_category='custom_rule' WHERE id=2",
        [],
    )
    .unwrap();
    c.execute(
        "UPDATE project_assets SET decision='rejected' WHERE asset_id=3",
        [],
    )
    .unwrap();
    let first = prepare_on(&mut c, 1, "明确单位", false, &HashSet::new()).unwrap();
    while !harvest_page(&mut c, 1, &first).unwrap() {}
    assert_eq!(
        view_on(&c, 1, "all", 1, "").unwrap()["run"]["totalAssets"],
        1
    );
    let second = prepare(&mut c);
    let v = view_on(&c, 1, "all", 1, "").unwrap();
    assert_eq!(v["run"]["totalAssets"], 4);
    assert_eq!(v["items"][0]["evidence"]["quarantinedAssets"], 3);
    assert_eq!(
        c.query_row("SELECT SUM(is_deleted) FROM project_assets", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    c.execute("UPDATE projects SET status='archived' WHERE id=1", [])
        .unwrap();
    assert_eq!(view_on(&c, 1, "all", 1, "").unwrap()["run"]["id"], second);
    assert!(prepare_on(&mut c, 1, "明确单位", true, &HashSet::new()).is_err());
}

#[test]
fn actual_background_owner_resumes_a_durable_local_checkpoint_and_cleans_up_after_failure() {
    let directory = std::env::temp_dir().join(format!("ovi-seed-worker-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("data.sqlite3");
    let mut c = fixture();
    for id in 1..=501 {
        add(&c, id, 1, "same.example.com", json!({}), false);
    }
    let id = prepare_on(&mut c, 1, "明确单位", true, &HashSet::new()).unwrap();
    assert!(!harvest_page(&mut c, 1, &id).unwrap());
    status(&c, &id, "paused", "saved checkpoint").unwrap();
    c.execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    let input = Input {
        project_id: 1,
        action: "start".into(),
        run_id: Some(id.clone()),
        organization: None,
        include_isolated: true,
        max_batches: 0,
    };
    control_on(&path, input).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while active(&id) && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!active(&id));
    let live = db::open(&path).unwrap();
    let view = view_on(&live, 1, "all", 1, "").unwrap();
    assert_eq!(view["run"]["processedAssets"], 501);
    assert_eq!(view["run"]["requests"], 0);
    assert_eq!(view["run"]["status"], "ready");
    controls()
        .lock()
        .unwrap()
        .insert(id.clone(), CancelToken::new());
    spawn_owned(path.clone(), id.clone(), "injected-local-error", || {
        Err("synthetic worker failure".into())
    })
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while active(&id) && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!active(&id));
    let view = view_on(&live, 1, "all", 1, "").unwrap();
    assert_eq!(view["run"]["status"], "interrupted");
    assert_eq!(view["run"]["processedAssets"], 501);
    assert!(view["run"]["reason"]
        .as_str()
        .unwrap()
        .contains("synthetic worker failure"));
    drop(live);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn partial_model_output_preserves_valid_rows_and_rejects_only_bad_aliases() {
    let items = (1..=4)
        .map(|i| (format!("r{i}"), json!({})))
        .collect::<Vec<_>>();
    let parsed = parse_advice(
        r#"```json
    [{"key":"r1","label":"exclude","confidence":0.99,"reason":"明确第三方品牌"},
     {"key":"r2","label":"retain","confidence":0.9,"reason":"重复"},
     {"key":"r2","label":"exclude","confidence":0.9,"reason":"重复"},
     {"key":"invented","label":"retain","confidence":1,"reason":"不存在"},
     {"key":"r3","label":"evidence","confidence":0.3,"reason":"缺少单位证据"}]
    ```"#,
        &items,
    )
    .unwrap();
    assert_eq!(
        parsed
            .valid
            .iter()
            .map(|a| a.key.as_str())
            .collect::<Vec<_>>(),
        vec!["r1", "r3"]
    );
    assert!(!parsed.diagnostic.is_empty());
}
#[test]
fn unrelated_advice_is_not_overridden_by_unbound_fingerprint_or_shared_san() {
    let mut v = Advice {
        key: "r1".into(),
        label: "exclude".into(),
        confidence: 0.99,
        reason: "明确其他企业".into(),
    };
    let evidence = json!({"ambiguousIdentity":true,"evidenceTruncated":true});
    assert_eq!(relevance_label(&v, &evidence), "exclude");
    v.label = "retain".into();
    assert_eq!(
        relevance_label(
            &v,
            &json!({"ambiguousIdentity":true,"organizationHints":["target organization"]})
        ),
        "retain"
    );
    assert_eq!(relevance_label(&v, &evidence), "evidence");
    v.label = "conflict".into();
    assert_eq!(relevance_label(&v, &evidence), "evidence");
    assert_eq!(
        relevance_label(
            &v,
            &json!({"subjectOrganizations":["target and conflicting entity"]})
        ),
        "conflict"
    );
}
#[test]
fn legacy_technical_errors_and_already_reviewed_items_do_not_inflate_human_queue() {
    let mut c = fixture();
    add(
        &c,
        1,
        1,
        "third.example.com",
        json!({"cert.domain":"third.example.com"}),
        false,
    );
    let id = prepare(&mut c);
    c.execute("UPDATE asset_seed_review_items SET state='done',advice='review',reason='旧版强制人工核验' WHERE kind='certificate_metadata'",[]).unwrap();
    c.execute(
        "UPDATE asset_seed_review_items SET state='unknown' WHERE kind='domain_group'",
        [],
    )
    .unwrap();
    let view = view_on(&c, 1, "actionable", 1, "").unwrap();
    assert_eq!(view["total"], 0);
    assert_eq!(view["run"]["queueCounts"]["evidence"], 1);
    assert_eq!(view["run"]["queueCounts"]["unknown"], 1);
    assert_eq!(
        view_on(&c, 1, "evidence", 1, "").unwrap()["items"][0]["modelAdvice"],
        "review"
    );
    c.execute("UPDATE asset_seed_review_items SET decision='rejected'", [])
        .unwrap();
    let view = view_on(&c, 1, "errors", 1, "").unwrap();
    assert_eq!(view["total"], 0);
    assert_eq!(view["run"]["manualCounts"]["rejected"], 2);
    assert!(view["run"]["queueCounts"].as_object().unwrap().is_empty());
    assert_eq!(view["run"]["requests"], 0);
    assert!(roots(&c, 1).unwrap().is_empty());
    triage::run(&mut c, 1, &id).unwrap();
}
#[test]
fn bare_pending_metadata_is_deferred_locally_while_rich_evidence_and_operator_decisions_survive() {
    let mut c = fixture();
    add(
        &c,
        1,
        1,
        "one.example.com",
        json!({"cert.domain":"one.example.com"}),
        false,
    );
    add(
        &c,
        2,
        1,
        "two.example.net",
        json!({"cert.domain":"two.example.net","cert.subject.org":"集团单位"}),
        false,
    );
    let id = prepare(&mut c);
    triage::run(&mut c, 1, &id).unwrap();
    triage::run(&mut c, 1, &id).unwrap();
    let v = view_on(&c, 1, "all", 1, "").unwrap();
    assert_eq!(v["run"]["counts"]["deferred"], 2);
    assert_eq!(v["run"]["requests"], 0);
    assert_eq!(v["run"]["queueCounts"]["evidence"], 2);
    assert_eq!(v["run"]["counts"]["pending"], 2);
    assert!(roots(&c, 1).unwrap().is_empty());
}
#[test]
fn gateway_partial_batch_saves_valid_exclusion_and_receipt_without_discarding_or_replaying_it() {
    let mut c = fixture();
    for id in 1..=3 {
        add(
            &c,
            id,
            1,
            &format!("n{id}.example.com"),
            json!({"cert_sha256":format!("{id:064x}"),"cert.domain":format!("n{id}.example.com"),"icp_name":"独立单位证据"}),
            false,
        );
    }
    let id = prepare(&mut c);
    let (gateway, hits, thread) = provider(|request| {
        let input: Value =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(input["untrustedEvidence"][0]["key"], "r1");
        assert_eq!(
            input["untrustedEvidence"][0]["evidence"]["kind"],
            "certificate"
        );
        Some(json!({"choices":[{"message":{"content":r#"[{"key":"r1","label":"exclude","confidence":0.99,"reason":"明确第三方"}]"#},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":30,"total_tokens":130}}).to_string())
    });
    analyze(&mut c, 1, &id, gateway, &CancelToken::new(), 0).unwrap();
    thread.join().unwrap();
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    let v = view_on(&c, 1, "all", 1, "").unwrap();
    assert_eq!(v["run"]["counts"]["exclude"], 1);
    assert_eq!(v["run"]["counts"]["unknown"], 3);
    assert_eq!(v["run"]["status"], "completed");
    assert_eq!(v["run"]["inputTokens"], 100);
    let receipt: (String, String) = c
        .query_row(
            "SELECT response_text,diagnostic FROM asset_seed_review_model_receipts WHERE run_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(receipt.0.contains("r1"));
    assert!(receipt.1.contains("3 项"));
    assert!(roots(&c, 1).unwrap().is_empty());
}

#[test]
fn real_trial_failure_collection_tags_cannot_become_positive_or_conflict_evidence() {
    let evidence = json!({"domains":["unrelated.example"],"companies":["中国移动通信集团有限公司","其他单位"],"subjectOrganizations":[],"icpOrganizations":[],"organizationHints":[],"confirmedDomainMatches":[]});
    for label in ["retain", "conflict"] {
        let advice = Advice {
            key: "r1".into(),
            label: label.into(),
            confidence: 0.99,
            reason: "companies明确为移动下属单位".into(),
        };
        assert_eq!(relevance_label(&advice, &evidence), "evidence");
    }
    let mut c = fixture();
    add(&c, 1, 1, "unrelated.example", json!({}), false);
    let id = prepare(&mut c);
    c.execute("UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=0.99,reason='仅采集标签被模型误当关联依据' WHERE run_id=?1",[&id]).unwrap();
    assert_eq!(view_on(&c, 1, "actionable", 1, "").unwrap()["total"], 0);
    let v = view_on(&c, 1, "evidence", 1, "").unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["modelAdvice"], "retain");
    assert!(!v["items"][0]["queueReason"].as_str().unwrap().is_empty());
}

#[test]
fn automatic_triage_uses_existing_scope_without_granting_external_names_or_spending() {
    let mut c = fixture();
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.com\"]')",
        [],
    )
    .unwrap();
    add(
        &c,
        1,
        1,
        "one.example.com",
        json!({"cert_sha256":"a".repeat(64),"cert.domain":"one.example.com"}),
        false,
    );
    add(
        &c,
        2,
        1,
        "two.example.com",
        json!({"cert_sha256":"b".repeat(64),"cert.domain":"two.example.com,external.net"}),
        false,
    );
    let id = prepare(&mut c);
    let v = view_on(&c, 1, "resolved", 1, "").unwrap();
    assert_eq!(v["run"]["requests"], 0);
    assert_eq!(v["run"]["automationCounts"]["existing_scope"], 2); // host group and one complete leaf
    assert!(v["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["target"] != "b".repeat(64)));
    assert_eq!(view_on(&c, 1, "pending", 1, "").unwrap()["total"], 1); // mixed SAN still needs reasoning
    assert_eq!(roots(&c, 1).unwrap(), vec!["example.com"]);
    triage::run(&mut c, 1, &id).unwrap();
    assert_eq!(view_on(&c, 1, "resolved", 1, "").unwrap()["total"], 2);
}

#[test]
fn local_correlation_joins_exact_names_and_does_not_spread_platform_identity_to_tenants() {
    let mut c = fixture();
    add(
        &c,
        1,
        1,
        "portal.example.com",
        json!({"cert.domain":"portal.example.com","cert.subject.org":"目标单位线索"}),
        false,
    );
    add(
        &c,
        2,
        1,
        "tenant.cloud.test",
        json!({"cert.domain":"cloud.test","cert.subject.org":"平台单位"}),
        false,
    );
    add(&c, 3, 1, "bare.other.test", json!({}), false);
    prepare(&mut c);
    let v = view_on(&c, 1, "all", 1, "").unwrap();
    let items = v["items"].as_array().unwrap();
    let portal = items.iter().find(|r| r["target"] == "example.com").unwrap();
    assert_eq!(
        portal["evidence"]["correlatedOrganizations"],
        json!(["目标单位线索"])
    );
    assert_eq!(portal["state"], "pending");
    let tenant = items.iter().find(|r| r["target"] == "cloud.test").unwrap();
    assert_eq!(tenant["evidence"]["correlatedOrganizations"], json!([]));
    // Remove the separately observed unit hint to isolate correlation's boundary.
    assert!(items
        .iter()
        .any(|r| r["target"] == "other.test" && r["state"] == "deferred"));
    assert_eq!(v["run"]["requests"], 0);
}

#[test]
fn exclusions_resolve_whole_groups_but_do_not_discard_mixed_or_truncated_certificates() {
    let mut c = fixture();
    c.execute_batch("CREATE TABLE asset_discovery_domain_exclusions(project_id INTEGER,domain TEXT,enabled INTEGER);INSERT INTO asset_discovery_domain_exclusions VALUES(1,'outside.test',1);").unwrap();
    add(
        &c,
        1,
        1,
        "one.outside.test",
        json!({"cert_sha256":"a".repeat(64),"cert.domain":"one.outside.test"}),
        false,
    );
    add(
        &c,
        2,
        1,
        "two.outside.test",
        json!({"cert_sha256":"b".repeat(64),"cert.domain":"two.outside.test,target.test"}),
        false,
    );
    let id = prepare(&mut c);
    let v = view_on(&c, 1, "resolved", 1, "").unwrap();
    assert_eq!(v["total"], 2);
    assert!(v["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["target"] != "b".repeat(64)));
    c.execute("UPDATE asset_seed_review_items SET state='pending',evidence_json=json_set(evidence_json,'$.evidenceTruncated',json('true')) WHERE run_id=?1 AND target=?2",params![id,"a".repeat(64)]).unwrap();
    triage::run(&mut c, 1, &id).unwrap();
    assert_eq!(view_on(&c, 1, "resolved", 1, "").unwrap()["total"], 1);
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn exact_manual_decisions_reuse_only_for_same_project_target_and_unchanged_evidence() {
    let mut c = fixture();
    add(
        &c,
        1,
        1,
        "one.example.com",
        json!({"cert.domain":"one.example.com","cert.subject.org":"原单位"}),
        false,
    );
    let first = prepare(&mut c);
    c.execute(
        "UPDATE asset_seed_review_items SET decision='rejected' WHERE run_id=?1",
        [&first],
    )
    .unwrap();
    let second = prepare(&mut c);
    assert_eq!(
        view_on(&c, 1, "resolved", 1, "").unwrap()["run"]["automationCounts"]["reused_rejected"],
        2
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM asset_seed_review_items WHERE run_id=?1 AND decision='rejected'",
            [&second],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    ); // not invented manual clicks
    c.execute(
        "UPDATE assets SET extra_json=json_set(extra_json,'$.\"cert.subject.org\"','新单位')",
        [],
    )
    .unwrap();
    prepare(&mut c);
    assert_eq!(view_on(&c, 1, "resolved", 1, "").unwrap()["total"], 0);
    assert_eq!(view_on(&c, 1, "pending", 1, "").unwrap()["total"], 2);
    let different = prepare_on(&mut c, 1, "另一个目标", true, &HashSet::new()).unwrap();
    while !harvest_page(&mut c, 1, &different).unwrap() {}
    assert_eq!(view_on(&c, 1, "resolved", 1, "").unwrap()["total"], 0);
    assert!(roots(&c, 1).unwrap().is_empty());
}

#[test]
#[ignore = "explicit offline replay against a disposable real-project backup"]
fn replay_local_triage_on_disposable_project_copy() {
    let path =
        std::env::var("OVIRAPTOR_TRIAGE_REPLAY_DB").expect("disposable database path required");
    assert!(path.contains("triage-replay"));
    let mut c =
        Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    let p: i64 = std::env::var("OVIRAPTOR_TRIAGE_REPLAY_PROJECT")
        .unwrap()
        .parse()
        .unwrap();
    let id: String = c
        .query_row(
            "SELECT id FROM asset_seed_reviews WHERE project_id=?1 ORDER BY rowid DESC LIMIT 1",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    let original:Vec<(String,String,String,String)>=c.prepare("SELECT item_key,state,advice,decision FROM asset_seed_review_items WHERE run_id=?1 AND (decision!='pending' OR (state IN ('done','unknown') AND COALESCE(json_extract(evidence_json,'$.automation.version'),0)=0)) ORDER BY item_key").unwrap().query_map([&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().map(Result::unwrap).collect();
    let before = view_on(&c, p, "pending", 1, "").unwrap();
    let routes = triage::run(&mut c, p, &id).unwrap();
    let after = view_on(&c, p, "pending", 1, "").unwrap();
    for (key, state, advice, decision) in original {
        let saved:(String,String,String)=c.query_row("SELECT state,advice,decision FROM asset_seed_review_items WHERE run_id=?1 AND item_key=?2",params![id,key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(saved, (state, advice, decision));
    }
    assert_eq!(before["run"]["requests"], after["run"]["requests"]);
    println!(
        "{}",
        json!({"beforePending":before["total"],"afterPending":after["total"],"routes":routes,"run":after["run"]})
    );
}

#[test]
fn partial_batch_does_not_block_later_valid_evidence_or_replay_failed_rows() {
    use std::sync::{Arc, Mutex};
    let mut c = fixture();
    for n in 1..=24 {
        add(
            &c,
            n,
            1,
            &format!("n{n}.example.com"),
            json!({"cert_sha256":format!("{n:064x}"),"cert.domain":format!("n{n}.example.com"),"icp_name":"独立单位证据"}),
            false,
        );
    }
    let id = prepare(&mut c);
    let observed = Arc::new(Mutex::new(Vec::<String>::new()));
    let record = observed.clone();
    let (gateway, hits, thread) = provider_many(2, move |request| {
        let input: Value =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        let rows = input["untrustedEvidence"].as_array().unwrap();
        let mut seen = record.lock().unwrap();
        let first = seen.is_empty();
        for r in rows {
            assert!(r["evidence"].get("companies").is_none());
            let target = r["evidence"]["target"].as_str().unwrap().to_owned();
            assert!(!seen.contains(&target));
            seen.push(target);
        }
        let advice=rows.iter().take(if first {1}else{12}).map(|r|json!({"key":r["key"],"label":"exclude","confidence":0.95,"reason":"明确第三方单位"})).collect::<Vec<_>>();
        Some(json!({"choices":[{"message":{"content":json!(advice).to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}}).to_string())
    });
    analyze(&mut c, 1, &id, gateway, &CancelToken::new(), 2).unwrap();
    thread.join().unwrap();
    let v = view_on(&c, 1, "all", 1, "").unwrap();
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(v["run"]["counts"]["exclude"], 13);
    assert_eq!(v["run"]["counts"]["unknown"], 11);
    assert_eq!(v["run"]["counts"]["pending"], 1);
    assert_eq!(v["run"]["inputTokens"], 200);
    assert_eq!(observed.lock().unwrap().len(), 24);
}

#[test]
fn truncated_relevance_is_not_confused_with_bulk_authorization_and_provider_only_is_not_exclusion()
{
    let v = Advice {
        key: "r1".into(),
        label: "retain".into(),
        confidence: 0.99,
        reason: "明确单位与域名相关".into(),
    };
    assert_eq!(
        relevance_label(
            &v,
            &json!({"kind":"domain_group","organizationHints":["目标集团"],"evidenceTruncated":true,"correlationTruncated":true})
        ),
        "retain"
    );
    let excluded = Advice {
        label: "exclude".into(),
        ..v
    };
    assert_eq!(
        relevance_label(
            &excluded,
            &json!({"organizationHints":["CloudFlare, Inc."]})
        ),
        "evidence"
    );
    assert_eq!(
        relevance_label(
            &excluded,
            &json!({"organizationHints":["CloudFlare, Inc."],"icpOrganizations":["明确第三方备案单位"]})
        ),
        "exclude"
    );
}
#[test]
fn versioned_saved_receipts_repair_derived_labels_without_new_requests_or_overwriting_originals() {
    let mut c = fixture();
    add(
        &c,
        1,
        1,
        "core.example.com",
        json!({"cert.domain":"core.example.com","cert.subject.org":"目标单位"}),
        false,
    );
    add(
        &c,
        2,
        1,
        "edge.other.test",
        json!({"cert.domain":"edge.other.test","cert.subject.org":"CloudFlare, Inc."}),
        false,
    );
    let id = prepare(&mut c);
    let keys=json!([{"alias":"r1","key":"domain_group:example.com"},{"alias":"r2","key":"domain_group:other.test"}]).to_string();
    let response=json!([{"key":"r1","label":"retain","confidence":0.99,"reason":"明确相关单位"},{"key":"r2","label":"exclude","confidence":0.99,"reason":"只有服务商单位"}]).to_string();
    c.execute(
        "INSERT INTO asset_seed_review_model_receipts VALUES(?1,1,?2,?3,'')",
        params![id, keys, response],
    )
    .unwrap();
    c.execute("UPDATE asset_seed_review_items SET state='done',advice='evidence',confidence=0.99,evidence_json=json_set(evidence_json,'$.evidenceTruncated',json('true')) WHERE run_id=?1 AND item_key='domain_group:example.com'",[&id]).unwrap();
    c.execute("UPDATE asset_seed_review_items SET state='done',advice='exclude',confidence=0.99 WHERE run_id=?1 AND item_key='domain_group:other.test'",[&id]).unwrap();
    triage::run(&mut c, 1, &id).unwrap();
    assert_eq!(view_on(&c, 1, "retain", 1, "").unwrap()["total"], 1);
    assert_eq!(view_on(&c, 1, "exclude", 1, "").unwrap()["total"], 0);
    assert_eq!(
        c.query_row(
            "SELECT response_text FROM asset_seed_review_model_receipts WHERE run_id=?1",
            [&id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        response
    );
    assert_eq!(view_on(&c, 1, "all", 1, "").unwrap()["run"]["requests"], 0);
    assert!(roots(&c, 1).unwrap().is_empty());
}

#[test]
fn newly_collected_leaf_resolves_old_unbound_evidence_without_overwriting_manual_decisions_or_receipts() {
    let mut c=fixture();
    c.execute("INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.test\"]')",[]).unwrap();
    add(&c,1,1,"fixture.example.test",json!({"cert.domain":"fixture.example.test","body":"original-page"}),true);
    let id=prepare(&mut c);
    c.execute("UPDATE asset_seed_review_items SET state='deferred',advice='evidence' WHERE run_id=?1 AND kind='certificate_metadata'",[&id]).unwrap();
    c.execute("UPDATE asset_seed_review_items SET decision='rejected',revision=7,reason='operator choice' WHERE run_id=?1 AND kind='domain_group'",[&id]).unwrap();
    let cert=crate::certificates::from_der(include_bytes!("../../test-fixtures/tls/leaf.der")).unwrap();
    c.execute("UPDATE assets SET extra_json=?1 WHERE id=1",[json!({"cert.domain":"fixture.example.test","probe_cert_der_base64":cert.der_base64,"probe_cert_sha256":cert.sha256,"body":"original-page"}).to_string()]).unwrap();
    refresh_enriched_asset(&mut c,1,1).unwrap();
    assert_eq!(c.query_row("SELECT state FROM asset_seed_review_items WHERE run_id=?1 AND kind='certificate_metadata'",[&id],|r|r.get::<_,String>(0)).unwrap(),"resolved");
    let manual:(String,i64,String)=c.query_row("SELECT decision,revision,reason FROM asset_seed_review_items WHERE run_id=?1 AND kind='domain_group'",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();assert_eq!(manual.0,"rejected");assert!(manual.1>=7);assert_eq!(manual.2,"operator choice");
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_seed_review_items WHERE run_id=?1 AND item_key=?2",params![id,format!("certificate:{}",cert.sha256)],|r|r.get::<_,i64>(0)).unwrap(),1);
    refresh_enriched_asset(&mut c,1,1).unwrap();
    assert_eq!(c.query_row("SELECT asset_count FROM asset_seed_review_items WHERE run_id=?1 AND item_key=?2",params![id,format!("certificate:{}",cert.sha256)],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(c.query_row("SELECT requests FROM asset_seed_reviews WHERE id=?1",[&id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(c.query_row("SELECT is_deleted FROM project_assets WHERE asset_id=1",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}

#[test]
fn incremental_certificate_refresh_obeys_the_original_isolation_choice() {
    let mut c = fixture();
    let cert = crate::certificates::from_der(include_bytes!("../../test-fixtures/tls/leaf.der")).unwrap();
    add(&c, 1, 1, "included.example.test", json!({"icp_name":"目标单位"}), false);
    for id in 2..=4 {
        add(&c, id, 1, "fixture.example.test", json!({"probe_cert_der_base64":cert.der_base64}), id == 2);
    }
    c.execute("UPDATE project_assets SET decision='rejected' WHERE asset_id=3", []).unwrap();
    c.execute("UPDATE assets SET probe_outcome='blocked_content' WHERE id=4", []).unwrap();
    let id = prepare_on(&mut c, 1, "目标单位", false, &HashSet::new()).unwrap();
    while !harvest_page(&mut c, 1, &id).unwrap() {}
    refresh_enriched_assets(&mut c, 1, [2, 3, 4]).unwrap();
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_seed_review_refs WHERE run_id=?1 AND asset_id IN (2,3,4)", [&id], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_seed_review_items WHERE run_id=?1 AND kind='certificate'", [&id], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(c.query_row("SELECT include_isolated FROM asset_seed_reviews WHERE id=?1", [&id], |r| r.get::<_, bool>(0)).unwrap(), false);
    assert_eq!(c.query_row("SELECT requests FROM asset_seed_reviews WHERE id=?1", [&id], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn new_facts_update_completed_unknown_and_manual_rows_without_replaying_old_requests() {
    for state in ["done", "unknown", "manual"] {
        let mut c = fixture();
        let cert = crate::certificates::from_der(include_bytes!("../../test-fixtures/tls/subject-organization.der")).unwrap();
        add(&c, 1, 1, "owned.example.test", json!({"cert_sha256":cert.sha256,"cert.domain":"owned.example.test"}), false);
        let id = prepare(&mut c);
        let key = format!("certificate:{}", cert.sha256);
        let decision = if state == "manual" { "rejected" } else { "pending" };
        c.execute("UPDATE asset_seed_review_items SET state=?3,advice='evidence',decision=?4,revision=7,reason='original decision' WHERE run_id=?1 AND item_key=?2", params![id,key,if state == "unknown" {"unknown"} else {"done"},decision]).unwrap();
        let response = "immutable original model response";
        c.execute("INSERT INTO asset_seed_review_model_receipts VALUES(?1,1,'[]',?2,'original diagnostic')", params![id,response]).unwrap();
        c.execute("UPDATE assets SET extra_json=?1 WHERE id=1", [json!({"probe_cert_der_base64":cert.der_base64}).to_string()]).unwrap();
        refresh_enriched_asset(&mut c, 1, 1).unwrap();
        let (raw, current_state, current_decision, revision):(String,String,String,i64) = c.query_row("SELECT evidence_json,state,decision,revision FROM asset_seed_review_items WHERE run_id=?1 AND item_key=?2", params![id,key], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        let evidence:Value=serde_json::from_str(&raw).unwrap();
        assert!(evidence["subjectOrganizations"].as_array().unwrap().contains(&json!("Synthetic Mobile Subsidiary")));
        assert!(revision > 7);
        assert_eq!(current_decision, decision);
        if state == "unknown" { assert_eq!(current_state, "unknown"); }
        if state == "done" { assert_eq!(current_state, "pending"); }
        assert_eq!(c.query_row("SELECT reason FROM asset_seed_review_evidence_revisions WHERE run_id=?1 AND item_key=?2 AND revision=7", params![id,key], |r| r.get::<_,String>(0)).unwrap(), "original decision");
        assert_eq!(c.query_row("SELECT response_text FROM asset_seed_review_model_receipts WHERE run_id=?1", [&id], |r| r.get::<_,String>(0)).unwrap(), response);
        assert_eq!(c.query_row("SELECT requests FROM asset_seed_reviews WHERE id=?1", [&id], |r| r.get::<_,i64>(0)).unwrap(), 0);
        refresh_enriched_asset(&mut c, 1, 1).unwrap();
        assert_eq!(c.query_row("SELECT asset_count FROM asset_seed_review_items WHERE run_id=?1 AND item_key=?2", params![id,key], |r| r.get::<_,i64>(0)).unwrap(), 1);
        assert!(roots(&c,1).unwrap().is_empty());
    }
}

#[test]
fn a_proven_before_transport_failure_can_resume_without_becoming_unknown() {
    let mut c = fixture();
    add(&c,1,1,"one.example.com",json!({"icp_name":"独立单位证据"}),false);
    let id=prepare(&mut c);
    let mut profile = crate::agent_runtime::model::GatewayProfile::resolve(&crate::agent_runtime::model::ResolvedModelProfile {
        model:"no-transport".into(),api_base:"http://127.0.0.1:1/v1".into(),api_key:"".into(),deployment:"local".into(),
    }, &Default::default(), None).unwrap();
    profile.max_output_tokens=Some(0);
    analyze(&mut c,1,&id,OpenAiCompatibleGateway::new(profile,&[]),&CancelToken::new(),0).unwrap();
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_seed_review_items WHERE state='unknown'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(c.query_row("SELECT COUNT(*) FROM asset_seed_review_items WHERE state='pending'",[],|r|r.get::<_,i64>(0)).unwrap()>0);
    assert_eq!(c.query_row("SELECT unknown_requests FROM asset_seed_reviews WHERE id=?1",[&id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(c.query_row("SELECT stage FROM asset_seed_review_request_errors WHERE run_id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),"before_transport");
    assert!(view_on(&c,1,"all",1,"").unwrap()["run"]["reason"].as_str().unwrap().contains("已确认尚未发送"));
}
