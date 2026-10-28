enum NativeModelRoundFailure {
    Model(AgentModelError),
    Outcome(Box<AgentTargetOutcome>),
}

impl NativeModelRoundFailure {
    fn outcome(value: AgentTargetOutcome) -> Self {
        Self::Outcome(Box::new(value))
    }
}

fn native_model_transport(
    context: &AgentRunContext,
    client: &AgentModelClient,
    messages: Vec<JsonValue>,
    specs: &[AgentToolSpec],
    round: i64,
) -> Result<crate::agent_runtime::model::gateway::ModelResponse, NativeModelRoundFailure> {
    let model_stop = |reason: String| {
        let message = if reason == "scan_model_profile_changed" {
            "当前模型配置已与这次执行开始时不一致，停止发送请求，没有沿用旧预检".to_string()
        } else if reason.starts_with("scan_") {
            format!("当前预检未通过（{reason}），停止发送请求，没有沿用旧预检")
        } else {
            "当前模型配置未能重新核对，停止发送请求，没有沿用旧预检".to_string()
        };
        NativeModelRoundFailure::outcome(AgentTargetOutcome::resume_incompatible(message))
    };
    if let Err(reason) = refuse_stale_capability_snapshot(&context.db_path, &context.scan_id, context.attempt_number, context.proxy.as_deref()) {
        return Err(model_stop(reason));
    }
    if let Err(reason) = refuse_stale_continuation_model(&context.db_path, &context.scan_id, context.attempt_number, context.proxy.as_deref(), context.continuation_holder.as_deref(), client) {
        return Err(model_stop(reason));
    }
    let mut request = agent_model_request(messages, specs);
    let unbounded = native_model_unbounded_window(context)
        .map_err(|e|NativeModelRoundFailure::outcome(persistence_stop(context,e)))?;
    // Only newly declared Native5 unbounded windows gain this per-call bound.
    let bounded_client;
    let client = if unbounded {
        let mut profile = client.profile().clone();
        let output = profile.max_output_tokens.unwrap_or(8192);
        if output == 0 { return Err(NativeModelRoundFailure::outcome(persistence_stop(context,"budget_unbounded_model_estimate_invalid".into()))); }
        profile.max_output_tokens=Some(output);
        bounded_client=AgentModelClient::new(profile,specs);
        &bounded_client
    } else {client};
    let unbounded_estimate=if unbounded {
        let bytes=serde_json::to_vec(&json!({"messages":request.messages,"tools":request.tools.iter().map(|tool|tool.as_function_spec()).collect::<Vec<_>>()}))
            .map_err(|e|NativeModelRoundFailure::outcome(persistence_stop(context,e.to_string())))?.len();
        Some(i64::try_from(bytes).ok().and_then(|n|i64::try_from(client.profile().max_output_tokens.unwrap()).ok().and_then(|o|n.checked_add(o)))
            .filter(|n|*n>0).ok_or_else(||NativeModelRoundFailure::outcome(persistence_stop(context,"budget_unbounded_model_estimate_invalid".into())))?)
    } else {None};
    let request_hash=crate::agent_runtime::store::stable_hash(&serde_json::json!({"messages":request.messages,
        "tools":request.tools.iter().map(|tool|tool.as_function_spec()).collect::<Vec<_>>(),
        "temperature":request.temperature,"model":client.profile().model,"endpoint":client.profile().endpoint,
        "maxOutputTokens":client.profile().max_output_tokens,"providerOptions":client.profile().bounded_wire_options(),"schemaHash":client.schema_hash()}).to_string());
    match native_root_model_transport(context, client, &request, round, &request_hash) {
        Ok(Some(result)) => return result,
        Ok(None) => {}
        Err(error) => {
            return Err(NativeModelRoundFailure::outcome(persistence_stop(
                context, error,
            )))
        }
    }
    let admitted = (|| {
        native_model_budget_preflight(context)?;
        native_coordinator_budget_consumption_before_action(context)?;
        native_coordinator_http_budget_consumption_before_action(context)?;
        if unbounded {
            native_model_budget_admission_with_estimate(context, round, request_hash, unbounded_estimate)
        } else {
            native_model_budget_admission(context, round, request_hash)
        }
    })();
    let admission = match admitted {
        Ok(admission) => admission,
        Err(reason) if reason == "budget_model_requests_exhausted" => {
            return Err(NativeModelRoundFailure::outcome(
                AgentTargetOutcome::Limited(AgentStop::new(
                    AGENT_STOP_HARD_REQUESTS,
                    "原子账本中的本执行分支模型请求预留已用尽，停止新请求",
                )),
            ))
        }
        Err(reason) => {
            let mapped = agent_http_claim_error(reason.clone());
            return Err(NativeModelRoundFailure::outcome(
                agent_tool_terminal_outcome(&mapped)
                    .unwrap_or_else(|| persistence_stop(context, reason)),
            ));
        }
    };
    let mut cancel =
        agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number);
    if let Some(admission) = &admission {
        request.request_timeout = Some(admission.remaining);
        let deadline = Instant::now() + admission.remaining;
        let scoped_cancel = cancel;
        let owned = context.clone();
        let capability = admission.capability.clone();
        cancel = CancelToken::from_checker(move || {
            scoped_cancel.is_cancelled()
                || Instant::now() >= deadline
                || agent_authorize_tool(&owned, &capability).is_err()
        });
    }
    let result = if let Some(admission) = &admission {
        match client.complete_once_observed(&request, &cancel) {
            Ok(turn) => {
                let unresolved =
                    native_model_terminal_receipt(context, admission, "received", Some(&turn), "")
                        .map_err(|e| {
                            NativeModelRoundFailure::outcome(persistence_stop(context, e))
                        })?;
                if !turn.usage_reported || unresolved {
                    return Err(NativeModelRoundFailure::outcome(AgentTargetOutcome::Incomplete(AgentStop::new(
                        terminal_code::REQUEST_RECONCILIATION_REQUIRED,"模型计费用量缺失或超过原预留，费用回执已保留、原估算保持未决；停止后续工具和模型派发，须先核对费用"))));
                }
                // A known bill is a durable fact, never permission to consume
                // model output after the original worker loses its grant.
                if agent_authorize_tool(context, &admission.capability).is_err() {
                    return Err(NativeModelRoundFailure::outcome(
                        AgentTargetOutcome::resume_incompatible(
                            "原模型账单已保存；此执行分支的权限已失效，停止使用该响应及派发后续动作",
                        ),
                    ));
                }
                Ok(turn)
            }
            Err(crate::agent_runtime::model::OneShotFailure::BeforeTransport(error)) => {
                native_model_terminal_receipt(context, admission, "unsent", None, error.code())
                    .map_err(|e| NativeModelRoundFailure::outcome(persistence_stop(context, e)))?;
                Err(error)
            }
            Err(crate::agent_runtime::model::OneShotFailure::TransportOutcomeUnknown(error)) => {
                note_transport_cancel(context, client);
                if let Err(persist) = native_model_terminal_receipt(
                    context,
                    admission,
                    "uncertain",
                    None,
                    error.code(),
                ) {
                    return Err(NativeModelRoundFailure::outcome(persistence_stop(
                        context,
                        format!(
                            "web_model_unknown_cost_record_failed:{persist}; cause={}",
                            error.code()
                        ),
                    )));
                }
                return Err(NativeModelRoundFailure::outcome(AgentTargetOutcome::Incomplete(AgentStop::new(terminal_code::REQUEST_RECONCILIATION_REQUIRED,
                        format!("模型调用结果未知，已保留原请求成本并停止此执行分支；不自动重试、退款或继续派发，请先核对。原始失败：{}（{}）",
                            crate::agent_runtime::secrets::redact_text_with(&error.detail(),None),error.code())))));
            }
        }
    } else {
        client.complete(&request, &cancel)
    };
    result.map_err(NativeModelRoundFailure::Model)
}

/// A saved preflight names the model host and port. A later call must not keep
/// going after that host or port changes, even when no continuation holder exists.
pub(crate) fn refuse_stale_capability_snapshot(db_path: &Path, scan_id: &str, attempt: i64, proxy: Option<&str>) -> Result<(), String> {
    let connection = match db::open(db_path) { Ok(connection) => connection, Err(_) => return Ok(()) };
    let raw: String = match connection.query_row(
        "SELECT capability_snapshot_json FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt], |row| row.get(0),
    ) { Ok(raw) => raw, Err(_) => return Ok(()) };
    if raw.trim().is_empty() { return Ok(()); }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else { return Ok(()); };
    if let Some((host, port)) = saved_model_endpoint(&value) {
        if let Ok(settings) = agent_policy_settings_for_attempt(&connection, scan_id, attempt) {
            if let Ok(environment) = model_runtime_env(&settings) {
                if let Ok(fresh) = agent_model_profile(&environment, proxy) {
                    let (live_host, live_port) = agent_channel::model_endpoint_port(&fresh.endpoint)?;
                    if live_host != host || live_port != port { return Err("scan_model_profile_changed".into()); }
                }
            }
        }
    }
    let project: i64 = connection.query_row("SELECT project_id FROM sentinel_scans WHERE id=?1", [scan_id], |row| row.get(0)).map_err(|_| "scan_task_unavailable".to_string())?;
    if let Some(reason) = agent_channel::fresh_execution_preflight_block(&connection, project, scan_id)? {
        return Err(reason);
    }
    Ok(())
}

fn saved_model_endpoint(value: &serde_json::Value) -> Option<(String, u16)> {
    let version = value.get("configVersion").and_then(serde_json::Value::as_str).filter(|item| !item.is_empty())
        .or_else(|| value.get("readiness").and_then(serde_json::Value::as_array).and_then(|items| {
            items.iter().find_map(|item| item.get("configVersion").and_then(serde_json::Value::as_str).filter(|text| !text.is_empty()))
        }))?;
    let (left, port) = version.rsplit_once(':')?;
    let host = left.split_once('@').map(|(_, host)| host).unwrap_or(left);
    let port = port.parse::<u16>().ok()?;
    if host.is_empty() { return None; }
    Some((host.to_string(), port))
}

/// A continuation may sit after an earlier preflight. Re-read the saved model
/// profile before any provider call. A mismatch or an unreadable profile stops
/// the call; a fresh start without a continuation holder keeps its client.
pub(crate) fn refuse_stale_continuation_model(
    db_path: &Path,
    scan_id: &str,
    attempt: i64,
    proxy: Option<&str>,
    holder: Option<&str>,
    client: &AgentModelClient,
) -> Result<(), String> {
    if holder.is_none() { return Ok(()); }
    let connection = db::open(db_path).map_err(|_| "scan_model_profile_unchecked".to_string())?;
    let settings = agent_policy_settings_for_attempt(&connection, scan_id, attempt).map_err(|_| "scan_model_profile_unchecked".to_string())?;
    let environment = model_runtime_env(&settings).map_err(|_| "scan_model_profile_changed".to_string())?;
    let fresh = agent_model_profile(&environment, proxy).map_err(|_| "scan_model_profile_changed".to_string())?;
    let current = client.profile();
    if fresh.endpoint != current.endpoint || fresh.proxy != current.proxy || fresh.model != current.model || fresh.local != current.local {
        return Err("scan_model_profile_changed".into());
    }
    Ok(())
}
