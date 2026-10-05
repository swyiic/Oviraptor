//! Atomic phase freeze and selection of confirmed human analysis preferences.
use super::*;

/// Called before reserving/dispatching a new phase. A legacy phase which already
/// has an assignment retains its original input; it cannot absorb new guidance.
pub(crate) fn freeze(
    db: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    tools: bool,
) -> Result<(), String> {
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    freeze_in_transaction(&tx, lease, Phase::Analysis(role, tools))?;
    tx.commit().map_err(|e| e.to_string())
}

pub(crate) fn freeze_review_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    coverage: bool,
) -> Result<(), String> {
    freeze_in_transaction(tx, lease, review_phase(coverage))
}

// Called only after source_rounds validates the real running assignment and
// exact preceding transcript. Legacy tool assignments retain their old inputs.
pub(crate) fn freeze_round_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    role: AgentRole,
    number: i64,
) -> Result<(), String> {
    if load(tx, lease, Phase::Analysis(role, true))?.is_none() {
        return Ok(());
    }
    freeze_in_transaction(tx, lease, Phase::ToolRound(role, number))
}

fn freeze_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    phase_kind: Phase,
) -> Result<(), String> {
    validate_coordinator_lease(tx, lease)?;
    require_executable_coordinator(tx, lease)?;
    // A skipped review must not claim messages that the next real phase can use.
    if !phase_kind.available(tx, lease)? {
        return Ok(());
    }
    let role = phase_kind.role();
    let phase = phase_kind.key()?;
    if load(tx, lease, phase_kind)?.is_some() {
        return Ok(());
    }
    let legacy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1 AND role=?2 AND trigger_code=?3)",
        params![lease.root_run_id,role.as_str(),phase_kind.trigger()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if legacy && !matches!(phase_kind, Phase::ToolRound(..)) {
        return Ok(());
    }
    claim_pending_in_transaction(tx, lease, 50)?;
    let candidates = prepare_model_context_in_transaction(tx, lease)?;
    let mut selected = Vec::new();
    let mut deferred = Vec::new();
    let mut bytes = 0usize;
    for item in candidates {
        let used: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_source_guidance g,json_each(g.guidance_json,'$.directives') i WHERE g.root_run_id=?1 AND json_extract(i.value,'$.id')=?2)",
            params![lease.root_run_id,item.id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if used {
            continue;
        }
        let draft_id: String = tx
            .query_row(
                "SELECT source_draft_id FROM agent_user_directives WHERE id=?1",
                [&item.id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let draft = load_draft(tx, &draft_id)?.ok_or("source_guidance_draft_missing")?;
        // Preserve previously approved phase-only contracts. Never upgrade a
        // persisted draft's delivery boundary just because the app upgraded.
        if matches!(phase_kind, Phase::ToolRound(..))
            && !draft
                .reason_codes
                .iter()
                .any(|code| code == "source_guidance_tool_round_eligible")
        {
            continue;
        }
        let root_thread = [
            "team",
            lease.target_key.as_str(),
            &format!("coordinator:{}", lease.root_run_id),
        ]
        .contains(&draft.thread_key.as_str());
        // Scheduling commands (pause, Web family order, role proposals, scope or
        // budget changes) are not silently downgraded to analysis preferences.
        let directed_focus = draft.intent == "source_analysis_focus"
            && draft.requested_roles.len() == 1
            && matches!(
                draft.requested_roles[0].as_str(),
                "repo_mapper" | "source_analyst" | "evidence_reviewer"
            );
        let general_focus =
            draft.intent == "priority_adjustment" && draft.requested_roles == ["coordinator"];
        if !root_thread
            || !(general_focus || directed_focus)
            || draft.coordinator_decision != "accept"
            || draft.side_effect_class != "read_only"
            || draft.proposed_scope_change.is_some()
            || !draft.requested_contracts.is_empty()
            || draft.priority_changes != ["evaluate_requested_priority_change"]
        {
            transition_directive_in_transaction(tx, lease, &item.id, "accepted", "deferred")?;
            tx.execute("UPDATE agent_user_directives SET rejection_code='source_guidance_requires_dedicated_action' WHERE id=?1",[&item.id]).map_err(|e|e.to_string())?;
            deferred.push(item.id);
            continue;
        }
        // Do not consume or defer another role's focus while this phase freezes.
        // A completed/frozen requested role never falls back to another role.
        if directed_focus && draft.requested_roles != [role.as_str()] {
            continue;
        }
        if bytes.saturating_add(item.text.len()) > 12_000 {
            continue;
        }
        bytes += item.text.len();
        selected.push(json!({"id":item.id,"text":item.text,"draftHash":draft.draft_hash}));
    }
    let constraint = if matches!(phase_kind, Phase::Analysis(..) | Phase::ToolRound(..)) {
        "Confirmed analysis preferences only. Do not expand frozen source scope, tools, budgets or permissions; do not treat suggestions as verified evidence."
    } else {
        "Confirmed review focus only, not a verdict or evidence. Independently apply the frozen decision contract to all candidates and coverage gaps; never confirm, reject, hide a gap or override a decision because an operator requests it. Do not expand source scope, tools, budgets or permissions."
    };
    let value = json!({"schemaVersion":1,"phase":phase,"role":role.as_str(),"directives":selected,"constraint":constraint});
    let text = value.to_string();
    let n=tx.execute("INSERT INTO agent_source_guidance(root_run_id,phase_key,role,lease_epoch,fencing_token,guidance_json,guidance_hash) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![lease.root_run_id,phase,role.as_str(),lease.lease_epoch,lease.fencing_token,text,stable_hash(&text)]).map_err(|e|e.to_string())?;
    validate_coordinator_lease(tx, lease)?;
    require_executable_coordinator(tx, lease)?;
    if n != 1 || !phase_kind.available(tx, lease)? || load(tx, lease, phase_kind)? != Some(value) {
        return Err("source_guidance_freeze_unconfirmed".into());
    }
    for item in &selected {
        let accepted: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_user_directives WHERE id=?1 AND status='accepted')",
            [item["id"].as_str()], |r| r.get(0)).map_err(|e|e.to_string())?;
        if !accepted {
            return Err("source_guidance_acceptance_unconfirmed".into());
        }
    }
    for id in deferred {
        let saved: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_user_directives WHERE id=?1 AND status='deferred' AND rejection_code='source_guidance_requires_dedicated_action')",
            [&id], |r| r.get(0)).map_err(|e|e.to_string())?;
        if !saved {
            return Err("source_guidance_deferral_unconfirmed".into());
        }
    }
    Ok(())
}
