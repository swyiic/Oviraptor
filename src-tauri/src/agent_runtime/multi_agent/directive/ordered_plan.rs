//! Versioned confirmation authority; frozen plans never become execution receipts.
use super::{DraftInterpretation, HumanDirectiveDraft};
use crate::agent_runtime::store::stable_hash;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub const MARKER: &str = "ordered_readonly_assessment_v2";
pub const NOT_CONNECTED: &str = "proposal_ordered_execution_not_connected";
pub const LIVE_MARKER: &str = "ordered_readonly_assessment_v3";
pub const DISPATCH_CHECKS: &str = "ordered_original_dispatch_checks_required";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderedAssessmentPlan {
    pub schema_version: i64,
    pub purpose: String,
    pub binding_hash: String,
    pub plan_hash: String,
    pub dispatch_state: String,
    pub total_token_ceiling: i64,
    pub total_model_requests: i64,
    pub target_requests: i64,
    pub actions: Vec<OrderedAssessmentAction>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderedAssessmentAction {
    pub action_id: String,
    pub order: i64,
    pub role: String,
    pub token_ceiling: i64,
    pub model_requests: i64,
    pub max_output_tokens: i64,
    pub target_requests: i64,
    pub previous_action_id: Option<String>,
    pub required_previous_state: String,
    pub advisory_only: bool,
    pub execution_state: String,
}

fn mention_order(text: &str, roles: &[String]) -> Option<Vec<String>> {
    let lower = text.to_ascii_lowercase();
    let mut ordered = Vec::new();
    for role in roles {
        let aliases: &[&str] = match role.as_str() {
            "coordinator" => &["@coordinator", "@协调", "@总控"],
            "spa_api_mapper" => &["@mapper", "@采集", "@映射"],
            "deep_investigator" => &["@investigator", "@调查", "@深度调查"],
            _ => return None,
        };
        let position = aliases.iter().filter_map(|a| lower.find(a)).min()?;
        ordered.push((position, role.clone()));
    }
    ordered.sort_by_key(|(position, _)| *position);
    Some(ordered.into_iter().map(|(_, role)| role).collect())
}

fn supported_pair(roles: &[String]) -> bool {
    let actions: Vec<_> = roles
        .iter()
        .filter(|r| r.as_str() != "coordinator")
        .collect();
    actions.len() == 2
        && actions.iter().any(|r| r.as_str() == "spa_api_mapper")
        && actions.iter().any(|r| r.as_str() == "deep_investigator")
        && roles.len() <= 3
        && roles.iter().filter(|r| r.as_str() == "coordinator").count() <= 1
}

pub(super) fn freeze_fresh_on(
    db: &rusqlite::Connection,
    root: &str,
    target: &str,
    value: &mut DraftInterpretation,
    text: &str,
) -> Result<(), String> {
    // This is a Web assessment preview, never a Source role upgrade. Read the
    // original persisted Source binding; do not infer authority from mentions.
    let source: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND root_run_id=id
         AND role='coordinator' AND target_url=?2 AND json_extract(plan_json,'$.surface')='source')",
        rusqlite::params![root, target], |row| row.get(0),
    ).map_err(|e| format!("directive_ordered_source_binding:{e}"))?;
    if !source { freeze_fresh(value, text); }
    Ok(())
}

fn freeze_fresh(value: &mut DraftInterpretation, text: &str) {
    if value.intent != "agent_proposal_request"
        || value.validation_result != "valid"
        || value.coordinator_decision != "accept"
        || value.status != "drafted"
        || value.side_effect_class != "read_only"
        || !value.requested_contracts.is_empty()
        || value.proposed_scope_change.is_some()
        || value
            .priority_changes
            .iter()
            .any(|p| p == "pause_or_deprioritize_requested_work")
        || !supported_pair(&value.requested_roles)
    {
        return;
    }
    let Some(roles) = mention_order(text, &value.requested_roles) else {
        return;
    };
    value.requested_roles = roles;
    value.estimated_tokens = 2 * super::proposals::PROPOSAL_TOKENS;
    value.estimated_requests = 2;
    value.reason_codes.push(LIVE_MARKER.into());
    value.reason_codes.push(DISPATCH_CHECKS.into());
}

fn plan(material: &Value) -> Result<Option<OrderedAssessmentPlan>, String> {
    let invalid = "directive_ordered_plan_unverified";
    let reasons: Vec<String> =
        serde_json::from_value(material["reasonCodes"].clone()).map_err(|_| invalid)?;
    let old = reasons.iter().any(|r| r == MARKER);
    let live = reasons.iter().any(|r| r == LIVE_MARKER);
    if !old && !live {
        return Ok(None);
    }
    if old && live {
        return Err(invalid.into());
    }
    let (marker, required, version, dispatch) = if live {
        (LIVE_MARKER, DISPATCH_CHECKS, 3, "requires_dispatch_checks")
    } else {
        (MARKER, NOT_CONNECTED, 2, "not_connected")
    };
    let roles: Vec<String> =
        serde_json::from_value(material["requestedRoles"].clone()).map_err(|_| invalid)?;
    let text = material["text"].as_str().ok_or(invalid)?;
    if reasons.iter().filter(|r| r.as_str() == marker).count() != 1
        || !reasons.iter().any(|r| r == required)
        || !supported_pair(&roles)
        || mention_order(text, &roles).as_ref() != Some(&roles)
        || material["intent"] != "agent_proposal_request"
        || material["sideEffectClass"] != "read_only"
        || material["validationResult"] != "valid"
        || material["coordinatorDecision"] != "accept"
        || material["requestedContracts"] != json!([])
        || !material["proposedScopeChange"].is_null()
        || material["estimatedTokens"] != 2 * super::proposals::PROPOSAL_TOKENS
        || material["estimatedRequests"] != 2
    {
        return Err(invalid.into());
    }
    let mut binding = json!({});
    for key in [
        "sourceMessageId",
        "scanId",
        "rootRunId",
        "targetKey",
        "recipientRole",
        "threadKey",
        "boundFencingToken",
    ] {
        let value = material[key]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or(invalid)?;
        binding[key] = json!(value);
    }
    for key in ["attemptNumber", "revision", "boundLeaseEpoch"] {
        let value = material[key].as_i64().filter(|v| *v > 0).ok_or(invalid)?;
        binding[key] = json!(value);
    }
    let binding_hash = stable_hash(&binding.to_string());
    let mut actions: Vec<OrderedAssessmentAction> = Vec::new();
    for (index, role) in roles
        .iter()
        .filter(|r| r.as_str() != "coordinator")
        .enumerate()
    {
        let order = (index + 1) as i64;
        let action_id = stable_hash(&json!([marker, binding_hash, order, role]).to_string());
        actions.push(OrderedAssessmentAction {
            action_id,
            order,
            role: role.clone(),
            token_ceiling: super::proposals::PROPOSAL_TOKENS,
            model_requests: 1,
            max_output_tokens: 512,
            target_requests: 0,
            previous_action_id: actions.last().map(|a| a.action_id.clone()),
            required_previous_state: if index == 0 {
                "frozen_original_evidence"
            } else {
                "valid_advisory_receipt"
            }
            .into(),
            advisory_only: true,
            execution_state: "not_started".into(),
        });
    }
    let mut result = OrderedAssessmentPlan {
        schema_version: version,
        purpose: "human_readonly_assessment".into(),
        binding_hash,
        plan_hash: String::new(),
        dispatch_state: dispatch.into(),
        total_token_ceiling: 2 * super::proposals::PROPOSAL_TOKENS,
        total_model_requests: 2,
        target_requests: 0,
        actions,
    };
    let mut canonical = json!(result);
    canonical.as_object_mut().unwrap().remove("planHash");
    result.plan_hash = stable_hash(&canonical.to_string());
    Ok(Some(result))
}

pub(super) fn append_hash_material(material: &mut Value) -> Result<(), String> {
    if let Some(plan) = plan(material)? {
        material["readonlyAssessmentPlan"] = json!(plan)
    }
    Ok(())
}

pub(super) fn attach(draft: &mut HumanDirectiveDraft) -> Result<(), String> {
    let material = json!({"sourceMessageId":draft.source_message_id,
        "scanId":draft.scan_id,"attemptNumber":draft.attempt_number,"rootRunId":draft.root_run_id,
        "targetKey":draft.target_key,"recipientRole":draft.recipient_role,"threadKey":draft.thread_key,
        "text":draft.text,"intent":draft.intent,"requestedRoles":draft.requested_roles,
        "reasonCodes":draft.reason_codes,"sideEffectClass":draft.side_effect_class,
        "validationResult":draft.validation_result,"coordinatorDecision":draft.coordinator_decision,
        "requestedContracts":draft.requested_contracts,"proposedScopeChange":draft.proposed_scope_change,
        "estimatedTokens":draft.estimated_tokens,"estimatedRequests":draft.estimated_requests,
        "revision":draft.revision,"boundLeaseEpoch":draft.bound_lease_epoch,
        "boundFencingToken":draft.bound_fencing_token});
    draft.readonly_assessment_plan = plan(&material)?;
    Ok(())
}
