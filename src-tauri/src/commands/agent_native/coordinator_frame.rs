// A private captured closed-worker fact, never a live worker capability.
#[derive(Clone)]
struct NativeCoordinatorFrame {
    kind: &'static str,
    semantic: JsonValue,
    rows: Vec<NativeCoordinatorFrozenRows>,
    review_refs: Option<Vec<String>>,
    review_meta: Option<(String, i64)>,
}
impl NativeCoordinatorFrame {
    fn closed_rows(
        tx: &rusqlite::Transaction<'_>,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Vec<NativeCoordinatorFrozenRows>, String> {
        use crate::agent_runtime::multi_agent::attempts;
        let worker = attempts::current(tx, actor, &child.assignment_id)?;
        let closed: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
            WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3 AND r.parent_run_id=?3
            AND r.assignment_id=a.id AND a.role=?4 AND r.role=a.role AND a.lane=r.lane
            AND a.target_key=?5 AND r.target_url=?5 AND r.scan_id=?6 AND r.attempt_number=?7
            AND a.lease_epoch=?8 AND a.fencing_token=?9 AND a.state='completed' AND a.budget_settled_at<>''
            AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.finished_at<>'' AND r.finished_at<>''
            AND r.status='terminal' AND r.terminal_state='completed')
            AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='')
            AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
            params![child.assignment_id,child.run_id,actor.root_run_id,child.role.as_str(),actor.target_key,
                actor.scan_id,actor.attempt_number,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !closed
            || worker.child_run_id != child.run_id
            || worker.finished_at.is_empty()
            || !matches!(worker.state.as_str(), "completed" | "failed" | "expired")
        {
            return Err("root_frame_worker_not_closed".into());
        }
        let capture = |sql: &str, key: &str| NativeCoordinatorFrozenRows::capture(tx, sql, &[key]);
        vec![
            capture(
                "SELECT rowid,* FROM agent_assignment_attempts WHERE id=?1",
                &worker.id,
            ),
            capture(
                "SELECT rowid,* FROM agent_assignments WHERE id=?1",
                &child.assignment_id,
            ),
            capture("SELECT rowid,* FROM agent_runs WHERE id=?1", &child.run_id),
            capture(
                "SELECT rowid,* FROM agent_specialist_calls WHERE child_run_id=?1",
                &child.run_id,
            ),
            NativeCoordinatorFrozenRows::capture_optional(
                tx,
                "SELECT rowid,* FROM agent_model_cost_facts WHERE child_run_id=?1 ORDER BY rowid",
                &[&child.run_id],
            ),
            capture(
                "SELECT rowid,* FROM agent_budget_entries WHERE lease_attempt_id=?1 ORDER BY rowid",
                &worker.id,
            ),
            // Client's dedicated receipt writer publishes no synthetic snapshot.
            // Its exact empty/nonempty snapshot state is still frozen; other
            // roles retain their existing required physical snapshot proof.
            if child.role == crate::agent_runtime::contract::AgentRole::ClientSide {
                NativeCoordinatorFrozenRows::capture_optional(tx,
                    "SELECT rowid,* FROM agent_snapshots WHERE run_id=?1 ORDER BY rowid", &[&child.run_id])
            } else {
                capture("SELECT rowid,* FROM agent_snapshots WHERE run_id=?1 ORDER BY rowid", &child.run_id)
            },
            capture(
                "SELECT rowid,* FROM agent_events WHERE run_id=?1 ORDER BY sequence",
                &child.run_id,
            ),
            capture(
                "SELECT rowid,* FROM agent_messages WHERE assignment_id=?1 ORDER BY rowid",
                &child.assignment_id,
            ),
        ]
        .into_iter()
        .collect()
    }
    fn mapper(
        tx: &rusqlite::Transaction<'_>,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{mailbox, specialist},
        };
        if child.role != AgentRole::SpaApiMapper {
            return Err("root_frame_role_invalid".into());
        }
        let receipt = specialist::received_for_reconciliation(tx, actor, child)?;
        if !receipt.rejection.is_empty() {
            return Err("root_frame_paid_output_rejected".into());
        }
        let id:String=tx.query_row("SELECT id FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2
            AND from_run_id=?3 AND to_run_id=?1 AND kind='evidence_summary' AND evidence_revision=1",
            params![actor.root_run_id,child.assignment_id,child.run_id],|r|r.get(0)).map_err(|_|"root_frame_output_missing")?;
        let message = mailbox::verified_completed_output(tx, actor, &id)?;
        if message.payload["summary"].as_str() != Some(&receipt.text) {
            return Err("root_frame_output_receipt_mismatch".into());
        }
        Ok(Self {
            kind: "mapper-output",
            semantic: json!({"messageId":id,"role":"spa_api_mapper",
            "output":native_coordinator_mapper_semantic(&receipt.text)?}),
            rows: Self::closed_rows(tx, actor, child)?,
            review_refs: None,
            review_meta: None,
        })
    }
    fn reviewer(
        tx: &rusqlite::Transaction<'_>,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        candidate: &str,
        revision: i64,
        missing: &JsonValue,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{mailbox, scheduler, specialist},
        };
        let (frozen, missing, refs) = sealed_gap_review_candidate_in_transaction(
            tx,
            &actor.root_run_id,
            candidate,
            revision,
            missing,
            &context.target_dir,
        )?;
        let (assignment,run,id):(String,String,String)=tx.query_row("SELECT q.assignment_id,q.reviewer_run_id,m.id
            FROM agent_review_requests q JOIN agent_messages m ON m.correlation_id=q.id
            AND m.assignment_id=q.assignment_id AND m.kind='review_decision' AND m.to_run_id=q.root_run_id
            WHERE q.root_run_id=?1 AND q.candidate_id=?2 AND q.candidate_revision=?3 AND q.status='insufficient_evidence'",
            params![actor.root_run_id,candidate,revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"root_frame_review_missing")?;
        let child = scheduler::ScheduledChild {
            assignment_id: assignment,
            run_id: run,
            role: AgentRole::EvidenceReviewer,
        };
        let receipt = specialist::received_for_reconciliation(tx, actor, &child)?;
        let decision = validated_review_decision(&receipt.text)?;
        if !receipt.rejection.is_empty()
            || decision.verdict != "insufficient_evidence"
            || decision.missing_evidence != missing
        {
            return Err("root_frame_review_receipt_mismatch".into());
        }
        let message = mailbox::verified_completed_output(tx, actor, &id)?;
        if message.payload["verdict"] != "insufficient_evidence"
            || message.payload["candidateId"] != candidate
            || message.payload["candidateRevision"] != revision
            || message.payload["summary"] != decision.summary
        {
            return Err("root_frame_review_envelope_mismatch".into());
        }
        let mut rows = Self::closed_rows(tx, actor, &child)?;
        rows.push(NativeCoordinatorFrozenRows::capture(tx,"SELECT rowid,* FROM agent_review_requests WHERE root_run_id=?1 AND candidate_id=?2 AND candidate_revision=CAST(?3 AS INTEGER)",&[&actor.root_run_id,candidate,&revision.to_string()])?);
        rows.push(NativeCoordinatorFrozenRows::capture(tx,"SELECT rowid,* FROM agent_review_decisions WHERE root_run_id=?1 AND candidate_id=?2 AND candidate_revision=CAST(?3 AS INTEGER)",&[&actor.root_run_id,candidate,&revision.to_string()])?);
        Ok(Self {
            kind: "reviewer-decision",
            semantic: json!({"messageId":id,"candidateId":candidate,"revision":revision,
            "candidateHash":crate::agent_runtime::store::stable_hash(&frozen.to_string()),"verdict":decision.verdict,
            "summary":decision.summary,"missingEvidence":missing,"trustedFactRefs":refs}),
            rows,
            review_refs: Some(refs),
            review_meta: Some((candidate.into(), revision)),
        })
    }
    fn fact(&self) -> JsonValue {
        json!({"kind":self.kind,"semantic":self.semantic,"physicalFacts":self.rows.iter().map(NativeCoordinatorFrozenRows::fact).collect::<Vec<_>>()})
    }
    fn verify(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        if let Some((candidate, revision)) = &self.review_meta {
            verify_review_snapshot(
                db,
                &actor.root_run_id,
                candidate,
                *revision,
                &context.target_dir,
            )?;
        }
        for row in &self.rows {
            row.verify(db)?;
        }
        if self.kind == "human-directive" {
            self.verify_human_source(db, actor)?;
        }
        if self.kind == "budget-allocation" {
            self.verify_budget_source(db, actor)?;
        }
        if self.kind == "identity-session-output" {
            self.verify_identity_source(db, actor)?;
        }
        if self.kind == "client-side-output" {
            self.verify_client_source(db, context, actor)?;
        }
        if let Some(refs) = &self.review_refs {
            if review_fact_refs(db, &actor.root_run_id, &context.target_dir)? != *refs {
                return Err("root_frame_review_facts_changed".into());
            }
        }
        Ok(())
    }
    fn step(&self) -> &'static str {
        match self.kind {
            "human-directive" => "assess:human_directive",
            "budget-allocation" => "assess:budget_allocation",
            "mapper-output" => "dispatch:web_executor",
            "investigator-proposal" => "assess:gap_proposal",
            "client-side-output" => "assess:client_side_configuration",
            "identity-session-output" => "assess:identity_session_metadata",
            _ => "dispatch:deep_investigator",
        }
    }
}
