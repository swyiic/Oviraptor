/// Atomic broker decision and durable, non-replayable side reservation. The
/// model cannot invoke this function, and a request is never sent before its
/// exact method, URL, identity, contract, lease, lane and budget are checked.
fn claim_authorization_probe(
    context: &AgentRunContext,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control: &SaveAuthorizationControlInput,
    side: &str,
    identity: &str,
    url: &str,
) -> Result<(), String> {
    let expected = match side {
        "owner" => (&control.owner_identity, &control.owner_object_url),
        "cross" => (&control.tester_identity, &control.owner_object_url),
        "tester" => (&control.tester_identity, &control.tester_control_url),
        _ => return Err("authorization_side_invalid".into()),
    };
    if (identity, url) != (expected.0.as_str(), expected.1.as_str())
        || control.scan_id != context.scan_id || control.target_url != context.target_url
        || control.attempt_number != context.attempt_number
        || child.role != crate::agent_runtime::contract::AgentRole::Authorization
        || context.run.as_ref().map(|run| run.run_id.as_str()) != Some(child.run_id.as_str())
    {
        return Err("authorization_side_binding_invalid".into());
    }
    let mut connection = db::open(&context.db_path)?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "authorization_claim_transaction_failed".to_string())?;
    // A dedicated side probe is still a target-touching Web request. Validate
    // the same authoritative attempt plan as the generic tool broker before
    // reserving a side or charging any of its three request slots.
    agent_require_frozen_web_plan(&tx, context).map_err(str::to_string)?;
    let (_, identities) = crate::auth_session::validated_scan_identities(
        &tx, &context.scan_id, &context.target_url,
    ).map_err(|_| "authorization_identity_binding_denied".to_string())?;
    let mut bound = context.identities.iter().map(|item| item.key.clone()).collect::<Vec<_>>();
    bound.sort();
    let mut identities = identities;
    identities.sort();
    if identities != bound || !identities.contains(&identity.to_string()) {
        return Err("authorization_identity_binding_denied".into());
    }
    let authorized=authorization_probe_authority(&tx,context,child,control,side,identity,url)?;
    let Some(budget) = authorized else { return Err("authorization_capability_or_fencing_denied".into()) };
    let spent: i64 = tx.query_row(
        "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE child_run_id=?1",
        [&child.run_id], |row| row.get(0),
    ).map_err(|_| "authorization_budget_unavailable".to_string())?;
    if spent >= budget || budget <= 0 {
        return Err("authorization_request_budget_exhausted".into());
    }
    let usage = agent_request_accounting(&tx,&context.scan_id,context.attempt_number,&context.target_url)?;
    let ceiling = context.execution_plan.hard_model_requests.max(1).saturating_mul(4).min(400);
    if usage.budget_committed >= ceiling {
        return Err("authorization_request_budget_exhausted".into());
    }
    let changed=tx.execute(
        "INSERT INTO agent_authorization_probe_claims(scan_id,attempt_number,target_url,contract_key,side,child_run_id,assignment_id) \
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![context.scan_id, context.attempt_number, context.target_url, control.contract_key,
            side, child.run_id, child.assignment_id],
    ).map_err(|_| "authorization_side_already_claimed".to_string())?;
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2
        AND target_url=?3 AND contract_key=?4 AND side=?5 AND child_run_id=?6 AND assignment_id=?7 AND artifact_id='')",
        params![context.scan_id,context.attempt_number,context.target_url,control.contract_key,side,child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if changed!=1 || !exact { return Err("authorization_claim_postcondition".into()); }
    let (lease,assignment)=crate::agent_runtime::multi_agent::budget::target::child_owner(&tx,&child.run_id)?
        .ok_or("authorization_budget_owner_missing")?;
    crate::agent_runtime::multi_agent::budget::target::claim(&tx,&lease,&assignment,
        &authorization_budget_source(child,control,side))?;
    agent_require_frozen_web_plan(&tx,context).map_err(str::to_string)?;
    if authorization_probe_authority(&tx,context,child,control,side,identity,url)?!=Some(budget) {
        return Err("authorization_capability_or_fencing_denied".into());
    }
    tx.commit().map_err(|_| "authorization_claim_commit_failed".to_string())
}

fn authorization_probe_authority(connection: &rusqlite::Connection, context: &AgentRunContext,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control: &SaveAuthorizationControlInput, side: &str, identity: &str, url: &str)
    -> Result<Option<i64>,String> {
    crate::agent_runtime::multi_agent::attempts::require_live_for_run(connection,&child.run_id)
        .map_err(|_| "authorization_capability_or_fencing_denied".to_string())?;
    connection.query_row(
        "SELECT (SELECT COUNT(*)*3 FROM agent_authorization_controls scope \
           WHERE scope.scan_id=r.scan_id AND scope.attempt_number=r.attempt_number AND scope.target_url=r.target_url) FROM agent_runs r \
         JOIN sentinel_scans s ON s.id=r.scan_id AND s.attempt_count=r.attempt_number AND s.status='scanning' \
         JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id \
         JOIN agent_coordinator_leases c ON c.root_run_id=a.coordinator_run_id \
           AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url \
         JOIN agent_runs root ON root.id=c.root_run_id AND root.root_run_id=root.id AND root.role='coordinator' \
           AND root.scan_id=c.scan_id AND root.attempt_number=c.attempt_number AND root.target_url=c.target_key \
           AND root.status IN ('prepared','running') AND r.root_run_id=root.id \
         JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id \
           AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane='target_touching' \
         JOIN agent_capability_leases p ON p.assignment_id=a.id AND p.child_run_id=r.id \
           AND p.root_run_id=a.coordinator_run_id AND p.capability='authorization_probe' \
         JOIN agent_authorization_controls k ON k.scan_id=r.scan_id AND k.attempt_number=r.attempt_number \
           AND k.target_url=r.target_url AND k.contract_key=?4 AND k.method='GET' \
           AND k.owner_object_value=?10 AND k.tester_object_value=?11 AND k.response_object_pointer=?12 \
         WHERE r.id=?1 AND r.scan_id=?2 AND r.attempt_number=?3 AND r.target_url=?5 \
           AND r.orchestration_policy='multi' AND r.role='authorization' AND r.lane='target_touching' AND r.status='running' \
           AND a.id=?6 AND a.role='authorization' AND a.lane='target_touching' AND a.state='running' \
           AND a.target_key=r.target_url \
           AND a.lease_epoch=c.lease_epoch AND a.fencing_token=c.fencing_token \
           AND p.lease_epoch=c.lease_epoch AND p.fencing_token=c.fencing_token AND p.revoked_at='' \
           AND a.lease_expires_at>datetime('now','localtime') AND p.lease_expires_at>datetime('now','localtime') \
           AND c.lease_expires_at>datetime('now','localtime') \
           AND ((?7='owner' AND k.owner_identity=?8 AND k.owner_object_url=?9) \
             OR (?7='cross' AND k.tester_identity=?8 AND k.owner_object_url=?9) \
             OR (?7='tester' AND k.tester_identity=?8 AND k.tester_control_url=?9))",
        params![child.run_id, context.scan_id, context.attempt_number, control.contract_key,
            context.target_url, child.assignment_id, side, identity, url,
            control.owner_object_value, control.tester_object_value, control.response_object_pointer],
        |row| row.get(0),
    ).optional().map_err(|_| "authorization_broker_unavailable".to_string())
}

fn authorization_budget_source(child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control: &SaveAuthorizationControlInput, side: &str) -> String {
    format!("authorization:{}:{}:{}",child.assignment_id,
        crate::agent_runtime::store::stable_hash(&control.contract_key),side)
}

/// The Authorization child has no model-controlled URL or HTTP tool. Only this
/// three-side driver can reach the dedicated broker above. It intentionally
/// does not run the generic replay/discovery pipeline (CORS and enumeration
/// probes would violate the exact three-request control group).
fn execute_authorization_side(
    context: &AgentRunContext,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control: &SaveAuthorizationControlInput,
    side: &str,
    identity_key: &str,
    url: &str,
) -> Result<(AuthorizationProbeResponse, String), String> {
    let cancel = agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number);
    if cancel.is_cancelled() { return Err("authorization_attempt_cancelled".into()); }
    let identity = agent_identity_of(context, identity_key)
        .ok_or_else(|| "authorization_identity_unbound".to_string())?;
    let (normalized_url, _) = agent_scope_check(context, url, "GET", ScopeSource::IdentityComparison)
        .map_err(|_| "authorization_frozen_scope_denied".to_string())?;
    if normalized_url != url { return Err("authorization_url_not_canonical".into()); }
    let headers = agent_identity_headers(context, &identity)
        .map_err(|_| "authorization_identity_unavailable".to_string())?;
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20));
    if let Some(proxy) = &context.proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)
            .map_err(|_| "authorization_proxy_invalid".to_string())?);
    }
    let client = builder.build().map_err(|_| "authorization_client_unavailable".to_string())?;
    let mut request = client.get(url)
        .header(reqwest::header::USER_AGENT, "oviraptor-authorization-control/1.0")
        .header(reqwest::header::CACHE_CONTROL, "no-store");
    for (name, value) in &headers {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "authorization_session_header_invalid".to_string())?;
        request = request.header(name, value);
    }
    if cancel.is_cancelled() { return Err("authorization_attempt_cancelled".into()); }
    claim_authorization_probe(context, child, control, side, identity_key, url)?;
    let connection = db::open(&context.db_path)?;
    agent_require_frozen_web_plan(&connection, context).map_err(str::to_string)?;
    if cancel.is_cancelled() { return Err("authorization_attempt_cancelled".into()); }
    if authorization_probe_authority(&connection,context,child,control,side,identity_key,url)?.is_none() {
        return Err("authorization_capability_or_fencing_denied".into());
    }
    request=request.timeout(crate::agent_runtime::multi_agent::budget::target::transport_timeout(
        &connection,&child.run_id,Duration::from_secs(20))?);
    let mut response = request.send().map_err(|_| "authorization_request_failed".to_string())?;
    let status = response.status().as_u16();
    let content_type = response.headers().get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok()).unwrap_or_default().to_string();
    let response_headers = response.headers().iter().filter_map(|(key, value)| {
        Some((key.as_str().to_string(), value.to_str().ok()?.to_string()))
    }).collect::<Vec<_>>();
    let cache_state = agent_cache_state(&response_headers).to_string();
    let redirect_code = if (300..400).contains(&status) { "redirect" } else { "" }.to_string();
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut response).take(AGENT_MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes).map_err(|_| "authorization_response_read_failed".to_string())?;
    let truncated = bytes.len() > AGENT_MAX_RESPONSE_BYTES;
    bytes.truncate(AGENT_MAX_RESPONSE_BYTES);
    let body = String::from_utf8(bytes).map_err(|_| "authorization_response_not_utf8".to_string())?;
    // A scan can be paused while the blocking GET is in flight. A claimed
    // request is still charged, but its response must not become proof after
    // the operator has stopped this attempt.
    if cancel.is_cancelled() { return Err("authorization_attempt_cancelled".into()); }
    let record = AuthorizationProbeResponse {
        attempt_number: context.attempt_number,
        contract_key: control.contract_key.clone(),
        identity: identity_key.to_string(), url: url.to_string(), status,
        content_type: content_type.clone(), redirect_code: redirect_code.clone(),
        cache_state: cache_state.clone(), truncated, body,
    };
    let artifact_id = agent_write_http_record(
        context, 1,
        &serde_json::json!({"identity": identity_key,"method":"GET","url":url,
            "headers":headers.iter().map(|(name,value)| serde_json::json!({"name":name,"value":value})).collect::<Vec<_>>(),
            "contractKey":control.contract_key}),
        &serde_json::json!({"identity":identity_key,"status":status,"contentType":content_type,
            "redirectCode":redirect_code,"cacheState":cache_state,"truncated":truncated,
            "authorizationSide":side,"rawBodyPrivate":true}),
        record.body.as_bytes(),
    ).map_err(|_| "authorization_evidence_write_failed".to_string())?;
    let connection=db::open(&context.db_path)?;
    let tx=connection.unchecked_transaction().map_err(|e|e.to_string())?;
    let updated = tx.execute(
        "UPDATE agent_authorization_probe_claims SET artifact_id=?1 WHERE scan_id=?2 AND attempt_number=?3 \
         AND target_url=?4 AND contract_key=?5 AND side=?6 AND child_run_id=?7 AND artifact_id=''",
        params![artifact_id, context.scan_id, context.attempt_number, context.target_url,
            control.contract_key, side, child.run_id],
    ).map_err(|_| "authorization_claim_artifact_failed".to_string())?;
    if updated != 1 { return Err("authorization_claim_artifact_fenced".into()); }
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2
        AND target_url=?3 AND contract_key=?4 AND side=?5 AND child_run_id=?6 AND assignment_id=?7 AND artifact_id=?8)",
        params![context.scan_id,context.attempt_number,context.target_url,control.contract_key,side,child.run_id,child.assignment_id,artifact_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact { return Err("authorization_claim_artifact_fenced".into()); }
    let (lease,assignment)=crate::agent_runtime::multi_agent::budget::target::child_owner(&tx,&child.run_id)?
        .ok_or("authorization_budget_owner_missing")?;
    crate::agent_runtime::multi_agent::budget::target::receive(&tx,&lease,&assignment,
        &authorization_budget_source(child,control,side))?;
    tx.commit().map_err(|_|"authorization_claim_artifact_commit_failed")?;
    Ok((record, artifact_id))
}
