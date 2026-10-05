// Unit tests for the loopback LLM hook, included from llm_hook.rs.

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use uuid::Uuid;

    #[test]
    fn accepts_https_upstreams_for_cloud_usage_accounting() {
        let upstream = parse_http_base("https://api.example.invalid/v1")
            .unwrap()
            .unwrap();
        assert_eq!(upstream.scheme, "https");
        assert_eq!(upstream.host, "api.example.invalid");
        assert_eq!(upstream.port, 443);
        assert_eq!(upstream.base_path, "/v1");
    }

    #[test]
    fn proxies_local_openai_requests_and_persists_full_local_usage() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let request = read_request(&mut stream).unwrap();
            assert_eq!(request.path, "/v1/chat/completions");
            assert!(request.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("authorization") && value == "Bearer upstream-test-key"
            }));
            assert!(!request
                .headers
                .iter()
                .any(|(_, value)| value.contains("strix-internal-key")));
            let body = json!({
                "model":"local-test",
                "choices":[{"message":{"role":"assistant","content":"OK"}}],
                "usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}
            })
            .to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).unwrap();
        });
        let root = std::env::temp_dir().join(format!("oviraptor-llm-hook-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let hook = start(
            &format!("http://127.0.0.1:{upstream_port}/v1"),
            "upstream-test-key",
            &root,
            "full",
            None,
            None,
            0,
            1,
        )
        .unwrap()
        .unwrap();
        assert!(hook.base_url().ends_with("/v1"));
        let authority = hook
            .base_url()
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap();
        let body = json!({
            "model":"local-test",
            "messages":[{"role":"user","content":"Authorization: Bearer top-secret"}],
            "stream":false
        })
        .to_string();
        let mut client = TcpStream::connect(authority).unwrap();
        client.write_all(format!("POST /v1/chat/completions HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer strix-internal-key\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.contains("200 OK"));
        server.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !root.join("llm-hook.jsonl").is_file() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let records = records_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["kind"], "model_call_started");
        let completed = records
            .iter()
            .find(|record| record["kind"] == "model_call")
            .unwrap();
        assert_eq!(completed["usage"]["total_tokens"], 15);
        assert_eq!(
            completed["request"]["messages"][0]["content"],
            "Authorization: Bearer top-secret"
        );
        let usage = usage_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(usage.requests, 1);
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 3);
        assert_eq!(usage.in_flight_requests, 0);
        drop(hook);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn dropping_hook_disconnects_an_in_flight_local_generation() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let _ = read_request(&mut stream).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut byte = [0u8; 1];
            stream.read(&mut byte).unwrap_or(0)
        });
        let root =
            std::env::temp_dir().join(format!("oviraptor-llm-hook-cancel-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let hook = start(
            &format!("http://127.0.0.1:{upstream_port}/v1"),
            "local",
            &root,
            "off",
            None,
            Some(4_096),
            49_152,
            1,
        )
        .unwrap()
        .unwrap();
        let authority = hook
            .base_url()
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap()
            .to_string();
        let client = thread::spawn(move || {
            let body = r#"{"model":"local-27b","messages":[{"role":"user","content":"slow"}]}"#;
            let mut stream = TcpStream::connect(&authority).unwrap();
            let _ = stream.write_all(format!("POST /v1/chat/completions HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            let mut response = Vec::new();
            let _ = stream.read_to_end(&mut response);
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while hook
            .active_upstreams
            .lock()
            .map(|active| active.is_empty())
            .unwrap_or(true)
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(hook.active_upstreams.lock().unwrap().len(), 1);
        drop(hook);
        assert_eq!(server.join().unwrap(), 0);
        client.join().unwrap();
        assert!(
            records_from_file(&root.join("llm-hook.jsonl"))
                .iter()
                .any(|record| record.get("callType").and_then(Value::as_str)
                    == Some("scan_cancelled"))
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let usage = usage_from_file(&root.join("llm-hook.jsonl"));
            if usage.in_flight_requests == 0 || Instant::now() >= deadline {
                assert_eq!(usage.in_flight_requests, 0);
                assert_eq!(usage.failed_requests, 0);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn estimates_usage_when_the_local_provider_omits_it() {
        let request = Request {
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            headers: Vec::new(),
            body: br#"{"model":"local","messages":[{"role":"user","content":"hello"}]}"#.to_vec(),
        };
        let response_body = br#"{"choices":[{"message":{"content":"world"}}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            response_body.len()
        )
        .into_bytes()
        .into_iter()
        .chain(response_body.iter().copied())
        .collect::<Vec<_>>();
        let record = build_record(
            &request,
            &serde_json::from_slice(&request.body).unwrap(),
            &response,
            "metadata",
            "request-1",
        );
        assert_eq!(record["usageEstimated"], true);
        assert!(record["usage"]["total_tokens"].as_i64().unwrap() > 0);
        assert!(record.get("request").is_none());
    }

    #[test]
    fn classifies_context_compaction_without_misclassifying_checkpointed_scan_turns() {
        let compaction = json!({
            "messages": [{"role":"system","content":"You are compacting the earlier part of an autonomous security-testing agent's conversation.\n\nConversation to summarise:"}]
        });
        let resumed_scan = json!({
            "messages": [{"role":"user","content":"<conversation-checkpoint>earlier summary</conversation-checkpoint>\nContinue validating /api/register"}]
        });
        assert!(is_context_compaction_request(&compaction));
        assert!(!is_context_compaction_request(&resumed_scan));
    }

    #[test]
    fn classifies_provider_health_checks_without_misclassifying_scan_prompts() {
        let health_check = json!({
            "messages": [
                {"role":"system","content":"You are a helpful assistant."},
                {"role":"user","content":"Reply with just 'OK'."}
            ]
        });
        let scan = json!({
            "messages": [{"role":"user","content":"Check whether /health replies with just OK."}]
        });
        assert!(is_health_check_request(&health_check));
        assert!(!is_health_check_request(&scan));
    }

    #[test]
    fn historical_recovery_text_cannot_change_the_native_tool_capabilities() {
        let tools = (0..34)
            .map(|index| {
                let name = if index == 33 {
                    "finish_scan".to_string()
                } else {
                    format!("tool_{index}")
                };
                json!({
                    "type":"function",
                    "function":{
                        "name":name,
                        "description":"x".repeat(2_000),
                        "parameters":{"type":"object"}
                    }
                })
            })
            .collect::<Vec<_>>();
        let request = json!({
            "model":"local-9b",
            "messages":[
                {"role":"system","content":"system".repeat(30_000)},
                {"role":"user","content":"authorized task"},
                {"role":"assistant","tool_calls":[{"id":"call-1"}],"content":""},
                {"role":"tool","tool_call_id":"call-1","content":"evidence".repeat(2_000)},
                {"role":"assistant","content":"long conclusion".repeat(2_000)},
                {"role":"user","content":"Your previous response ended the autonomous Strix run without a lifecycle tool call. That is invalid in non-interactive mode; plain text final answers are ignored. Continue immediately and call exactly one tool. If your work is complete, call finish_scan. This is recovery attempt 1/3."}
            ],
            "tools":tools,
            "max_tokens":4096
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        let guarded: Value = serde_json::from_slice(&guarded).unwrap();
        let messages = guarded["messages"].as_array().unwrap();
        assert!(messages.len() <= 4);
        assert!(!messages.iter().any(|message| message["role"] == "tool"));
        assert_eq!(guarded["tools"].as_array().unwrap().len(), 34);
        for (before, after) in request["tools"].as_array().unwrap().iter().zip(guarded["tools"].as_array().unwrap()) {
            assert_eq!(before["function"]["name"], after["function"]["name"]);
            assert_eq!(before["function"]["parameters"], after["function"]["parameters"]);
        }
        assert_eq!(guarded["max_tokens"], 4096);
        assert_eq!(summary["applied"], true);
        assert_eq!(summary["reason"], "context_headroom");
        assert_eq!(summary["filteredTools"], 0);
        assert!(summary["afterEstimatedTokens"].as_u64().unwrap() < 49_152);
    }

    #[test]
    fn historical_recovery_text_in_a_small_request_has_no_runtime_effect() {
        let request = json!({"messages":[{"role":"user","content":
            "Your previous response ended the autonomous Strix run without a lifecycle tool call. This is recovery attempt 1/3."}],
            "tools":[{"type":"function","function":{"name":"http_request"}}, {"type":"function","function":{"name":"finish_scan"}}]});
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        assert_eq!(guarded, body);
        assert!(summary.is_null());
    }

    #[test]
    fn context_guard_leaves_normal_in_window_requests_unchanged() {
        let request = json!({
            "model":"local-9b",
            "messages":[{"role":"user","content":"inspect the supplied evidence"}],
            "tools":[]
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        assert_eq!(guarded, body);
        assert!(summary.is_null());
    }

    #[test]
    fn lifecycle_documentation_in_system_prompt_does_not_strip_scan_tools() {
        let tools = (0..34)
            .map(|index| {
                let name = if index == 33 {
                    "finish_scan".to_string()
                } else {
                    format!("tool_{index}")
                };
                json!({
                    "type":"function",
                    "function":{
                        "name":name,
                        "description":"ordinary scan tool",
                        "parameters":{"type":"object"}
                    }
                })
            })
            .collect::<Vec<_>>();
        let request = json!({
            "model":"local-9b",
            "messages":[
                {"role":"system","content":"Autonomous runs must finish with a lifecycle tool call such as finish_scan."},
                {"role":"user","content":"Validate the supplied target evidence."}
            ],
            "tools":tools
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        let guarded: Value = serde_json::from_slice(&guarded).unwrap();
        assert_eq!(guarded["tools"].as_array().unwrap().len(), 34);
        assert!(summary.is_null());
    }

    #[test]
    fn oversized_first_scan_turn_compacts_headroom_without_removing_tools() {
        let tools = (0..34)
            .map(|index| {
                let name = if index == 33 {
                    "finish_scan".to_string()
                } else {
                    format!("tool_{index}")
                };
                json!({
                    "type":"function",
                    "function":{
                        "name":name,
                        "description":"tool documentation ".repeat(300),
                        "parameters":{"type":"object","properties":{"url":{"type":"string"}}}
                    }
                })
            })
            .collect::<Vec<_>>();
        let request = json!({
            "model":"local-9b",
            "messages":[
                {"role":"system","content":format!("{} finish_scan is the lifecycle completion tool", "system ".repeat(10_000))},
                {"role":"user","content":"Validate the inline request/response contract."}
            ],
            "tools":tools
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        let guarded: Value = serde_json::from_slice(&guarded).unwrap();
        assert_eq!(guarded["tools"].as_array().unwrap().len(), 34);
        assert_eq!(summary["applied"], true);
        assert_eq!(summary["reason"], "context_headroom");
        assert_eq!(summary["filteredTools"], 0);
    }

    #[test]
    fn user_text_that_merely_mentions_finish_scan_is_not_protocol_recovery() {
        let request = json!({
            "model":"local-9b",
            "messages":[
                {"role":"system","content":"system"},
                {"role":"user","content":"Explain why a lifecycle tool call named finish_scan exists."}
            ],
            "tools":[{"type":"function","function":{"name":"finish_scan","parameters":{"type":"object"}}}]
        });
        let body = serde_json::to_vec(&request).unwrap();
        let (guarded, summary) = guard_local_model_context(&body, 49_152);
        assert_eq!(guarded, body);
        assert!(summary.is_null());
    }

    #[test]
    fn maintenance_calls_keep_token_accounting_but_do_not_become_scan_failures() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-llm-hook-maintenance-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let records = [
            json!({"status":"200","callType":"scan","usage":{"input_tokens":100,"output_tokens":10,"total_tokens":110}}),
            json!({"status":"200","callType":"context_compaction","usage":{"input_tokens":80,"output_tokens":20,"total_tokens":100}}),
            json!({"status":"200","callType":"health_check","usage":{"input_tokens":30,"output_tokens":1,"total_tokens":31}}),
            json!({"status":"500","callType":"context_compaction","error":"summary failed","usage":{}}),
            json!({"status":"500","callType":"health_check","error":"provider warming up","usage":{}}),
        ];
        fs::write(
            root.join("llm-hook.jsonl"),
            records
                .into_iter()
                .map(|record| record.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let usage = usage_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(usage.requests, 3);
        assert_eq!(usage.maintenance_requests, 2);
        assert_eq!(usage.maintenance_failed_requests, 2);
        assert_eq!(usage.failed_requests, 0);
        assert_eq!(usage.total_tokens, 241);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn records_an_unfinished_scan_request_as_active_inference() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-llm-hook-in-flight-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("llm-hook.jsonl"),
            json!({
                "kind":"model_call_started",
                "requestId":"slow-local-prefill",
                "callType":"scan",
                "status":""
            })
            .to_string(),
        )
        .unwrap();
        let usage = usage_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(usage.requests, 0);
        assert_eq!(usage.in_flight_requests, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejected_context_requests_are_not_counted_as_model_turns() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-llm-hook-error-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let record = json!({
            "kind": "model_call",
            "status": "400",
            "error": "request exceeds the available context size",
            "usage": {"input_tokens": 34000, "total_tokens": 34000}
        });
        fs::write(root.join("llm-hook.jsonl"), format!("{}\n", record)).unwrap();
        let usage = usage_from_file(&root.join("llm-hook.jsonl"));
        assert_eq!(usage.requests, 0);
        assert_eq!(usage.failed_requests, 1);
        assert_eq!(usage.context_errors, 1);
        assert_eq!(usage.total_tokens, 0);
        let _ = fs::remove_dir_all(root);
    }
}
