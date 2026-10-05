/// One A/B comparison written out as a difference record: the file a confirmed
/// finding has to cite, so the claim points at two real responses (§10.2).
fn agent_write_diff_record(
    context: &AgentRunContext,
    index: usize,
    record: &JsonValue,
) -> Result<String, String> {
    let (directory, directory_handle) = prepare_agent_artifact_write_directory(
        &context.target_dir,
        AGENT_DIFF_DIRECTORY,
    )?;
    let mut slot = index.max(1);
    while agent_artifact_slot_exists(directory_handle.as_ref(), &format!("diff-{slot:04}.json"))? {
        slot += 1;
    }
    let name = format!("diff-{slot:04}.json");
    write_agent_artifact_file(
        &directory,
        directory_handle.as_ref(),
        &name,
        serde_json::json!({"index": slot, "record": record})
            .to_string()
            .as_bytes(),
    )?;
    if let Some(directory) = directory_handle.as_ref() {
        directory.sync_all().map_err(|_| "artifact_directory_sync_failed")?;
    }
    Ok(name)
}

/// Read a difference record back. `None` means the cited artifact does not exist,
/// which is enough to refuse a confirmed finding (§10.2).
fn agent_read_diff_record(context: &AgentRunContext, artifact_id: &str) -> Option<JsonValue> {
    if artifact_id.is_empty()
        || artifact_id.contains('/')
        || artifact_id.contains("..")
        || !artifact_id.starts_with("diff-")
    {
        return None;
    }
    let directory = open_agent_artifact_directory(&context.target_dir.join(AGENT_DIFF_DIRECTORY))?;
    let (bytes, _) = read_agent_artifact_file(&directory, artifact_id, 1_048_576)?;
    let parsed = serde_json::from_slice::<JsonValue>(&bytes).ok()?;
    crate::agent_runtime::secrets::redact_json(&parsed)
        .get("record")
        .cloned()
}

/// The business parameter names a request carried: query keys plus form or JSON
/// body keys, sorted. Values never enter the ledger (§10.2).
fn agent_request_parameters(url: &str, body: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = reqwest::Url::parse(url)
        .map(|parsed| parsed.query_pairs().map(|(key, _)| key.to_string()).collect())
        .unwrap_or_default();
    if let Some(body) = body.filter(|value| !value.trim().is_empty()) {
        if let Ok(parsed) = serde_json::from_str::<JsonValue>(body) {
            names.extend(
                parsed
                    .as_object()
                    .map(|map| map.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
            );
        } else {
            names.extend(
                body.split('&')
                    .filter_map(|pair| pair.split_once('=').map(|(key, _)| key.to_string())),
            );
        }
    }
    names.sort();
    names.dedup();
    names.iter().take(40).cloned().collect()
}

/// `hit` when a cache in front of the target answered this response (§10.2).
fn agent_cache_state(headers: &[(String, String)]) -> &'static str {
    for (name, value) in headers {
        let lowered = value.trim().to_ascii_lowercase();
        let answered_from_cache = match name.to_ascii_lowercase().as_str() {
            "age" => lowered.parse::<u64>().unwrap_or(0) > 0,
            "x-cache" | "cf-cache-status" | "x-boomerang-delivery-method" => {
                lowered.contains("hit")
            }
            _ => false,
        };
        if answered_from_cache {
            return "hit";
        }
    }
    "miss"
}

/// The model-visible projection of a tool result (§6.1, §6.3). Structure, status
/// codes, field paths, types, hashes and lengths survive; credential and personal
/// values become stable, comparable markers. The untouched bytes live in the raw
/// artifact the caller points at, never here.
fn agent_model_view(summary: JsonValue, runtime: &AgentToolRuntime) -> JsonValue {
    use crate::agent_runtime::secrets::redact_json_with;
    // The HTTP path projects its payload first because the audit record embeds
    // that same view; a second pass would re-scan the markers it just wrote.
    if summary
        .get("redaction")
        .and_then(|value| value.get("applied"))
        .and_then(JsonValue::as_bool)
        == Some(true)
    {
        return summary;
    }
    let mut view = summary;
    if let Some(object) = view.as_object_mut() {
        object.insert(
            "redaction".to_string(),
            serde_json::json!({"applied": true, "mode": "per-run-stable-fingerprint"}),
        );
    }
    redact_json_with(&view, Some(&runtime.redaction))
}

fn agent_response_structure(text: &str, content_type: &str) -> Vec<String> {
    if content_type.contains("json") || text.trim_start().starts_with('{') || text.trim_start().starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<JsonValue>(text) {
            let mut keys = Vec::new();
            match &parsed {
                JsonValue::Object(map) => keys.extend(map.keys().cloned()),
                JsonValue::Array(rows) => {
                    for row in rows.iter().take(3) {
                        if let Some(map) = row.as_object() {
                            keys.extend(map.keys().cloned());
                        }
                    }
                }
                _ => {}
            }
            keys.sort();
            keys.dedup();
            keys.truncate(40);
            return keys;
        }
    }
    if content_type.contains("html") || text.trim_start().starts_with("<!DOCTYPE") {
        return vec!["html".to_string()];
    }
    Vec::new()
}

/// The raw request/response pair is written to the target directory for audit.
///
/// §6.4 splits the record: `<slot>.json` holds metadata plus the same redacted
/// view the model received, while `<slot>.body` (and `<slot>.request` when a
/// request body exists) holds the untouched bytes. Only the metadata file is ever
/// read back into a prompt or shown by default.
fn agent_write_http_record(
    context: &AgentRunContext,
    index: usize,
    request: &JsonValue,
    response: &JsonValue,
    body_bytes: &[u8],
) -> Result<String, String> {
    use crate::agent_runtime::secrets::redact_json;
    let (directory, directory_handle) = prepare_agent_artifact_write_directory(
        &context.target_dir,
        AGENT_HTTP_DIRECTORY,
    )?;
    // The per-run sequence restarts on a fresh attempt over the same target
    // directory, so the slot is chosen by what is free rather than by the counter
    // — otherwise the second attempt would fail to record anything at all.
    let mut slot = index.max(1);
    while ["json", "body", "request"].iter().try_fold(false, |occupied, extension| {
        agent_artifact_slot_exists(directory_handle.as_ref(), &format!("{slot:04}.{extension}"))
            .map(|exists| occupied || exists)
    })? {
        slot += 1;
    }
    let name = format!("{slot:04}.json");
    let raw_request_body = request
        .get("body")
        .and_then(JsonValue::as_str)
        .filter(|body| !body.is_empty())
        .map(str::to_string);
    let header_names: Vec<String> = request
        .get("headers")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row.get("name").and_then(JsonValue::as_str))
        .map(str::to_string)
        .collect();
    let metadata = serde_json::json!({
        "identity": request.get("identity"),
        "method": request.get("method"),
        "url": request.get("url"),
        "headerNames": header_names,
        "contractKey": request.get("contractKey"),
        "bodyBytes": raw_request_body.as_ref().map(String::len),
    });
    // TECH DEBT (§6.4): the payload files are unredacted and rely on 0600
    // permissions plus the private target directory; they are not yet encrypted
    // with a task-level key or a system-keychain-derived one.
    let record = serde_json::json!({
        "index": slot,
        "request": redact_json(&metadata),
        "response": response,
        "payloadFile": format!("{slot:04}.body"),
        "payloadBytes": body_bytes.len(),
        "requestPayloadFile": raw_request_body.as_ref().map(|_| format!("{slot:04}.request")),
    });
    write_agent_artifact_file(
        &directory,
        directory_handle.as_ref(),
        &format!("{slot:04}.body"),
        body_bytes,
    )?;
    if let Some(body) = raw_request_body {
        // The complete header set — including the credential the identity carried —
        // is what makes the raw request replayable for reproduction.
        let headers = request
            .get("headers")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let dump = format!(
            "{} {}\n{}\n\n{}",
            request
                .get("method")
                .and_then(JsonValue::as_str)
                .unwrap_or("GET"),
            request
                .get("url")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
            headers
                .iter()
                .filter_map(|row| Some(format!(
                    "{}: {}",
                    row.get("name")?.as_str()?,
                    row.get("value")?.as_str()?
                )))
                .collect::<Vec<_>>()
                .join("\n"),
            body
        );
        write_agent_artifact_file(
            &directory,
            directory_handle.as_ref(),
            &format!("{slot:04}.request"),
            dump.as_bytes(),
        )?;
    }
    // Publish metadata last: readers cannot mistake an interrupted body write
    // for a complete Broker record. The files themselves were synced first.
    write_agent_artifact_file(
        &directory,
        directory_handle.as_ref(),
        &name,
        record.to_string().as_bytes(),
    )?;
    if let Some(directory) = directory_handle.as_ref() {
        directory.sync_all().map_err(|_| "artifact_directory_sync_failed")?;
    }
    Ok(name)
}

