// §4: a synthetic CDP failure must keep its codes all the way from the helper
// result to the summary line the runner log and the UI show.
    fn synthetic_cdp_failure() -> JsonValue {
        serde_json::json!({
            "available": false,
            "captureStatus": "failed",
            "captureError": "cdp_command_pipe_unavailable",
            "runtimeStopReason": "runtime_probe_error",
            "stopReason": "unavailable",
            "probeStage": "cdp_handshake",
            "cdpTransport": "pipe",
            "browser": "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "browserVersion": "Chrome/126.0.0.0",
            "nodeVersion": "v24.18.0",
            "browserExitCode": null,
            "browserSignal": null,
            "browserStderr": "launching browser; Authorization: Bearer super-secret-token-value",
            "requests": [],
            "states": [],
            "actions": []
        })
    }

    fn labelled_diagnostics(runtime: &JsonValue, identity_key: &str) -> JsonValue {
        let mut item = native_runtime_diagnostics(runtime);
        if let Some(object) = item.as_object_mut() {
            object.insert(
                "identityKey".into(),
                JsonValue::String(identity_key.to_string()),
            );
        }
        item
    }

    #[test]
    fn a_clean_helper_exit_with_available_false_is_still_a_failed_capture() {
        let failed = synthetic_cdp_failure();
        assert!(!native_identity_capture_complete(&failed));
        let partial = serde_json::json!({"available":true,"captureStatus":"partial"});
        assert!(!native_identity_capture_complete(&partial));
        let silent = serde_json::json!({"available":true});
        assert!(
            !native_identity_capture_complete(&silent),
            "a helper that never reported a capture status cannot be complete"
        );
        let complete = serde_json::json!({"available":true,"captureStatus":"complete"});
        assert!(native_identity_capture_complete(&complete));
    }

    #[test]
    fn synthetic_cdp_failure_keeps_every_diagnostic_code_into_the_summary() {
        let diagnostics = labelled_diagnostics(&synthetic_cdp_failure(), "anonymous");
        assert_eq!(value_first(&diagnostics, &["captureStatus"]), "failed");
        assert_eq!(
            value_first(&diagnostics, &["captureError"]),
            "cdp_command_pipe_unavailable"
        );
        assert_eq!(
            value_first(&diagnostics, &["runtimeStopReason"]),
            "runtime_probe_error"
        );
        assert_eq!(value_first(&diagnostics, &["failedStage"]), "cdp_handshake");
        assert_eq!(value_first(&diagnostics, &["cdpTransport"]), "pipe");
        assert_eq!(
            value_first(&diagnostics, &["browserVersion"]),
            "Chrome/126.0.0.0"
        );
        assert_eq!(value_first(&diagnostics, &["nodeVersion"]), "v24.18.0");
        // The browser never exited, so there is no code to report: it stays unknown.
        assert_eq!(diagnostic_scalar(&diagnostics, "browserExitCode"), "unknown");
        assert_eq!(diagnostic_scalar(&diagnostics, "browserSignal"), "unknown");
        assert_eq!(diagnostics.get("available"), Some(&JsonValue::Bool(false)));

        let note = native_diagnostic_code_note(std::slice::from_ref(&diagnostics));
        assert!(note.contains("anonymous"), "{note}");
        assert!(note.contains("cdp_command_pipe_unavailable"), "{note}");
        assert!(note.contains("runtime_probe_error"), "{note}");
        assert!(note.contains("cdp_handshake"), "{note}");
        // A local browser failure is not renamed into a WAF, session or model error.
        let lowered = note.to_ascii_lowercase();
        assert!(!lowered.contains("waf"), "{note}");
        assert!(!note.contains("会话"), "{note}");
        assert!(!note.contains("模型"), "{note}");
    }

    #[test]
    fn helper_stderr_is_masked_and_bounded_before_it_is_stored() {
        let mut runtime = synthetic_cdp_failure();
        let filler = "y".repeat(5000);
        runtime["browserStderr"] =
            JsonValue::String(format!("token=super-secret-token-value {filler}"));
        let diagnostics = labelled_diagnostics(&runtime, "anonymous");
        let stderr = value_first(&diagnostics, &["browserStderr"]);
        assert!(stderr.chars().count() <= 2000, "stderr must stay bounded");
        assert!(!stderr.contains("super-secret-token-value"), "{stderr}");
        assert!(stderr.contains("<redacted:auth:"), "{stderr}");
    }

    #[test]
    fn unanswered_helper_fields_stay_unknown_instead_of_a_success_default() {
        let diagnostics = labelled_diagnostics(&serde_json::json!({"requests": []}), "anonymous");
        for field in [
            "captureStatus",
            "captureError",
            "runtimeStopReason",
            "failedStage",
            "cdpTransport",
            "browserVersion",
            "nodeVersion",
            "browserStderr",
        ] {
            assert_eq!(
                value_first(&diagnostics, &[field]),
                "unknown",
                "{field} must not default to a success value"
            );
        }
        assert_eq!(diagnostics.get("available"), Some(&JsonValue::Bool(false)));
        let note = native_diagnostic_code_note(std::slice::from_ref(&diagnostics));
        assert!(note.contains("unknown"), "{note}");
    }

    #[test]
    fn the_code_note_covers_every_identity_and_stays_bounded() {
        let mut first = synthetic_cdp_failure();
        first["captureError"] = JsonValue::String("cdp_command_pipe_unavailable".into());
        let mut second = synthetic_cdp_failure();
        second["captureError"] = JsonValue::String("browser_exit_137".into());
        second["browserExitCode"] = JsonValue::from(137);
        second["captureStatus"] = JsonValue::String("partial".into());
        second["available"] = JsonValue::Bool(true);
        let rows = vec![
            labelled_diagnostics(&first, "anonymous"),
            labelled_diagnostics(&second, "session:abc"),
        ];
        let note = native_diagnostic_code_note(&rows);
        assert!(note.contains("anonymous") && note.contains("session:abc"), "{note}");
        assert!(note.contains("browser_exit_137"), "{note}");
        assert!(note.contains("退出码=137"), "{note}");
        assert!(note.chars().count() <= 900, "the note must stay bounded");
    }

    #[test]
    fn synthetic_failure_codes_survive_helper_to_runner_log_line() {
        // §4.9: helper JSON is valid but available=false / captureStatus=failed.
        // The same codes must still be present after the recon Err string is
        // truncated the way scan_execution_frontend appends runner-log lines.
        let runtime = synthetic_cdp_failure();
        assert_eq!(runtime.get("available"), Some(&JsonValue::Bool(false)));
        assert_eq!(value_first(&runtime, &["captureStatus"]), "failed");
        assert!(!native_identity_capture_complete(&runtime));

        let diagnostics = labelled_diagnostics(&runtime, "anonymous");
        let detail = native_diagnostic_code_note(std::slice::from_ref(&diagnostics));
        let recon_error = format!(
            "CDP 运行时探测未完整成功；本轮仅写出部分侦察证据，不代表采集成功。底层诊断：{detail}"
        );
        let runner_line = format!(
            "frontend target 1/1: 浏览器采集阶段结束（watchdog 120s）· 采集失败：{}",
            recon_error.chars().take(1200).collect::<String>()
        );

        for code in [
            "cdp_command_pipe_unavailable",
            "runtime_probe_error",
            "cdp_handshake",
            "failed",
            "anonymous",
        ] {
            assert!(
                runner_line.contains(code),
                "runner log lost diagnostic code `{code}` after the 1200-char Err truncation:\n{runner_line}"
            );
        }
        assert!(
            recon_error.contains("底层诊断：") && recon_error.contains("cdp_command_pipe_unavailable"),
            "{recon_error}"
        );
    }
