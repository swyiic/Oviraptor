use super::*;

fn ledger() -> rusqlite::Connection {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active'),(3,'archived');").unwrap();
    c
}

#[test]
fn prepared_operations_refuse_replay_or_changed_payload_and_keep_project_scope() {
    let c = ledger();
    let args = json!({"taskId":"real-task"});
    assert!(claim(&c, 1, "start-once", "start", &args)
        .unwrap()
        .is_none());
    let again = claim(&c, 1, "start-once", "start", &args).unwrap().unwrap();
    assert_eq!(again["outcomeUnknown"], true);
    assert_eq!(again["retryable"], false);
    assert_eq!(
        claim(&c, 1, "start-once", "resume", &args).unwrap_err(),
        "scan_operation_conflict"
    );
    assert_eq!(
        claim(&c, 1, "start-once", "start", &json!({"taskId":"other"})).unwrap_err(),
        "scan_operation_conflict"
    );
    assert!(claim(&c, 2, "start-once", "start", &args)
        .unwrap()
        .is_none());
    assert!(claim(&c, 3, "archived", "start", &args).is_err());
    assert!(claim(&c, 3, "pause-archived", "pause", &args)
        .unwrap()
        .is_none());
    let count: i64 = c
        .query_row("SELECT COUNT(*) FROM agent_scan_operations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 3);
}

#[test]
fn completed_operation_returns_historical_receipt_without_second_mutation() {
    let c = ledger();
    let args = json!({"assetIds":[42]});
    assert!(claim(&c, 1, "draft", "from_assets", &args)
        .unwrap()
        .is_none());
    let receipt = json!({"projectId":1,"task":{"id":"engine-draft","status":"draft"},"receiptIsHistorical":true});
    c.execute(
        "UPDATE agent_scan_operations SET state='completed',receipt_json=?1",
        [receipt.to_string()],
    )
    .unwrap();
    assert_eq!(
        claim(&c, 1, "draft", "from_assets", &args).unwrap(),
        Some(receipt)
    );
}

#[test]
fn channel_arguments_reject_scope_paths_arbitrary_urls_and_empty_or_duplicate_assets() {
    for (action, args) in [
        ("start", json!({"taskId":"x","projectId":2})),
        ("resume", json!({"taskId":"x","dbPath":"/tmp/foreign"})),
        ("from_assets", json!({"assetIds":[]})),
        ("from_assets", json!({"assetIds":[1,1]})),
        (
            "from_assets",
            json!({"assetIds":[1],"targets":["https://outside.test"]}),
        ),
        ("from_assets", json!({"assetIds":[-1]})),
        ("list", json!({"offset":-1})),
        ("list", json!({"limit":21})),
    ] {
        assert!(arguments(action, &args).is_err(), "{action} {args}");
    }
    assert!(arguments("from_assets", &json!({"assetIds":[1,2],"scanMode":"quick"})).is_ok());
    assert!(arguments("status", &json!({"taskId":"native-id"})).is_ok());
}

#[test]
fn original_asset_draft_is_visible_in_same_owner_and_safe_projection() {
    let root = std::env::temp_dir().join(format!("ovi-scan-channel-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let c = db::open(&path).unwrap();
    c.execute("INSERT INTO projects(id,name,status) VALUES(301,'channel-one','active'),(302,'channel-two','active')",[]).unwrap();
    c.execute("INSERT INTO assets(id,asset_key,company,host,link) VALUES(5001,'agent-channel-asset','fixture','channel.example.test','https://channel.example.test')",[]).unwrap();
    c.execute(
        "INSERT INTO project_assets(project_id,asset_id) VALUES(301,5001)",
        [],
    )
    .unwrap();
    let args =
        json!({"assetIds":[5001],"scanMode":"quick","maxModelRequests":3,"maxModelTokens":64000});
    assert!(claim(&c, 301, "real-draft", "from_assets", &args)
        .unwrap()
        .is_none());
    let draft = create_sentinel_asset_scan_with_receipt_in(
        &c,
        301,
        vec![5001],
        Some("quick".into()),
        (Some(3), Some(64_000)),
        |tx, scan| creation_receipt(tx, 301, "real-draft", scan).map(|_| ()),
    )
    .unwrap();
    assert_eq!(draft.status, "draft");
    assert_eq!(draft.attempt_count, 0);
    assert!(owned(&c, 302, &draft.id).is_err());
    let value = projection(&c, &owned(&c, 301, &draft.id).unwrap()).unwrap();
    assert_eq!(value["targetCount"], 1);
    assert_eq!(value["executionStarted"], false);
    assert_eq!(value["modelLimits"]["maxModelRequests"], 3);
    assert_eq!(value["modelLimits"]["maxModelTokens"], 64000);
    assert_eq!(value["modelLimits"]["usdLimitEnforced"], false);
    let settings = json!({"modelProfiles":[{"id":"preflight-model","llm":"openai/preflight-model","deployment":"cloud","apiBase":"https://provider.example.test/v1","apiKey":"synthetic-private-model-key"}],"activeModelProfileId":"preflight-model"});
    assert_eq!(
        preflight(&c, 301, &draft.id).unwrap()["modelConfigured"],
        false
    );
    let current = crate::configuration_catalog::read_category(&c, "models", None).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    crate::configuration_catalog::save_on(
        &tx,
        &crate::configuration_catalog::SaveCategoryInput {
            request_id: Uuid::new_v4().to_string(),
            section: "models".into(),
            mode: crate::configuration_catalog::SaveMode::Create,
            name: "Preflight fixture".into(),
            settings,
            expected_settings: current.settings,
            expected_reference: current.reference,
            expected_default: current.default_selection,
            make_default: true,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    let before = c.total_changes();
    let ready = preflight(&c, 301, &draft.id).unwrap();
    assert_eq!(ready["modelConfigured"], true);
    assert_eq!(ready["model"]["name"], "preflight-model");
    assert_eq!(ready["engineAdmissionValidated"], false);
    assert_eq!(ready["executionAuthorizedByThisRead"], false);
    assert!(!ready.to_string().contains("synthetic-private-model-key"));
    assert_eq!(c.total_changes(), before);
    assert!(preflight(&c, 302, &draft.id).is_err());
    assert!(value.get("taskPath").is_none());
    assert!(value.get("sourcePath").is_none());
    drop(c);
    let c = db::open(&path).unwrap();
    let saved = claim(&c, 301, "real-draft", "from_assets", &args)
        .unwrap()
        .unwrap();
    assert_eq!(saved["task"]["id"], draft.id);
    assert_eq!(saved["receiptIsHistorical"], true);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sentinel_scans WHERE project_id=301",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    // A late receipt failure must roll back the owner's draft and signed
    // creation identity together, leaving only the non-replayable preparation.
    assert!(claim(&c, 301, "receipt-failure", "from_assets", &args)
        .unwrap()
        .is_none());
    let failed = create_sentinel_asset_scan_with_receipt_in(
        &c,
        301,
        vec![5001],
        None,
        (None, None),
        |tx, scan| {
            creation_receipt(tx, 301, "receipt-failure", scan)?;
            Err("injected_late_receipt_failure".into())
        },
    );
    assert!(failed.is_err());
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sentinel_scans WHERE project_id=301",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT state FROM agent_scan_operations WHERE operation_id='receipt-failure'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "prepared"
    );
    c.execute(
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",
        [&draft.id],
    )
    .unwrap();
    assert!(owned(&c, 301, &draft.id).is_err());
    drop(c);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn result_pages_match_original_visibility_and_keep_hidden_rows_secrets_and_other_projects_out() {
    let root = std::env::temp_dir().join(format!("ovi-result-channel-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let c = db::open(&path).unwrap();
    c.execute_batch("INSERT INTO projects(id,name,status) VALUES(301,'results','active'),(302,'other','active');
        INSERT INTO sentinel_scans(id,project_id,status) VALUES('result-fixture',301,'completed');
        INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('result-fixture','{\"closure\":\"proof\"}');").unwrap();
    for id in 1..=220 {
        c.execute("INSERT INTO sentinel_findings(id,scan_id,stage,record_key,kind,title,record_json) VALUES(?1,'result-fixture','native',CAST(?1 AS TEXT),'vulnerability','HIDDEN PRIVATE FINDING','{}')",[id]).unwrap();
    }
    let proof = json!({"controlRequestId":"control","testRequestId":"test","impact":"Observed effect","password":"never-forward-record-secret"});
    c.execute("INSERT INTO sentinel_findings(id,scan_id,stage,target_url,kind,title,record_json) VALUES(221,'result-fixture','native','https://account:private-pass@example.test/login?api_key=private-query#private-fragment','vulnerability','Visible pair',?1)",[proof.to_string()]).unwrap();
    c.execute("INSERT INTO sentinel_findings(id,scan_id,stage,kind,title,record_json) VALUES(222,'result-fixture','native','endpoint','Saved endpoint','{\"cookie\":\"private-cookie\"}')",[]).unwrap();
    c.execute("INSERT INTO sentinel_targets(project_id,scan_id,url,status,routing_reason) VALUES(301,'result-fixture','https://account:private-pass@example.test/?password=private-query','recon_only',?1)",["no API contract; password=private-reason; /Users/private-user/secret.txt" ]).unwrap();
    let before = c.total_changes();
    c.execute_batch("PRAGMA query_only=ON").unwrap();
    let read = |args| results::read(&c,301,results::Results::parse(&args).unwrap()).unwrap();
    let first = read(json!({"taskId":"result-fixture","section":"findings"}));
    assert!(first["rows"].as_array().unwrap().is_empty());
    assert_eq!(first["nextAfterId"],200);
    assert_eq!(first["summary"]["visibleVulnerabilityRecords"],1);
    assert!(first["summary"]["confirmedVulnerabilities"].is_null());
    let second = read(json!({"taskId":"result-fixture","section":"findings","afterId":200}));
    let mut original: Vec<i64> = sentinel_findings_in(&c,"result-fixture",None).unwrap().iter().map(|r|r.id).collect();
    original.sort();
    assert_eq!(second["rows"].as_array().unwrap().iter().map(|r|r["id"].as_i64().unwrap()).collect::<Vec<_>>(), original);
    assert!(second["nextAfterId"].is_null());
    assert_eq!(second["rows"][0]["proofPairFieldsRecorded"],true);
    assert_eq!(second["rows"][0]["independentVerificationByThisRead"],false);
    let targets = read(json!({"taskId":"result-fixture"}));
    assert_eq!(targets["rows"][0]["status"],"recon_only");
    let output = format!("{first}{second}{targets}");
    for private in ["HIDDEN PRIVATE FINDING","never-forward-record-secret","private-pass","private-query","private-fragment","private-cookie","private-reason","/Users/private-user"] {
        assert!(!output.contains(private),"disclosed {private}");
    }
    assert!(results::read(&c,302,results::Results::parse(&json!({"taskId":"result-fixture"})).unwrap()).is_err());
    assert_eq!(c.total_changes(),before);
    c.execute_batch("PRAGMA query_only=OFF;UPDATE sentinel_scan_contexts SET policy_json='broken-json'").unwrap();
    assert_eq!(results::read(&c,301,results::Results::parse(&json!({"taskId":"result-fixture"})).unwrap()).unwrap_err(),"findings_policy_invalid");
    c.execute_batch("UPDATE sentinel_scan_contexts SET policy_json='{}';INSERT INTO sentinel_deleted_scans(scan_id) VALUES('result-fixture')").unwrap();
    assert!(results::read(&c,301,results::Results::parse(&json!({"taskId":"result-fixture"})).unwrap()).is_err());
    drop(c);fs::remove_dir_all(root).unwrap();
}

#[test]
fn result_output_budget_keeps_forward_progress_without_dropping_targets() {
    let root = std::env::temp_dir().join(format!("ovi-result-size-{}",Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();let c = db::open(&path).unwrap();
    c.execute_batch("INSERT INTO projects(id,name,status) VALUES(301,'size','active');INSERT INTO sentinel_scans(id,project_id) VALUES('size-fixture',301)").unwrap();
    for n in 1..=50 {
        let target=format!("https://example.test/{n}/{}", "a".repeat(1800));
        c.execute("INSERT INTO sentinel_targets(project_id,scan_id,url,routing_reason) VALUES(301,'size-fixture',?1,?2)",params![target,"reason ".repeat(500)]).unwrap();
    }
    // Large classification labels must not displace all rows and return the
    // same cursor forever. Compacted categories still count in the total.
    for n in 1..=40 {
        let kind=format!("kind-{n}-{}", "类".repeat(150));
        c.execute("INSERT INTO sentinel_findings(scan_id,stage,kind,title) VALUES('size-fixture','native',?1,'classification')",[kind]).unwrap();
    }
    let before=c.total_changes();c.execute_batch("PRAGMA query_only=ON").unwrap();
    let mut after=0;let mut ids=Vec::new();let mut limited=false;
    loop {
        let value=results::read(&c,301,results::Results::parse(&json!({"taskId":"size-fixture","limit":50,"afterId":after})).unwrap()).unwrap();
        assert!(serde_json::to_vec(&value).unwrap().len()<=24_000);
        assert_eq!(value["summary"]["totalVisibleRecords"],40);
        assert_eq!(value["summary"]["otherVisibleRecords"],8);
        assert_eq!(value["summary"]["recordKindLabelsCompacted"],true);
        limited|=value["outputSizeLimited"]==true;
        ids.extend(value["rows"].as_array().unwrap().iter().map(|r|r["id"].as_i64().unwrap()));
        let Some(next)=value["nextAfterId"].as_i64() else {break};assert!(next>after);after=next;
    }
    assert!(limited);assert_eq!(ids.len(),50);assert_eq!(ids.iter().collect::<HashSet<_>>().len(),50);assert_eq!(c.total_changes(),before);
    for args in [json!({"taskId":"size-fixture","afterId":-1}),json!({"taskId":"size-fixture","limit":51}),json!({"taskId":"size-fixture","section":"raw"}),json!({"taskId":"size-fixture","dbPath":"/private/foreign"})] {assert!(results::Results::parse(&args).is_err());}
    drop(c);fs::remove_dir_all(root).unwrap();
}

#[test]
fn result_kind_pages_and_single_evidence_keep_owner_visibility_secrets_and_forward_cursors() {
    let root = std::env::temp_dir().join(format!("ovi-result-detail-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let c = db::open(&path).unwrap();
    c.execute_batch("INSERT INTO projects(id,name,status) VALUES(301,'detail','active'),(302,'other','active');
        INSERT INTO sentinel_scans(id,project_id,status) VALUES('detail-fixture',301,'completed'),('other-task',301,'completed');
        INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('detail-fixture','{\"closure\":\"proof\"}');").unwrap();
    let js = json!({"url":"https://account:private-pass@example.test/app.js?custom=private-query#private-fragment","statusCode":200,"size":32,"analysisStatus":"parsed","external":false,
        "body":"PRIVATE SCRIPT BODY","headers":{"Cookie":"PRIVATE COOKIE"},"analysis":{"apiCandidates":3,"extractionEngine":"ast","parseErrors":["PRIVATE PARSE ERROR"],"routes":["PRIVATE ROUTE"],"stringEvidence":{"businessPaths":["PRIVATE PATH"],"storageReferences":["PRIVATE TOKEN"]}}});
    let diag = json!({"analysisSummary":{"verifiedApis":0,"runtimeRequests":2,"runtimeCaptureStatus":"partial"},"collectionOutcome":{"runtimeCaptureComplete":false,"partialEvidence":true,"failedIdentities":1,"evidenceFile":"/Users/private-user/raw.json"},
        "runtimeDiagnostics":[{"available":true,"captureStatus":"partial","failedStage":"fetch","runtimeStopReason":"missing_contract","browserExecutable":"/Applications/Private.app/raw","browserStderr":"PRIVATE BROWSER ERROR","captureError":"PRIVATE CAPTURE ERROR","identityKey":"PRIVATE IDENTITY"}]});
    let decision = json!({"eligibleForModel":false,"informationGain":2,"observedRequestCount":2,"verifiedRuntimeApiCount":0,"stopReason":"no_api_contract","coverage":{"requestCount":2,"identityCount":1},
        "manualDeepDive":[{"title":"Saved candidate","missingEvidence":"request contract missing; Cookie: private-cookie","reason":"see https://name:private-pass@example.test/path?unusual=private-query#private-fragment and /Users/private-user/raw","steps":["PRIVATE EXECUTION STEP"],"evidence":["PRIVATE EVIDENCE"]}]});
    let visible = json!({"controlRequestId":"control","testRequestId":"test","impact":"response difference; password=private-impact; https://example.test/?unusual=private-query","rawRequest":"PRIVATE RAW REQUEST"});
    for (id, kind, raw) in [
        (1, "vulnerability", "{}".into()),
        (2, "js_file", js.to_string()),
        (3, "js_file", js.to_string()),
        (4, "js_file", js.to_string()),
        (5, "js_file", js.to_string()),
        (6, "runtime_diagnostics", diag.to_string()),
        (7, "investigation_decision", decision.to_string()),
        (8, "vulnerability", visible.to_string()),
        (
            9,
            "unknown_kind",
            json!({"raw":"PRIVATE UNKNOWN"}).to_string(),
        ),
        (10, "js_file", "invalid JSON PRIVATE".into()),
        (
            11,
            "js_file",
            json!({"body":"PRIVATE LARGE".repeat(30000)}).to_string(),
        ),
    ] {
        c.execute("INSERT INTO sentinel_findings(id,scan_id,stage,record_key,kind,title,target_url,record_json) VALUES(?1,'detail-fixture','native',CAST(?1 AS TEXT),?2,'Saved record','https://example.test/?unusual=private-query',?3)",params![id,kind,raw]).unwrap();
    }
    c.execute("INSERT INTO sentinel_findings(id,scan_id,stage,kind,title) VALUES(12,'other-task','native','js_file','PRIVATE OTHER TASK')",[]).unwrap();
    let before = c.total_changes();
    c.execute_batch("PRAGMA query_only=ON").unwrap();
    let read = |args| results::read(&c, 301, results::Results::parse(&args).unwrap()).unwrap();
    let mut after = 0;
    let mut ids = Vec::new();
    loop {
        let page = read(
            json!({"taskId":"detail-fixture","section":"findings","kind":"js_file","limit":2,"afterId":after}),
        );
        assert_eq!(page["summary"]["filteredVisibleRecords"], 6);
        assert_eq!(page["summary"]["totalVisibleRecords"], 10);
        assert_eq!(page["filters"]["kind"], "js_file");
        ids.extend(
            page["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["id"].as_i64().unwrap()),
        );
        let Some(next) = page["nextAfterId"].as_i64() else {
            assert_eq!(page["pageComplete"], true);
            break;
        };
        assert!(next > after);
        after = next;
    }
    assert_eq!(ids, vec![2, 3, 4, 5, 10, 11]);
    let empty = read(json!({"taskId":"detail-fixture","section":"findings","kind":"absent"}));
    assert_eq!(empty["summary"]["filteredVisibleRecords"], 0);
    assert_eq!(empty["pageComplete"], true);
    assert!(empty["nextAfterId"].is_null());
    let mut serialized = String::new();
    for id in [2, 6, 7, 8, 9, 10, 11] {
        let value = read(json!({"taskId":"detail-fixture","section":"evidence","recordId":id}));
        assert_eq!(value["record"]["id"], id);
        assert_eq!(value["rawEvidenceIncluded"], false);
        assert_eq!(value["record"]["independentVerificationByThisRead"], false);
        assert!(serde_json::to_vec(&value).unwrap().len() <= 24000);
        serialized.push_str(&value.to_string());
        match id {
            2 => {
                assert_eq!(value["record"]["evidence"]["analysis"]["apiCandidates"], 3);
                assert_eq!(
                    value["record"]["evidence"]["analysis"]["parseErrorCount"],
                    1
                );
                assert_eq!(
                    value["record"]["evidence"]["url"],
                    "https://example.test/app.js"
                );
            }
            6 => {
                assert_eq!(
                    value["record"]["evidence"]["analysisSummary"]["verifiedApis"],
                    0
                );
                assert_eq!(
                    value["record"]["evidence"]["diagnostics"]["items"][0]["captureErrorRecorded"],
                    true
                );
            }
            7 => assert_eq!(value["record"]["evidence"]["eligibleForModel"], false),
            8 => assert_eq!(value["record"]["proofPairFieldsRecorded"], true),
            9 => assert_eq!(
                value["record"]["evidence"]["reason"],
                "unsupported_record_kind"
            ),
            10 => assert_eq!(value["record"]["evidence"]["reason"], "invalid_record_json"),
            11 => assert_eq!(value["record"]["evidence"]["reason"], "record_size_limit"),
            _ => unreachable!(),
        }
    }
    for secret in [
        "PRIVATE SCRIPT",
        "PRIVATE COOKIE",
        "PRIVATE PARSE",
        "PRIVATE ROUTE",
        "PRIVATE PATH",
        "PRIVATE TOKEN",
        "PRIVATE BROWSER",
        "PRIVATE CAPTURE",
        "PRIVATE IDENTITY",
        "PRIVATE EXECUTION",
        "PRIVATE EVIDENCE",
        "PRIVATE RAW",
        "PRIVATE UNKNOWN",
        "PRIVATE LARGE",
        "private-pass",
        "private-query",
        "private-fragment",
        "private-cookie",
        "private-impact",
        "/Users/private-user",
        "/Applications/Private",
    ] {
        assert!(!serialized.contains(secret), "disclosed {secret}");
    }
    for id in [1, 12, 999] {
        assert_eq!(
            results::read(
                &c,
                301,
                results::Results::parse(
                    &json!({"taskId":"detail-fixture","section":"evidence","recordId":id})
                )
                .unwrap()
            )
            .unwrap_err(),
            "scan_record_unavailable"
        );
    }
    assert!(results::read(
        &c,
        302,
        results::Results::parse(
            &json!({"taskId":"detail-fixture","section":"evidence","recordId":7})
        )
        .unwrap()
    )
    .is_err());
    for args in [
        json!({"taskId":"detail-fixture","kind":"js_file"}),
        json!({"taskId":"detail-fixture","section":"findings","kind":"bad' OR 1=1"}),
        json!({"taskId":"detail-fixture","section":"evidence"}),
        json!({"taskId":"detail-fixture","section":"evidence","recordId":7,"afterId":0}),
        json!({"taskId":"detail-fixture","section":"evidence","recordId":7,"kind":"investigation_decision"}),
        json!({"taskId":"detail-fixture","section":"findings","recordId":7}),
    ] {
        assert!(results::Results::parse(&args).is_err());
    }
    let plan=c.prepare("EXPLAIN QUERY PLAN SELECT id FROM sentinel_findings WHERE scan_id=?1 AND kind=?2 AND id>?3 ORDER BY id LIMIT 200").unwrap().query_map(params!["detail-fixture","js_file",0],|r|r.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap().join("\n");
    assert!(plan.contains("idx_sentinel_findings_scan_kind_cursor"));
    assert!(!plan.contains("TEMP B-TREE"));
    assert_eq!(c.total_changes(), before);
    drop(c);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resume_financial_refusal_precedes_claim_but_keeps_existing_operation_receipts() {
    let c=ledger();c.execute_batch(SCHEMA).unwrap();let args=json!({"taskId":"paid-task"});let before=c.total_changes();
    assert_eq!(claim_checked(&c,1,"new-paid-resume","resume",&args,|_|Err("scan_financial_continuation_required".into())).unwrap_err(),"scan_financial_continuation_required");
    assert_eq!(c.total_changes(),before);assert_eq!(c.query_row("SELECT COUNT(*) FROM agent_scan_operations",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    claim(&c,1,"old-receipt","resume",&args).unwrap();
    let receipt=claim_checked(&c,1,"old-receipt","resume",&args,|_|panic!("historical receipt must not invoke a new admission or operation")).unwrap().unwrap();
    assert_eq!(receipt["outcomeUnknown"],true);
}
