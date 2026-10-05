// A seeded historical Native row uses the pre-v2 hash recipe exactly. This is
// test fixture import, never a production upgrade or re-confirmation shortcut.
fn ordered_seed_legacy_snapshot(
    db: &rusqlite::Connection,
    draft: &crate::agent_runtime::multi_agent::directive::HumanDirectiveDraft,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> String {
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
        "referencedFactIds",
        "requestedContracts",
        "priorityChanges",
        "proposedScopeChange",
        "sideEffectClass",
        "requiredApprovals",
        "validationResult",
        "coordinatorDecision",
        "safeExecutionText",
        "revision",
    ] {
        material[key] = value[key].clone();
    }
    material["requestedRoles"] = json!(["spa_api_mapper", "deep_investigator"]);
    material["reasonCodes"] = json!(["read_only_change_within_frozen_plan"]);
    material["estimatedTokens"] = json!(4000);
    material["estimatedRequests"] = json!(1);
    material["boundLeaseEpoch"] = json!(lease.lease_epoch);
    material["boundFencingToken"] = json!(lease.fencing_token);
    let hash = crate::agent_runtime::store::stable_hash(&material.to_string());
    db.execute("UPDATE agent_directive_drafts SET requested_roles_json=?1,reason_codes_json=?2,estimated_tokens=4000,estimated_requests=1,draft_hash=?3 WHERE id=?4",
        rusqlite::params![material["requestedRoles"].to_string(),material["reasonCodes"].to_string(),hash,draft.id]).unwrap();
    hash
}

#[test]
fn ordered_human_plan_original_legacy_two_roles_keep_old_hash_and_blocked_receipt() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("ordered-legacy-hash", 20_000, 20);
    let db = db::open(&path).unwrap();
    let original = ordered_human_draft(&db, &lease, "@investigator 然后 @mapper 评估已有证据");
    let hash = ordered_seed_legacy_snapshot(&db, &original, &lease);
    let old = directive::list_open_drafts(&db, &lease.scan_id, 1)
        .unwrap()
        .into_iter()
        .find(|d| d.id == original.id)
        .unwrap();
    assert_eq!(old.draft_hash, hash);
    assert!(json!(old).get("readonlyAssessmentPlan").is_none());
    let id = directive::confirm_draft(
        &db,
        &lease.scan_id,
        1,
        &lease.root_run_id,
        &lease.target_key,
        &old.id,
        old.revision,
        &hash,
    )
    .unwrap()
    .id;
    let raw_before: String = db
        .query_row(
            "SELECT receipt_json FROM agent_directive_human_reviews WHERE draft_id=?1",
            [&old.id],
            |r| r.get(0),
        )
        .unwrap();
    let review = directive::human_review::receipt_for(&db, &old.id)
        .unwrap()
        .unwrap();
    for action in review["actions"].as_array().unwrap() {
        assert_eq!(action["reasonCode"], "proposal_role_decomposition_required");
        assert_eq!(action["capabilityState"], "blocked");
    }
    directive::claim_pending_directives(&db, &lease, 20).unwrap();
    assert!(directive::proposals::prepare_next(&db, &lease, &json!({}))
        .unwrap()
        .is_none());
    let hash_after: String = db
        .query_row(
            "SELECT draft_hash FROM agent_directive_drafts WHERE id=?1",
            [&old.id],
            |r| r.get(0),
        )
        .unwrap();
    let raw_after: String = db
        .query_row(
            "SELECT receipt_json FROM agent_directive_human_reviews WHERE draft_id=?1",
            [&old.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hash_after, hash);
    assert_eq!(raw_before, raw_after);
    let reason: String = db
        .query_row(
            "SELECT rejection_code FROM agent_user_directives WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reason, "proposal_role_decomposition_required");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ordered_human_plan_native_field_tamper_and_trigger_changes_reject_confirmation() {
    use crate::agent_runtime::multi_agent::directive;
    for change in [
        "estimated_tokens=4000",
        "estimated_requests=1",
        "requested_roles_json='[\"spa_api_mapper\",\"deep_investigator\"]'",
        "reason_codes_json='[\"read_only_change_within_frozen_plan\"]'",
        "text_redacted='@mapper 然后 @investigator 评估已有证据'",
    ] {
        let (root, path, _, lease) = multi_agent_test_root("ordered-cost-tamper", 20_000, 20);
        let db = db::open(&path).unwrap();
        let draft = ordered_human_draft(&db, &lease, "@investigator 然后 @mapper 评估已有证据");
        db.execute(
            &format!("UPDATE agent_directive_drafts SET {change} WHERE id=?1"),
            [&draft.id],
        )
        .unwrap();
        let before = receipt_database_snapshot(&db);
        assert!(
            directive::confirm_draft(
                &db,
                &lease.scan_id,
                1,
                &lease.root_run_id,
                &lease.target_key,
                &draft.id,
                draft.revision,
                &draft.draft_hash
            )
            .is_err(),
            "{change}"
        );
        assert_eq!(receipt_database_snapshot(&db), before, "{change}");
        let _ = fs::remove_dir_all(root);
    }
    let (root, path, _, lease) = multi_agent_test_root("ordered-confirm-trigger", 20_000, 20);
    let db = db::open(&path).unwrap();
    let draft = ordered_human_draft(&db, &lease, "@investigator 然后 @mapper 评估已有证据");
    db.execute_batch("CREATE TRIGGER damage_ordered_cost AFTER INSERT ON agent_directive_human_reviews BEGIN UPDATE agent_directive_drafts SET estimated_tokens=4000; END;").unwrap();
    let before = receipt_database_snapshot(&db);
    assert!(directive::confirm_draft(
        &db,
        &lease.scan_id,
        1,
        &lease.root_run_id,
        &lease.target_key,
        &draft.id,
        draft.revision,
        &draft.draft_hash
    )
    .is_err());
    assert_eq!(receipt_database_snapshot(&db), before);
    let _ = fs::remove_dir_all(root);
}
