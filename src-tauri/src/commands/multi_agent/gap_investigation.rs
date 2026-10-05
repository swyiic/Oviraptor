/// One bounded read-only proposal for a Reviewer insufficiency. A proposal is
/// never a target request or an authorization grant: the Coordinator records a
/// deferred assessment until a new verified contract/evidence revision exists.
fn multi_agent_investigate_review_gap(
    context: &AgentRunContext,
    session: &MultiAgentSession,
    candidate_id: &str,
    revision: i64,
    missing_evidence: &JsonValue,
) -> Result<(), String> {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let connection = db::open(&context.db_path)?;
    // Replay must see the sealed review and both mailbox records in one snapshot.
    let snapshot = connection
        .unchecked_transaction()
        .map_err(|error| format!("gap_replay_lock:{error}"))?;
    let (frozen_candidate, frozen_missing, trusted_fact_refs) =
        sealed_gap_review_candidate_in_transaction(
            &snapshot,
            &session.lease.root_run_id,
            candidate_id,
            revision,
            missing_evidence,
            &context.target_dir,
        )?;
    let input = serde_json::json!({
        "candidateId":candidate_id, "evidenceRevision":revision,
        "reviewerMissingEvidence":frozen_missing, "trustedFactRefs":trusted_fact_refs,
        "frozenCandidate":frozen_candidate,
    });
    let prior: Option<(String, String, String, String)> = snapshot
        .query_row(
            "SELECT a.id,a.child_run_id,a.state,a.task_slice_json \
             FROM agent_assignments a WHERE a.coordinator_run_id=?1 \
             AND a.role='deep_investigator' AND a.trigger_code='reviewer_insufficient_evidence' \
             AND a.evidence_revision=?2",
            params![session.lease.root_run_id, revision],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| format!("无法检查 EvidenceGap 协商轮次：{error}"))?;
    if let Some((assignment_id, child_run_id, state, task_slice)) = prior {
        let task: JsonValue = serde_json::from_str(&task_slice)
            .map_err(|_| "gap_existing_assignment_invalid".to_string())?;
        if task["candidateId"] != candidate_id || task["revision"] != revision {
            return Err("gap_revision_candidate_conflict".into());
        }
        // Older saved assignments did not persist missingEvidence in the task
        // slice. Their acknowledged proposal still has to match it exactly.
        let same_gap = task.get("missingEvidence").is_none_or(|saved| {
            *saved == crate::agent_runtime::secrets::redact_json(missing_evidence)
        });
        if matches!(state.as_str(), "failed" | "paused") && same_gap {
            let child = scheduler::ScheduledChild {
                assignment_id,
                run_id: child_run_id,
                role: AgentRole::DeepInvestigator,
            };
            drop(snapshot);
            return complete_gap_delivery(
                &connection,
                context,
                session,
                &child,
                candidate_id,
                revision,
                missing_evidence,
                &input,
                "",
                true,
            )
            .map_err(|error| format!("gap_assessment_incomplete_requires_recovery:{error}"))
            .and_then(|()| {
                native_coordinator_gap_feedback(
                    context,
                    session,
                    candidate_id,
                    revision,
                    missing_evidence,
                )
            });
        }
        return if state == "completed"
            && same_gap
            && completed_gap_round_valid(
                &snapshot,
                &session.lease.root_run_id,
                &assignment_id,
                &child_run_id,
                candidate_id,
                revision,
                missing_evidence,
                &context.target_dir,
            )? {
            drop(snapshot);
            native_coordinator_gap_feedback(
                context,
                session,
                candidate_id,
                revision,
                missing_evidence,
            )
        } else {
            // A failed or interrupted round is not success. In particular, do
            // not spend a second model call in the same evidence revision.
            Err("gap_assessment_incomplete_requires_recovery".into())
        };
    }
    drop(snapshot);
    let root_context = native_coordinator_root_context(context, &session.lease);
    let decision_frame = if context.target_url.starts_with("source:") {
        None
    } else {
        let tx = rusqlite::Transaction::new_unchecked(
            &connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| e.to_string())?;
        native_coordinator_tick_authority(&tx, &root_context, &session.lease)?;
        let frame = NativeCoordinatorFrame::reviewer(
            &tx,
            context,
            &session.lease,
            candidate_id,
            revision,
            missing_evidence,
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        let decision = native_coordinator_tick_for_frame(&root_context, &session.lease, &frame)?;
        Some((frame, decision))
    };
    let task = serde_json::json!({"candidateId":candidate_id,"revision":revision,
        "missingEvidence":crate::agent_runtime::secrets::redact_json(missing_evidence)});
    if let Some((frame, decision)) = &decision_frame {
        if !decision.summary.as_json()["suggestions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(frame.step())))
        {
            return Ok(()); // paid visible deferral, no role grant or execution claim
        }
    }
    let child = if let Some((frame, decision)) = &decision_frame {
        native_coordinator_schedule_decision(
            &root_context,
            &session.lease,
            frame,
            decision,
            AgentRole::DeepInvestigator,
            AgentLane::ReadOnlyAnalysis,
            "reviewer_insufficient_evidence",
            &task,
            revision,
            &["evidence.read".into(), "mailbox.write".into()],
            4_000,
            1,
        )?
    } else {
        scheduler::schedule_child(
            &connection,
            &session.lease,
            AgentRole::DeepInvestigator,
            AgentLane::ReadOnlyAnalysis,
            "reviewer_insufficient_evidence",
            &task,
            revision,
            &["evidence.read".into(), "mailbox.write".into()],
            4_000,
            1,
        )?
    };
    scheduler::start_child_or_release(&connection, &session.lease, &child)?;
    let round = multi_agent_child_round(
        context,
        &session.lease,
        &child,
        "你是独立的只读 Deep Investigator。只分析 Reviewer 的证据缺口，不访问目标、不指挥其他 Agent、不声称新事实。输出严格 JSON，字段：summary、nextStep(observe_existing_evidence|request_new_contract|manual_review)、gapCode(小写下划线)、supportingFactRefs(只能引用输入中的 trustedFactRefs)、missingEvidence(必须逐项原样复制 reviewerMissingEvidence)、prerequisites、proposedContracts、expectedInformationGain(0..1)、impactCeiling(low|medium|high|critical)、estimatedCost{modelTokens,modelRequests,targetRequests}、sideEffectClass(read_only)、overlapKeys、falsificationCondition、stopCondition。manual_review 不得提出合同或目标请求；observe_existing_evidence 仅可提出 existing_evidence_review 且目标请求数为 0；request_new_contract 只能提出 new_attempt_control_group_request，表示未来新 attempt 的人工申请，不是当前已获批准的合同，预计三侧请求数为 3。成本上限 4000 tokens/1 模型请求/3 预计目标请求；本轮不会签发合同或目标请求。",
        input.clone(),
    );
    let (text, usage) = match round {
        Ok(value) => value,
        Err(error) => {
            return Err(failed_specialist_error(
                &connection,
                &session.lease,
                &child,
                &error,
            ));
        }
    };
    let delivery = settle_child_usage(&connection, &session.lease, &child, &usage).and_then(|()| {
        complete_gap_delivery(
            &connection,
            context,
            session,
            &child,
            candidate_id,
            revision,
            missing_evidence,
            &input,
            &text,
            false,
        )
    });
    if let Err(error) = delivery {
        if let Err(cleanup) =
            stop_failed_child_preserving_usage(&connection, &session.lease, &child, &error)
        {
            return Err(format!("{error};specialist_cleanup:{cleanup}"));
        }
        return complete_gap_delivery(
            &connection,
            context,
            session,
            &child,
            candidate_id,
            revision,
            missing_evidence,
            &input,
            "",
            true,
        )
        .map_err(|recovery| format!("{error};gap_local_recovery:{recovery}"))
        .and_then(|()| {
            native_coordinator_gap_feedback(
                context,
                session,
                candidate_id,
                revision,
                missing_evidence,
            )
        });
    }
    native_coordinator_gap_feedback(context, session, candidate_id, revision, missing_evidence)
}
