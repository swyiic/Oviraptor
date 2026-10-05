//! Read-only verification for historical delivery projections.
use super::*;

/// Validate a claimed delivery against its frozen input, real assignment,
/// persisted provider response and model event. This cannot grant a capability
/// or revive an expired Coordinator lease.
pub(crate) fn project_delivery(db: &Connection, id: &str) -> Result<Option<Value>, String> {
    if db.is_autocommit() {
        return Err("source_guidance_projection_transaction_required".into());
    }
    let text: String = db
        .query_row(
            "SELECT payload_json FROM agent_user_directives WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let payload: Value =
        serde_json::from_str(&text).map_err(|_| "source_guidance_delivery_invalid")?;
    let receipt = &payload["sourceGuidance"];
    let mut query=db.prepare("SELECT d.scan_id,d.attempt_number,d.target_key,d.root_run_id,g.lease_epoch,g.fencing_token,g.role,g.phase_key
        FROM agent_user_directives d JOIN agent_source_guidance g ON g.root_run_id=d.root_run_id
        WHERE d.id=?1 AND EXISTS(SELECT 1 FROM json_each(g.guidance_json,'$.directives') i WHERE json_extract(i.value,'$.id')=d.id)").map_err(|e|e.to_string())?;
    let rows = query
        .query_map([id], |r| {
            Ok((
                CoordinatorLease {
                    scan_id: r.get(0)?,
                    attempt_number: r.get(1)?,
                    target_key: r.get(2)?,
                    root_run_id: r.get(3)?,
                    lease_epoch: r.get(4)?,
                    fencing_token: r.get(5)?,
                    lease_expires_at: String::new(),
                },
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if rows.is_empty()
        && receipt.is_null()
        && payload["taskClosure"]["disposition"] != "analysis_guidance_delivered"
    {
        return Ok(None);
    }
    if rows.len() != 1 {
        return Err("source_guidance_snapshot_binding_invalid".into());
    }
    let (lease, role_name, phase) = &rows[0];
    let role = match role_name.as_str() {
        "repo_mapper" => AgentRole::RepoMapper,
        "source_analyst" => AgentRole::SourceAnalyst,
        "evidence_reviewer" => AgentRole::EvidenceReviewer,
        _ => return Err("source_guidance_role_invalid".into()),
    };
    let phase_kind = if role == AgentRole::EvidenceReviewer {
        match phase.as_str() {
            "review:source_candidates" => Phase::CandidateReview,
            "review:source_coverage" => Phase::CoverageReview,
            _ => return Err("source_guidance_phase_invalid".into()),
        }
    } else if let Some(number) = phase.strip_prefix(&format!("{}:round:", key(role, true)?)) {
        Phase::ToolRound(
            role,
            number
                .parse()
                .map_err(|_| "source_guidance_round_invalid")?,
        )
    } else {
        Phase::Analysis(role, phase == &key(role, true)?)
    };
    let tools = phase_kind.tools();
    if phase != &phase_kind.key()? {
        return Err("source_guidance_phase_invalid".into());
    }
    if receipt.is_null() {
        // A frozen phase awaiting its response is normal. A response already
        // saved for that phase makes a missing receipt corruption, even when
        // both mutable delivery fields have been removed. Never repair it here.
        let received: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a
             WHERE a.coordinator_run_id=?1 AND a.role=?2 AND a.trigger_code=?3
             AND ((?4=0 AND EXISTS(SELECT 1 FROM agent_specialist_calls c WHERE c.assignment_id=a.id AND c.child_run_id=a.child_run_id AND c.state='received'))
               OR (?4=1 AND EXISTS(SELECT 1 FROM agent_source_model_rounds c WHERE c.assignment_id=a.id AND c.child_run_id=a.child_run_id AND c.round_number=?5 AND c.state='received'))))",
            params![lease.root_run_id,role_name,phase_kind.trigger(),tools,phase_kind.round()], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if received
            || payload["modelDelivery"]["state"] == "model_received"
            || payload["taskClosure"]["disposition"] == "analysis_guidance_delivered"
        {
            return Err("source_guidance_delivery_missing".into());
        }
        return Ok(None);
    }
    let assignment = receipt["assignmentId"]
        .as_str()
        .ok_or("source_guidance_delivery_invalid")?;
    let child_id:String=db.query_row("SELECT a.child_run_id FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_runs root ON root.id=a.coordinator_run_id
        WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.role=?3 AND a.target_key=?4 AND a.lease_epoch=?5 AND a.fencing_token=?6 AND a.trigger_code=?7
        AND r.assignment_id=a.id AND r.root_run_id=root.id AND r.parent_run_id=root.id AND r.scan_id=?8 AND r.attempt_number=?9 AND r.target_url=a.target_key AND r.role=a.role
        AND a.lane=?10 AND r.lane=a.lane AND root.role='coordinator' AND root.scan_id=r.scan_id AND root.attempt_number=r.attempt_number AND root.target_url=r.target_url",
        params![assignment,lease.root_run_id,role_name,lease.target_key,lease.lease_epoch,lease.fencing_token,phase_kind.trigger(),lease.scan_id,lease.attempt_number,phase_kind.lane()],|r|r.get(0)).map_err(|_|"source_guidance_assignment_binding_invalid")?;
    let child = ScheduledChild {
        assignment_id: assignment.into(),
        run_id: child_id,
        role,
    };
    let sequence = receipt["eventSequence"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("source_guidance_delivery_invalid")?;
    verify_phase_delivery(db, lease, &child, phase_kind, sequence)?;
    let sql = if tools {
        "SELECT request_json FROM agent_source_model_rounds WHERE assignment_id=?1 AND round_number=?2 AND child_run_id=?3"
    } else {
        "SELECT request_json FROM agent_specialist_calls WHERE assignment_id=?1 AND ?2=1 AND child_run_id=?3"
    };
    let request: String = db
        .query_row(
            sql,
            params![assignment, phase_kind.round(), child.run_id],
            |r| r.get(0),
        )
        .map_err(|_| "source_guidance_request_missing")?;
    let request: Value =
        serde_json::from_str(&request).map_err(|_| "source_guidance_request_invalid")?;
    let input = request["messages"]
        .as_array()
        .and_then(|items| items.iter().rev().find(|item| item["role"] == "user"))
        .and_then(|item| item["content"].as_str())
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .ok_or("source_guidance_request_invalid")?;
    let expected = attach_phase(db, lease, phase_kind, json!({}))?;
    if input.get("operatorGuidance") != expected.get("operatorGuidance") {
        return Err("source_guidance_input_mismatch".into());
    }
    if tools {
        super::super::super::source_rounds::guidance_received_for_audit(
            db,
            lease,
            &child,
            phase_kind.round(),
        )?;
    } else {
        super::super::super::specialist::source_received_for_completion(db, lease, &child)?;
    }
    Ok(Some(receipt.clone()))
}
