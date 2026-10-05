// Immutable original confirmation and consumer; status transitions are not new facts.
impl NativeCoordinatorFrame {
    fn human_directive(
        db: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        consumer: &str,
        id: &str,
    ) -> Result<Self, String> {
        let mut semantic =
            crate::agent_runtime::multi_agent::directive::confirmed_fact_for_root(db, actor, id)?;
        let draft = semantic["draftId"]
            .as_str()
            .ok_or("root_human_directive_invalid")?
            .to_owned();
        let run = crate::agent_runtime::store::load_run(db, consumer)?
            .ok_or("root_human_consumer_missing")?;
        semantic["consumerRunId"] = consumer.into();
        semantic["consumerAssignmentId"] = run.assignment_id.into();
        if consumer != actor.root_run_id {
            let worker = crate::agent_runtime::multi_agent::attempts::current(
                db,
                actor,
                semantic["consumerAssignmentId"]
                    .as_str()
                    .ok_or("root_human_consumer_missing")?,
            )?;
            semantic["consumerWorkerId"] = worker.id.into();
        }
        let rows=vec![
            NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM agent_directive_drafts WHERE id=?1",&[&draft])?,
            NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM agent_directive_human_reviews WHERE draft_id=?1",&[&draft])?,
            NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,id,scan_id,attempt_number,root_run_id,target_key,recipient_role,thread_key,text_redacted,source_draft_id,confirmed_revision,confirmed_hash,confirmation_at,claim_run_id,claim_lease_epoch,claim_fencing_token,created_at,claimed_at,accepted_at FROM agent_user_directives WHERE id=?1",&[id])?,
        ];
        let frame = Self {
            kind: "human-directive",
            semantic,
            rows,
            review_refs: None,
            review_meta: None,
        };
        frame.verify_human_source(db, actor)?;
        Ok(frame)
    }
    fn verify_human_source(
        &self,
        db: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{attempts, scheduler},
            store,
        };
        let consumer = self.semantic["consumerRunId"]
            .as_str()
            .ok_or("root_human_consumer_missing")?;
        let run = store::load_run(db, consumer)?.ok_or("root_human_consumer_missing")?;
        if consumer != actor.root_run_id {
            let child = scheduler::ScheduledChild {
                assignment_id: run.assignment_id.clone(),
                run_id: run.id,
                role: run.role,
            };
            scheduler::verify_original_scheduled_child(db, actor, &child)?;
            let worker = attempts::current(db, actor, &child.assignment_id)?;
            if self.semantic["consumerWorkerId"] != worker.id
                || self.semantic["consumerAssignmentId"] != child.assignment_id
            {
                return Err("root_human_original_consumer_changed".into());
            }
        } else if run.role != AgentRole::Coordinator || run.root_run_id != actor.root_run_id {
            return Err("root_human_original_consumer_changed".into());
        }
        let id = self.semantic["directiveId"]
            .as_str()
            .ok_or("root_human_directive_invalid")?;
        let mut actual =
            crate::agent_runtime::multi_agent::directive::confirmed_fact_for_root(db, actor, id)?;
        actual["consumerRunId"] = self.semantic["consumerRunId"].clone();
        actual["consumerAssignmentId"] = self.semantic["consumerAssignmentId"].clone();
        if let Some(worker) = self.semantic.get("consumerWorkerId") {
            actual["consumerWorkerId"] = worker.clone();
        }
        if actual != self.semantic {
            return Err("root_human_original_confirmation_changed".into());
        }
        Ok(())
    }
}
