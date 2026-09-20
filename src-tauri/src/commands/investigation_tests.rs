// Unit tests for the investigation module, included from investigation.rs.

#[cfg(test)]
mod investigation_tests {
    use super::*;

    #[test]
    fn verification_contracts_are_bounded() {
        let contract = verification_contract("idor", &serde_json::json!({"endpoint":"/api/users/1","parameters":["id"]}));
        assert_eq!(contract["maxAttempts"], 3);
        assert_eq!(contract["mutationPolicy"], "automatic_bounded_same_contract");
        assert!(contract["stopRules"].as_array().unwrap().iter().any(|value| value == "confirmed_waf_or_challenge"));
    }

    #[test]
    fn paths_and_queries_are_stable() {
        assert_eq!(normalized_investigation_path("https://example.test/api/users/?q=1#x"), "/api/users");
        assert_eq!(query_parameter_names("/api?a=1&b=2&a=3"), vec!["a", "b"]);
        assert_eq!(identity_diff_endpoint_key("GET|api.example.test|/api/users|id"), "GET|/api/users");
        assert_eq!(identity_diff_endpoint_key("GET|/api/users"), "GET|/api/users");
    }

    #[test]
    fn identity_nodes_keep_runtime_validity_and_capture_status() {
        let target = serde_json::json!({"identityRuns":[{
            "identityKey":"session:a","identityLabel":"账号 A","sessionValid":true,
            "captureStatus":"complete","statusCode":200,"apiCount":7,
            "validationReason":"session_active"
        }]});
        let payload = identity_node_payload(&target, "session:a", 0);
        assert_eq!(payload["identityLabel"], "账号 A");
        assert_eq!(payload["sessionValid"], true);
        assert_eq!(payload["captureStatus"], "complete");
        assert_eq!(payload["apiCount"], 7);
    }

    #[test]
    fn generic_read_only_gets_do_not_enter_high_value_queue() {
        let generic = serde_json::json!({
            "method":"GET",
            "endpoint":"https://example.test/bbs/app/feeds",
            "category":"data_query",
            "title":"接口测试面"
        });
        assert!(opportunity_is_low_value(&generic));
        let suspicious = serde_json::json!({
            "method":"GET",
            "endpoint":"https://example.test/api/account/profile",
            "category":"identity_surface",
            "title":"账户资料"
        });
        assert!(!opportunity_is_low_value(&suspicious));
        let object = serde_json::json!({
            "method":"GET",
            "endpoint":"https://example.test/api/items/12345",
            "category":"api_surface",
            "title":"详情"
        });
        assert!(!opportunity_is_low_value(&object));
    }

    #[test]
    fn graph_excludes_background_transport_and_serialized_response_keys() {
        for value in [
            serde_json::json!({"method":"GET","url":"https://cdn.test/avatar/user.jpeg","resourceType":"Fetch"}),
            serde_json::json!({"method":"POST","url":"https://fp-it.portal101.cn/deviceprofile/v4"}),
            serde_json::json!({"method":"GET","url":"https://example.test/bbs/app/topic/categories"}),
            serde_json::json!({"method":"UNKNOWN","url":"/bbs/app/api/general/search/v1/web","source":"string-heuristic"}),
        ] {
            assert!(investigation_background_noise(&value));
        }
        let keys = serde_json::json!(["{\"msg\":\"\",\"result\":{}}", "msg", "result"]);
        assert_eq!(sanitized_investigation_response_keys(Some(&keys)), vec!["msg", "result"]);
    }

    #[test]
    fn telemetry_is_retained_as_related_service_with_identity_evidence() {
        let target = serde_json::json!({"runtimeExploration":{"requests":[
            {"method":"POST","url":"https://monitor.example.test/api/34/envelope/?sentry_version=7&sentry_key=public","resourceType":"XHR","source":"browser-runtime","identityKey":"account-a","actionId":"action-1"},
            {"method":"POST","url":"https://monitor.example.test/api/34/envelope/?sentry_version=7&sentry_key=public","resourceType":"Fetch","source":"browser-runtime-intercept","identityKey":"account-a","actionId":"action-1"},
            {"method":"POST","url":"https://monitor.example.test/api/34/envelope/?sentry_version=7&sentry_key=public","resourceType":"XHR","source":"browser-runtime","identityKey":"account-b","actionId":"action-2"}
        ]}});
        let services = investigation_related_services_from_target("https://www.example.test/app", &target);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0]["host"], "monitor.example.test");
        assert_eq!(services[0]["classification"], "monitoring_telemetry");
        assert_eq!(services[0]["relation"], "same_party");
        assert_eq!(services[0]["requestCount"], 2);
        assert_eq!(services[0]["identityKeys"].as_array().unwrap().len(), 2);
        assert!(services[0]["queryKeys"].as_array().unwrap().iter().any(|value| value == "sentry_key"));
    }

    #[test]
    fn related_services_read_from_checkpoint_table_without_an_id_column() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE sentinel_checkpoints(scan_id TEXT,url TEXT,stage TEXT,raw_json TEXT,updated_at TEXT);").unwrap();
        let raw = serde_json::json!({"runtimeExploration":{"requests":[{
            "method":"POST","url":"https://monitor.example.test/api/1/envelope/","resourceType":"XHR","source":"browser-runtime"
        }]}}).to_string();
        connection.execute(
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json,updated_at) VALUES('scan-1','https://www.example.test','frontend_recon',?1,'2026-08-23 00:00:00')",
            [raw],
        ).unwrap();
        let services = read_investigation_related_services(&connection, "scan-1", "https://www.example.test").unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0]["host"], "monitor.example.test");
    }

    #[test]
    fn volatile_query_values_share_one_stable_contract() {
        let first = serde_json::json!({
            "method":"GET","category":"identity_surface",
            "endpoint":"https://example.test/api/account?nonce=one&ts=1","score":72
        });
        let second = serde_json::json!({
            "method":"GET","category":"identity_surface",
            "endpoint":"https://example.test/api/account?nonce=two&ts=2","score":88
        });
        assert_eq!(stable_opportunity_key(&first), stable_opportunity_key(&second));
        let grouped = deduplicated_actionable_opportunities(&[first, second]);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0]["score"], 88);
    }

    #[test]
    fn opportunity_card_merges_both_identity_observations() {
        let mut left = serde_json::json!({
            "identityKeys":["account-a","account-b"],
            "identityScopeKeys":["account-a","account-b"],
            "identityRuns":[
                {"identityKey":"account-a","observed":true,"statusCode":200},
                {"identityKey":"account-b","observed":false}
            ]
        });
        let right = serde_json::json!({
            "identityKeys":["account-b"],
            "identityRuns":[
                {"identityKey":"account-a","observed":false},
                {"identityKey":"account-b","observed":true,"statusCode":200}
            ]
        });
        merge_opportunity_record(&mut left, &right);
        let runs = left["identityRuns"].as_array().unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run["observed"] == true));
    }

    #[test]
    fn only_concrete_requests_enter_the_agent_verification_queue() {
        let inferred = serde_json::json!({"score":88,"category":"identity_surface","endpoint":"/login","method":"UNKNOWN","candidateOnly":true,"source":"evidence-reconstruction"});
        assert!(!opportunity_agent_readiness(&inferred).0);
        let route = serde_json::json!({"score":80,"category":"frontend_feature","route":"/admin","source":"babel-ast"});
        assert!(!opportunity_agent_readiness(&route).0);
        let concrete = serde_json::json!({"score":82,"category":"identity_surface","endpoint":"/api/account","method":"GET","parameters":["id"],"source":"runtime-request","requestContext":{"status":200},"riskEvidence":{"present":true,"signals":[{"type":"object_boundary_parameter"}]}});
        assert!(opportunity_agent_readiness(&concrete).0);
        let ordinary = serde_json::json!({"score":86,"category":"identity_surface","endpoint":"/account/restore_login","method":"GET","source":"runtime-request","requestContext":{"status":200}});
        assert!(!opportunity_agent_readiness(&ordinary).0);
    }

    #[test]
    fn runtime_and_exact_source_mapped_reads_can_open_bounded_investigation() {
        let runtime = serde_json::json!({
            "method":"GET","url":"https://example.test/api/search?q=one",
            "source":"browser-runtime","statusCode":200
        });
        assert!(standard_investigation_api(&runtime));
        let static_candidate = serde_json::json!({
            "method":"GET","url":"/api/search","source":"babel-ast"
        });
        assert!(!standard_investigation_api(&static_candidate));
        assert!(!source_mapped_readonly_api(&static_candidate));
        let source_mapped_read = serde_json::json!({
            "method":"GET","url":"https://example.test/api/oauth/state",
            "source":"https://example.test/static/js/main.js.map#components/Login.js",
            "confidence":"high"
        });
        assert!(source_mapped_readonly_api(&source_mapped_read));
        let unrelated_source_mapped_read = serde_json::json!({
            "method":"GET","url":"https://api.github.com/repos/example/demo",
            "source":"https://example.test/static/js/main.js.map#vendor/example.js",
            "confidence":"high"
        });
        assert!(!source_mapped_readonly_api(&unrelated_source_mapped_read));
        let source_mapped_write = serde_json::json!({
            "method":"POST","url":"https://example.test/api/user/manage",
            "source":"https://example.test/static/js/main.js.map#components/Users.js",
            "confidence":"high"
        });
        assert!(!source_mapped_readonly_api(&source_mapped_write));
        let source_mapped_placeholder = serde_json::json!({
            "method":"GET","url":"https://example.test/api/user/<id>",
            "source":"https://example.test/static/js/main.js.map#components/Users.js",
            "confidence":"high"
        });
        assert!(!source_mapped_readonly_api(&source_mapped_placeholder));
        let telemetry = serde_json::json!({
            "method":"POST","url":"https://example.test/account/data_report_web",
            "source":"browser-runtime","statusCode":200
        });
        assert!(!standard_investigation_api(&telemetry));
        let unknown = serde_json::json!({
            "method":"UNKNOWN","url":"/api/general/search/v1/web",
            "source":"string-heuristic"
        });
        assert!(!standard_investigation_api(&unknown));
    }

    #[test]
    fn manual_deep_dive_is_target_specific_bounded_and_never_a_finding() {
        let apis = vec![
            (
                "GET|example.test|/api/orders/42".into(),
                serde_json::json!({
                    "method":"GET","url":"https://example.test/api/orders/42",
                    "parameters":["order_id","user_id"],"responseKeys":["id","status"]
                }),
            ),
            (
                "POST|example.test|/api/orders/export".into(),
                serde_json::json!({
                    "method":"POST","url":"https://example.test/api/orders/export",
                    "parameters":["order_id","callback_url"],"responseKeys":["task_id"]
                }),
            ),
        ];
        let actions = vec![serde_json::json!({"label":"导出订单","outcome":"clicked"})];
        let plan = manual_deep_dive_plan(
            &serde_json::json!({"businessEntrypoints":["订单详情","导出"]}),
            &apis,
            &actions,
            &["anonymous".into()],
            "standard",
        );
        let rows = plan.as_array().unwrap();
        assert!(!rows.is_empty());
        assert!(rows.len() <= 5);
        assert_eq!(rows[0]["category"], "authorization");
        assert!(rows.iter().any(|row| row["category"] == "business_flow"));
        assert!(rows.iter().any(|row| row["category"] == "file_handling"));
        assert!(rows.iter().any(|row| row["category"] == "server_side_integration"));
        assert!(rows.iter().all(|row| {
            row["classification"] == "coverage_gap_not_vulnerability"
                && row["steps"].as_array().is_some_and(|steps| !steps.is_empty())
                && !row["stopCondition"].as_str().unwrap_or_default().is_empty()
        }));
    }

    #[test]
    fn graph_persistence_builds_incremental_and_learning_layers() {
        let directory = std::env::temp_dir().join(format!(
            "oviraptor-investigation-test-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let database = db::initialize(&directory).unwrap();
        let connection = db::open(&database).unwrap();
        connection.execute("INSERT INTO projects(name) VALUES('investigation-test')", []).unwrap();
        let project_id = connection.last_insert_rowid();
        let target = serde_json::json!({
            "url":"https://example.test/app",
            "finalUrl":"https://example.test/app",
            "fingerprint":{"backend":{"name":"Django","confidence":"medium"}},
            "apis":[{"method":"GET","url":"https://example.test/api/users/1?id=1","parameters":["id"],"source":"browser-runtime","confidence":"high","stateId":"state-1","actionId":"action-1","statusCode":200,"responseKeys":["id","name"]}],
            "opportunities":[{"opportunityKey":"idor-users","category":"idor","title":"用户对象权限差异","score":85,"confidence":"high","endpoint":"/api/users/1","method":"GET","parameters":["id"],"source":"runtime-request","requestContext":{"status":200},"riskEvidence":{"present":true,"signals":[{"type":"object_boundary_parameter"}]},"evidenceRefs":[{"type":"runtime_request"}]}],
            "runtimeExploration":{
                "states":[{"id":"state-1","url":"https://example.test/app","title":"Users","highValueLabels":["用户详情"]}],
                "actions":[{"id":"action-1","stateId":"state-1","label":"用户详情","role":"button","score":60,"outcome":"clicked","stateChanged":true,"requestCount":1,"beforeUrl":"https://example.test/app","afterUrl":"https://example.test/app"}],
                "requests":[{"method":"GET","url":"https://example.test/api/users/1?id=1","resourceType":"Fetch","stateId":"state-1","actionId":"action-1","queryKeys":["id"],"status":200,"responseKeys":["id","name"]}],
                "coverage":{"deduplicatedStateCount":0},"stopReason":"no_more_valuable_states"
            },
            "authSessionValidation":{"wafDetected":false}
        });
        for scan_id in ["scan-one", "scan-two"] {
            connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status) VALUES(?1,?2,'investigation-test','completed')", params![scan_id,project_id]).unwrap();
            connection.execute("INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES(?1,'{}')", [scan_id]).unwrap();
            connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,'investigation-test','https://example.test/app','completed')", params![project_id,scan_id]).unwrap();
            let metrics = persist_investigation_graph(&connection, Some(project_id), scan_id, "https://example.test/app", &target).unwrap();
            let login_identity_nodes: i64 = connection.query_row(
                "SELECT COUNT(*) FROM investigation_nodes WHERE scan_id=?1 AND node_type='identity'",
                [scan_id],
                |row| row.get(0),
            ).unwrap();
            assert_eq!(login_identity_nodes, 0, "anonymous scope must not become a login identity");
            if scan_id == "scan-one" {
                assert!(metrics.token_worthy);
                assert!(metrics.node_count >= 6);
            } else {
                assert!(!metrics.token_worthy);
                assert_eq!(metrics.stop_reason, "incremental_no_new_value");
            }
        }
        let fact_count: i64 = connection.query_row("SELECT COUNT(*) FROM knowledge_facts", [], |row| row.get(0)).unwrap();
        let promoted: i64 = connection.query_row("SELECT promoted FROM knowledge_strategies WHERE category='idor'", [], |row| row.get(0)).unwrap();
        assert!(fact_count >= 4);
        assert_eq!(promoted, 1);
        let hypothesis_id: i64 = connection.query_row("SELECT id FROM investigation_hypotheses WHERE scan_id='scan-one' LIMIT 1", [], |row| row.get(0)).unwrap();
        connection.execute("INSERT INTO investigation_mutation_approvals(hypothesis_id,approved,scope_json,max_attempts,expires_at) VALUES(?1,1,'{\"method\":\"GET\",\"endpoint\":\"/api/users/1\"}',1,datetime('now','localtime','+30 minutes'))", [hypothesis_id]).unwrap();
        let approved = read_investigation_hypotheses(&connection, "scan-one", "", "").unwrap();
        assert_eq!(approved[0].contract["method"], "GET");
        assert_eq!(approved[0].mutation_approval["active"], true);
        connection.execute("UPDATE investigation_mutation_approvals SET expires_at=datetime('now','localtime','-1 minute') WHERE hypothesis_id=?1", [hypothesis_id]).unwrap();
        let expired = read_investigation_hypotheses(&connection, "scan-one", "", "").unwrap();
        assert_eq!(expired[0].mutation_approval["active"], false);
        drop(connection);
        fs::remove_dir_all(&directory).unwrap();
    }
}
