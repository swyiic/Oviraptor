//! Original per-child SDK proof, with no live lease adoption.
use rusqlite::params;
use serde_json::json;
use std::path::Path;
pub(crate) const WEB_MODEL_INVOCATION_KIND: &str = "web-executor-sdk";

fn original_children(
    db: &rusqlite::Connection,
    scope: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<std::collections::BTreeSet<String>, String> {
    if db.is_autocommit() {
        return Err("web_model_lifetime_transaction_required".into());
    }

    let mut q=db.prepare("SELECT call_id,assignment_id,child_run_id,lease_epoch,fencing_token,round,request_hash,receipt_json
        FROM agent_web_model_journal WHERE root_run_id=?1 AND phase='dispatch' ORDER BY rowid").map_err(|e|e.to_string())?;
    let calls = q
        .query_map([&scope.root_run_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if calls.is_empty() {
        return Ok(std::collections::BTreeSet::new());
    }
    let owner = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        db,
        &scope.root_run_id,
    )?;
    owner.require_original_coordinator(db, scope)?;
    let mut children = std::collections::BTreeSet::new();
    for (id, assignment, child, epoch, fence, round, hash, raw) in calls {
        let expected = crate::agent_runtime::store::stable_hash(
            &json!({"root":scope.root_run_id,"assignment":assignment,
            "child":child,"epoch":epoch,"fence":fence,"round":round,"requestHash":hash})
            .to_string(),
        );
        if epoch != scope.lease_epoch
            || fence != scope.fencing_token
            || round <= 0
            || hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || expected != id
            || raw != "{}"
        {
            return Err("web_model_lifetime_original_call_conflict".into());
        }
        let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_assignments a
            ON a.id=r.assignment_id AND a.child_run_id=r.id WHERE r.id=?1 AND r.root_run_id=?2
            AND r.parent_run_id=?2 AND r.assignment_id=?3 AND r.scan_id=?4 AND r.attempt_number=?5 AND r.target_url=?6
            AND r.role='web_executor' AND r.backend='native' AND r.orchestration_policy='multi'
            AND a.coordinator_run_id=?2 AND a.lease_epoch=?7 AND a.fencing_token=?8)",
            params![child,scope.root_run_id,assignment,scope.scan_id,scope.attempt_number,scope.target_key,epoch,fence],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !exact {
            return Err("web_model_lifetime_original_worker_conflict".into());
        }
        crate::agent_runtime::multi_agent::budget::receipts::original_owner(
            db,
            &child,
            scope,
            &assignment,
        )?
        .verify(db)?;
        children.insert(child);
    }
    Ok(children)
}
pub(crate) fn verify_closed_original(
    db: &rusqlite::Connection,
    scope: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    verified_usage_original(db, scope, None).map(|_| ())
}

/// Only original dispatches and their verified immutable paid receipts can
/// supply final WebExecutor usage. A checkpoint may lag a stopped tool call.
pub(crate) fn child_usage_original(
    db: &rusqlite::Connection,
    scope: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    assignment: &str,
    child: &str,
) -> Result<crate::agent_runtime::store::UsageDelta, String> {
    super::receipts::original_owner(db, child, scope, assignment)?.verify(db)?;
    verified_usage_original(db, scope, Some(child))
}

fn verified_usage_original(
    db: &rusqlite::Connection,
    scope: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    only_child: Option<&str>,
) -> Result<crate::agent_runtime::store::UsageDelta, String> {
    original_children(db, scope)?;
    let mut total = crate::agent_runtime::store::UsageDelta::default();
    let mut q=db.prepare("SELECT d.assignment_id,d.child_run_id,d.round,d.request_hash,t.receipt_json,t.phase
        FROM agent_web_model_journal d LEFT JOIN agent_web_model_journal t ON t.call_id=d.call_id AND t.phase IN ('received','unsent')
        AND t.root_run_id=d.root_run_id AND t.assignment_id=d.assignment_id AND t.child_run_id=d.child_run_id
        AND t.lease_epoch=d.lease_epoch AND t.fencing_token=d.fencing_token AND t.round=d.round AND t.request_hash=d.request_hash
        WHERE d.root_run_id=?1 AND d.phase='dispatch' AND (?2 IS NULL OR d.child_run_id=?2) ORDER BY d.rowid").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![scope.root_run_id, only_child], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for (assignment, child, round, request, text, phase) in rows {
        let raw = text.ok_or(if only_child.is_some() {
            "budget_indeterminate_requires_reconciliation"
        } else {
            "web_model_original_received_required"
        })?;
        if only_child.is_some() && phase.as_deref() == Some("unsent") {
            if raw != r#"{"code":"user_cancelled"}"# {
                return Err("web_model_original_receipt_invalid".into());
            }
            // BeforeTransport cancellation is the only unsent SDK producer.
            // The original binding above and final model settlement still
            // verify the worker and reject unresolved or conflicting costs.
            continue;
        }
        if phase.as_deref() != Some("received") {
            return Err("web_model_original_received_required".into());
        }
        let value: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| "web_model_original_receipt_invalid")?;
        let hash = value["responseHash"]
            .as_str()
            .filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or("web_model_original_receipt_invalid")?;
        let canonical = value.to_string();
        if only_child.is_some() && value["usageReported"] == false {
            return Err("budget_indeterminate_requires_reconciliation".into());
        }
        if canonical != raw
            || value["usageReported"] != true
            || value.as_object().is_none_or(|o| o.len() != 3)
        {
            return Err("web_model_original_receipt_invalid".into());
        }
        let n = |k: &str| {
            value["usage"][k]
                .as_i64()
                .ok_or("web_model_original_usage_invalid")
        };
        let usage = crate::agent_runtime::store::UsageDelta {
            input_tokens: n("inputTokens")?,
            cached_input_tokens: n("cachedInputTokens")?,
            output_tokens: n("outputTokens")?,
            total_tokens: n("totalTokens")?,
            model_requests: n("modelRequests")?,
        };
        if usage.model_requests != 1
            || usage.input_tokens < 0
            || usage.cached_input_tokens < 0
            || usage.output_tokens < 0
            || usage.cached_input_tokens > usage.input_tokens
            || usage.input_tokens.checked_add(usage.output_tokens) != Some(usage.total_tokens)
        {
            return Err("web_model_original_usage_invalid".into());
        }
        crate::agent_runtime::multi_agent::budget::receipts::original_owner(
            db,
            &child,
            scope,
            &assignment,
        )?
        .verify(db)?;
        crate::agent_runtime::multi_agent::budget::receipts::verify(
            db,
            scope,
            &assignment,
            &format!("web-model:{assignment}:{round}:{request}"),
            hash,
            &usage,
        )?;
        total.input_tokens = total
            .input_tokens
            .checked_add(usage.input_tokens)
            .ok_or("budget_amount_overflow")?;
        total.cached_input_tokens = total
            .cached_input_tokens
            .checked_add(usage.cached_input_tokens)
            .ok_or("budget_amount_overflow")?;
        total.output_tokens = total
            .output_tokens
            .checked_add(usage.output_tokens)
            .ok_or("budget_amount_overflow")?;
        total.total_tokens = total
            .total_tokens
            .checked_add(usage.total_tokens)
            .ok_or("budget_amount_overflow")?;
        total.model_requests = total
            .model_requests
            .checked_add(usage.model_requests)
            .ok_or("budget_amount_overflow")?;
    }
    Ok(total)
}
pub(crate) fn require_idle_original(
    db: &rusqlite::Connection,
    scope: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>, String> {
    let children = original_children(db, scope)?;
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("web_model_lifetime_database_missing")?;
    let mut guards = vec![];
    for child in children {
        let guard = crate::agent_runtime::execution_owner::probe_native_invocation(
            Path::new(path),
            &scope.scan_id,
            scope.attempt_number,
            WEB_MODEL_INVOCATION_KIND,
            &child,
        )
        .map_err(|e| format!("web_model_transport_not_idle:{e}"))?
        .ok_or("web_model_transport_original_exit_proof_missing")?;
        guards.push(guard);
    }
    // These guards prove only local SDK return, not HTTP/tool/process exit.
    Ok(guards)
}
