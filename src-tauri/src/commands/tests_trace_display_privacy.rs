#[test]
fn trace_display_privacy_filters_nested_provider_blocks_and_encoded_payloads() {
    let input = serde_json::json!({
        "response": {"content": [
            {"type":"text", "text":"public answer"},
            {"type":"thinking", "thinking":"private-block"},
            {"channel":"analysis", "content":"private-channel"},
            {"role":"analysis", "content":"private-role"}
        ], "reasoningContent":"private-field"},
        "encoded": r#"{"reasoning_details":"private-encoded","content":"public encoded"}"#,
        "usage":{"total_tokens":7}
    });
    let displayed = trace_display_text(&input.to_string(), 0);
    for sentinel in [
        "private-block",
        "private-channel",
        "private-role",
        "private-field",
        "private-encoded",
    ] {
        assert!(!displayed.contains(sentinel), "{sentinel}");
    }
    assert!(displayed.contains("public answer"));
    assert!(displayed.contains("public encoded"));
    assert_eq!(
        serde_json::from_str::<JsonValue>(&displayed).unwrap()["usage"]["total_tokens"],
        7
    );
}

#[test]
fn trace_display_privacy_incomplete_structured_previews_fail_closed() {
    for notice in [TRACE_PRIVATE_BODY, TRACE_UNPARSEABLE_BODY] {
        assert_eq!(trace_display_text(notice, 0), notice);
    }
    for input in [
        r#"{"response":{"reasoning_content":"private-truncated"#,
        r#"[{"type":"thinking","text":"private-truncated"#,
        r#""private-truncated"#,
    ] {
        assert_eq!(trace_display_text(input, 0), TRACE_UNPARSEABLE_BODY);
    }
    assert_eq!(
        trace_display_text("plain public log", 0),
        "plain public log"
    );
    let nested = (0..30).fold(
        JsonValue::String("private-deep".into()),
        |value, _| serde_json::json!({"nested":value}),
    );
    assert!(!trace_display_text(&nested.to_string(), 0).contains("private-deep"));
}

#[test]
fn trace_display_privacy_keeps_existing_credential_redaction() {
    let input = serde_json::json!({"password":"credential-fixture", "content":"public"});
    let displayed = trace_display_text(&input.to_string(), 0);
    assert!(!displayed.contains("credential-fixture"));
    assert!(displayed.contains("public"));
    for label in [
        "reasoning",
        "reasoning_content",
        "chain-of-thought",
        "Thinking",
        "redacted_thinking",
        "encrypted_content",
        "reasoningText",
    ] {
        assert!(trace_private_label(label), "{label}");
    }
    assert!(!trace_private_label("hypothesis_updated"));
}

#[test]
fn trace_display_privacy_checks_message_envelopes_before_discarding_metadata() {
    let mut failures = Vec::new();
    for (marker, label) in [
        ("channel", "analysis"),
        ("channel", "ANALYSIS"),
        ("role", "analysis"),
        ("type", "thinking"),
        ("event_type", "reasoning_content"),
        ("eventType", "chain-of-thought"),
    ] {
        for (event_type, body_field) in [
            ("message", "content"),
            ("function_call", "arguments"),
            ("function_call_output", "output"),
        ] {
            let body = "private-envelope-fixture".repeat(80);
            let message = serde_json::json!({marker:label, body_field:body});
            let displayed = trace_display_value(message.clone(), 0);
            if displayed != json!(TRACE_PRIVATE_BODY) {
                failures.push(format!("{marker}={label}, {event_type}"));
            }
            assert_eq!(message[body_field], body, "display must not mutate source evidence");
        }
    }
    let public = serde_json::json!({"channel":"final", "content":"public answer"});
    assert_eq!(
        trace_display_value(public.clone(), 0),
        public
    );
    assert!(
        failures.is_empty(),
        "private envelopes displayed: {}",
        failures.join(", ")
    );
}

#[test]
fn trace_display_privacy_imported_analysis_envelope_never_reaches_display() {
    let (root, connection) = historical_trace_fixture();
    let source = root.join("source");
    let path = source.join("llm-hook.jsonl");
    let private = serde_json::json!({"type":"message", "role":"assistant", "channel":"analysis",
        "content":"private-imported-channel-fixture"})
    ;
    let public = serde_json::json!({"type":"message", "role":"assistant", "channel":"final",
        "content":"public final fixture"})
    ;
    let original = serde_json::json!({"kind":"model_call", "requestId":"channel-privacy",
        "response":{"output":[private, public]}}).to_string();
    fs::write(&path, &original).unwrap();
    crate::artifact_import::import_roots(&crate::artifact_import::ImportContext {
        connection: &connection,
        cas_dir: &root.join("cas"),
        key_path: &root.join("key"),
        roots: &[source],
        limits: &crate::artifact_import::Limits::default(),
    });
    let revisions = || {
        connection
            .prepare("SELECT envelope_json FROM import_record_revisions ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let before = revisions();
    assert!(before
        .iter()
        .any(|row| row.contains("private-imported-channel-fixture")));
    let detail = read_agent_trace_detail(&connection, "trace-boundary").unwrap();
    let displayed = serde_json::to_string(&detail).unwrap();
    assert_eq!(detail.summary.message_count, 0);
    assert_eq!(detail.events.iter().filter(|event| event.event_type == "model_request").count(), 1);
    assert!(displayed.contains("public final fixture"));
    assert_eq!(
        revisions(),
        before,
        "display must not rewrite canonical revisions"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
    assert!(!displayed.contains("private-imported-channel-fixture"));
    assert!(displayed.contains(TRACE_PRIVATE_BODY));
}
