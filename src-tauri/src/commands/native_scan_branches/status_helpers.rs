/// A chat projection is informational, never an execution authorization. Only
/// bounded, explicitly selected fields leave the persisted mailbox payload.
fn gap_timeline_projection(kind: &str, payload: &JsonValue) -> Option<(&'static str, JsonValue)> {
    let version = payload["schemaVersion"].as_i64()?;
    if !matches!(version, 2 | 3) {
        return None;
    }
    if kind == "proposal_assessed" {
        let decision = payload["decision"].as_str()?;
        let reason = payload["reasonCode"].as_str()?;
        if decision != "deferred_requires_new_evidence_revision"
            || !(if version == 3 {
                matches!(reason, "human_review_required" | "no_new_verified_fact_in_revision"
                    | "operator_approval_new_attempt_required")
                    && payload["targetRequestsGranted"] == 0
                    && payload["newAttemptRequired"].as_bool()
                        == Some(reason == "operator_approval_new_attempt_required")
            } else {
                matches!(reason, "human_review_required" | "no_new_verified_fact_in_revision"
                    | "proposal_is_not_a_verified_execution_contract")
            })
        {
            return None;
        }
        return Some(("gapAssessment", json!({"decision":decision,"reasonCode":reason,
            "newAttemptRequired":version == 3 && payload["newAttemptRequired"] == true,
            "targetRequestsGranted":0})));
    }
    if kind != "gap_proposed" {
        return None;
    }
    let proposal = payload.get("proposal")?;
    let code = proposal["gapCode"].as_str()?;
    let step = payload["nextStep"].as_str()?;
    let effect = proposal["sideEffectClass"].as_str()?;
    if code.is_empty() || code.len() > 64
        || !code.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        || !matches!(step, "observe_existing_evidence" | "request_new_contract" | "manual_review")
        || effect != "read_only"
    {
        return None;
    }
    let bounded_list = |key: &str, max: usize| -> Option<Vec<String>> {
        let values = proposal.get(key)?.as_array()?;
        if values.len() > max { return None; }
        values.iter().map(|value| {
            let text = value.as_str()?;
            if text.is_empty() || text.chars().count() > 240 { return None; }
            Some(crate::agent_runtime::secrets::redact_text_with(text, None))
        }).collect()
    };
    let missing = bounded_list("missingEvidence", 8)?;
    let prerequisites = bounded_list("prerequisites", 8)?;
    let refs = bounded_list("supportingFactRefs", 16)?;
    let contracts = bounded_list("proposedContracts", 2)?;
    if contracts.iter().any(|contract| !matches!(contract.as_str(),
        "existing_evidence_review" | "operator_approved_control_group"
        | "new_attempt_control_group_request")) {
        return None;
    }
    let cost = proposal.get("estimatedCost")?;
    let tokens = cost["modelTokens"].as_u64()?;
    let model_requests = cost["modelRequests"].as_u64()?;
    let target_requests = cost["targetRequests"].as_u64()?;
    if tokens > 4_000 || model_requests > 1 || target_requests > 3 {
        return None;
    }
    if version == 3 {
        let expected = match step {
            "manual_review" => (Vec::<String>::new(), 0),
            "observe_existing_evidence" => (vec!["existing_evidence_review".into()], 0),
            "request_new_contract" => (vec!["new_attempt_control_group_request".into()], 3),
            _ => return None,
        };
        if contracts.as_slice() != expected.0.as_slice() || target_requests != expected.1 {
            return None;
        }
    }
    Some(("gapDetail", json!({
        "gapCode":code, "nextStep":step, "sideEffectClass":effect,
        "missingEvidence":missing, "prerequisites":prerequisites,
        "supportingFactRefs":refs, "proposedContracts":contracts,
        "estimatedCost":{"modelTokens":tokens,"modelRequests":model_requests,"targetRequests":target_requests},
    })))
}

fn native_scan_status(connection: &rusqlite::Connection, scan_id: &str) -> Result<JsonValue, String> {
    native_scan_status_after(connection, scan_id, None)
}

// This is a read-only interpretation of durable receipts, not a resume permit.
// In particular, free-form checkpoints are never parsed as WAF, authorization,
// capability or retry evidence.
fn native_stop_diagnostic(
    status: &str, branches: &[JsonValue], targets: &[JsonValue],
    source_gaps: &[String], unresolved: &[JsonValue],
) -> JsonValue {
    const MAX_OBLIGATIONS: usize = 100;
    let mut obligations = Vec::new();
    let mut add = |kind: &str, reference: &str, state: &str| {
        if obligations.len() < MAX_OBLIGATIONS {
            obligations.push(json!({"kind":kind,"reference":reference,"status":state}));
        }
    };
    for receipt in unresolved {
        add("cleanup", receipt["receiptId"].as_str().unwrap_or(""),
            receipt["status"].as_str().unwrap_or("unknown"));
    }
    for branch in branches {
        let state = branch["status"].as_str().unwrap_or("unknown");
        if state == "failed" && branch["report"]["code"] == WEB_CLOSURE_REASON {
            add("branch_not_executed", branch["branch"].as_str().unwrap_or("unknown"), "operator_closed");
            continue;
        }
        let kind = match state {
            "pending" => "branch_pending",
            "failed" | "partial" => "branch_failed",
            "completed_with_gaps" => "branch_gap",
            "completed" => continue,
            _ => "branch_unknown",
        };
        add(kind, branch["branch"].as_str().unwrap_or("unknown"), state);
    }
    for target in targets {
        let state = target["status"].as_str().unwrap_or("unknown");
        if matches!(state, "completed" | "completed_with_gaps" | "recon_only" | "skipped") {
            continue;
        }
        add("target", target["url"].as_str().unwrap_or(""), state);
    }
    for gap in source_gaps {
        add("source_gap", gap, "unresolved");
    }
    let shown = obligations.len();
    let truncated = shown == MAX_OBLIGATIONS || unresolved.len() == 50;
    let has_branch_failure = branches.iter().any(|branch| matches!(branch["status"].as_str(),
        Some("failed" | "partial" | "unknown")));
    let retired_backend = branches.iter().any(|branch| branch["status"] == "failed"
        && branch["report"]["error"] == "backend_retired");
    let has_gap = !source_gaps.is_empty() || branches.iter().any(|branch| branch["status"] == "completed_with_gaps");
    let pending = branches.iter().any(|branch| branch["status"] == "pending");
    let target_unfinished = targets.iter().any(|target| !matches!(target["status"].as_str(),
        Some("completed" | "completed_with_gaps" | "recon_only" | "skipped")));
    let (category, code, next_action, constraint) = if !unresolved.is_empty() {
        ("hard", "cleanup_unconfirmed", "verify_owned_cleanup", "blocked_until_cleanup_confirmed")
    } else if status == "pausing" {
        ("active", "pause_waiting_for_quiescence", "wait_for_worker_exit", "blocked_until_quiescent")
    } else if status == "paused" {
        ("soft", "paused_requires_review", "review_before_manual_resume", "manual_review_new_attempt")
    } else if status == "cancelled" {
        if branches.iter().any(|b| b["report"]["code"] == WEB_CLOSURE_REASON) {
            ("soft", WEB_CLOSURE_REASON, "review_scope_before_new_attempt", "manual_review_new_attempt")
        } else {
            ("hard", "cancelled", "review_scope_before_new_attempt", "manual_review_new_attempt")
        }
    } else if retired_backend {
        ("capability", "backend_retired", "select_native_new_attempt", "manual_review_new_attempt")
    } else if has_branch_failure {
        ("hard", "branch_failed", "inspect_branch_receipt", "manual_review_new_attempt")
    } else if matches!(status, "scanning" | "queued") {
        ("active", "in_progress", "wait_for_receipts", "current_attempt_only")
    } else if status == "failed" || status == "partial" {
        ("hard", "execution_incomplete", "inspect_attempt_and_targets", "manual_review_new_attempt")
    } else if matches!(status, "completed" | "completed_with_gaps") && (pending || target_unfinished || status == "completed" && has_gap) {
        ("hard", "incomplete_obligations", "inspect_attempt_and_targets", "manual_review_new_attempt")
    } else if status == "completed_with_gaps" || has_gap {
        ("natural", "completed_with_gaps", "review_coverage_gaps", "manual_review_new_attempt")
    } else if status == "completed" && !branches.is_empty() {
        ("natural", "completed", "none", "new_attempt_only")
    } else {
        ("hard", "native_receipts_unavailable", "inspect_attempt_and_targets", "manual_review_new_attempt")
    };
    let stage = if !unresolved.is_empty() { "cleanup" }
        else if branches.iter().any(|branch| branch["status"] != "completed") { "branch" }
        else if target_unfinished { "target" }
        else if !source_gaps.is_empty() { "source" }
        else { "scan" };
    json!({
        "category":category,"code":code,"stage":stage,"nextAction":next_action,
        "continuationConstraint":constraint,"automaticResumeAllowed":false,
        "obligations":obligations,"obligationsTruncated":truncated,
    })
}
