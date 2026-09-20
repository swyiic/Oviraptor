fn agent_tool_browser_action(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let action_key = value_first(arguments, &["actionKey"]);
    let identity_key = value_first(arguments, &["identity"]);
    let family = value_first(arguments, &["family"]);
    let Some(action) = agent_find_evidence_action(&context.evidence, &action_key) else {
        return serde_json::json!({
            "status": "insufficient_evidence",
            "reason": format!("{action_key} 不在本次任务的前端动作图中；只能执行已定位的按钮、链接、表单或路由"),
            "availableActions": agent_evidence_action_keys(&context.evidence).iter().take(16).collect::<Vec<_>>(),
        });
    };
    let method_argument = value_first(&action, &["method"]).to_ascii_uppercase();
    let method = if method_argument.is_empty() { "GET".to_string() } else { method_argument };
    if !AGENT_READ_METHODS.contains(&method.as_str()) {
        return serde_json::json!({
            "error": "动作图中该条目是写操作；只能由 replay_http 走已授权契约",
            "code": "mutation_requires_contract",
        });
    }
    let Some(identity) = agent_identity_of(context, &identity_key) else {
        return agent_tool_error("该身份不属于当前任务", "identity_not_found");
    };
    // Preferred path: perform the located control inside a real browser session,
    // so the delta is what that step actually requested.
    let mut browser_attempt: Option<String> = None;
    let browser_report = match agent_browser_action_run(
        context, runtime, &identity, &action, &action_key, &family,
    ) {
        Ok(value) => value,
        Err(error) => {
            browser_attempt = Some(error);
            None
        }
    };
    if let Some(report) = browser_report {
        return report;
    }
    let Some(url) = action.get("url").and_then(JsonValue::as_str).map(str::to_string) else {
        return serde_json::json!({
            "status": "insufficient_evidence",
            "reason": "该动作没有可定位的目标 URL，且本任务没有可用的浏览器运行时",
            "browserDriven": false,
            "browserAttempt": browser_attempt.unwrap_or_else(|| "no_browser_runtime".to_string()),
        });
    };
    let response = match agent_http_request(
        context,
        runtime,
        &identity_key,
        &method,
        &url,
        Vec::new(),
        None,
        action.get("contentType").and_then(JsonValue::as_str).map(str::to_string),
        "",
        &family,
        ScopeSource::BrowserEntry,
        "browser_action",
    ) {
        Ok(value) => value,
        Err(error) => return error,
    };
    if runtime.credit_family(&family) {
        runtime.last_progress.new_families += 1;
    }
    let entry_request = runtime
        .requests
        .last()
        .map(|trace| trace.id.clone())
        .unwrap_or_default();
    runtime.credit_coverage(&family, "browser_action", "", &[entry_request]);
    // Degraded path: only the located URL was fetched. The APIs under
    // `triggeredApis` are attributions from the earlier deterministic capture,
    // not requests this call made, and the report says exactly that.
    let mut attributed = Vec::new();
    for item in action
        .get("triggeredApis")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let path = normalized_investigation_path(&value_first(&item, &["url", "path"]));
        let verb = value_first(&item, &["method"]).to_ascii_uppercase();
        if verb.is_empty() {
            continue;
        }
        if runtime.record_endpoint(&verb, &path) {
            runtime.last_progress.new_endpoints += 1;
        }
        attributed.push(format!("{verb} {path}"));
        let names: Vec<String> = item
            .get("parameters")
            .and_then(JsonValue::as_array)
            .map(|rows| rows.iter().filter_map(JsonValue::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        runtime.last_progress.new_parameters += runtime.record_parameters(&verb, &path, &names);
    }
    serde_json::json!({
        "action": agent_compact_item(&action),
        "browserDriven": false,
        "browserAttempt": browser_attempt.unwrap_or_else(|| "no_browser_runtime".to_string()),
        "navigationStatus": response.get("status"),
        "navigation": serde_json::json!({
            "method": value_first(&response, &["method"]),
            "url": value_first(&response, &["url"]),
            "status": response.get("status"),
            "bytes": response.get("bytes"),
        }),
        "attributedApis": attributed,
        "note": "未能驱动真实浏览器：本结果只包含该动作 URL 的直接请求，attributedApis 是此前确定性采集的归因而非本次观测",
    })
}

/// Perform one located control in a real browser session through the bundled CDP
/// probe and report the requests that step produced. `Ok(None)` means this build
/// has no browser runtime, so the caller must say the run was not browser-driven
/// instead of presenting HTTP replay as a browser action.
fn agent_browser_action_run(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    identity: &AgentIdentity,
    action: &JsonValue,
    action_key: &str,
    family: &str,
) -> Result<Option<JsonValue>, String> {
    let Some(browser) = context.browser.clone() else {
        return Ok(None);
    };
    let entry = [value_first(action, &["url"]), value_first(action, &["href"])]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| context.target_url.clone());
    // The probe matches by the id/label/href it re-reads from the live DOM, so
    // the human-readable label is the stablest key available here.
    let filter = [
        value_first(action, &["label"]),
        value_first(action, &["text"]),
        value_first(action, &["name"]),
        value_first(action, &["href"]),
        action_key.to_string(),
    ]
    .into_iter()
    .find(|value| value.trim().chars().count() >= 2)
    .unwrap_or_default();
    if filter.is_empty() {
        return Err("该动作没有可用于在浏览器中定位它的标签或链接".to_string());
    }
    // §5.2: the entry URL is checked by the same gate before a browser is spent on
    // it, so an out-of-scope panel can never be opened on the target's session.
    if let ScopeDecision::Reject { code, reason } = agent_scope_assess(
        context,
        &entry,
        "GET",
        "document",
        ScopeSource::BrowserEntry,
    )
    .decision
    {
        return Ok(Some(agent_tool_error(&reason, &code)));
    }
    let session = agent_session_document(context, identity)?;
    let control = NativeReconControl::new(Duration::from_secs(AGENT_BROWSER_ACTION_SECONDS));
    let thread_control = control.clone();
    let (sender, receiver) = mpsc::sync_channel::<Result<JsonValue, String>>(1);
    let helper = browser.helper;
    let runtime_path = browser.runtime_path.clone();
    let no_proxy = browser.no_proxy.clone();
    let deployment = context.environment.deployment.clone();
    let proxy = context.proxy.clone();
    thread::spawn(move || {
        let result = native_runtime_probe(
            &helper,
            &entry,
            session.as_ref(),
            20,
            45,
            &deployment,
            proxy.as_deref(),
            &no_proxy,
            &runtime_path,
            &[],
            false,
            Some(filter.as_str()),
            &thread_control,
        );
        let _ = sender.send(result);
    });
    let probed = loop {
        match receiver.try_recv() {
            Ok(value) => break value,
            Err(mpsc::TryRecvError::Disconnected) => {
                break Err("浏览器动作执行线程异常退出".to_string())
            }
            Err(mpsc::TryRecvError::Empty) => {
                if runtime.cancelled() {
                    // Stop waiting and let the helper kill its browser process
                    // group instead of holding the round open.
                    control.cancel();
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    };
    let probe = probed?;
    if probe.get("available").and_then(JsonValue::as_bool) == Some(false) {
        return Ok(None);
    }
    let performed = probe
        .get("actions")
        .and_then(JsonValue::as_array)
        .and_then(|rows| rows.last().cloned());
    let Some(performed) = performed else {
        return Ok(Some(serde_json::json!({
            "browserDriven": true,
            "status": "insufficient_evidence",
            "reason": "该动作在本次浏览器会话中未定位到，或被判定为不可转发的控件",
            "stopReason": value_first(&probe, &["stopReason"]),
            "captureStatus": value_first(&probe, &["captureStatus"]),
        })));
    };
    let action_id = value_first(&performed, &["id"]);
    let observed: Vec<JsonValue> = probe
        .get("requests")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|row| value_first(row, &["actionId"]) == action_id)
        .collect();
    let mut delta = Vec::new();
    let mut documents = Vec::new();
    let mut statics = Vec::new();
    let mut third_party = Vec::new();
    let mut rejected = Vec::new();
    let mut telemetry = 0usize;
    for row in &observed {
        let verb = value_first(row, &["method"]).to_ascii_uppercase();
        let url = value_first(row, &["url"]);
        let resource_type = value_first(row, &["resourceType"]);
        let Ok(parsed) = reqwest::Url::parse(&url) else {
            continue;
        };
        if verb.is_empty() {
            continue;
        }
        let assessment =
            agent_scope_assess(context, &url, &verb, &resource_type, ScopeSource::BrowserObserved);
        let path = normalized_investigation_path(parsed.path());
        let host = parsed.host_str().unwrap_or_default().to_string();
        let redirect_code = match row.get("redirectedTo").and_then(JsonValue::as_str) {
            Some(target) => agent_redirect_verdict(context, target, &url),
            None => String::new(),
        };
        match assessment.class {
            ScopeClass::TelemetryOrNoise => {
                telemetry += 1;
                continue;
            }
            ScopeClass::ThirdPartyRequired => {
                // Audit the dependency; never let it become a formal API, and
                // never let it widen what the next request may be (§5.3).
                let entry = serde_json::json!({"host": host, "url": url, "class": assessment.class.as_str()});
                if is_agent_static_resource(&path, &resource_type) {
                    third_party.push(entry);
                } else {
                    rejected.push(serde_json::json!({
                        "host": host,
                        "url": url,
                        "class": assessment.class.as_str(),
                        "reason": assessment.decision.reason(),
                    }));
                }
                continue;
            }
            ScopeClass::OutOfScope => {
                rejected.push(serde_json::json!({
                    "host": host,
                    "url": url,
                    "class": assessment.class.as_str(),
                    "reason": assessment.decision.reason(),
                }));
                continue;
            }
            _ => {}
        }
        if matches!(redirect_code.as_str(), "redirect_out_of_scope" | "redirect_without_location") {
            rejected.push(serde_json::json!({
                "host": host,
                "url": url,
                "class": assessment.class.as_str(),
                "redirectCode": redirect_code,
                "reason": "重定向目标不在冻结授权范围内，只作边界证据",
            }));
            continue;
        }
        if assessment.class != ScopeClass::AuthorizedBusinessApi {
            let row_summary = serde_json::json!({
                "method": verb,
                "path": path,
                "status": row.get("status"),
                "resourceType": resource_type,
                "redirectCode": redirect_code,
            });
            if assessment.class == ScopeClass::AuthorizedDocument {
                documents.push(row_summary);
            } else {
                statics.push(row_summary);
            }
            continue;
        }
        let mut names: Vec<String> = parsed
            .query_pairs()
            .map(|(key, _)| key.to_string())
            .collect();
        names.extend(
            row.get("bodyKeys")
                .and_then(JsonValue::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        );
        // Only an authorized business API is work the target really did for us:
        // it spends budget and it can close a queue entry (§5.3).
        let mut observed_parameters = names.clone();
        observed_parameters.sort();
        runtime.note_observed_request(AgentRequestTrace {
            id: String::new(),
            method: verb.clone(),
            origin: assessment.origin.clone(),
            path: path.clone(),
            identity: identity.key.clone(),
            status: row.get("status").and_then(JsonValue::as_i64).unwrap_or(0),
            family: family.to_string(),
            contract_key: String::new(),
            tool: "browser_action".to_string(),
            invocation_id: 0,
            artifact_id: String::new(),
            structure_hash: String::new(),
            scope_class: assessment.class.as_str().to_string(),
            parameters: observed_parameters,
        });
        runtime.last_progress.new_parameters += runtime.record_parameters(&verb, &path, &names);
        delta.push(serde_json::json!({
            "method": verb,
            "path": path,
            "status": row.get("status"),
            "contentType": resource_type,
            "parameters": names.into_iter().take(24).collect::<Vec<_>>(),
        }));
    }
    if runtime.credit_family(family) {
        runtime.last_progress.new_families += 1;
    }
    let observed: Vec<String> = runtime
        .requests
        .iter()
        .rev()
        .take_while(|trace| trace.tool == "browser_action")
        .map(|trace| trace.id.clone())
        .collect();
    runtime.credit_coverage(family, "browser_action", "", &observed);
    let report = serde_json::json!({
        "browserDriven": true,
        "performedAction": agent_compact_item(&performed),
        "networkDelta": delta.into_iter().take(AGENT_MAX_INSPECT_ITEMS).collect::<Vec<_>>(),
        "authorizedDocuments": documents.into_iter().take(12).collect::<Vec<_>>(),
        "staticResources": statics.into_iter().take(12).collect::<Vec<_>>(),
        "thirdPartyDependencies": third_party.into_iter().take(12).collect::<Vec<_>>(),
        "rejectedObservations": rejected.into_iter().take(12).collect::<Vec<_>>(),
        "telemetryCount": telemetry,
        "stateChanged": performed.get("stateChanged"),
        "afterUrl": value_first(&performed, &["afterUrl"]),
        "stopReason": value_first(&probe, &["stopReason"]),
        "captureStatus": value_first(&probe, &["captureStatus"]),
        "note": "网络增量来自本次真实浏览器执行，按该动作的 actionId 归属",
    });
    if let Err(error) = persist_agent_evidence(context, &report, "browser_action") {
        return Ok(Some(serde_json::json!({
            "error": error,
            "code": "evidence_write_failed",
        })));
    }
    Ok(Some(report))
}

fn agent_evidence_action_entries(evidence: &JsonValue) -> Vec<JsonValue> {
    let mut rows = Vec::new();
    for pointer in ["/investigation/actions", "/actions", "/routeCandidates"] {
        if let Some(items) = evidence.pointer(pointer).and_then(JsonValue::as_array) {
            rows.extend(items.iter().cloned());
        }
    }
    rows
}

fn agent_evidence_action_keys(evidence: &JsonValue) -> Vec<String> {
    agent_evidence_action_entries(evidence)
        .iter()
        .map(agent_action_key)
        .collect()
}

fn agent_action_key(item: &JsonValue) -> String {
    let key = value_first(item, &["key", "id", "actionKey"]);
    if !key.is_empty() {
        return key;
    }
    format!(
        "{}|{}",
        value_first(item, &["type", "role"]),
        normalized_investigation_path(&value_first(item, &["url", "path", "name"]))
    )
}

fn agent_find_evidence_action(evidence: &JsonValue, action_key: &str) -> Option<JsonValue> {
    let needle = action_key.trim();
    if needle.is_empty() {
        return None;
    }
    agent_evidence_action_entries(evidence)
        .into_iter()
        .find(|item| {
            [
                agent_action_key(item),
                value_first(item, &["name", "label", "text"]),
                value_first(item, &["url", "path"]),
            ]
            .into_iter()
            .any(|value| {
                !value.is_empty()
                    && (value == needle || value.contains(needle) || needle.contains(value.as_str()))
            })
        })
}
