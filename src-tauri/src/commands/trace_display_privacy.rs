// Read-output policy only: never mutate imported evidence or runtime/model inputs.
const TRACE_PRIVATE_BODY: &str = "[私有推理内容已隐藏]";
const TRACE_UNPARSEABLE_BODY: &str = "[结构化详情不完整，已隐藏正文]";
const TRACE_DISPLAY_DEPTH: usize = 24;

fn trace_private_label(label: &str) -> bool {
    let normalized: String = label
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect();
    matches!(
        normalized.as_str(),
        "reasoning"
            | "reasoningcontent"
            | "reasoningdetails"
            | "reasoningtext"
            | "chainofthought"
            | "thinking"
            | "redactedthinking"
            | "encryptedcontent"
    )
}

fn trace_private_block(object: &serde_json::Map<String, JsonValue>) -> bool {
    ["type", "event_type", "eventType"].iter().any(|key| {
        object
            .get(*key)
            .and_then(JsonValue::as_str)
            .is_some_and(trace_private_label)
    }) || ["channel", "role"].iter().any(|key| {
        object
            .get(*key)
            .and_then(JsonValue::as_str)
            .is_some_and(|label| label.eq_ignore_ascii_case("analysis"))
    })
}

fn trace_display_text(text: &str, depth: usize) -> String {
    // Read projection can already have replaced the body. Preserve that notice
    // instead of treating its opening bracket as a malformed JSON payload.
    if text == TRACE_PRIVATE_BODY || text == TRACE_UNPARSEABLE_BODY {
        return text.into();
    }
    if depth >= TRACE_DISPLAY_DEPTH {
        return TRACE_UNPARSEABLE_BODY.into();
    }
    let trimmed = text.trim_start();
    if trimmed.starts_with(['{', '[', '"']) {
        return match serde_json::from_str::<JsonValue>(text) {
            Ok(value) => trace_display_value(value, depth + 1).to_string(),
            Err(_) => TRACE_UNPARSEABLE_BODY.into(),
        };
    }
    retained_trace_text(text)
}

fn trace_display_value(value: JsonValue, depth: usize) -> JsonValue {
    if depth >= TRACE_DISPLAY_DEPTH {
        return JsonValue::String(TRACE_UNPARSEABLE_BODY.into());
    }
    match value {
        JsonValue::Object(mut object) => {
            if trace_private_block(&object) {
                return JsonValue::String(TRACE_PRIVATE_BODY.into());
            }
            for (key, value) in object.iter_mut() {
                *value = if trace_private_label(key) {
                    JsonValue::String(TRACE_PRIVATE_BODY.into())
                } else {
                    trace_display_value(std::mem::take(value), depth + 1)
                };
            }
            retained_trace_value(&JsonValue::Object(object))
        }
        JsonValue::Array(values) => JsonValue::Array(
            values
                .into_iter()
                .map(|value| trace_display_value(value, depth + 1))
                .collect(),
        ),
        JsonValue::String(text) => JsonValue::String(trace_display_text(&text, depth + 1)),
        scalar => scalar,
    }
}

fn project_trace_display_privacy(mut detail: AgentTraceDetail) -> AgentTraceDetail {
    for event in &mut detail.events {
        event.detail = if trace_private_label(&event.event_type)
            || event.role.eq_ignore_ascii_case("analysis")
        {
            TRACE_PRIVATE_BODY.into()
        } else {
            trace_display_text(&event.detail, 0)
        };
        event.target_url = retained_trace_text(&event.target_url);
        event.name = retained_trace_text(&event.name);
        // Size/truncation describe the source preview, not the filtered body.
        // Keep IDs, event types, timestamps and counts for historical navigation.
    }
    if let Some(audit) = &mut detail.prompt_audit {
        audit.instruction = audit
            .instruction
            .as_deref()
            .map(|text| trace_display_text(text, 0));
        audit.notice = retained_trace_text(&audit.notice);
    }
    detail
}
