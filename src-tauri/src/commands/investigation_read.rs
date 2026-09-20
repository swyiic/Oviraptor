// Read side of the investigation desk: graph nodes, edges, actions, APIs, hypotheses,
// identity diffs, metrics and the status mutations the UI issues. Included from
// investigation.rs.

fn read_investigation_nodes(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationNode>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,node_key,node_type,label,confidence,value_score,status,payload_json,first_seen,last_seen FROM investigation_nodes WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY CASE node_type WHEN 'target' THEN 0 WHEN 'identity' THEN 1 WHEN 'page_state' THEN 2 WHEN 'action' THEN 3 WHEN 'api' THEN 4 WHEN 'parameter' THEN 5 ELSE 6 END,value_score DESC,id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationNode { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,node_key:row.get(3)?,node_type:row.get(4)?,label:row.get(5)?,confidence:row.get(6)?,value_score:row.get(7)?,status:row.get(8)?,payload:investigation_json(row.get(9)?),first_seen:row.get(10)?,last_seen:row.get(11)? })).map_err(|error| error.to_string())?;
    let mut nodes = rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    drop(statement);
    // `anonymous` is an API observation scope, not a browser login identity.
    // Older graphs persisted it as an identity node and the UI consequently
    // rendered a fictitious "账号 A / 会话已失效" on public scans.
    nodes.retain(|node| {
        node.node_type != "identity"
            || !anonymous_identity(&value_first(&node.payload, &["identityKey"]))
    });
    // Repair pre-1.1.21 graphs at read time. Their checkpoint already contains
    // the complete identityRuns data, but the old graph node persisted only an
    // identityKey and therefore rendered every account as unknown.
    let checkpoint = connection.query_row(
        "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage='frontend_recon'",
        params![scan_id, target_url],
        |row| row.get::<_, String>(0),
    ).optional().map_err(|error| error.to_string())?
        .and_then(|raw| serde_json::from_str::<JsonValue>(&raw).ok());
    if let Some(target) = checkpoint {
        let mut identity_index = 0usize;
        for node in nodes.iter_mut().filter(|node| node.node_type == "identity") {
            let identity_key = value_first(&node.payload, &["identityKey"]);
            if identity_key.is_empty() || identity_run_summary(&target, &identity_key).is_none() {
                identity_index += 1;
                continue;
            }
            let payload = identity_node_payload(&target, &identity_key, identity_index);
            node.label = value_first(&payload, &["identityLabel"]);
            node.status = match payload.get("sessionValid").and_then(JsonValue::as_bool) {
                Some(true) => "active".into(),
                Some(false) => "invalid".into(),
                None => "unknown".into(),
            };
            node.payload = payload;
            identity_index += 1;
        }
    }
    Ok(nodes)
}

fn read_investigation_edges(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationEdge>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,source_key,relation,target_key,confidence,evidence_json,created_at FROM investigation_edges WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationEdge { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,source_key:row.get(3)?,relation:row.get(4)?,target_key:row.get(5)?,confidence:row.get(6)?,evidence:investigation_json(row.get(7)?),created_at:row.get(8)? })).map_err(|error| error.to_string())?;
    let anonymous_key = format!("identity:{}", investigation_hash("anonymous"));
    let mut values = rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    values.retain(|edge| edge.source_key != anonymous_key && edge.target_key != anonymous_key);
    Ok(values)
}

fn read_investigation_actions(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationAction>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,action_key,state_key,action_type,label,outcome,value_score,protocol_json,created_at,updated_at FROM investigation_actions WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY value_score DESC,id").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationAction { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,action_key:row.get(3)?,state_key:row.get(4)?,action_type:row.get(5)?,label:row.get(6)?,outcome:row.get(7)?,value_score:row.get(8)?,protocol:investigation_json(row.get(9)?),created_at:row.get(10)?,updated_at:row.get(11)? })).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())
}

fn read_investigation_apis(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Vec<InvestigationApiModel>, String> {
    let mut statement = connection.prepare("SELECT id,scan_id,target_url,api_key,method,url,normalized_path,source,confidence,auth_scope,parameters_json,request_schema_json,response_schema_json,state_keys_json,action_keys_json,identity_keys_json,observed_count,baseline_status,payload_json,updated_at FROM investigation_api_models WHERE scan_id=?1 AND (?2='' OR target_url=?2) ORDER BY CASE baseline_status WHEN 'new' THEN 0 WHEN 'changed' THEN 1 ELSE 2 END,method,normalized_path").map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![scan_id,target_url], |row| Ok(InvestigationApiModel { id:row.get(0)?,scan_id:row.get(1)?,target_url:row.get(2)?,api_key:row.get(3)?,method:row.get(4)?,url:row.get(5)?,normalized_path:row.get(6)?,source:row.get(7)?,confidence:row.get(8)?,auth_scope:row.get(9)?,parameters:investigation_json(row.get(10)?),request_schema:investigation_json(row.get(11)?),response_schema:investigation_json(row.get(12)?),state_keys:investigation_json(row.get(13)?),action_keys:investigation_json(row.get(14)?),identity_keys:investigation_json(row.get(15)?),observed_count:row.get(16)?,baseline_status:row.get(17)?,payload:investigation_json(row.get(18)?),updated_at:row.get(19)? })).map_err(|error| error.to_string())?;
    let mut values = rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    values.retain(|api| !investigation_background_noise(&serde_json::json!({
        "url":api.url,"method":api.method,"source":api.source,"contentType":api.payload.get("contentType"),"resourceType":api.payload.get("resourceType")
    })));
    for api in &mut values {
        let keys = api.response_schema.get("keys").cloned().unwrap_or_else(|| serde_json::json!([]));
        if let Some(schema) = api.response_schema.as_object_mut() {
            schema.insert("keys".into(), serde_json::json!(sanitized_investigation_response_keys(Some(&keys))));
        }
        if let Some(payload) = api.payload.as_object_mut() {
            let keys = payload.get("responseKeys").cloned().unwrap_or_else(|| serde_json::json!([]));
            payload.insert("responseKeys".into(), serde_json::json!(sanitized_investigation_response_keys(Some(&keys))));
            if let Some(observations) = payload.get_mut("identityObservations").and_then(JsonValue::as_array_mut) {
                for observation in observations {
                    if let Some(object) = observation.as_object_mut() {
                        let keys = object.get("responseKeys").cloned().unwrap_or_else(|| serde_json::json!([]));
                        object.insert("responseKeys".into(), serde_json::json!(sanitized_investigation_response_keys(Some(&keys))));
                    }
                }
            }
        }
    }
    Ok(values)
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
    let mut values = rows.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
    values.retain(|diff| {
        if diff.difference_type == "feature_surface" || diff.api_key.to_ascii_lowercase().starts_with("feature:") {
            return false;
        }
        let endpoint = diff.api_key.split('|').find(|part| part.starts_with('/')).unwrap_or(&diff.api_key);
        !investigation_background_noise(&serde_json::json!({"url":endpoint,"method":diff.api_key.split('|').next().unwrap_or("GET")}))
    });
    for diff in &mut values {
        sanitize_identity_matrix(&mut diff.matrix);
    }
    Ok(values)
}

fn read_investigation_metrics(connection: &rusqlite::Connection, scan_id: &str, target_url: &str) -> Result<Option<InvestigationMetrics>, String> {
    connection.query_row("SELECT scan_id,target_url,node_count,edge_count,state_count,action_count,api_count,parameter_count,hypothesis_count,added_count,changed_count,removed_count,duplicate_count,information_gain,token_worthy,stop_reason,decision_json,updated_at FROM investigation_metrics WHERE scan_id=?1 AND target_url=?2", params![scan_id,target_url], |row| Ok(InvestigationMetrics { scan_id:row.get(0)?,target_url:row.get(1)?,node_count:row.get(2)?,edge_count:row.get(3)?,state_count:row.get(4)?,action_count:row.get(5)?,api_count:row.get(6)?,parameter_count:row.get(7)?,hypothesis_count:row.get(8)?,added_count:row.get(9)?,changed_count:row.get(10)?,removed_count:row.get(11)?,duplicate_count:row.get(12)?,information_gain:row.get(13)?,token_worthy:row.get::<_,i64>(14)?!=0,stop_reason:row.get(15)?,decision:investigation_json(row.get(16)?),updated_at:row.get(17)? })).optional().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_investigation_graph(state: State<AppState>, scan_id: String, target_url: Option<String>) -> Result<InvestigationGraph, String> {
    let connection = db::open(&state.db_path)?;
    let target_url = target_url.unwrap_or_default();
    let apis = read_investigation_apis(&connection,&scan_id,&target_url)?;
    let actions = read_investigation_actions(&connection,&scan_id,&target_url)?;
    let nodes = read_investigation_nodes(&connection,&scan_id,&target_url)?;
    let identity_diffs = read_investigation_identity_diffs(&connection,&scan_id,&target_url)?;
    let mut metrics = if target_url.is_empty() { None } else { read_investigation_metrics(&connection, &scan_id, &target_url)? };
    if let Some(value) = &mut metrics {
        // Historical rows retain their raw audit records. Keep the visible KPI
        // aligned with the sanitized formal API list returned by this read.
        value.api_count = apis.len() as i64;
        if value.decision.get("manualDeepDive").and_then(JsonValue::as_array).is_none() {
            let persisted_apis = apis.iter().map(|api| (
                api.api_key.clone(),
                serde_json::json!({
                    "method":api.method,"url":api.url,"path":api.normalized_path,
                    "parameters":api.parameters,
                    "responseKeys":api.response_schema.get("keys").cloned().unwrap_or_default()
                }),
            )).collect::<Vec<_>>();
            let raw_actions = actions.iter().map(|action| serde_json::json!({
                "label":action.label,"type":action.action_type,"outcome":action.outcome
            })).collect::<Vec<_>>();
            let mut identity_keys = nodes.iter()
                .filter(|node| node.node_type == "identity")
                .filter_map(|node| node.payload.get("identityKey").and_then(JsonValue::as_str))
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            if identity_keys.is_empty() { identity_keys.push("anonymous".into()); }
            let mode = requested_web_mode_ceiling(&connection, &scan_id);
            let manual = manual_deep_dive_plan(
                &JsonValue::Null,
                &persisted_apis,
                &raw_actions,
                &identity_keys,
                &mode,
            );
            if let Some(decision) = value.decision.as_object_mut() {
                decision.insert("manualDeepDive".into(), manual);
                decision.insert("coverageSemantics".into(), serde_json::json!({
                    "completed":"listed contract executed with usable evidence",
                    "notFound":"executed without security impact",
                    "notTested":"missing identity, state, data, protocol or environment",
                    "neverAssumeSafe":true
                }));
            }
        }
    }
    let related_services = read_investigation_related_services(&connection, &scan_id, &target_url)?;
    Ok(InvestigationGraph { scan_id:scan_id.clone(),target_url:target_url.clone(),nodes,edges:read_investigation_edges(&connection,&scan_id,&target_url)?,actions,apis,related_services,hypotheses:read_investigation_hypotheses(&connection,&scan_id,&target_url,"")?,identity_diffs,metrics })
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
