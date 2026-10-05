include!("native_source_determined_reducer.rs");

// Source closure is a proof over four distinct phase/role assignments, not a
// count of rows labelled completed. No receipt audit here grants execution.
#[derive(Debug, PartialEq)]
struct SourceCompletionProof {
    completion: AgentCompletion,
    cached_tokens: i64,
    manifest: JsonValue,
    review_material: JsonValue,
    coverage_preparation: crate::agent_runtime::multi_agent::source_coverage::CoveragePreparation,
    candidate_review: Option<JsonValue>,
    decision_projection: Option<crate::agent_runtime::multi_agent::source_decisions::SourceDecisionSet>,
    coverage_decision: Option<crate::agent_runtime::multi_agent::source_coverage_decisions::CoverageDecision>,
}

fn source_assessment_completion(
    connection:&rusqlite::Connection,
    lease:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    bases:&[(crate::agent_runtime::contract::AgentRole,JsonValue)],
)->Result<SourceCompletionProof,String> {
    use crate::agent_runtime::{multi_agent::source,store::stable_hash};
    if connection.is_autocommit() || bases.len()!=2 {return Err("source_completion_transaction_required".into());}
    let phases=crate::agent_runtime::multi_agent::source_phases::audit(connection,lease)?;
    if phases.bases!=bases {return Err("source_completion_base_contract_changed".into());}
    let (mut tokens,mut cached,mut requests,verified)=(phases.tokens,phases.cached,phases.requests,phases.verified);
    let mut manifest=phases.manifest;
    let review_material=phases.review_material;
    crate::agent_runtime::multi_agent::source_review_subject::inventory(connection,lease)?;
    let (candidate_count,coverage_count):(i64,i64)=connection.query_row("SELECT
        (SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='evidence_reviewer' AND trigger_code='source_candidates_ready'),
        (SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='evidence_reviewer' AND trigger_code='source_coverage_ready')",
        [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let review_count=candidate_count+coverage_count;
    let candidate_review=if candidate_count==0 {None} else {
        let review=crate::agent_runtime::multi_agent::source_reviewer::audit_delivery(connection,lease)?;
        if review.payload["sourceTask"]["reviewMaterial"]!=review_material {return Err("source_completion_review_material_changed".into());}
        tokens=tokens.checked_add(review.usage.total_tokens).ok_or("source_completion_usage_overflow")?;
        cached=cached.checked_add(review.usage.cached_input_tokens).ok_or("source_completion_usage_overflow")?;
        requests=requests.checked_add(review.usage.model_requests).ok_or("source_completion_usage_overflow")?;
        manifest.push(json!({"assignmentId":review.child.assignment_id,"runId":review.child.run_id,"role":"evidence_reviewer",
            "phase":"source_review","evidenceRevision":1,"messageId":review.message_id,"payloadHash":stable_hash(&review.payload.to_string()),
            "usage":review.usage.as_json(),"verifiedToolResults":0,"sourceDecisionIds":review.decision_ids}));
        Some(review.payload)
    };
    let coverage_decision=if coverage_count==0 {None} else {
        use crate::agent_runtime::multi_agent::{source_coverage_reviewer,source_coverage_decisions};
        let review=source_coverage_reviewer::audit_delivery(connection,lease)?;
        tokens=tokens.checked_add(review.usage.total_tokens).ok_or("source_completion_usage_overflow")?;
        cached=cached.checked_add(review.usage.cached_input_tokens).ok_or("source_completion_usage_overflow")?;
        requests=requests.checked_add(review.usage.model_requests).ok_or("source_completion_usage_overflow")?;
        manifest.push(json!({"assignmentId":review.child.assignment_id,"runId":review.child.run_id,"role":"evidence_reviewer",
            "phase":"source_coverage_review","evidenceRevision":1,"messageId":review.message_id,"payloadHash":stable_hash(&review.payload.to_string()),
            "usage":review.usage.as_json(),"verifiedToolResults":0,"sourceCoverageDecisionIds":review.decision_ids}));
        Some(source_coverage_decisions::read_audited(connection,lease)?)
    };
    let root_plan:String=connection.query_row("SELECT plan_json FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let parsed:JsonValue=serde_json::from_str(&root_plan).map_err(|_|"source_completion_plan_invalid")?;
    let phase_contract=source::SourcePhaseContract::from_plan(&parsed)?;
    let request_limit=phase_contract.model_request_limit;
    let decision_projection=if candidate_review.is_some() && phase_contract.candidate_decisions {
        Some(crate::agent_runtime::multi_agent::source_decisions::read_audited(connection,lease)?)
    } else {None};
    let confirmed_findings=i64::try_from(decision_projection.as_ref().map_or(0,|set|set.confirmed_count()))
        .map_err(|_|"source_completion_count_overflow")?;
    let settled:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_ledger b JOIN agent_runs r ON r.id=b.root_run_id
        WHERE b.root_run_id=?1 AND b.lease_epoch=?2 AND b.fencing_token=?3 AND b.reserved_tokens=0 AND b.reserved_requests=0
        AND b.spent_tokens=?4 AND b.spent_requests=?5 AND b.total_tokens=r.hard_token_budget AND b.total_requests=r.hard_request_budget
        AND r.hard_request_budget=?6 AND r.reserved_tokens=0 AND r.reserved_requests=0)
        AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE root_run_id=?1 AND revoked_at='')
        AND (SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND id<>?1)=4+?7
        AND (SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind IN ('source_tool_result','evidence_summary'))=4
        AND (SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1)=2+?7
        AND (SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1)=?5-2-?7",
        params![lease.root_run_id,lease.lease_epoch,lease.fencing_token,tokens,requests,request_limit,review_count],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !settled {return Err("source_completion_ledger_or_extra_work_mismatch".into());}
    let coverage_preparation=crate::agent_runtime::multi_agent::source_coverage::audit(connection,lease)?;
    let uncovered=match &coverage_decision {
        Some(decision)=>serde_json::from_value::<Vec<String>>(decision.as_json()["decision"]["outstandingGaps"].clone())
            .map_err(|_|"source_completion_coverage_gaps_invalid")?,
        None=>coverage_preparation.gaps.clone(),
    };
    Ok(SourceCompletionProof {completion:AgentCompletion {
        summary:if coverage_decision.is_some() {"源码总体覆盖独立审查与邮箱交付完成；覆盖充分性按裁决和保留缺口判定"} else if candidate_review.is_some() {"源码候选独立审查与邮箱交付完成；总体覆盖审查尚未完成"} else {"源码初评、工具分析与邮箱交付完成；独立源码 Reviewer 尚未完成"}.into(),
        terminal_code:AGENT_STOP_DERIVED,ledger_reported:false,model_requests:requests,total_tokens:tokens,
        verified_tool_results:verified,covered_families:Vec::new(),uncovered_families:uncovered,confirmed_findings},
        cached_tokens:cached,manifest:json!(manifest),
        review_material,coverage_preparation,candidate_review,decision_projection,coverage_decision})
}

fn finish_native_source_coordinator(
    connection:&rusqlite::Connection,context:&SpecialistTransportContext<'_>,
    lease:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
)->Result<(SourceCompletionProof,Option<crate::native_pipeline::source_ci::SourceGateReport>),String> {
    crate::agent_runtime::multi_agent::budget::clock::elapsed_fact::record_if_exceptional(
        connection, lease,
    )?;
    crate::agent_runtime::multi_agent::budget::clock::closure_write(connection,lease,true,|tx|
        finish_native_source_coordinator_in_scope(tx,context,lease))
}

fn finish_native_source_coordinator_in_scope(
    tx:&rusqlite::Transaction<'_>,context:&SpecialistTransportContext<'_>,
    lease:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
)->Result<(SourceCompletionProof,Option<crate::native_pipeline::source_ci::SourceGateReport>),String> {
    use crate::agent_runtime::{contract::{AgentRole,AgentEventKind},multi_agent::{source,lease as leases},store};
    leases::validate_coordinator_lease(tx,lease)?;
    authorize_source_specialist(tx,context,lease)?;
    let bases=[AgentRole::RepoMapper,AgentRole::SourceAnalyst].into_iter()
        .map(|role|source::task_slice(tx,lease,role).map(|slice|(role,slice))).collect::<Result<Vec<_>,_>>()?;
    let plan:String=tx.query_row("SELECT plan_json FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let parsed:JsonValue=serde_json::from_str(&plan).map_err(|_|"source_completion_plan_invalid")?;
    if parsed["sourceToolsPhaseVersion"]!=1 || parsed["surface"]!="source" {return Err("source_completion_plan_version_invalid".into());}
    let proof=source_assessment_completion(tx,lease,&bases)?;
    if source::SourcePhaseContract::from_plan(&parsed)?.candidate_review && !proof.review_material["decisionContract"].is_null() && proof.candidate_review.is_none() {
        return Err("source_completion_independent_candidate_review_missing".into());
    }
    if source::SourcePhaseContract::from_plan(&parsed)?.coverage_review && proof.coverage_decision.is_none() {
        return Err("source_completion_independent_coverage_review_missing".into());
    }
    let gate=source_completion_gate(tx,lease,&proof)?;
    if let Some(gate)=&gate {
        let changed=tx.execute("UPDATE sentinel_scan_contexts SET gate_status=?2,gate_reason=?3 WHERE scan_id=?1
            AND EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?4 AND status='scanning')",
            params![lease.scan_id,gate.status.as_str(),gate.reasons.join("; "),lease.attempt_number]).map_err(|_|"source_ci_projection_write_failed")?;
        if changed!=1 {return Err("source_ci_projection_write_unconfirmed".into());}
    }
    let outcome=source_determined_terminal_outcome(&proof);
    let final_clock=finish_coordinator_run_in_transaction(tx,lease,&outcome)?;
    // These costs belong to the child model calls. Keep the root's own usage
    // at zero; copying the aggregate there would double-count Native traces.
    let event=json!({"sourceClosureVersion":parsed["schemaVersion"],"independentReviewCompleted":proof.coverage_decision.is_some(),
        "sourceCoverageDecision":proof.coverage_decision.as_ref().map(|decision|decision.as_json()),
        "independentCandidateReviewCompleted":proof.candidate_review.is_some(),"candidateReview":proof.candidate_review,"assignments":proof.manifest,
        "sourceDecisionProjection":proof.decision_projection.as_ref().map(|set|set.as_json()),
        "gate":gate.as_ref().map(|gate|gate.as_json()),
        "reviewMaterial":proof.review_material,
        "coverageReviewPreparation":proof.coverage_preparation.as_json(),
        "verifiedToolResults":proof.completion.verified_tool_results,"totalTokens":proof.completion.total_tokens,
        "cachedTokens":proof.cached_tokens,"modelRequests":proof.completion.model_requests});
    let sequence=store::append_event(tx,&lease.root_run_id,AgentEventKind::TerminalReduced,&event,&[])?;
    leases::require_active_attempt(tx,&lease.scan_id,lease.attempt_number)?;
    leases::validate_coordinator_lease(tx,lease)?;
    if source_assessment_completion(tx,lease,&bases)?!=proof {return Err("source_completion_proof_changed".into());}
    if source_completion_gate(tx,lease,&proof)?!=gate {return Err("source_ci_projection_changed".into());}
    if let Some(gate)=&gate {
        let saved:(String,String)=tx.query_row("SELECT gate_status,gate_reason FROM sentinel_scan_contexts WHERE scan_id=?1",
            [&lease.scan_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"source_ci_projection_missing")?;
        if saved!=(gate.status.as_str().into(),gate.reasons.join("; ")) {return Err("source_ci_projection_changed".into());}
    }
    let terminal:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND root_run_id=id AND parent_run_id IS NULL
        AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND role='coordinator' AND backend='native' AND orchestration_policy='multi'
        AND status='terminal' AND terminal_state=?5 AND terminal_code=?6 AND terminal_reason=?7 AND finished_at<>'' AND cancel_requested_at=''
        AND plan_json=?8 AND used_tokens=0 AND used_cached_tokens=0 AND used_requests=0
        AND hard_token_budget=?11 AND hard_request_budget=?14 AND assignment_id='' AND lane='' AND plan_hash=?12 AND evidence_hash=?13)
        AND EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1 AND sequence=?9 AND event_type='terminal_reduced' AND payload_json=?10)",
        params![lease.root_run_id,lease.scan_id,lease.attempt_number,lease.target_key,outcome.terminal_status(),outcome.terminal_code(),
            crate::agent_runtime::secrets::redact_text_with(&outcome.detail(),None),plan,sequence,event.to_string(),
            parsed["runtime"]["budget"]["tokenLimit"].as_i64().ok_or("source_completion_budget_invalid")?,
            bases[0].1["sourcePlanHash"].as_str().ok_or("source_completion_plan_invalid")?,
            bases[0].1["analysisResultsDigest"].as_str().ok_or("source_completion_plan_invalid")?,source::model_request_limit(&parsed)?],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !terminal {return Err("source_completion_terminal_unconfirmed".into());}
    // A terminal trigger must not change the published runtime or source view.
    let (view,results,source_plan)=source::restore_materials(tx,&lease.scan_id,lease.attempt_number)?;
    if bases.iter().any(|(_,base)|base["analysisDigest"]!=view.manifest.digest()
        || base["analysisResultsDigest"]!=results.digest() || base["sourcePlanHash"]!=source_plan) {
        return Err("source_completion_material_changed".into());
    }
    let model=model_runtime_env(&sentinel_settings(tx))?;
    let runtime=resolve_agent_web_pipeline_runtime(tx,&lease.scan_id,&model.deployment,PathBuf::new())?;
    let directory=WebBindingDirectory::open(context.usage_dir)?;
    let contract=verify_source_runtime_in(tx,&lease.scan_id,lease.attempt_number,&directory,&model,&runtime)?;
    if contract!=parsed["runtime"] {return Err("source_completion_runtime_changed".into());}
    // The Source caller performs local publication after shared finalization.
    // Verify the captured financial cutoff again after its last mutable write.
    final_clock.verify_closed(tx,outcome.terminal_status(),outcome.terminal_code(),
        &crate::agent_runtime::secrets::redact_text_with(&outcome.detail(),None))?;
    Ok((proof,gate))
}

fn source_completion_gate(
    connection:&rusqlite::Connection,
    lease:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    proof:&SourceCompletionProof,
)->Result<Option<crate::native_pipeline::source_ci::SourceGateReport>,String> {
    if proof.decision_projection.is_none() && proof.coverage_decision.is_none() {return Ok(None);}
    let plan=NativeSourcePlan::load(connection,&lease.scan_id,lease.attempt_number)?.ok_or("source_ci_plan_missing")?;
    if plan.scan_type!="cicd" {return Ok(None);}
    let policy=load_workbench_ci_policy(connection,&lease.scan_id,lease.attempt_number)?;
    let review=crate::agent_runtime::multi_agent::source_review_projection::read_audited(connection,lease)?;
    if review.candidates()!=proof.decision_projection.as_ref() || review.coverage()!=proof.coverage_decision.as_ref() {
        return Err("source_ci_completion_review_changed".into());
    }
    crate::native_pipeline::source_ci::evaluate(connection,lease,&review,policy).map(Some)
}

// Audit-only bridge for lossless deletion reconstruction. No runtime settings,
// binding-directory secret, lease renewal, mailbox ACK or execution escapes.
pub(crate) fn verify_closed_source_for_deletion(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::source;
    if db.is_autocommit() { return Err("deleted_audit_snapshot_required".into()); }
    let (terminal, plan): (String,String) = db.query_row(
        "SELECT terminal_state,plan_json FROM agent_runs WHERE id=?1", [&actor.root_run_id],
        |r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if terminal == "paused" {
        return verify_exhausted_source_for_deletion(db, actor, &plan);
    }
    if !matches!(terminal.as_str(), "completed" | "completed_with_gaps") {
        return Err("deleted_audit_original_source_completion_required".into());
    }
    let parsed: JsonValue = serde_json::from_str(&plan).map_err(|_|"source_completion_plan_invalid")?;
    let runtime: String = db.query_row("SELECT contract_json FROM source_runtime_contracts
        WHERE scan_id=?1 AND attempt_number=?2 AND schema_version=1",
        params![actor.scan_id,actor.attempt_number],|r|r.get(0)).map_err(|_|"deleted_audit_source_runtime_missing")?;
    if parsed["runtime"] != serde_json::from_str::<JsonValue>(&runtime).map_err(|_|"deleted_audit_source_runtime_invalid")? {
        return Err("deleted_audit_source_runtime_changed".into());
    }
    let roots = source_branch_original_roots(db, &actor.scan_id, actor.attempt_number)?;
    if roots != vec![actor.root_run_id.clone()] { return Err("source_branch_original_root_ambiguous".into()); }
    let (status,checkpoint,raw):(String,String,String)=db.query_row("SELECT status,checkpoint,report_json FROM native_scan_branches
        WHERE scan_id=?1 AND attempt_number=?2 AND branch='source'",params![actor.scan_id,actor.attempt_number],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"deleted_audit_source_branch_missing")?;
    let report:JsonValue=serde_json::from_str(&raw).map_err(|_|"deleted_audit_source_branch_invalid")?;
    let outcome = source_branch_original_outcome(db,&actor.scan_id,actor.attempt_number,&report,&roots)?;
    if outcome != (status.as_str(),checkpoint.clone()) { return Err("deleted_audit_source_branch_changed".into()); }
    let bases = source::completion_task_slices(db, actor)?;
    let proof = source_assessment_completion(db, actor, &bases)?;
    let contract = source::SourcePhaseContract::from_plan(&parsed)?;
    if (contract.candidate_review && !proof.review_material["decisionContract"].is_null() && proof.candidate_review.is_none())
        || (contract.coverage_review && proof.coverage_decision.is_none()) {
        return Err("deleted_audit_source_independent_review_missing".into());
    }
    let source_plan = NativeSourcePlan::load(db,&actor.scan_id,actor.attempt_number)?.ok_or("source_ci_plan_missing")?;
    let gate = if source_plan.scan_type == "cicd" {
        let policy = load_historical_source_ci_policy(db,&actor.scan_id,actor.attempt_number)?;
        let review = crate::agent_runtime::multi_agent::source_review_projection::read_audited(db,actor)?;
        if review.candidates()!=proof.decision_projection.as_ref() || review.coverage()!=proof.coverage_decision.as_ref() {
            return Err("source_ci_completion_review_changed".into());
        }
        Some(crate::native_pipeline::source_ci::evaluate(db,actor,&review,policy)?)
    } else { None };
    let event=json!({"sourceClosureVersion":parsed["schemaVersion"],"independentReviewCompleted":proof.coverage_decision.is_some(),
        "sourceCoverageDecision":proof.coverage_decision.as_ref().map(|decision|decision.as_json()),
        "independentCandidateReviewCompleted":proof.candidate_review.is_some(),"candidateReview":proof.candidate_review,"assignments":proof.manifest,
        "sourceDecisionProjection":proof.decision_projection.as_ref().map(|set|set.as_json()),
        "gate":gate.as_ref().map(|gate|gate.as_json()),"reviewMaterial":proof.review_material,
        "coverageReviewPreparation":proof.coverage_preparation.as_json(),"verifiedToolResults":proof.completion.verified_tool_results,
        "totalTokens":proof.completion.total_tokens,"cachedTokens":proof.cached_tokens,"modelRequests":proof.completion.model_requests});
    let exact:bool=db.query_row("SELECT count(*)=1 AND coalesce(sum(payload_json=?2),0)=1 FROM agent_events
        WHERE run_id=?1 AND event_type='terminal_reduced' AND json_extract(payload_json,'$.sourceClosureVersion') IS NOT NULL",
        params![actor.root_run_id,event.to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact { return Err("deleted_audit_source_closure_event_changed".into()); }
    Ok(())
}

include!("native_source_failed_deletion.rs");
