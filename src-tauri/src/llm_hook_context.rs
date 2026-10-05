// Local context guard: keeps a small local model's request inside its window by
// compacting conversation history and tool payloads instead of failing the run.
// Included from llm_hook.rs.

/// Keeps local OpenAI-compatible requests below the configured context window
/// without spending another model call on summarisation. Context pressure may
/// compact history and descriptions, but request prose must never select or
/// revoke executable tool capabilities.
fn guard_local_model_context(body: &[u8], max_context_tokens: u64) -> (Vec<u8>, Value) {
    if max_context_tokens == 0 {
        return (body.to_vec(), Value::Null);
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return (body.to_vec(), Value::Null);
    };
    if is_health_check_request(&value) || is_context_compaction_request(&value) {
        return (body.to_vec(), Value::Null);
    }
    let before_tokens = estimated_request_tokens(body);
    let before_messages = value
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let trigger_tokens = max_context_tokens.saturating_mul(94) / 100;
    if before_tokens <= trigger_tokens {
        return (body.to_vec(), Value::Null);
    }

    let removed_messages = compact_conversation(&mut value, true, 2_400);
    let mut trimmed_descriptions = 0usize;
    let target_tokens = max_context_tokens.saturating_mul(92) / 100;
    if serialized_estimated_tokens(&value) > target_tokens {
        trim_named_strings(&mut value, "description", 320, &mut trimmed_descriptions);
    }
    if serialized_estimated_tokens(&value) > target_tokens {
        // The assistant's prose is useful context but is never authoritative;
        // retain the first and most recent user messages alongside the system.
        compact_conversation(&mut value, false, 0);
    }
    if serialized_estimated_tokens(&value) > target_tokens {
        compact_user_messages(&mut value, 3_000);
        trim_named_strings(&mut value, "description", 160, &mut trimmed_descriptions);
    }

    let guarded = serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec());
    let after_tokens = estimated_request_tokens(&guarded);
    let after_messages = value
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    (
        guarded,
        json!({
            "applied": true,
            "reason": "context_headroom",
            "maxContextTokens": max_context_tokens,
            "beforeEstimatedTokens": before_tokens,
            "afterEstimatedTokens": after_tokens,
            "beforeMessages": before_messages,
            "afterMessages": after_messages,
            "removedMessages": removed_messages,
            "filteredTools": 0,
            "trimmedToolDescriptions": trimmed_descriptions,
        }),
    )
}

fn estimated_request_tokens(body: &[u8]) -> u64 {
    (body.len() as u64).div_ceil(4).max(1)
}

fn serialized_estimated_tokens(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .map(|body| estimated_request_tokens(&body))
        .unwrap_or(u64::MAX)
}

fn compact_conversation(
    value: &mut Value,
    keep_assistant_summary: bool,
    assistant_chars: usize,
) -> usize {
    let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) else {
        return 0;
    };
    if messages.len() <= 2 {
        return 0;
    }
    let original_len = messages.len();
    let system = messages
        .iter()
        .position(|message| message_role(message) == "system");
    let first_user = messages
        .iter()
        .position(|message| message_role(message) == "user");
    let last_user = messages
        .iter()
        .rposition(|message| message_role(message) == "user");
    let last_assistant = last_user.and_then(|user_index| {
        messages[..user_index]
            .iter()
            .rposition(|message| message_role(message) == "assistant")
    });
    let mut selected = Vec::new();
    for index in [system, first_user]
        .into_iter()
        .flatten()
        .chain(keep_assistant_summary.then_some(last_assistant).flatten())
        .chain(last_user)
    {
        if !selected.contains(&index) {
            selected.push(index);
        }
    }
    selected.sort_unstable();
    let mut compacted = selected
        .into_iter()
        .filter_map(|index| messages.get(index).cloned())
        .collect::<Vec<_>>();
    if keep_assistant_summary {
        if let Some(message) = compacted
            .iter_mut()
            .find(|message| message_role(message) == "assistant")
        {
            let text = message_content_text(message);
            if text.trim().is_empty() {
                compacted.retain(|item| message_role(item) != "assistant");
            } else if let Some(object) = message.as_object_mut() {
                object.remove("tool_calls");
                object.remove("function_call");
                object.remove("tool_call_id");
                object.insert(
                    "content".into(),
                    Value::String(format!(
                        "[Earlier execution compacted deterministically]\n{}",
                        compact_text(&text, assistant_chars)
                    )),
                );
            }
        }
    }
    *messages = compacted;
    original_len.saturating_sub(messages.len())
}

fn message_role(message: &Value) -> &str {
    message.get("role").and_then(Value::as_str).unwrap_or("")
}

fn message_content_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars < 160 {
        return text.chars().take(max_chars).collect();
    }
    let tail = (max_chars / 4).max(80);
    let head = max_chars.saturating_sub(tail);
    format!(
        "{}\n[... compacted ...]\n{}",
        text.chars().take(head).collect::<String>(),
        text.chars()
            .skip(count.saturating_sub(tail))
            .collect::<String>()
    )
}

fn compact_user_messages(value: &mut Value, max_chars: usize) {
    let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages
        .iter_mut()
        .filter(|message| message_role(message) == "user")
    {
        let text = message_content_text(message);
        if text.chars().count() > max_chars {
            if let Some(object) = message.as_object_mut() {
                object.insert(
                    "content".into(),
                    Value::String(compact_text(&text, max_chars)),
                );
            }
        }
    }
}

fn trim_named_strings(value: &mut Value, key: &str, max_chars: usize, changed: &mut usize) {
    match value {
        Value::Object(map) => {
            for (name, child) in map.iter_mut() {
                if name == key {
                    if let Value::String(text) = child {
                        if text.chars().count() > max_chars {
                            *text = compact_text(text, max_chars);
                            *changed += 1;
                        }
                    }
                } else {
                    trim_named_strings(child, key, max_chars, changed);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                trim_named_strings(item, key, max_chars, changed);
            }
        }
        _ => {}
    }
}
