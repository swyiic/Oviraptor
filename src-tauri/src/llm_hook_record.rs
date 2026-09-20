// Capture side of the loopback LLM hook: the JSONL records every request leaves
// behind, and the usage rollups read back from them. Included from llm_hook.rs.

pub fn usage_from_file(path: &Path) -> UsageTotals {
    let Ok(text) = fs::read_to_string(path) else {
        return UsageTotals::default();
    };
    let mut totals = UsageTotals::default();
    let mut in_flight = HashMap::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let request_id = value.get("requestId").and_then(Value::as_str).unwrap_or("");
        let maintenance = value
            .get("callType")
            .and_then(Value::as_str)
            .is_some_and(is_maintenance_call_type);
        if value.get("kind").and_then(Value::as_str) == Some("model_call_started") {
            // Health checks also perform real provider work (and may load a
            // 27B model), so keep them visible as active inference until the
            // matching completion record arrives.
            if !request_id.is_empty() {
                in_flight.insert(
                    request_id.to_string(),
                    value
                        .get("estimatedInputTokens")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                );
            }
            continue;
        }
        if !request_id.is_empty() {
            in_flight.remove(request_id);
        }
        if value
            .get("callType")
            .and_then(Value::as_str)
            .is_some_and(is_aborted_call_type)
        {
            continue;
        }
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u16>().ok());
        // Older hook records did not persist a status field. Preserve their
        // historical accounting; only an explicit non-2xx response is a
        // failed model attempt.
        let success = status
            .map(|value| (200..300).contains(&value))
            .unwrap_or(true);
        if !success {
            if maintenance {
                totals.maintenance_failed_requests += 1;
                continue;
            }
            totals.failed_requests += 1;
            let error = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let context_error = is_context_error(status.unwrap_or(0), error);
            if context_error {
                totals.context_errors += 1;
            }
            if !error.is_empty() {
                totals.last_error = error.to_string();
            }
            continue;
        }
        let usage = value.get("usage").unwrap_or(&Value::Null);
        totals.requests += 1;
        if maintenance {
            totals.maintenance_requests += 1;
        }
        totals.input_tokens += number(usage, &["input_tokens", "prompt_tokens"]);
        totals.output_tokens += number(usage, &["output_tokens", "completion_tokens"]);
        totals.cached_tokens += cached_tokens(usage);
        let total = number(usage, &["total_tokens"]);
        totals.total_tokens += if total > 0 {
            total
        } else {
            number(usage, &["input_tokens", "prompt_tokens"])
                + number(usage, &["output_tokens", "completion_tokens"])
        };
    }
    totals.in_flight_requests = in_flight.len() as i64;
    totals.in_flight_input_tokens = in_flight.values().sum();
    totals
}

pub fn records_from_file(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn append_record(output_path: &Path, write_lock: &Mutex<()>, record: &Value) {
    if let Ok(_guard) = write_lock.lock() {
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(output_path)
        {
            let _ = serde_json::to_writer(&mut file, &record);
            let _ = file.write_all(b"\n");
        }
    }
}

fn append_failure_record(
    output_path: &Path,
    write_lock: &Mutex<()>,
    request_id: &str,
    request_value: &Value,
    status: u16,
    error: &str,
) {
    append_record(
        output_path,
        write_lock,
        &json!({
            "kind":"model_call",
            "requestId":request_id,
            "callType":call_type_for_request(request_value),
            "recordedAt":chrono::Utc::now().to_rfc3339(),
            "status":status.to_string(),
            "model":request_value.get("model").cloned().unwrap_or(Value::Null),
            "usage":{},
            "error":error.chars().take(500).collect::<String>(),
        }),
    );
}

fn append_cancelled_record(
    output_path: &Path,
    write_lock: &Mutex<()>,
    request_id: &str,
    request_value: &Value,
) {
    append_record(
        output_path,
        write_lock,
        &json!({
            "kind":"model_call",
            "requestId":request_id,
            "callType":"scan_cancelled",
            "recordedAt":chrono::Utc::now().to_rfc3339(),
            "status":"499",
            "model":request_value.get("model").cloned().unwrap_or(Value::Null),
            "usage":{},
            "error":"Oviraptor 任务已停止，已断开本地模型上游推理",
        }),
    );
}

fn append_client_disconnected_record(
    output_path: &Path,
    write_lock: &Mutex<()>,
    request_id: &str,
    request_value: &Value,
) {
    append_record(
        output_path,
        write_lock,
        &json!({
            "kind":"model_call",
            "requestId":request_id,
            "callType":"scan_client_disconnected",
            "recordedAt":chrono::Utc::now().to_rfc3339(),
            "status":"499",
            "model":request_value.get("model").cloned().unwrap_or(Value::Null),
            "usage":{},
            "error":"Strix 在模型响应返回前关闭了本次连接；常见原因是单次模型调用达到 LLM_TIMEOUT，不能记为用户停止任务",
        }),
    );
}

fn clamp_output_tokens(body: &[u8], limit: u64) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return body.to_vec();
    };
    let Some(object) = value.as_object_mut() else {
        return body.to_vec();
    };
    let mut changed = false;
    for key in ["max_tokens", "max_completion_tokens"] {
        if let Some(current) = object.get(key).and_then(Value::as_u64) {
            if current > limit {
                object.insert(key.to_string(), Value::from(limit));
                changed = true;
            }
        }
    }
    if !object.contains_key("max_tokens") && !object.contains_key("max_completion_tokens") {
        object.insert("max_tokens".into(), Value::from(limit));
        changed = true;
    }
    if changed {
        serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec())
    } else {
        body.to_vec()
    }
}
