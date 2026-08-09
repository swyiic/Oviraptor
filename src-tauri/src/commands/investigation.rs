fn investigation_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("{digest:x}").chars().take(24).collect()
}

fn investigation_json(text: String) -> JsonValue {
    serde_json::from_str(&text).unwrap_or(JsonValue::Null)
}

fn investigation_strings(value: Option<&JsonValue>) -> Vec<String> {
    let mut values = value
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| match item {
            JsonValue::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
            JsonValue::Object(_) => item
                .get("name")
                .or_else(|| item.get("key"))
                .or_else(|| item.get("path"))
                .and_then(JsonValue::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(|text| text.trim().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn normalized_investigation_path(value: &str) -> String {
    let without_fragment = value.split('#').next().unwrap_or(value);
    let without_query = without_fragment.split('?').next().unwrap_or(without_fragment);
    let path = if let Some(scheme) = without_query.find("://") {
        without_query[scheme + 3..]
            .find('/')
            .map(|index| &without_query[scheme + 3 + index..])
            .unwrap_or("/")
    } else {
        without_query
    };
    let mut normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

fn query_parameter_names(value: &str) -> Vec<String> {
    let Some(query) = value.split('?').nth(1).map(|part| part.split('#').next().unwrap_or(part)) else {
        return Vec::new();
    };
    let mut names = query
        .split('&')
        .filter_map(|pair| pair.split('=').next())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn investigation_node(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    node_key: &str,
    node_type: &str,
    label: &str,
    confidence: &str,
    value_score: i64,
    status: &str,
    payload: &JsonValue,
) -> Result<(), String> {
    connection.execute(
        "INSERT INTO investigation_nodes(project_id,scan_id,target_url,node_key,node_type,label,confidence,value_score,status,payload_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(scan_id,target_url,node_key) DO UPDATE SET project_id=excluded.project_id,node_type=excluded.node_type,label=excluded.label,confidence=excluded.confidence,value_score=excluded.value_score,status=excluded.status,payload_json=excluded.payload_json,last_seen=datetime('now','localtime')",
        params![project_id, scan_id, target_url, node_key, node_type, label, confidence, value_score, status, payload.to_string()],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn investigation_edge(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    scan_id: &str,
    target_url: &str,
    source_key: &str,
    relation: &str,
    target_key: &str,
    confidence: &str,
    evidence: &JsonValue,
) -> Result<(), String> {
    connection.execute(
        "INSERT OR REPLACE INTO investigation_edges(project_id,scan_id,target_url,source_key,relation,target_key,confidence,evidence_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![project_id, scan_id, target_url, source_key, relation, target_key, confidence, evidence.to_string()],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn verification_contract(category: &str, opportunity: &JsonValue) -> JsonValue {
    let normalized = category.to_ascii_lowercase();
    let common_stop = serde_json::json!([
        "confirmed_waf_or_challenge",
        "rate_limit_detected",
        "scope_boundary",
        "two_consecutive_no_information_gain"
    ]);
    let endpoint = value_first(opportunity, &["endpoint", "url", "path"]);
    let method = value_first(opportunity, &["method"]).to_ascii_uppercase();
    let parameters = investigation_strings(opportunity.get("parameters"));
    if normalized.contains("idor")
        || normalized.contains("author")
        || normalized.contains("permission")
        || normalized.contains("access")
    {
        return serde_json::json!({
            "kind":"authorization-differential",
            "objective":"比较至少两个身份对同一业务对象的授权边界，不以单个 401/403 判定会话失效",
            "preconditions":["two_valid_identities_or_authenticated_plus_anonymous","stable_object_reference","same_request_shape"],
            "requiredEvidence":["control_response","cross_identity_response","object_ownership_context","status_body_or_field_difference"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":3,
            "mutationPolicy":"read_only_unless_explicitly_approved","successRule":"unauthorized_identity_obtains_protected_object_or_action",
            "stopRules":common_stop
        });
    }
    if normalized.contains("auth") || normalized.contains("session") || normalized.contains("login") {
        return serde_json::json!({
            "kind":"session-boundary-differential",
            "objective":"比较匿名与有效会话的可达页面、请求头和响应结构",
            "preconditions":["validated_session","anonymous_control"],
            "requiredEvidence":["authenticated_request","anonymous_control","redirect_or_response_difference","session_validity_signal"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":3,
            "mutationPolicy":"read_only","successRule":"protected_data_or_action_available_without_required_identity",
            "stopRules":common_stop
        });
    }
    if normalized.contains("upload") || normalized.contains("file") {
        return serde_json::json!({
            "kind":"safe-upload-contract",
            "objective":"仅使用无害标记文件验证类型、存储和访问控制",
            "preconditions":["upload_endpoint_observed","test_artifact_is_benign","no_overwrite"],
            "requiredEvidence":["control_upload_policy","server_response","retrieval_or_rejection_result","cleanup_result"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":2,
            "mutationPolicy":"benign_marker_only_and_cleanup","successRule":"policy_allows_disallowed_type_or_cross_identity_access",
            "stopRules":common_stop
        });
    }
    if normalized.contains("inject") || normalized.contains("sql") || normalized.contains("xss") {
        return serde_json::json!({
            "kind":"bounded-input-differential",
            "objective":"使用控制值与无害探测值比较确定性响应差异",
            "preconditions":["parameter_observed","stable_control_response"],
            "requiredEvidence":["control_request","test_request","status_timing_or_schema_difference","parameter_source"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":3,
            "mutationPolicy":"non_destructive_payloads_only","successRule":"repeatable_security_relevant_response_difference",
            "stopRules":common_stop
        });
    }
    if normalized.contains("register") || normalized.contains("account") {
        return serde_json::json!({
            "kind":"registration-entry-contract",
            "objective":"确认注册入口、字段和前置约束，不自动创建真实账户",
            "preconditions":["registration_entry_observed"],
            "requiredEvidence":["entry_source","field_schema","request_method","server_precondition"],
            "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":2,
            "mutationPolicy":"discovery_only_no_account_creation","successRule":"registration_surface_and_constraints_are_reproducible",
            "stopRules":common_stop
        });
    }
    serde_json::json!({
        "kind":"bounded-evidence-validation",
        "objective":"只验证已有证据指向的安全假设",
        "preconditions":["concrete_endpoint_or_feature","reproducible_control"],
        "requiredEvidence":["source_evidence","control_result","test_result","impact_explanation"],
        "endpoint":endpoint,"method":method,"parameters":parameters,"maxAttempts":2,
        "mutationPolicy":"read_only_or_non_destructive","successRule":"repeatable_security_impact_with_request_evidence",
        "stopRules":common_stop
    })
}

fn scan_identity_keys(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Vec<String>, String> {
    let policy = connection
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .map(investigation_json)
        .unwrap_or(JsonValue::Null);
    let mut ids = investigation_strings(policy.get("authSessionIds"));
    if let Some(primary) = policy
        .get("authSessionId")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        ids.push(primary.trim().to_string());
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(vec!["anonymous".into()]);
    }
    let mut identities = Vec::new();
    for id in ids {
        let name = connection
            .query_row(
                "SELECT name FROM browser_auth_sessions WHERE id=?1",
                [&id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        identities.push(if name.trim().is_empty() {
            format!("session:{id}")
        } else {
            format!("session:{id}:{name}")
        });
    }
    Ok(identities)
}

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
    for identity in &identity_keys {
        let identity_node_key = format!("identity:{}", investigation_hash(identity));
        investigation_node(connection, project_id, scan_id, target_url, &identity_node_key, "identity", identity, "high", 70, "active", &serde_json::json!({"identityKey":identity}))?;
        investigation_edge(connection, project_id, scan_id, target_url, &root_key, "observed_as", &identity_node_key, "high", &serde_json::json!({"source":"scan-policy"}))?;
    }

    let exploration = target.get("runtimeExploration").unwrap_or(&JsonValue::Null);
    let states = exploration.get("states").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    let actions = exploration.get("actions").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    let runtime_requests = exploration.get("requests").and_then(JsonValue::as_array).cloned().unwrap_or_default();
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

    let mut api_records: HashMap<String, JsonValue> = HashMap::new();
    for api in target.get("apis").and_then(JsonValue::as_array).into_iter().flatten() {
        let method = value_first(api, &["method"]).to_ascii_uppercase();
        let api_url = value_first(api, &["url", "path"]);
        if api_url.is_empty() { continue }
        let path = normalized_investigation_path(&api_url);
        let key = format!("api:{}", investigation_hash(&format!("{}|{}", if method.is_empty() { "UNKNOWN" } else { &method }, path)));
        api_records.insert(key, api.clone());
    }
    for request in &runtime_requests {
        let resource_type = value_first(request, &["resourceType"]).to_ascii_lowercase();
        if !["xhr", "fetch", "eventsource", "websocket"].contains(&resource_type.as_str()) { continue }
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
        let response_keys = investigation_strings(api.get("responseKeys"));
        let request_schema = serde_json::json!({"parameters":parameters,"headers":investigation_strings(api.get("requestHeaderNames")),"contentType":value_first(api, &["requestContentType"])});
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
    let mut hypothesis_records = Vec::new();
    for (index, opportunity) in opportunities.iter().enumerate() {
        let category = value_first(opportunity, &["category"]);
        let title = value_first(opportunity, &["title"]);
        let source_key = {
            let value = value_first(opportunity, &["opportunityKey"]);
            if value.is_empty() { format!("opportunity-{index}") } else { value }
        };
        let hypothesis_key = format!("hypothesis:{}", investigation_hash(&format!("{category}|{source_key}|{title}")));
        let score = opportunity.get("score").and_then(JsonValue::as_i64).unwrap_or(0).clamp(0, 100);
        let confidence = value_first(opportunity, &["confidence"]);
        let evidence = opportunity.get("evidenceRefs").cloned().unwrap_or_else(|| serde_json::json!([]));
        let parameters = investigation_strings(opportunity.get("parameters"));
        let endpoint = value_first(opportunity, &["endpoint", "url", "path"]);
        let ready = score >= 70 && (!endpoint.is_empty() || !parameters.is_empty() || evidence.as_array().is_some_and(|items| !items.is_empty()));
        let status = if ready { "ready" } else { "candidate" };
        let contract = verification_contract(&category, opportunity);
        let decision = serde_json::json!({"eligibleForModel":ready,"reason":if ready { "high_score_with_concrete_evidence" } else { "insufficient_deterministic_evidence" },"requiresHuman":category.to_ascii_lowercase().contains("business_logic")});
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
    let ready_hypothesis_count = connection.query_row(
        "SELECT COUNT(*) FROM investigation_hypotheses WHERE scan_id=?1 AND target_url=?2 AND status IN ('ready','in_progress')",
        params![scan_id, target_url],
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
    let stop_reason = if waf_detected {
        "confirmed_waf_or_challenge"
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
    let decision = serde_json::json!({
        "schemaVersion":1,"eligibleForModel":token_worthy,"informationGain":information_gain,
        "baseline":{"available":has_baseline,"addedApis":added_api_count,"removedApis":removed_api_count,"addedParameters":added_parameter_count,"removedParameters":removed_parameter_count},
        "coverage":coverage,"readyHypotheses":ready_hypothesis_count,"identityCount":identity_keys.len(),
        "stopReason":stop_reason,"rules":{"wafStopsImmediately":true,"authorizationStatusDoesNotInvalidateSession":true,"minimumHypothesisScore":70}
    });
    connection.execute(
        "INSERT INTO investigation_metrics(scan_id,target_url,project_id,node_count,edge_count,state_count,action_count,api_count,parameter_count,hypothesis_count,added_count,changed_count,removed_count,duplicate_count,information_gain,token_worthy,stop_reason,decision_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18) ON CONFLICT(scan_id,target_url) DO UPDATE SET project_id=excluded.project_id,node_count=excluded.node_count,edge_count=excluded.edge_count,state_count=excluded.state_count,action_count=excluded.action_count,api_count=excluded.api_count,parameter_count=excluded.parameter_count,hypothesis_count=excluded.hypothesis_count,added_count=excluded.added_count,changed_count=excluded.changed_count,removed_count=excluded.removed_count,duplicate_count=excluded.duplicate_count,information_gain=excluded.information_gain,token_worthy=excluded.token_worthy,stop_reason=excluded.stop_reason,decision_json=excluded.decision_json,updated_at=datetime('now','localtime')",
        params![scan_id,target_url,project_id,node_count,edge_count,states.len() as i64,actions.len() as i64,api_signatures.len() as i64,parameter_signatures.len() as i64,opportunities.len() as i64,added_api_count+added_parameter_count,added_parameter_count+removed_parameter_count,removed_api_count+removed_parameter_count,duplicate_count,information_gain,token_worthy as i64,stop_reason,decision.to_string()],
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
            let matrix = comparison.get("matrix").cloned().unwrap_or_else(|| serde_json::json!({}));
            let identities = matrix.as_object().map(|value| value.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
            if identities.len() < 2 { continue }
            connection.execute(
                "INSERT OR REPLACE INTO investigation_identity_diffs(project_id,scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type,risk_score,status,matrix_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'observed',?9)",
                params![project_id,scan_id,target_url,value_first(comparison,&["apiKey"]),identities[0],identities[1],value_first(comparison,&["differenceType"]),comparison.get("riskScore").and_then(JsonValue::as_i64).unwrap_or(0),matrix.to_string()],
            ).map_err(|error| error.to_string())?;
        }
    }
    persist_identity_differences(connection, project_id, scan_id, target_url, &identity_keys)?;
    persist_knowledge_layers(connection, project_id, scan_id, target_url, target, &persisted_apis, &hypothesis_records, &stop_reason)?;
    insert_finding(connection, scan_id, target_url, "investigation", "investigation_decision", "information-gain", "调查决策", if token_worthy { "medium" } else { "info" }, &decision)?;
    connection.execute(
        "UPDATE sentinel_targets SET value_score=MAX(value_score,?1),scan_mode=CASE WHEN ?2=1 AND scan_mode NOT IN ('deep','manual_review') THEN 'evidence_guided' ELSE scan_mode END,routing_reason=?3,updated_at=datetime('now','localtime') WHERE scan_id=?4 AND url=?5",
        params![information_gain,token_worthy as i64,if token_worthy { format!("调查图谱新增高价值证据；信息增益 {information_gain}/100") } else { format!("本地调查停止：{stop_reason}；信息增益 {information_gain}/100") },scan_id,target_url],
    ).map_err(|error| error.to_string())?;

    Ok(InvestigationMetrics {
        scan_id: scan_id.into(), target_url: target_url.into(), node_count, edge_count,
        state_count: states.len() as i64, action_count: actions.len() as i64,
        api_count: api_signatures.len() as i64, parameter_count: parameter_signatures.len() as i64,
        hypothesis_count: opportunities.len() as i64, added_count: added_api_count + added_parameter_count,
        changed_count: added_parameter_count + removed_parameter_count, removed_count: removed_api_count + removed_parameter_count,
        duplicate_count, information_gain, token_worthy, stop_reason, decision,
        updated_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

fn read_investigation_nodes(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationNode>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,node_key,node_type,label,confidence,value_score,status,payload_json,first_seen,last_seen FROM investigation_nodes WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY CASE node_type WHEN 'target' THEN 0 WHEN 'identity' THEN 1 WHEN 'page_state' THEN 2 WHEN 'action' THEN 3 WHEN 'api' THEN 4 WHEN 'parameter' THEN 5 ELSE 6 END,value_score DESC,id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationNode { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,node_key:row.get(3)?,node_type:row.get(4)?,label:row.get(5)?,confidence:row.get(6)?,value_score:row.get(7)?,status:row.get(8)?,payload:investigation_json(row.get(9)?),first_seen:row.get(10)?,last_seen:row.get(11)? })).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
}

fn read_investigation_edges(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationEdge>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,source_key,relation,target_key,confidence,evidence_json,created_at FROM investigation_edges WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationEdge { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,source_key:row.get(3)?,relation:row.get(4)?,target_key:row.get(5)?,confidence:row.get(6)?,evidence:investigation_json(row.get(7)?),created_at:row.get(8)? })).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
}

fn read_investigation_actions(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationAction>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,action_key,state_key,action_type,label,outcome,value_score,protocol_json,created_at,updated_at FROM investigation_actions WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY value_score DESC,id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationAction { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,action_key:row.get(3)?,state_key:row.get(4)?,action_type:row.get(5)?,label:row.get(6)?,outcome:row.get(7)?,value_score:row.get(8)?,protocol:investigation_json(row.get(9)?),created_at:row.get(10)?,updated_at:row.get(11)? })).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
}

fn read_investigation_apis(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationApiModel>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,api_key,method,url,normalized_path,source,confidence,auth_scope,parameters_json,request_schema_json,response_schema_json,state_keys_json,action_keys_json,identity_keys_json,observed_count,baseline_status,payload_json,updated_at FROM investigation_api_models WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY CASE baseline_status WHEN 'new' THEN 0 WHEN 'changed' THEN 1 ELSE 2 END,method,normalized_path").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationApiModel { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,api_key:row.get(3)?,method:row.get(4)?,url:row.get(5)?,normalized_path:row.get(6)?,source:row.get(7)?,confidence:row.get(8)?,auth_scope:row.get(9)?,parameters:investigation_json(row.get(10)?),request_schema:investigation_json(row.get(11)?),response_schema:investigation_json(row.get(12)?),state_keys:investigation_json(row.get(13)?),action_keys:investigation_json(row.get(14)?),identity_keys:investigation_json(row.get(15)?),observed_count:row.get(16)?,baseline_status:row.get(17)?,payload:investigation_json(row.get(18)?),updated_at:row.get(19)? })).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
}

fn read_investigation_hypotheses(connection: &rusqlite::Connection, scan_id: &str, target_url: &str, status: &str) -> Result<Vec<InvestigationHypothesis>, String> {
    let mut statement = connection.prepare("SELECT h.id,h.project_id,h.scan_id,h.target_url,h.hypothesis_key,h.category,h.title,h.status,h.score,h.confidence,h.contract_json,h.evidence_json,h.decision_json,h.source_opportunity_key,h.created_at,h.updated_at,COALESCE(a.approved,0),COALESCE(a.scope_json,'{}'),COALESCE(a.max_attempts,1),COALESCE(a.note,''),COALESCE(a.expires_at,''),COALESCE(a.updated_at,''),CASE WHEN COALESCE(a.approved,0)=1 AND datetime(a.expires_at)>datetime('now','localtime') THEN 1 ELSE 0 END FROM investigation_hypotheses h LEFT JOIN investigation_mutation_approvals a ON a.hypothesis_id=h.id WHERE (?1='' OR h.scan_id=?1) AND (?2='' OR h.target_url=?2) AND (?3='' OR h.status=?3) ORDER BY h.score DESC,h.updated_at DESC").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url,status], |row| {
        let scope = investigation_json(row.get(17)?);
        Ok(InvestigationHypothesis {
            id:row.get(0)?,project_id:row.get(1)?,scan_id:row.get(2)?,target_url:row.get(3)?,hypothesis_key:row.get(4)?,category:row.get(5)?,title:row.get(6)?,status:row.get(7)?,score:row.get(8)?,confidence:row.get(9)?,contract:investigation_json(row.get(10)?),evidence:investigation_json(row.get(11)?),decision:investigation_json(row.get(12)?),source_opportunity_key:row.get(13)?,created_at:row.get(14)?,updated_at:row.get(15)?,
            mutation_approval: serde_json::json!({
                "approved":row.get::<_,i64>(16)?!=0,
                "active":row.get::<_,i64>(22)?!=0,
                "scope":scope,
                "maxAttempts":row.get::<_,i64>(18)?,
                "note":row.get::<_,String>(19)?,
                "expiresAt":row.get::<_,String>(20)?,
                "updatedAt":row.get::<_,String>(21)?,
            }),
        })
    }).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
}

fn read_investigation_identity_diffs(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationIdentityDiff>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type,risk_score,status,matrix_json,created_at FROM investigation_identity_diffs WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY risk_score DESC,id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationIdentityDiff { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,api_key:row.get(3)?,left_identity_key:row.get(4)?,right_identity_key:row.get(5)?,difference_type:row.get(6)?,risk_score:row.get(7)?,status:row.get(8)?,matrix:investigation_json(row.get(9)?),created_at:row.get(10)? })).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
}

fn read_investigation_metrics(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Option<InvestigationMetrics>, String> {
    connection.query_row("SELECT scan_id,target_url,node_count,edge_count,state_count,action_count,api_count,parameter_count,hypothesis_count,added_count,changed_count,removed_count,duplicate_count,information_gain,token_worthy,stop_reason,decision_json,updated_at FROM investigation_metrics WHERE scan_id=?1 AND target_url=?2", params![scan_id,target_url], |row| Ok(InvestigationMetrics { scan_id:row.get(0)?,target_url:row.get(1)?,node_count:row.get(2)?,edge_count:row.get(3)?,state_count:row.get(4)?,action_count:row.get(5)?,api_count:row.get(6)?,parameter_count:row.get(7)?,hypothesis_count:row.get(8)?,added_count:row.get(9)?,changed_count:row.get(10)?,removed_count:row.get(11)?,duplicate_count:row.get(12)?,information_gain:row.get(13)?,token_worthy:row.get::<_,i64>(14)?!=0,stop_reason:row.get(15)?,decision:investigation_json(row.get(16)?),updated_at:row.get(17)? })).optional().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_investigation_graph(state: State<AppState>, scan_id: String, target_url: Option<String>) -> Result<InvestigationGraph, String> {
    let connection = db::open(&state.db_path)?;
    let target_url = target_url.unwrap_or_default();
    let metrics = if target_url.is_empty() { None } else { read_investigation_metrics(&connection, &scan_id, &target_url)? };
    Ok(InvestigationGraph { scan_id:scan_id.clone(),target_url:target_url.clone(),nodes:read_investigation_nodes(&connection,&scan_id,&target_url)?,edges:read_investigation_edges(&connection,&scan_id,&target_url)?,actions:read_investigation_actions(&connection,&scan_id,&target_url)?,apis:read_investigation_apis(&connection,&scan_id,&target_url)?,hypotheses:read_investigation_hypotheses(&connection,&scan_id,&target_url,"")?,identity_diffs:read_investigation_identity_diffs(&connection,&scan_id,&target_url)?,metrics })
}

#[tauri::command]
pub fn list_investigation_hypotheses(state: State<AppState>, scan_id: Option<String>, status: Option<String>) -> Result<Vec<InvestigationHypothesis>, String> {
    let connection = db::open(&state.db_path)?;
    read_investigation_hypotheses(&connection, scan_id.as_deref().unwrap_or(""), "", status.as_deref().unwrap_or(""))
}

#[tauri::command]
pub fn update_investigation_hypothesis(state: State<AppState>, input: InvestigationHypothesisUpdateInput) -> Result<(), String> {
    let allowed = ["candidate","ready","in_progress","validated","rejected","exhausted"];
    if !allowed.contains(&input.status.as_str()) { return Err("不支持的假设状态".into()) }
    let connection = db::open(&state.db_path)?;
    let changed = connection.execute("UPDATE investigation_hypotheses SET status=?1,updated_at=datetime('now','localtime') WHERE id=?2", params![input.status,input.hypothesis_id]).map_err(|error| error.to_string())?;
    if changed == 0 { return Err("调查假设不存在".into()) }
    connection.execute("UPDATE knowledge_outcomes SET outcome=?1 WHERE hypothesis_key=(SELECT hypothesis_key FROM investigation_hypotheses WHERE id=?2) AND scan_id=(SELECT scan_id FROM investigation_hypotheses WHERE id=?2)", params![input.status,input.hypothesis_id]).map_err(|error| error.to_string())?;
    let strategies = {
        let mut statement = connection.prepare("SELECT DISTINCT project_id,strategy_key FROM knowledge_outcomes WHERE hypothesis_key=(SELECT hypothesis_key FROM investigation_hypotheses WHERE id=?1) AND scan_id=(SELECT scan_id FROM investigation_hypotheses WHERE id=?1)").map_err(|error| error.to_string())?;
        let rows = statement.query_map([input.hypothesis_id], |row| Ok((row.get::<_,Option<i64>>(0)?,row.get::<_,String>(1)?))).map_err(|error| error.to_string())?;
        rows.flatten().collect::<Vec<_>>()
    };
    for (project_id, strategy_key) in strategies {
        let Some(project_id) = project_id else { continue };
        connection.execute(
            "UPDATE knowledge_strategies SET support_count=(SELECT COUNT(DISTINCT scan_id||'|'||target_url) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2),success_count=(SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('validated','confirmed')),failure_count=(SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('rejected','failed','exhausted')),promoted=CASE WHEN (SELECT COUNT(DISTINCT scan_id||'|'||target_url) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2)>=2 OR (SELECT COUNT(*) FROM knowledge_outcomes WHERE project_id=?1 AND strategy_key=?2 AND outcome IN ('validated','confirmed'))>0 THEN 1 ELSE 0 END,updated_at=datetime('now','localtime') WHERE project_id=?1 AND strategy_key=?2",
            params![project_id,strategy_key],
        ).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_investigation_mutation_approval(
    state: State<AppState>,
    input: InvestigationMutationApprovalInput,
) -> Result<(), String> {
    if input.hypothesis_id <= 0 {
        return Err("调查假设 ID 无效".into());
    }
    let max_attempts = input.max_attempts.unwrap_or(1).clamp(1, 3);
    let expires_minutes = input.expires_minutes.unwrap_or(30).clamp(5, 240);
    let note = input.note.unwrap_or_default();
    if note.chars().count() > 500 {
        return Err("授权说明不能超过 500 个字符".into());
    }
    let connection = db::open(&state.db_path)?;
    let hypothesis = connection
        .query_row(
            "SELECT target_url,contract_json FROM investigation_hypotheses WHERE id=?1",
            [input.hypothesis_id],
            |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "调查假设不存在".to_string())?;
    let contract = investigation_json(hypothesis.1);
    let scope = serde_json::json!({
        "targetUrl":hypothesis.0,
        "endpoint":value_first(&contract,&["endpoint"]),
        "method":value_first(&contract,&["method"]),
        "contractKind":value_first(&contract,&["kind"]),
        "mutationPolicy":value_first(&contract,&["mutationPolicy"]),
    });
    if input.approved && value_first(&contract, &["endpoint"]).trim().is_empty() {
        return Err("该假设没有具体端点，不能授予状态变更权限".into());
    }
    connection.execute(
        "INSERT INTO investigation_mutation_approvals(hypothesis_id,approved,scope_json,max_attempts,note,expires_at) VALUES(?1,?2,?3,?4,?5,datetime('now','localtime',?6)) ON CONFLICT(hypothesis_id) DO UPDATE SET approved=excluded.approved,scope_json=excluded.scope_json,max_attempts=excluded.max_attempts,note=excluded.note,expires_at=excluded.expires_at,updated_at=datetime('now','localtime')",
        params![input.hypothesis_id,input.approved as i64,scope.to_string(),max_attempts,note,format!("+{expires_minutes} minutes")],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn investigation_overview(state: State<AppState>, project_id: Option<i64>) -> Result<InvestigationOverview, String> {
    let connection = db::open(&state.db_path)?;
    let filter = project_id.map(|value| format!("={value}")).unwrap_or_else(|| "IS NOT NULL".into());
    if project_id.is_some_and(|value| value <= 0) { return Err("工作空间 ID 无效".into()) }
    let metric_sql = format!("SELECT COUNT(*),COALESCE(SUM(node_count),0),COALESCE(SUM(edge_count),0),COALESCE(SUM(api_count),0),COALESCE(SUM(parameter_count),0),COALESCE(SUM(hypothesis_count),0),COALESCE(SUM(token_worthy),0),COALESCE(AVG(information_gain),0) FROM investigation_metrics WHERE project_id {filter}");
    let metrics = connection.query_row(&metric_sql, [], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?,row.get::<_,i64>(4)?,row.get::<_,i64>(5)?,row.get::<_,i64>(6)?,row.get::<_,f64>(7)? as i64))).map_err(|error| error.to_string())?;
    let hypothesis_filter = if let Some(value)=project_id { format!("project_id={value}") } else { "project_id IS NOT NULL".into() };
    let ready = connection.query_row(&format!("SELECT COUNT(*) FROM investigation_hypotheses WHERE {hypothesis_filter} AND status IN ('ready','in_progress')"), [], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let diffs = connection.query_row(&format!("SELECT COUNT(*) FROM investigation_identity_diffs WHERE {hypothesis_filter}"), [], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let facts = connection.query_row(&format!("SELECT COUNT(*) FROM knowledge_facts WHERE {hypothesis_filter}"), [], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    let strategies = connection.query_row(&format!("SELECT COUNT(*) FROM knowledge_strategies WHERE {hypothesis_filter} AND promoted=1"), [], |row| row.get::<_,i64>(0)).map_err(|error| error.to_string())?;
    Ok(InvestigationOverview { target_count:metrics.0,node_count:metrics.1,edge_count:metrics.2,api_count:metrics.3,parameter_count:metrics.4,hypothesis_count:metrics.5,ready_hypothesis_count:ready,identity_diff_count:diffs,token_worthy_count:metrics.6,average_information_gain:metrics.7,fact_count:facts,promoted_strategy_count:strategies })
}

#[cfg(test)]
mod investigation_tests {
    use super::*;

    #[test]
    fn verification_contracts_are_bounded() {
        let contract = verification_contract("idor", &serde_json::json!({"endpoint":"/api/users/1","parameters":["id"]}));
        assert_eq!(contract["maxAttempts"], 3);
        assert_eq!(contract["mutationPolicy"], "read_only_unless_explicitly_approved");
        assert!(contract["stopRules"].as_array().unwrap().iter().any(|value| value == "confirmed_waf_or_challenge"));
    }

    #[test]
    fn paths_and_queries_are_stable() {
        assert_eq!(normalized_investigation_path("https://example.test/api/users/?q=1#x"), "/api/users");
        assert_eq!(query_parameter_names("/api?a=1&b=2&a=3"), vec!["a", "b"]);
    }

    #[test]
    fn graph_persistence_builds_incremental_and_learning_layers() {
        let directory = std::env::temp_dir().join(format!(
            "oviraptor-investigation-test-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let database = db::initialize(&directory).unwrap();
        let connection = db::open(&database).unwrap();
        connection.execute("INSERT INTO projects(name) VALUES('investigation-test')", []).unwrap();
        let project_id = connection.last_insert_rowid();
        let target = serde_json::json!({
            "url":"https://example.test/app",
            "finalUrl":"https://example.test/app",
            "fingerprint":{"backend":{"name":"Django","confidence":"medium"}},
            "apis":[{"method":"GET","url":"https://example.test/api/users/1?id=1","parameters":["id"],"source":"browser-runtime","confidence":"high","stateId":"state-1","actionId":"action-1","statusCode":200,"responseKeys":["id","name"]}],
            "opportunities":[{"opportunityKey":"idor-users","category":"idor","title":"用户对象权限差异","score":85,"confidence":"high","endpoint":"/api/users/1","method":"GET","parameters":["id"],"evidenceRefs":[{"type":"runtime_request"}]}],
            "runtimeExploration":{
                "states":[{"id":"state-1","url":"https://example.test/app","title":"Users","highValueLabels":["用户详情"]}],
                "actions":[{"id":"action-1","stateId":"state-1","label":"用户详情","role":"button","score":60,"outcome":"clicked","stateChanged":true,"requestCount":1,"beforeUrl":"https://example.test/app","afterUrl":"https://example.test/app"}],
                "requests":[{"method":"GET","url":"https://example.test/api/users/1?id=1","resourceType":"Fetch","stateId":"state-1","actionId":"action-1","queryKeys":["id"],"status":200,"responseKeys":["id","name"]}],
                "coverage":{"deduplicatedStateCount":0},"stopReason":"no_more_valuable_states"
            },
            "authSessionValidation":{"wafDetected":false}
        });
        for scan_id in ["scan-one", "scan-two"] {
            connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status) VALUES(?1,?2,'investigation-test','completed')", params![scan_id,project_id]).unwrap();
            connection.execute("INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES(?1,'{}')", [scan_id]).unwrap();
            connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,'investigation-test','https://example.test/app','completed')", params![project_id,scan_id]).unwrap();
            let metrics = persist_investigation_graph(&connection, Some(project_id), scan_id, "https://example.test/app", &target).unwrap();
            if scan_id == "scan-one" {
                assert!(metrics.token_worthy);
                assert!(metrics.node_count >= 6);
            } else {
                assert!(!metrics.token_worthy);
                assert_eq!(metrics.stop_reason, "incremental_no_new_value");
            }
        }
        let fact_count: i64 = connection.query_row("SELECT COUNT(*) FROM knowledge_facts", [], |row| row.get(0)).unwrap();
        let promoted: i64 = connection.query_row("SELECT promoted FROM knowledge_strategies WHERE category='idor'", [], |row| row.get(0)).unwrap();
        assert!(fact_count >= 4);
        assert_eq!(promoted, 1);
        let hypothesis_id: i64 = connection.query_row("SELECT id FROM investigation_hypotheses WHERE scan_id='scan-one' LIMIT 1", [], |row| row.get(0)).unwrap();
        connection.execute("INSERT INTO investigation_mutation_approvals(hypothesis_id,approved,scope_json,max_attempts,expires_at) VALUES(?1,1,'{\"method\":\"GET\",\"endpoint\":\"/api/users/1\"}',1,datetime('now','localtime','+30 minutes'))", [hypothesis_id]).unwrap();
        let approved = read_investigation_hypotheses(&connection, "scan-one", "", "").unwrap();
        assert_eq!(approved[0].contract["method"], "GET");
        assert_eq!(approved[0].mutation_approval["active"], true);
        connection.execute("UPDATE investigation_mutation_approvals SET expires_at=datetime('now','localtime','-1 minute') WHERE hypothesis_id=?1", [hypothesis_id]).unwrap();
        let expired = read_investigation_hypotheses(&connection, "scan-one", "", "").unwrap();
        assert_eq!(expired[0].mutation_approval["active"], false);
        drop(connection);
        fs::remove_dir_all(&directory).unwrap();
    }
}
