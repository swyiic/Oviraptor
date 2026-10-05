// Read fixed local artifacts through the existing no-follow descriptor reader.
fn client_side_verify_input(
    context: &SpecialistTransportContext<'_>,
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    input: &JsonValue,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::client_side;
    let task = client_side::assignment(db, lease, child)?;
    if serde_json::to_value(&task).map_err(|e| e.to_string())? != *input {
        return Err("client_side_dispatch_task_changed".into());
    }
    client_side_verify_frozen_artifacts(&task, db, lease, context.usage_dir)
}
fn client_side_verify_frozen_artifacts(
    task: &crate::agent_runtime::multi_agent::client_side::Task,
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    target_dir: &Path,
) -> Result<(), String> {
    for observation in &task.observations {
        if let Some(proof) = &observation.source_proof {
            if client_side_capture_source(db, lease, &observation.id, target_dir)? != *observation
                || proof.is_empty()
            {
                return Err("client_side_original_source_changed".into());
            }
        }
        let path = &observation.artifact.path;
        let (base, name) = match path.strip_prefix("agent-http/") {
            Some(name) => (target_dir.join(AGENT_HTTP_DIRECTORY), name),
            None => (target_dir.to_path_buf(), path.as_str()),
        };
        let directory =
            open_agent_artifact_directory(&base).ok_or("client_side_artifact_root_invalid")?;
        let (bytes, _) = read_agent_artifact_file(&directory, name, 1_048_576)
            .ok_or("client_side_artifact_missing")?;
        if crate::agent_runtime::store::artifact_id(&bytes) != observation.artifact.content_hash {
            return Err("client_side_artifact_hash_changed".into());
        }
        let value = client_side_local_value(&observation.kind, &bytes)?;
        if value != observation.value {
            return Err("client_side_observation_not_in_artifact".into());
        }
    }
    Ok(())
}
fn client_side_model_budget(
    messages: &[JsonValue],
    profile: &AgentModelProfile,
) -> Result<i64, String> {
    let bytes = serde_json::to_vec(messages)
        .map_err(|_| "client_side_input_invalid")?
        .len();
    let tokens = i64::try_from(bytes)
        .ok()
        .and_then(|n| n.checked_add(512 + 256))
        .filter(|n| *n <= 8000)
        .ok_or("client_side_input_oversized")?;
    if profile.max_context_tokens > 0 && tokens as u64 > profile.max_context_tokens {
        return Err("client_side_context_budget_exceeded".into());
    }
    Ok(tokens)
}
fn client_side_validate_received(
    db: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    payload: &JsonValue,
    target_dir: Option<&Path>,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{client_side, specialist};
    let task = client_side::assignment(db, lease, child)?;
    if payload["clientSideTask"] != serde_json::to_value(&task).map_err(|e| e.to_string())? {
        return Err("client_side_delivery_task_changed".into());
    }
    let receipt = specialist::received_for_reconciliation(db, lease, child)?;
    let text = payload["summary"]
        .as_str()
        .ok_or("client_side_response_missing")?;
    if receipt.text != text || receipt.usage != *usage || !receipt.rejection.is_empty() {
        return Err("client_side_delivery_receipt_mismatch".into());
    }
    client_side::validate_assessment(text, &task)?;
    if task.observations.iter().any(|o| o.source_proof.is_some()) {
        client_side_verify_frozen_artifacts(
            &task,
            db,
            lease,
            target_dir.ok_or("client_side_original_artifact_root_required")?,
        )?;
    }
    Ok(())
}

// Parse configuration literals; never execute markup, JavaScript or a package.
fn client_side_local_value(kind: &str, bytes: &[u8]) -> Result<String, String> {
    if kind == "meta_csp" {
        let html = std::str::from_utf8(bytes).map_err(|_| "client_side_markup_invalid")?;
        let tags = regex::Regex::new(r"(?is)<meta\b[^>]{0,4096}>")
            .map_err(|_| "client_side_parser_invalid")?;
        let equiv = regex::Regex::new(r#"(?is)http-equiv\s*=\s*["']content-security-policy["']"#)
            .map_err(|_| "client_side_parser_invalid")?;
        let content = regex::Regex::new(r#"(?is)content\s*=\s*(?:"([^"]*)"|'([^']*)')"#)
            .map_err(|_| "client_side_parser_invalid")?;
        let mut values = Vec::new();
        for tag in tags.find_iter(html) {
            if equiv.is_match(tag.as_str()) {
                if let Some(c) = content.captures(tag.as_str()) {
                    if let Some(v) = c.get(1).or_else(|| c.get(2)) {
                        values.push(v.as_str().to_string());
                    }
                }
            }
        }
        if values.len() != 1 {
            return Err("client_side_meta_csp_ambiguous".into());
        }
        return Ok(values.remove(0));
    }
    let record: JsonValue =
        serde_json::from_slice(bytes).map_err(|_| "client_side_http_artifact_invalid")?;
    let headers = record
        .pointer("/response/securityRelevantHeaders")
        .and_then(JsonValue::as_array)
        .ok_or("client_side_http_headers_missing")?;
    let values: Vec<_> = headers
        .iter()
        .filter(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case("content-security-policy"))
        })
        .filter_map(|h| h["value"].as_str())
        .collect();
    // securityRelevantHeaders is an allowlist, so omission there is not absence.
    let names = record
        .pointer("/response/responseHeaderNames")
        .and_then(JsonValue::as_array)
        .ok_or("client_side_complete_header_names_missing")?;
    if names.iter().any(|n| n.as_str().is_none()) {
        return Err("client_side_complete_header_names_invalid".into());
    }
    let present = names.iter().any(|n| {
        n.as_str()
            .is_some_and(|n| n.eq_ignore_ascii_case("content-security-policy"))
    });
    match (kind, present, values.as_slice()) {
        ("http_csp_absent", false, []) => Ok("not_present".into()),
        ("csp_header", true, [one]) => Ok(one.to_string()),
        _ => Err("client_side_http_csp_ambiguous".into()),
    }
}
