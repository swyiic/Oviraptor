// Original paid Client output, source configuration and consumed mailbox.
impl NativeCoordinatorFrame {
    fn client_side(
        tx: &rusqlite::Transaction<'_>,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{contract::AgentRole, multi_agent::{client_side, mailbox, specialist}, store};
        if child.role != AgentRole::ClientSide { return Err("root_client_frame_role_invalid".into()); }
        let task = client_side::assignment(tx, actor, child)?;
        let receipt = specialist::received_for_reconciliation(tx, actor, child)?;
        if !receipt.rejection.is_empty() { return Err("root_client_frame_paid_output_rejected".into()); }
        let id: String = tx.query_row("SELECT id FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2
            AND from_run_id=?3 AND to_run_id=?1 AND from_agent='client_side' AND to_agent='coordinator'
            AND kind='evidence_summary' AND correlation_id=?4 AND evidence_revision=1",
            params![actor.root_run_id,child.assignment_id,child.run_id,format!("client-side:{}",child.assignment_id)], |r|r.get(0))
            .map_err(|_|"root_client_frame_output_missing")?;
        let message = mailbox::verified_completed_output(tx, actor, &id)?;
        client_side_validate_received(tx, actor, child, &receipt.usage, &message.payload, Some(&context.target_dir))?;
        let output: JsonValue = serde_json::from_str(&receipt.text).map_err(|_|"root_client_frame_output_invalid")?;
        let observations: Vec<_> = task.observations.iter().map(|o|json!({"id":o.id,"kind":o.kind,
            "classification":o.classification,"value":o.value,"artifactHash":o.artifact.content_hash})).collect();
        let mut rows = Self::closed_rows(tx, actor, child)?;
        rows.push(NativeCoordinatorFrozenRows::capture(tx,
            "SELECT rowid,* FROM agent_collaboration_events WHERE entity_id IN (?1,?2,?3) ORDER BY sequence",
            &[&child.run_id,&child.assignment_id,&id])?);
        let frame = Self {
            kind: "client-side-output",
            semantic: json!({"messageId":id,"role":"client_side","assignmentId":child.assignment_id,"runId":child.run_id,
                "taskHash":store::stable_hash(&serde_json::to_value(&task).map_err(|e|e.to_string())?.to_string()),
                "observations":crate::agent_runtime::secrets::redact_json(&json!(observations)),
                "output":crate::agent_runtime::secrets::redact_json(&output),
                "targetRequestsGranted":0,"browserActionsGranted":0,"newTargetEvidenceProven":false,"browserImpactProven":false}),
            rows, review_refs: None, review_meta: None,
        };
        frame.verify(tx, context, actor)?;
        Ok(frame)
    }
    fn verify_client_source(
        &self, db: &rusqlite::Connection, context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        use crate::agent_runtime::{contract::AgentRole, multi_agent::{client_side, scheduler::ScheduledChild}, store};
        crate::collaboration_events::client_delivery_schema::verify(db)?;
        let child = ScheduledChild {
            assignment_id: self.semantic["assignmentId"].as_str().ok_or("root_client_frame_scope_invalid")?.into(),
            run_id: self.semantic["runId"].as_str().ok_or("root_client_frame_scope_invalid")?.into(),
            role: AgentRole::ClientSide,
        };
        let task = client_side::assignment(db, actor, &child)?;
        if self.semantic["taskHash"] != store::stable_hash(&serde_json::to_value(&task).map_err(|e|e.to_string())?.to_string()) {
            return Err("root_client_frame_original_task_changed".into());
        }
        client_side_verify_frozen_artifacts(&task, db, actor, &context.target_dir)
    }
}
