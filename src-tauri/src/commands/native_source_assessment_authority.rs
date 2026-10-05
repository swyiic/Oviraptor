fn source_assessment_messages(system: &str, input: &JsonValue) -> Vec<JsonValue> {
    vec![
        json!({"role":"system","content":system}),
        json!({"role":"user","content":crate::agent_runtime::secrets::redact_json(input).to_string()}),
    ]
}

// Conservative byte-based INPUT admission, not provider billing/tokenization.
// Reserve an explicit output limit as well; never send unlimited cloud output
// against a finite child reservation. A provider overrun remains unreconciled.
fn source_assessment_budget(
    messages: &[JsonValue],
    profile: &AgentModelProfile,
) -> Result<(i64, u64), String> {
    let output = profile.max_output_tokens.unwrap_or(2048).min(2048);
    if output == 0 {
        return Err("source_model_output_limit_invalid".into());
    }
    let bytes = serde_json::to_vec(messages)
        .map_err(|_| "source_model_input_invalid")?
        .len();
    let input = i64::try_from(bytes)
        .ok()
        .and_then(|n| n.checked_add(512))
        .ok_or("source_model_input_oversized")?;
    let total = input
        .checked_add(output as i64)
        .ok_or("source_model_input_oversized")?;
    if profile.max_context_tokens > 0 && total as u64 > profile.max_context_tokens {
        return Err("source_model_context_budget_exceeded".into());
    }
    Ok((total, output))
}

fn source_runtime_proxy(runtime: &AgentWebPipelineRuntime) -> Result<Option<&str>, String> {
    if runtime.proxies.is_empty() {
        return Ok(None);
    }
    runtime
        .proxies
        .iter()
        .find(|(tag, _)| tag == "ALL")
        .map(|(_, url)| Some(url.as_str()))
        .ok_or_else(|| "source_model_proxy_route_missing".into())
}

fn authorize_source_specialist(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    context.require_supervision(lease)?;
    // A live current C and worker cannot issue missing original financial
    // authority. This pure read also protects local paid-result authorization.
    let owner = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        connection, &lease.root_run_id,
    )?;
    owner.require_original_coordinator(connection, lease)?;
    let work: String = connection
        .query_row(
            "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
            params![lease.scan_id, lease.attempt_number],
            |r| r.get(0),
        )
        .map_err(|_| "source_dispatch_work_directory_missing")?;
    let directory = WebBindingDirectory::open(Path::new(&work))?;
    let runtime = resolve_agent_web_pipeline_runtime(
        connection,
        &lease.scan_id,
        &context.environment.deployment,
        PathBuf::new(),
    )?;
    let contract = verify_source_runtime_in(
        connection,
        &lease.scan_id,
        lease.attempt_number,
        &directory,
        context.environment,
        &runtime,
    )?;
    if context.proxy != source_runtime_proxy(&runtime)? {
        return Err("source_dispatch_proxy_changed".into());
    }
    source_model_cost_admission(!contract["budget"]["maxBudgetUsd"].is_null())?;
    let frozen: String = connection
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .map_err(|_| "source_dispatch_root_missing")?;
    let frozen: JsonValue =
        serde_json::from_str(&frozen).map_err(|_| "source_dispatch_root_invalid")?;
    if frozen["runtime"] != contract {
        return Err("source_dispatch_runtime_changed".into());
    }
    let root_valid: bool=connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND role='coordinator' AND root_run_id=id AND status IN ('prepared','running') AND cancel_requested_at='' AND hard_token_budget=?2 AND hard_request_budget=?3)",
        params![lease.root_run_id,contract["budget"]["tokenLimit"].as_i64().ok_or("source_dispatch_token_limit_invalid")?,
            crate::agent_runtime::multi_agent::source::model_request_limit(&frozen)?],|r|r.get(0))
        .map_err(|_|"source_dispatch_root_lookup")?;
    if !root_valid {
        return Err("source_dispatch_root_changed".into());
    }
    Ok(())
}

// Renew only a still-live, still-authorized source model call. This is not a
// recovery claim: expired/revoked leases are never resurrected and no new fence,
// capability, budget or model request is created. The caller owns the write tx.
fn heartbeat_source_specialist(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{lease as leases, source};
    if connection.is_autocommit() || !source::uses_source_runtime(connection, lease, child.role)? {
        return Err("source_heartbeat_context_invalid".into());
    }
    let verify = |db: &rusqlite::Connection| {
        if source::is_source_role(child.role) {
            source::verify_assignment(db, lease, child)
        } else {
            crate::agent_runtime::multi_agent::source_review_subject::verify_assignment(
                db, lease, child,
            )
        }
    };
    leases::validate_coordinator_lease(connection, lease)?;
    crate::agent_runtime::multi_agent::attempts::require_live_for_run(connection, &child.run_id)?;
    let due: bool=connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE root_run_id=?1 AND lease_expires_at<=datetime('now','+120 seconds','localtime')) OR EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?2 AND child_run_id=?3 AND revoked_at='' AND lease_expires_at<=datetime('now','+120 seconds','localtime')) OR EXISTS(SELECT 1 FROM agent_assignments WHERE id=?2 AND lease_expires_at<=datetime('now','+120 seconds','localtime')) OR EXISTS(SELECT 1 FROM agent_assignment_attempts WHERE child_run_id=?3 AND expires_at<=datetime('now','+120 seconds','localtime'))",
        params![lease.root_run_id,child.assignment_id,child.run_id],|r|r.get(0)).map_err(|_|"source_heartbeat_lookup")?;
    if !due {
        return Ok(());
    }
    verify(connection)?;
    let write_capability = if source::is_source_role(child.role) {
        "mailbox.write"
    } else {
        "review.write"
    };
    let active = || -> Result<bool, String> {
        connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND a.state='running' AND r.status='running' AND r.cancel_requested_at='' AND a.budget_settled_at='')
         AND (SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 AND revoked_at='' AND lease_expires_at>datetime('now','localtime') AND capability IN ('evidence.read',?6))=2
         AND (SELECT count(*) FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='')=2
         AND EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 AND state='executing')
         AND EXISTS(SELECT 1 FROM agent_lane_leases l JOIN agent_assignments a ON a.id=l.assignment_id WHERE a.id=?1 AND l.lane=a.lane AND l.target_key=a.target_key)",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.lease_epoch,lease.fencing_token,write_capability],|r|r.get(0)).map_err(|_|"source_heartbeat_binding_lookup".into())
    };
    if !active()? {
        return Err("source_heartbeat_not_live".into());
    }
    renew_source_worker_leases(
        connection,
        lease,
        child,
        &["evidence.read".into(), write_capability.into()],
    )?;
    leases::validate_coordinator_lease(connection, lease)?;
    verify(connection)?;
    let renewed: i64=connection.query_row("SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 AND revoked_at='' AND lease_expires_at>datetime('now','localtime') AND capability IN ('evidence.read',?6)",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.lease_epoch,lease.fencing_token,write_capability],|r|r.get(0)).map_err(|_|"source_heartbeat_postcondition")?;
    // Recheck the whole live binding after every write. Counting only the two
    // expected grants misses injected extra permissions or a lost lane/call.
    if renewed != 2 || !active()? {
        return Err("source_heartbeat_postcondition".into());
    }
    Ok(())
}

#[cfg(test)]
fn source_model_remaining_seconds(
    connection: &rusqlite::Connection,
    root_id: &str,
    timeout: u64,
) -> Result<u64, String> {
    let elapsed: i64 = connection
        .query_row(
            "SELECT CASE
        WHEN started_at='' AND status='prepared' THEN 0
        WHEN started_at<>'' AND julianday(started_at)<=julianday('now','localtime')
          THEN CAST((julianday('now','localtime')-julianday(started_at))*86400 AS INTEGER)
        ELSE NULL END FROM agent_runs WHERE id=?1",
            [root_id],
            |r| r.get(0),
        )
        .map_err(|_| "source_model_start_time_invalid")?;
    Ok(timeout.saturating_sub(elapsed.max(0) as u64))
}

/// Own the source-model invocation independently of reusable capability leases.
/// Initial assessments produce mailbox evidence, NOT independent review/CI pass.
fn run_native_source_assessments(
    db_path: &Path,
    scan_id: &str,
    attempt: i64,
    work_dir: &Path,
) -> Result<JsonValue, String> {
    let _owner = claim_native_invocation(db_path, scan_id, attempt, "source-model", "source")?;
    let connection = db::open(db_path)?;
    let (model, runtime, contract) =
        verify_source_runtime_contract(&connection, scan_id, attempt, work_dir)?;
    source_model_cost_admission(!contract["budget"]["maxBudgetUsd"].is_null())?;
    let proxy = source_runtime_proxy(&runtime)?;
    let profile = agent_model_profile(&model, proxy)?;
    let _timeout = contract["budget"]["timeoutSeconds"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or("source_model_timeout_invalid")?;
    let root = prepare_native_source_coordinator(&connection, scan_id, attempt, work_dir)?;
    // Capture financial identity before checking live execution. Expiry can
    // deny new work while the original incurred clock still needs settlement.
    let coordinator = native_source_fresh_finance::original_for_financial_exit(&connection, &root.run_id)?;
    let context = SpecialistTransportContext {
        supervision: None, db_path, scan_id, attempt_number: attempt,
        target_key: &root.target_key, run_id: &root.run_id,
        environment: &model, proxy, usage_dir: work_dir, deadline: None,
    };
    let result = run_native_source_assessments_owned(&connection, &context, &coordinator, &profile);
    if let Err(primary) = result {
        return match crate::agent_runtime::multi_agent::budget::clock::observation::record_original_exit(
            &connection, &coordinator,
        ) {
            Ok(()) => Err(primary),
            Err(financial) => Err(format!("{primary};source_financial_exit:{financial}")),
        };
    }
    result
}
include!("native_source_execution_owned.rs");
