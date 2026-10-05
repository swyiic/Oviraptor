// One immutable original allocation fact; lease renewal and its own fees are not new facts.
impl NativeCoordinatorFrame {
    fn budget_allocation(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
        origin: NativeCoordinatorFrame,
        paid: &NativeCoordinatorTickReceipt,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{attempts, budget, scheduler},
            store,
        };
        scheduler::verify_original_scheduled_child(db, actor, child)?;
        let (tokens, requests) =
            budget::model::original_web_grant(db, actor, &child.assignment_id)?;
        if paid.tick.published(db, &paid.saved)? != Some(paid.event_sequence) {
            return Err("root_budget_original_dispatch_unpublished".into());
        }
        paid.tick.require_executable(db)?;
        origin.verify(db, context, actor)?;
        let raw: String = db
            .query_row(
                "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
                [&child.assignment_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let task: JsonValue =
            serde_json::from_str(&raw).map_err(|_| "root_budget_original_task_invalid")?;
        let mapper = task["mapperAssignmentId"]
            .as_str()
            .ok_or("root_budget_original_mapper_missing")?;
        let mapper_role: String = db
            .query_row(
                "SELECT role FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2",
                params![mapper, actor.root_run_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if mapper_role != "spa_api_mapper" {
            return Err("root_budget_original_mapper_conflict".into());
        }
        let policy = native_coordinator_dispatch_policy_on(
            db,
            context,
            actor,
            &origin,
            paid,
            AgentRole::WebExecutor,
            tokens,
            requests,
            true,
        )?;
        let expected = json!({"target":actor.target_key,
            "objective":"在授权范围内执行冻结计划中的验证合同，所有目标访问都必须经过工具 Broker",
            "mapperAssignmentId":mapper,
            "rootDecision":{"eventSequence":paid.event_sequence,"summaryHash":store::stable_hash(&paid.summary.as_json().to_string()),
                "frameHash":store::stable_hash(&origin.fact().to_string()),"step":origin.step(),"rustPolicy":policy}});
        let canonical_task = task.to_string();
        if task != expected || raw != canonical_task {
            return Err("root_budget_original_task_conflict".into());
        }
        let worker = attempts::current(db, actor, &child.assignment_id)?;
        let mut rows = origin.rows;
        let capture =
            |sql: &str, keys: &[&str]| NativeCoordinatorFrozenRows::capture(db, sql, keys);
        rows.push(capture("SELECT rowid,id,coordinator_run_id,child_run_id,role,lane,target_key,state,dedup_key,trigger_code,task_slice_json,evidence_revision,contract_keys_json,identity_handles_json,reserved_tokens,reserved_requests,budget_settled_at,capability_lease_json,lease_epoch,fencing_token,finished_at FROM agent_assignments WHERE id=?1", &[&child.assignment_id])?);
        rows.push(capture("SELECT rowid,id,scan_id,attempt_number,target_url,backend,role,parent_run_id,root_run_id,assignment_id,lane,orchestration_policy,status,capability_lease_json,reserved_tokens,reserved_requests,cancel_requested_at FROM agent_runs WHERE id=?1", &[&child.run_id])?);
        rows.push(capture("SELECT rowid,id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,state,leased_at,finished_at,failure_class FROM agent_assignment_attempts WHERE id=?1", &[&worker.id])?);
        rows.push(capture("SELECT rowid,id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,revoked_at FROM agent_capability_leases WHERE assignment_id=?1 ORDER BY id", &[&child.assignment_id])?);
        rows.push(capture(
            "SELECT rowid,* FROM agent_lane_leases WHERE assignment_id=?1",
            &[&child.assignment_id],
        )?);
        rows.push(NativeCoordinatorFrozenRows::capture_optional(db,"SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 AND lease_attempt_id=?2 AND kind='reserve' AND source_id=?3 ORDER BY rowid", &[&child.assignment_id,&worker.id,&format!("assignment:{}",child.assignment_id)])?);
        let seq = paid.event_sequence.to_string();
        rows.push(capture(
            "SELECT rowid,* FROM agent_events WHERE run_id=?1 AND sequence=CAST(?2 AS INTEGER)",
            &[&actor.root_run_id, &seq],
        )?);
        rows.push(capture("SELECT rowid,* FROM agent_root_tick_receipts WHERE root_run_id=?1 AND round=(SELECT json_extract(payload_json,'$.turns') FROM agent_events WHERE run_id=?1 AND sequence=CAST(?2 AS INTEGER)) ORDER BY rowid", &[&actor.root_run_id,&seq])?);
        rows.push(capture("SELECT rowid,* FROM agent_root_tick_timeline_receipts WHERE root_run_id=?1 AND model_event_sequence=CAST(?2 AS INTEGER)", &[&actor.root_run_id,&seq])?);
        let frame = Self {
            kind: "budget-allocation",
            semantic: json!({"assignmentId":child.assignment_id,"runId":child.run_id,
            "workerAttemptId":worker.id,"originalTokens":tokens,"originalRequests":requests,
            "originalTaskHash":store::stable_hash(&raw),"originalDispatchSequence":paid.event_sequence,
            "trigger":"capability_budget_state_changed","advisoryOnly":true,"additionalScopeGranted":false}),
            rows,
            review_refs: None,
            review_meta: None,
        };
        frame.verify_budget_source(db, actor)?;
        Ok(frame)
    }
    fn verify_budget_source(
        &self,
        db: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{budget, scheduler},
        };
        let child = scheduler::ScheduledChild {
            assignment_id: self.semantic["assignmentId"]
                .as_str()
                .ok_or("root_budget_frame_invalid")?
                .into(),
            run_id: self.semantic["runId"]
                .as_str()
                .ok_or("root_budget_frame_invalid")?
                .into(),
            role: AgentRole::WebExecutor,
        };
        scheduler::verify_original_scheduled_child(db, actor, &child)?;
        let (tokens, requests) =
            budget::model::original_web_grant(db, actor, &child.assignment_id)?;
        let projection: (i64, i64) = db
            .query_row(
                "SELECT reserved_tokens,reserved_requests FROM agent_runs WHERE id=?1",
                [&child.run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if self.semantic["originalTokens"] != tokens
            || self.semantic["originalRequests"] != requests
            || projection != (tokens, requests)
        {
            return Err("root_budget_original_grant_changed".into());
        }
        Ok(())
    }
}
