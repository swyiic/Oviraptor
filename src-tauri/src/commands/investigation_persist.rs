// Everything that writes the investigation graph and its knowledge layers to SQLite.
// Included from investigation.rs.

#[allow(clippy::too_many_arguments)]
fn persist_knowledge_layers(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    target: &JsonValue,
    apis: &[(String, JsonValue)],
    hypotheses: &[(String, String)],
    stop_reason: &str,
) -> Result<(), String> {
    let Some(project_id) = project_id else { return Ok(()) };
    for (api_key, api) in apis {
        let method = value_first(api, &["method"]);
        let path = normalized_investigation_path(&value_first(api, &["url", "path"]));
        let fact_key = format!("api:{}", investigation_hash(&format!("{method}|{path}")));
        let evidence_hash = investigation_hash(&api.to_string());
        connection.execute(
            "INSERT INTO knowledge_facts(project_id,fact_key,fact_type,subject,predicate,object_json,confidence,source_scan_id,target_url,evidence_hash) VALUES(?1,?2,'api',?3,'exposes',?4,?5,?6,?7,?8) ON CONFLICT(project_id,fact_key,source_scan_id,target_url) DO UPDATE SET object_json=excluded.object_json,confidence=excluded.confidence,evidence_hash=excluded.evidence_hash,last_seen=datetime('now','localtime')",
            params![project_id, fact_key, target_url, serde_json::json!({"apiKey":api_key,"method":method,"path":path,"parameters":investigation_strings(api.get("parameters"))}).to_string(), value_first(api, &["confidence"]), scan_id, target_url, evidence_hash],
        ).map_err(|error| error.to_string())?;
    }
    if let Some(fingerprint) = target.get("fingerprint") {
        for layer in ["frontend", "backend", "server", "waf", "cdn"] {
            let Some(value) = fingerprint.get(layer) else { continue };
            let name = value_first(value, &["name"]);
            if name.is_empty() || name.eq_ignore_ascii_case("unknown") {
                continue;
            }
            let fact_key = format!("technology:{}", investigation_hash(&format!("{layer}|{name}")));
            connection.execute(
                "INSERT INTO knowledge_facts(project_id,fact_key,fact_type,subject,predicate,object_json,confidence,source_scan_id,target_url,evidence_hash) VALUES(?1,?2,'technology',?3,?4,?5,?6,?7,?3,?8) ON CONFLICT(project_id,fact_key,source_scan_id,target_url) DO UPDATE SET object_json=excluded.object_json,confidence=excluded.confidence,evidence_hash=excluded.evidence_hash,last_seen=datetime('now','localtime')",
                params![project_id, fact_key, target_url, layer, value.to_string(), value_first(value, &["confidence"]), scan_id, investigation_hash(&value.to_string())],
            ).map_err(|error| error.to_string())?;
        }
    }
    for (hypothesis_key, category) in hypotheses {
        let strategy_key = format!("bounded:{}", category.to_ascii_lowercase());
        connection.execute(
            "INSERT INTO knowledge_outcomes(project_id,scan_id,target_url,hypothesis_key,strategy_key,outcome,stop_reason,evidence_json) VALUES(?1,?2,?3,?4,?5,'not_executed',?6,'{}') ON CONFLICT(scan_id,target_url,hypothesis_key,strategy_key) DO UPDATE SET stop_reason=excluded.stop_reason",
            params![project_id, scan_id, target_url, hypothesis_key, strategy_key, stop_reason],
        ).map_err(|error| error.to_string())?;
        let contract = verification_contract(category, &JsonValue::Null);
        connection.execute(
            "INSERT INTO knowledge_strategies(project_id,strategy_key,category,title,conditions_json,playbook_json) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(project_id,strategy_key) DO UPDATE SET playbook_json=excluded.playbook_json,updated_at=datetime('now','localtime')",
            params![project_id, strategy_key, category, format!("{} 的有界验证", category), serde_json::json!({"requiresDeterministicEvidence":true,"requiresIndependentSupport":2}).to_string(), contract.to_string()],
        ).map_err(|error| error.to_string())?;
        connection.execute(
            "UPDATE knowledge_strategies SET support_count=(SELECT COUNT(DISTINCT scan_id||'|'||target_url) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2),success_count=(SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('validated','confirmed')),failure_count=(SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('rejected','failed','exhausted')),promoted=CASE WHEN (SELECT COUNT(DISTINCT scan_id||'|'||target_url) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2)>=2 OR (SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('validated','confirmed'))>0 THEN 1 ELSE 0 END,updated_at=datetime('now','localtime') WHERE project_id=?1 AND strategy_key=?2",
            params![project_id, strategy_key],
        ).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn persist_identity_differences(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    identity_keys: &[String],
) -> Result<(), String> {
    let Some(project_id) = project_id else { return Ok(()) };
    if identity_keys.len() < 2 {
        return Ok(());
    }
    let mut existing_endpoints = HashSet::new();
    {
        let mut statement = connection
            .prepare("SELECT api_key FROM investigation_identity_diffs WHERE scan_id=?1 AND target_url=?2")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![scan_id, target_url], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for row in rows {
            existing_endpoints.insert(identity_diff_endpoint_key(&row.map_err(|error| error.to_string())?));
        }
    }
    let mut baselines: HashMap<String, HashSet<String>> = HashMap::new();
    for identity in identity_keys {
        let raw = connection
            .query_row(
                "SELECT api_signatures_json FROM investigation_baselines WHERE project_id=?1 AND target_url=?2 AND identity_key=?3 ORDER BY created_at DESC,id DESC LIMIT 1",
                params![project_id, target_url, identity],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|| "[]".into());
        baselines.insert(
            identity.clone(),
            investigation_strings(Some(&investigation_json(raw)))
                .into_iter()
                .collect(),
        );
    }
    for left_index in 0..identity_keys.len() {
        for right_index in left_index + 1..identity_keys.len() {
            let left = &identity_keys[left_index];
            let right = &identity_keys[right_index];
            let left_apis = baselines.get(left).cloned().unwrap_or_default();
            let right_apis = baselines.get(right).cloned().unwrap_or_default();
            for api_signature in left_apis.symmetric_difference(&right_apis) {
                let endpoint_key = identity_diff_endpoint_key(api_signature);
                if existing_endpoints.contains(&endpoint_key) {
                    continue;
                }
                let present_left = left_apis.contains(api_signature);
                let matrix = serde_json::json!({
                    "left":{"identity":left,"observed":present_left},
                    "right":{"identity":right,"observed":!present_left},
                    "note":"可达性差异是权限边界候选，不会直接判定为漏洞"
                });
                connection.execute(
                    "INSERT OR REPLACE INTO investigation_identity_diffs(project_id,scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type,risk_score,status,matrix_json) VALUES(?1,?2,?3,?4,?5,?6,'reachability',55,'observed',?7)",
                    params![project_id, scan_id, target_url, api_signature, left, right, matrix.to_string()],
                ).map_err(|error| error.to_string())?;
                existing_endpoints.insert(endpoint_key);
            }
        }
    }
    Ok(())
}

pub(crate) fn persist_investigation_graph(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    target: &JsonValue,
) -> Result<InvestigationMetrics, String> {
    for table in [
        "investigation_edges",
        "investigation_nodes",
        "investigation_actions",
        "investigation_api_models",
        "investigation_identity_diffs",
    ] {
        connection.execute(
            &format!("DELETE FROM {table} WHERE scan_id=?1 AND target_url=?2"),
            params![scan_id, target_url],
        ).map_err(|error| error.to_string())?;
    }

    let root_key = format!("target:{}", investigation_hash(target_url));
    investigation_node(connection, project_id, scan_id, target_url, &root_key, "target", target_url, "high", 100, "observed", &serde_json::json!({"url":target_url}))?;
    let identity_keys = scan_identity_keys(connection, scan_id)?;
    for (identity_index, identity) in identity_keys
        .iter()
        .filter(|identity| !anonymous_identity(identity))
        .enumerate()
    {
        let identity_node_key = format!("identity:{}", investigation_hash(identity));
        let payload = identity_node_payload(target, identity, identity_index);
        let label = value_first(&payload, &["identityLabel"]);
        let status = if payload.get("sessionValid").and_then(JsonValue::as_bool) == Some(true) {
            "active"
        } else if payload.get("sessionValid").and_then(JsonValue::as_bool) == Some(false) {
            "invalid"
        } else {
            "unknown"
        };
        investigation_node(connection, project_id, scan_id, target_url, &identity_node_key, "identity", &label, "high", 70, status, &payload)?;
        investigation_edge(connection, project_id, scan_id, target_url, &root_key, "observed_as", &identity_node_key, "high", &serde_json::json!({"source":"scan-policy"}))?;
    }

    let exploration = target.get("runtimeExploration").unwrap_or(&JsonValue::Null);
    let states = exploration.get("states").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    let actions = exploration.get("actions").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    let mut runtime_requests = exploration.get("requests").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    if runtime_requests.is_empty() {
        if let Some(auth_requests) = exploration.get("authSessionRequests").and_then(JsonValue::as_array) {
            runtime_requests = auth_requests.clone();
        }
    }
    let coverage = exploration.get("coverage").cloned().unwrap_or_else(|| serde_json::json!({}));

    for state in &states {
        let state_id = value_first(state, &["id"]);
        if state_id.is_empty() { continue }
        let state_key = format!("state:{state_id}");
        let label = value_first(state, &["title", "url"]);
        let score = state.get("highValueLabels").and_then(JsonValue::as_array).map(|items| (items.len() as i64 * 8).min(40) + 35).unwrap_or(35);
        investigation_node(connection, project_id, scan_id, target_url, &state_key, "page_state", &label, "high", score, "observed", state)?;
        investigation_edge(connection, project_id, scan_id, target_url, &root_key, "contains_state", &state_key, "high", &serde_json::json!({"discoveredFrom":value_first(state, &["discoveredFrom"])}))?;
    }

    for action in &actions {
        let action_id = value_first(action, &["id"]);
        if action_id.is_empty() { continue }
        let action_key = format!("action:{action_id}");
        let state_id = value_first(action, &["stateId"]);
        let state_key = if state_id.is_empty() { String::new() } else { format!("state:{state_id}") };
        let label = value_first(action, &["label", "role"]);
        let request_count = action.get("requestCount").and_then(JsonValue::as_i64).unwrap_or(0);
        let state_changed = action.get("stateChanged").and_then(JsonValue::as_bool).unwrap_or(false);
        let value_score = (action.get("score").and_then(JsonValue::as_i64).unwrap_or(0) + request_count * 8 + if state_changed { 15 } else { 0 }).clamp(0, 100);
        let action_type = if value_first(action, &["role"]).contains("link") { "navigate" } else { "interact" };
        let protocol = serde_json::json!({
            "version":1,
            "preconditions":{"stateKey":state_key,"identities":identity_keys},
            "operation":{"type":action_type,"label":label,"role":value_first(action, &["role"]),"destructive":false},
            "observations":{"beforeUrl":value_first(action, &["beforeUrl"]),"afterUrl":value_first(action, &["afterUrl"]),"requestCount":request_count,"stateChanged":state_changed},
            "outcome":value_first(action, &["outcome"]),
            "stopRules":["confirmed_waf_or_challenge","rate_limit_detected","mutation_blocked"],
            "raw":action
        });
        connection.execute(
            "INSERT INTO investigation_actions(project_id,scan_id,target_url,action_key,state_key,action_type,label,outcome,value_score,protocol_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![project_id, scan_id, target_url, action_key, state_key, action_type, label, value_first(action, &["outcome"]), value_score, protocol.to_string()],
        ).map_err(|error| error.to_string())?;
        investigation_node(connection, project_id, scan_id, target_url, &action_key, "action", &label, "high", value_score, value_first(action, &["outcome"]).as_str(), &protocol)?;
        if !state_key.is_empty() {
            investigation_edge(connection, project_id, scan_id, target_url, &state_key, "offers_action", &action_key, "high", action)?;
        }
        let after_url = value_first(action, &["afterUrl"]);
        if !after_url.is_empty() {
            if let Some(next_state) = states.iter().find(|state| value_first(state, &["url"]) == after_url) {
                let next_id = value_first(next_state, &["id"]);
                if !next_id.is_empty() {
                    investigation_edge(connection, project_id, scan_id, target_url, &action_key, "transitions_to", &format!("state:{next_id}"), "medium", &serde_json::json!({"afterUrl":after_url}))?;
                }
            }
        }
    }

    let requested_mode_ceiling = requested_web_mode_ceiling(connection, scan_id);
    let source_guided_api_limit = match requested_mode_ceiling.as_str() {
        "deep" => 20,
        "standard" => 8,
        _ => 0,
    };
    let mut api_records: HashMap<String, JsonValue> = HashMap::new();
    for api in target.get("apis").and_then(JsonValue::as_array).into_iter().flatten() {
        if investigation_background_noise(api) { continue }
        let method = value_first(api, &["method"]).to_ascii_uppercase();
        let api_url = value_first(api, &["url", "path"]);
        if api_url.is_empty() { continue }
        let path = normalized_investigation_path(&api_url);
        let key = format!("api:{}", investigation_hash(&format!("{}|{}", if method.is_empty() { "UNKNOWN" } else { &method }, path)));
        api_records.insert(key, api.clone());
    }
    for api in target
        .get("apiCandidates")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter(|api| source_mapped_readonly_api(api))
        .take(source_guided_api_limit)
    {
        let method = value_first(api, &["method"]).to_ascii_uppercase();
        let api_url = value_first(api, &["url", "path"]);
        let path = normalized_investigation_path(&api_url);
        let key = format!(
            "api:{}",
            investigation_hash(&format!("{method}|{path}"))
        );
        api_records.entry(key).or_insert_with(|| api.clone());
    }
    for request in &runtime_requests {
        let resource_type = value_first(request, &["resourceType", "transport"]).to_ascii_lowercase();
        if !["xhr", "fetch", "eventsource", "websocket"].contains(&resource_type.as_str()) { continue }
        if investigation_background_noise(request) { continue }
        let method = value_first(request, &["method"]).to_ascii_uppercase();
        let api_url = value_first(request, &["url"]);
        if api_url.is_empty() { continue }
        let path = normalized_investigation_path(&api_url);
        let key = format!("api:{}", investigation_hash(&format!("{}|{}", if method.is_empty() { "GET" } else { &method }, path)));
        api_records.entry(key).and_modify(|current| {
            if let Some(object) = current.as_object_mut() {
                object.insert("runtimeObservation".into(), request.clone());
                object.insert("source".into(), JsonValue::String("browser-runtime".into()));
                object.insert("confidence".into(), JsonValue::String("high".into()));
            }
        }).or_insert_with(|| request.clone());
    }

    let identity_key = identity_keys.first().cloned().unwrap_or_else(|| "anonymous".into());
    let previous = if let Some(project_id) = project_id {
        connection.query_row(
            "SELECT api_signatures_json,parameter_signatures_json,signature FROM investigation_baselines WHERE project_id=?1 AND target_url=?2 AND identity_key=?3 AND source_scan_id<>?4 ORDER BY created_at DESC,id DESC LIMIT 1",
            params![project_id, target_url, identity_key, scan_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        ).optional().map_err(|error| error.to_string())?
    } else { None };
    let previous_apis = previous.as_ref().map(|row| investigation_strings(Some(&investigation_json(row.0.clone()))).into_iter().collect::<HashSet<_>>()).unwrap_or_default();
    let previous_params = previous.as_ref().map(|row| investigation_strings(Some(&investigation_json(row.1.clone()))).into_iter().collect::<HashSet<_>>()).unwrap_or_default();

    let mut api_signatures = Vec::new();
    let mut parameter_signatures = Vec::new();
    let mut persisted_apis = Vec::new();
    let mut identity_api_signatures: HashMap<String, Vec<String>> = identity_keys
        .iter()
        .map(|identity| (identity.clone(), Vec::new()))
        .collect();
    let mut identity_parameter_signatures: HashMap<String, Vec<String>> = identity_keys
        .iter()
        .map(|identity| (identity.clone(), Vec::new()))
        .collect();
    for (api_key, api) in &api_records {
        let method = {
            let value = value_first(api, &["method"]).to_ascii_uppercase();
            if value.is_empty() { "GET".to_string() } else { value }
        };
        let api_url = value_first(api, &["url", "path"]);
        let path = normalized_investigation_path(&api_url);
        let signature = format!("{method}|{path}");
        api_signatures.push(signature.clone());
        let mut api_identity_keys = investigation_strings(api.get("identityKeys"));
        if api_identity_keys.is_empty() {
            let identity = value_first(api, &["identityKey"]);
            if !identity.is_empty() {
                api_identity_keys.push(identity);
            }
        }
        if api_identity_keys.is_empty() {
            api_identity_keys = identity_keys.clone();
        }
        let mut parameters = investigation_strings(api.get("parameters"));
        parameters.extend(investigation_strings(api.get("queryKeys")));
        parameters.extend(investigation_strings(api.get("bodyKeys")));
        parameters.extend(query_parameter_names(&api_url));
        if let Some(observation) = api.get("runtimeObservation") {
            parameters.extend(investigation_strings(observation.get("queryKeys")));
            parameters.extend(investigation_strings(observation.get("bodyKeys")));
        }
        parameters.sort(); parameters.dedup();
        parameter_signatures.extend(parameters.iter().map(|parameter| format!("{signature}|{parameter}")));
        for identity in &api_identity_keys {
            identity_api_signatures.entry(identity.clone()).or_default().push(signature.clone());
            identity_parameter_signatures.entry(identity.clone()).or_default().extend(
                parameters.iter().map(|parameter| format!("{signature}|{parameter}")),
            );
        }
        let baseline_status = if !previous_apis.contains(&signature) { "new" } else if parameters.iter().any(|parameter| !previous_params.contains(&format!("{signature}|{parameter}"))) { "changed" } else { "unchanged" };
        let source = value_first(api, &["source", "extractionEngine"]);
        let confidence = {
            let value = value_first(api, &["confidence"]);
            if value.is_empty() { if api.get("runtimeObservation").is_some() { "high".into() } else { "medium".into() } } else { value }
        };
        let state_id = value_first(api, &["stateId"]);
        let action_id = value_first(api, &["actionId"]);
        let state_keys = if state_id.is_empty() { Vec::new() } else { vec![format!("state:{state_id}")] };
        let action_keys = if action_id.is_empty() || action_id == "initial-load" || action_id == "navigation" { Vec::new() } else { vec![format!("action:{action_id}")] };
        let response_keys = sanitized_investigation_response_keys(api.get("responseKeys"));
        let mut header_names = investigation_strings(api.get("requestHeaderNames"));
        if header_names.is_empty() {
            if let Some(headers) = api.get("requestHeaders").and_then(JsonValue::as_object) {
                header_names = headers.keys().cloned().collect();
            }
        }
        let request_schema = serde_json::json!({"parameters":parameters,"headers":header_names,"contentType":value_first(api, &["contentType", "requestContentType"])});
        let response_schema = serde_json::json!({"status":api.get("statusCode").or_else(|| api.get("status")).cloned().unwrap_or(JsonValue::Null),"contentType":value_first(api, &["contentType"]),"keys":response_keys});
        let auth_scope = if api_identity_keys.iter().all(|identity| identity == "anonymous") { "anonymous_or_unknown" } else if api_identity_keys.iter().any(|identity| identity == "anonymous") { "mixed_identity_observation" } else { "authenticated_observation" };
        connection.execute(
            "INSERT INTO investigation_api_models(project_id,scan_id,target_url,api_key,method,url,normalized_path,source,confidence,auth_scope,parameters_json,request_schema_json,response_schema_json,state_keys_json,action_keys_json,identity_keys_json,observed_count,baseline_status,payload_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,1,?17,?18)",
            params![project_id, scan_id, target_url, api_key, method, api_url, path, source, confidence, auth_scope, serde_json::to_string(&parameters).unwrap_or_else(|_| "[]".into()), request_schema.to_string(), response_schema.to_string(), serde_json::to_string(&state_keys).unwrap_or_else(|_| "[]".into()), serde_json::to_string(&action_keys).unwrap_or_else(|_| "[]".into()), serde_json::to_string(&api_identity_keys).unwrap_or_else(|_| "[]".into()), baseline_status, api.to_string()],
        ).map_err(|error| error.to_string())?;
        let value_score = (45 + if source.contains("runtime") { 25 } else { 0 } + (parameters.len() as i64 * 3).min(18)).min(100);
        investigation_node(connection, project_id, scan_id, target_url, api_key, "api", &format!("{method} {path}"), &confidence, value_score, baseline_status, api)?;
        investigation_edge(connection, project_id, scan_id, target_url, &root_key, "exposes_api", api_key, &confidence, &serde_json::json!({"source":source}))?;
        for state_key in &state_keys {
            investigation_edge(connection, project_id, scan_id, target_url, state_key, "issued_request", api_key, "high", &serde_json::json!({"source":"browser-runtime"}))?;
        }
        for action_key in &action_keys {
            investigation_edge(connection, project_id, scan_id, target_url, action_key, "triggered_request", api_key, "high", &serde_json::json!({"source":"browser-runtime"}))?;
        }
        for parameter in &parameters {
            let parameter_key = format!("parameter:{}", investigation_hash(&format!("{api_key}|{parameter}")));
            investigation_node(connection, project_id, scan_id, target_url, &parameter_key, "parameter", parameter, &confidence, 50, baseline_status, &serde_json::json!({"name":parameter,"apiKey":api_key,"source":source}))?;
            investigation_edge(connection, project_id, scan_id, target_url, api_key, "has_parameter", &parameter_key, &confidence, &serde_json::json!({"name":parameter}))?;
        }
        for identity in &api_identity_keys {
            investigation_edge(connection, project_id, scan_id, target_url, api_key, "observed_with_identity", &format!("identity:{}", investigation_hash(identity)), "high", &serde_json::json!({"identityKey":identity}))?;
        }
        persisted_apis.push((api_key.clone(), api.clone()));
    }
    api_signatures.sort(); api_signatures.dedup();
    parameter_signatures.sort(); parameter_signatures.dedup();

    let opportunities = target.get("opportunities").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    // Keep low-value transport/telemetry in raw runtime evidence, but never
    // turn it into a graph hypothesis or a manual validation queue item.
    let actionable_opportunities = deduplicated_actionable_opportunities(&opportunities);
    // A retry may ingest the same endpoint with a fresh nonce/timestamp. Drop
    // only unstarted queue rows before rebuilding stable contracts; terminal
    // decisions and explicit authorization states remain audit history.
    connection.execute(
        "DELETE FROM investigation_edges WHERE scan_id=?1 AND target_url=?2 AND (source_key IN (SELECT hypothesis_key FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('candidate','ready','needs_more_evidence')) OR target_key IN (SELECT hypothesis_key FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('candidate','ready','needs_more_evidence')))",
        params![scan_id, target_url],
    ).map_err(|error| error.to_string())?;
    connection.execute(
        "DELETE FROM investigation_nodes WHERE scan_id=?1 AND target_url=?2 AND node_key IN (SELECT hypothesis_key FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('candidate','ready','needs_more_evidence'))",
        params![scan_id, target_url],
    ).map_err(|error| error.to_string())?;
    connection.execute(
        "DELETE FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('candidate','ready','needs_more_evidence')",
        params![scan_id, target_url],
    ).map_err(|error| error.to_string())?;
    let mut hypothesis_records = Vec::new();
    for opportunity in &actionable_opportunities {
        let category = value_first(opportunity, &["category"]);
        let title = value_first(opportunity, &["title"]);
        let source_key = stable_opportunity_key(opportunity);
        let hypothesis_key = format!("hypothesis:{}", investigation_hash(&source_key));
        let score = opportunity.get("score").and_then(JsonValue::as_i64).unwrap_or(0).clamp(0, 100);
        let confidence = value_first(opportunity, &["confidence"]);
        let evidence = opportunity.get("evidenceRefs").cloned().unwrap_or_else(|| serde_json::json!([]));
        let endpoint = value_first(opportunity, &["endpoint", "url", "path"]);
        let (ready, readiness_reason) = opportunity_agent_readiness(opportunity);
        let status = if ready { "ready" } else { "candidate" };
        let contract = verification_contract(&category, opportunity);
        let decision = serde_json::json!({
            "eligibleForModel":ready,
            "reason":readiness_reason,
            "requiresHuman":false,
            "authorizationMode":"automatic_bounded",
            "verificationMode":if ready { "ai_auto" } else { "needs_evidence" },
            "humanReviewStage":if ready { "final_verdict_only" } else { "evidence_collection" },
            "suspiciousOnlyEscalation":true,
        });
        connection.execute(
            "INSERT INTO investigation_hypotheses(project_id,scan_id,target_url,hypothesis_key,category,title,status,score,confidence,contract_json,evidence_json,decision_json,source_opportunity_key) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(scan_id,target_url,hypothesis_key) DO UPDATE SET project_id=excluded.project_id,category=excluded.category,title=excluded.title,status=CASE WHEN investigation_hypotheses.status IN ('in_progress','validated','rejected','exhausted') THEN investigation_hypotheses.status ELSE excluded.status END,score=excluded.score,confidence=excluded.confidence,contract_json=excluded.contract_json,evidence_json=excluded.evidence_json,decision_json=excluded.decision_json,source_opportunity_key=excluded.source_opportunity_key,updated_at=datetime('now','localtime')",
            params![project_id, scan_id, target_url, hypothesis_key, category, title, status, score, confidence, contract.to_string(), evidence.to_string(), decision.to_string(), source_key],
        ).map_err(|error| error.to_string())?;
        investigation_node(connection, project_id, scan_id, target_url, &hypothesis_key, "hypothesis", &title, &confidence, score, status, opportunity)?;
        investigation_edge(connection, project_id, scan_id, target_url, &root_key, "raises_hypothesis", &hypothesis_key, &confidence, &evidence)?;
        if !endpoint.is_empty() {
            let path = normalized_investigation_path(&endpoint);
            if let Some((api_key, _)) = persisted_apis.iter().find(|(_, api)| normalized_investigation_path(&value_first(api, &["url", "path"])) == path) {
                investigation_edge(connection, project_id, scan_id, target_url, api_key, "supports_hypothesis", &hypothesis_key, &confidence, opportunity)?;
            }
        }
        if ready {
            hypothesis_records.push((hypothesis_key, category));
        }
    }

    let current_api_set = api_signatures.iter().cloned().collect::<HashSet<_>>();
    let current_param_set = parameter_signatures.iter().cloned().collect::<HashSet<_>>();
    let added_api_count = current_api_set.difference(&previous_apis).count() as i64;
    let removed_api_count = previous_apis.difference(&current_api_set).count() as i64;
    let added_parameter_count = current_param_set.difference(&previous_params).count() as i64;
    let removed_parameter_count = previous_params.difference(&current_param_set).count() as i64;
    let mode_hypothesis_score = match requested_mode_ceiling.as_str() {
        "deep" => 35,
        "standard" => 50,
        _ => 65,
    };
    let ready_hypothesis_count = connection.query_row(
        "SELECT COUNT(*) FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND score>=?3 AND status IN ('ready','in_progress')",
        params![scan_id, target_url, mode_hypothesis_score],
        |row| row.get::<_, i64>(0),
    ).map_err(|error| error.to_string())?;
    let duplicate_count = coverage.get("deduplicatedStateCount").and_then(JsonValue::as_i64).unwrap_or(0) + coverage.get("lowValueStateSkipped").and_then(JsonValue::as_i64).unwrap_or(0);
    let has_baseline = previous.is_some();
    let information_gain = if has_baseline {
        (added_api_count * 12 + added_parameter_count * 6 + ready_hypothesis_count * 8 + actions.len() as i64 * 2 - duplicate_count.min(20)).clamp(0, 100)
    } else {
        ((states.len() as i64 * 3) + (actions.len() as i64 * 3) + (api_signatures.len() as i64 * 5) + (parameter_signatures.len() as i64 * 2) + ready_hypothesis_count * 8).clamp(0, 100)
    };
    let runtime_stop_reason = value_first(exploration, &["stopReason"]);
    let waf_detected = target.get("authSessionValidation").and_then(|value| value.get("wafDetected")).and_then(JsonValue::as_bool).unwrap_or(false) || runtime_stop_reason == "confirmed_waf_or_challenge";
    let token_worthy = !waf_detected && ready_hypothesis_count > 0 && (!has_baseline || added_api_count > 0 || added_parameter_count > 0 || information_gain >= 35);
    let verified_runtime_api_count = persisted_apis
        .iter()
        .filter(|(_, api)| standard_investigation_api(api))
        .count() as i64;
    let source_mapped_readonly_api_count = persisted_apis
        .iter()
        .filter(|(_, api)| source_mapped_readonly_api(api))
        .count() as i64;
    let source_guided_investigation_allowed = !waf_detected
        && !token_worthy
        && requested_mode_ceiling != "quick"
        && verified_runtime_api_count == 0
        && source_mapped_readonly_api_count > 0
        && information_gain >= 30;
    // A real browser request contract is enough for one bounded standard
    // investigation. It is not promoted to a vulnerability hypothesis and it
    // does not weaken the stricter evidence gate used for deep validation.
    let standard_investigation_allowed = source_guided_investigation_allowed || (!waf_detected
        && !token_worthy
        && verified_runtime_api_count > 0
        && (information_gain >= 20
            || verified_runtime_api_count >= 2
            || !actions.is_empty()
            || identity_keys.len() >= 2));
    // Standard/deep tasks must not collapse to reconnaissance-only merely
    // because an SPA did not naturally emit a business XHR during the first
    // browser pass. This opens a small, progressive baseline investigation;
    // the execution plan still enforces WAF, progress and hard-cost limits.
    let baseline_investigation_allowed = !waf_detected
        && !token_worthy
        && !standard_investigation_allowed
        && requested_mode_ceiling != "quick"
        && (exploration
            .get("runtimeProbeAvailable")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
            || !runtime_requests.is_empty());
    let stop_reason = if waf_detected {
        "confirmed_waf_or_challenge"
    } else if source_guided_investigation_allowed {
        "source_mapped_readonly_contracts"
    } else if !runtime_requests.is_empty() && api_signatures.is_empty() {
        "request_evidence_present_no_api_contract"
    } else if has_baseline && added_api_count == 0 && added_parameter_count == 0 {
        "incremental_no_new_value"
    } else if ready_hypothesis_count == 0 && information_gain < 25 {
        "no_high_value_hypothesis"
    } else if runtime_stop_reason.is_empty() {
        "evidence_collection_complete"
    } else {
        runtime_stop_reason.as_str()
    }.to_string();
    let node_count = connection.query_row("SELECT COUNT(*) FROM investigation_nodes WHERE scan_id=?1 AND target_url=?2", params![scan_id,target_url], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let edge_count = connection.query_row("SELECT COUNT(*) FROM investigation_edges WHERE scan_id=?1 AND target_url=?2", params![scan_id,target_url], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let automation_tier = if token_worthy {
        "evidence_deep_validation"
    } else if standard_investigation_allowed {
        "runtime_standard_investigation"
    } else if baseline_investigation_allowed {
        "progressive_baseline_investigation"
    } else {
        "recon_only"
    };
    let manual_deep_dive = manual_deep_dive_plan(
        target,
        &persisted_apis,
        &actions,
        &identity_keys,
        &requested_mode_ceiling,
    );
    let decision = serde_json::json!({
        "schemaVersion":3,"eligibleForModel":token_worthy,
        "standardInvestigationAllowed":standard_investigation_allowed,
        "baselineInvestigationAllowed":baseline_investigation_allowed,
        "automationTier":automation_tier,"informationGain":information_gain,
        "baseline":{"available":has_baseline,"addedApis":added_api_count,"removedApis":removed_api_count,"addedParameters":added_parameter_count,"removedParameters":removed_parameter_count},
        "coverage":coverage,"readyHypotheses":ready_hypothesis_count,"identityCount":identity_keys.len(),
        "observedRequestCount":runtime_requests.len(),
        "verifiedRuntimeApiCount":verified_runtime_api_count,
        "sourceMappedReadOnlyApiCount":source_mapped_readonly_api_count,
        "sourceGuidedInvestigationAllowed":source_guided_investigation_allowed,
        "requestedModeCeiling":requested_mode_ceiling,
        "apiEvidenceSource": if exploration.get("authSessionFallbackUsed").and_then(JsonValue::as_bool).unwrap_or(false) { "auth-session-fallback" } else if exploration.get("runtimeProbeAvailable").and_then(JsonValue::as_bool).unwrap_or(false) { "browser-runtime" } else { "deterministic-or-static" },
        "runtimeProbeAvailable":exploration.get("runtimeProbeAvailable").and_then(JsonValue::as_bool).unwrap_or(false),
        "authSessionCaptureAvailable":exploration.get("authSessionCapture").and_then(|value| value.get("available")).and_then(JsonValue::as_bool).unwrap_or(false),
        "manualDeepDive":manual_deep_dive,
        "coverageSemantics":{"completed":"listed contract executed with usable evidence","notFound":"executed without security impact","notTested":"missing identity, state, data, protocol or environment","neverAssumeSafe":true},
        "stopReason":stop_reason,"rules":{"wafStopsImmediately":true,"authorizationStatusDoesNotInvalidateSession":true,"minimumHypothesisScoreByMode":{"quick":65,"standard":50,"deep":35},"unresolvedStaticCandidatesNeverOpenStandardGate":true,"sourceMappedReadOnlyContractsMayOpenBoundedGate":true}
    });
    connection.execute(
        "INSERT INTO investigation_metrics(scan_id,target_url,project_id,node_count,edge_count,state_count,action_count,api_count,parameter_count,hypothesis_count,added_count,changed_count,removed_count,duplicate_count,information_gain,token_worthy,stop_reason,decision_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18) ON CONFLICT(scan_id,target_url) DO UPDATE SET project_id=excluded.project_id,node_count=excluded.node_count,edge_count=excluded.edge_count,state_count=excluded.state_count,action_count=excluded.action_count,api_count=excluded.api_count,parameter_count=excluded.parameter_count,hypothesis_count=excluded.hypothesis_count,added_count=excluded.added_count,changed_count=excluded.changed_count,removed_count=excluded.removed_count,duplicate_count=excluded.duplicate_count,information_gain=excluded.information_gain,token_worthy=excluded.token_worthy,stop_reason=excluded.stop_reason,decision_json=excluded.decision_json,updated_at=datetime('now','localtime')",
        params![scan_id,target_url,project_id,node_count,edge_count,states.len() as i64,actions.len() as i64,api_signatures.len() as i64,parameter_signatures.len() as i64,actionable_opportunities.len() as i64,added_api_count+added_parameter_count,added_parameter_count+removed_parameter_count,removed_api_count+removed_parameter_count,duplicate_count,information_gain,token_worthy as i64,stop_reason,decision.to_string()],
    ).map_err(|error| error.to_string())?;
    if let Some(project_id) = project_id {
        let signature = investigation_hash(&format!("{}|{}", api_signatures.join("\n"), parameter_signatures.join("\n")));
        for identity in &identity_keys {
            let mut identity_apis = identity_api_signatures.remove(identity).unwrap_or_default();
            let mut identity_parameters = identity_parameter_signatures.remove(identity).unwrap_or_default();
            identity_apis.sort(); identity_apis.dedup();
            identity_parameters.sort(); identity_parameters.dedup();
            let identity_signature = if identity_keys.len() == 1 { signature.clone() } else { investigation_hash(&format!("{}|{}", identity_apis.join("\n"), identity_parameters.join("\n"))) };
            connection.execute(
                "INSERT INTO investigation_baselines(project_id,target_url,identity_key,source_scan_id,signature,api_signatures_json,parameter_signatures_json,metrics_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(project_id,target_url,identity_key,source_scan_id) DO UPDATE SET signature=excluded.signature,api_signatures_json=excluded.api_signatures_json,parameter_signatures_json=excluded.parameter_signatures_json,metrics_json=excluded.metrics_json",
                params![project_id,target_url,identity,scan_id,identity_signature,serde_json::to_string(&identity_apis).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&identity_parameters).unwrap_or_else(|_| "[]".into()),decision.to_string()],
            ).map_err(|error| error.to_string())?;
        }
    }
    if let Some(comparisons) = target.get("identityComparisons").and_then(JsonValue::as_array) {
        for comparison in comparisons {
            let api_key = value_first(comparison,&["apiKey"]);
            if value_first(comparison,&["differenceType"]) == "feature_surface" || api_key.to_ascii_lowercase().starts_with("feature:") { continue }
            let endpoint = api_key.split('|').find(|part| part.starts_with('/')).unwrap_or(&api_key);
            if investigation_background_noise(&serde_json::json!({"url":endpoint,"method":api_key.split('|').next().unwrap_or("GET")})) { continue }
            let mut matrix = comparison.get("matrix").cloned().unwrap_or_else(|| serde_json::json!({}));
            sanitize_identity_matrix(&mut matrix);
            let identities = matrix.as_object().map(|value| value.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
            if identities.len() < 2 { continue }
            connection.execute(
                "INSERT OR REPLACE INTO investigation_identity_diffs(project_id,scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type,risk_score,status,matrix_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'observed',?9)",
                params![project_id,scan_id,target_url,api_key,identities[0],identities[1],value_first(comparison,&["differenceType"]),comparison.get("riskScore").and_then(JsonValue::as_i64).unwrap_or(0),matrix.to_string()],
            ).map_err(|error| error.to_string())?;
        }
    }
    persist_identity_differences(connection, project_id, scan_id, target_url, &identity_keys)?;
    persist_knowledge_layers(connection, project_id, scan_id, target_url, target, &persisted_apis, &hypothesis_records, &stop_reason)?;
    insert_finding(connection, scan_id, target_url, "investigation", "investigation_decision", "information-gain", "调查决策", if token_worthy { "medium" } else { "info" }, &decision)?;
    connection.execute(
        "UPDATE sentinel_targets SET value_score=MAX(value_score,?1),scan_mode=CASE WHEN ?2=1 AND scan_mode<>'deep' THEN 'evidence_guided' WHEN ?3=1 AND scan_mode<>'deep' THEN 'standard' ELSE scan_mode END,routing_reason=?4,updated_at=datetime('now','localtime') WHERE scan_id=?5 AND url=?6",
        params![information_gain,token_worthy as i64,standard_investigation_allowed as i64,if token_worthy { format!("调查图谱存在风险证据，进入证据后深挖；信息增益 {information_gain}/100") } else if source_guided_investigation_allowed { format!("已从源码映射还原 {source_mapped_readonly_api_count} 个高置信度只读接口，进入有界目标调查；信息增益 {information_gain}/100") } else if standard_investigation_allowed { format!("已采集 {verified_runtime_api_count} 个真实运行时接口，进入一次有界标准调查；信息增益 {information_gain}/100") } else { format!("本地调查停止：{stop_reason}；信息增益 {information_gain}/100") },scan_id,target_url],
    ).map_err(|error| error.to_string())?;

    Ok(InvestigationMetrics {
        scan_id: scan_id.into(), target_url: target_url.into(), node_count, edge_count,
        state_count: states.len() as i64, action_count: actions.len() as i64,
        api_count: api_signatures.len() as i64, parameter_count: parameter_signatures.len() as i64,
        hypothesis_count: actionable_opportunities.len() as i64, added_count: added_api_count + added_parameter_count,
        changed_count: added_parameter_count + removed_parameter_count, removed_count: removed_api_count + removed_parameter_count,
        duplicate_count, information_gain, token_worthy, stop_reason, decision,
        updated_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}
