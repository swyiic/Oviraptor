// Explicit prerequisite: actual g2 RootTickFixture + human owned SDK transport.
// Tests use temporary SQLite and localhost providers only, never role doubles.
fn ordered_exec_fixture(tag: &str, endpoint: &str) -> (RootTickFixture, String) {
    let f = root_tick_fixture(tag, endpoint);
    let db = db::open(&f.context.db_path).unwrap();
    let id = confirm_queue_directive(
        &db,
        &f.actor,
        "@investigator 先评估原证据，再由 @mapper 评估建议",
    );
    let raw: String = db
        .query_row(
            "SELECT payload_json FROM agent_user_directives WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<JsonValue>(&raw).unwrap()["readonlyAssessmentPlan"]["schemaVersion"],
        3
    );
    (f, id)
}
fn ordered_exec_count(db: &rusqlite::Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn ordered_exec_receipts(db: &rusqlite::Connection, id: &str) -> Vec<JsonValue> {
    let mut q = db.prepare("SELECT receipt_json FROM agent_directive_ordered_receipts WHERE directive_id=?1 ORDER BY action_order").unwrap();
    q.query_map([id], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|v| serde_json::from_str(&v.unwrap()).unwrap())
        .collect()
}
fn ordered_exec_native(db: &rusqlite::Connection, root: &str) -> String {
    db.query_row(
        "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
        [root],
        |r| r.get(0),
    )
    .unwrap()
}
fn ordered_exec_projection(db: &rusqlite::Connection, id: &str) -> JsonValue {
    crate::agent_runtime::multi_agent::directive::ordered_execution::project(db, id)
        .unwrap()
        .unwrap()
}
fn ordered_exec_apply(f: &RootTickFixture) -> Result<Vec<JsonValue>, String> {
    let mut inbox = take_human_directives(&f.context)?;
    apply_human_proposal_actions(&f.context, &mut inbox)
}
fn ordered_exec_sdk(f: &RootTickFixture, child: &str) -> Vec<JsonValue> {
    let page = crate::agent_runtime::model::diagnostics::replay::read(
        &f.context.db_path,
        &f.actor.scan_id,
        1,
        None,
        None,
        0,
        300,
    )
    .unwrap();
    serde_json::to_value(page).unwrap()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["runId"] == child)
        .cloned()
        .collect()
}
// Historical v2 import uses the original serialized/hash recipe, not a live
// production upgrader. Confirming this row must never create v3 permission.
fn ordered_exec_seed_v2(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> String {
    let draft = ordered_human_draft(db, lease, "@investigator 然后 @mapper 评估已有证据");
    let value = json!(draft);
    let mut material = json!({});
    for key in [
        "sourceMessageId",
        "scanId",
        "attemptNumber",
        "rootRunId",
        "targetKey",
        "recipientRole",
        "threadKey",
        "text",
        "intent",
        "requestedRoles",
        "referencedFactIds",
        "requestedContracts",
        "priorityChanges",
        "proposedScopeChange",
        "estimatedTokens",
        "estimatedRequests",
        "sideEffectClass",
        "requiredApprovals",
        "validationResult",
        "coordinatorDecision",
        "safeExecutionText",
        "revision",
    ] {
        material[key] = value[key].clone();
    }
    let mut reasons: Vec<String> = draft
        .reason_codes
        .iter()
        .filter(|r| {
            r.as_str() != "ordered_readonly_assessment_v3"
                && r.as_str() != "ordered_original_dispatch_checks_required"
        })
        .cloned()
        .collect();
    reasons.extend([
        "ordered_readonly_assessment_v2".into(),
        "proposal_ordered_execution_not_connected".into(),
    ]);
    material["reasonCodes"] = json!(reasons);
    material["boundLeaseEpoch"] = json!(lease.lease_epoch);
    material["boundFencingToken"] = json!(lease.fencing_token);
    let mut binding = json!({});
    for key in [
        "sourceMessageId",
        "scanId",
        "rootRunId",
        "targetKey",
        "recipientRole",
        "threadKey",
        "boundFencingToken",
        "attemptNumber",
        "revision",
        "boundLeaseEpoch",
    ] {
        binding[key] = material[key].clone();
    }
    let bh = crate::agent_runtime::store::stable_hash(&binding.to_string());
    let mut actions = vec![];
    for (index, role) in draft
        .requested_roles
        .iter()
        .filter(|r| r.as_str() != "coordinator")
        .enumerate()
    {
        let id = crate::agent_runtime::store::stable_hash(
            &json!(["ordered_readonly_assessment_v2", bh, index + 1, role]).to_string(),
        );
        let previous = actions
            .last()
            .map(|v: &JsonValue| v["actionId"].clone())
            .unwrap_or(json!(null));
        actions.push(json!({"actionId":id,"order":index+1,"role":role,"tokenCeiling":4000,"modelRequests":1,"maxOutputTokens":512,
        "targetRequests":0,"previousActionId":previous,"requiredPreviousState":if index==0 {"frozen_original_evidence"}else{"valid_advisory_receipt"},"advisoryOnly":true,"executionState":"not_started"}));
    }
    let mut plan = json!({"schemaVersion":2,"purpose":"human_readonly_assessment","bindingHash":bh,"dispatchState":"not_connected",
      "totalTokenCeiling":8000,"totalModelRequests":2,"targetRequests":0,"actions":actions});
    plan["planHash"] = json!(crate::agent_runtime::store::stable_hash(&plan.to_string()));
    material["readonlyAssessmentPlan"] = plan;
    let hash = crate::agent_runtime::store::stable_hash(&material.to_string());
    db.execute(
        "UPDATE agent_directive_drafts SET reason_codes_json=?1,draft_hash=?2 WHERE id=?3",
        params![json!(reasons).to_string(), hash, draft.id],
    )
    .unwrap();
    crate::agent_runtime::multi_agent::directive::confirm_draft(
        db,
        &lease.scan_id,
        1,
        &lease.root_run_id,
        &lease.target_key,
        &draft.id,
        draft.revision,
        &hash,
    )
    .unwrap()
    .id
}
