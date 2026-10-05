// Call the real draft/confirm/review/claim/scheduler entry points. These tests
// prove a fresh confirmation contract, never completed ordered execution.
fn ordered_human_draft(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    text: &str,
) -> crate::agent_runtime::multi_agent::directive::HumanDirectiveDraft {
    crate::agent_runtime::multi_agent::directive::create_draft(
        db,
        &lease.scan_id,
        lease.attempt_number,
        &lease.root_run_id,
        &lease.target_key,
        "coordinator",
        text,
        lease.lease_epoch,
        &lease.fencing_token,
    )
    .unwrap()
}

#[test]
fn ordered_human_plan_actual_draft_freezes_written_order_and_each_cost() {
    let (root, path, _, lease) = multi_agent_test_root("ordered-fresh-cost", 20_000, 20);
    let db = db::open(&path).unwrap();
    for (text, wanted) in [
        (
            "@investigator 先评估原证据，再由 @mapper 评估建议",
            vec!["deep_investigator", "spa_api_mapper"],
        ),
        (
            "@映射 先看原证据，再由 @深度调查 评估建议",
            vec!["spa_api_mapper", "deep_investigator"],
        ),
        (
            "@总控 请让 @调查 先看，再由 @采集 检查",
            vec!["deep_investigator", "spa_api_mapper"],
        ),
    ] {
        let draft = ordered_human_draft(&db, &lease, text);
        assert_eq!(
            draft.estimated_tokens, 8000,
            "two frozen actions cannot estimate one call"
        );
        assert_eq!(draft.estimated_requests, 2);
        let plan = serde_json::to_value(&draft).unwrap()["readonlyAssessmentPlan"].clone();
        assert_eq!(plan["schemaVersion"], 3);
        assert_eq!(plan["dispatchState"], "requires_dispatch_checks");
        assert_eq!(plan["totalTokenCeiling"], 8000);
        assert_eq!(plan["totalModelRequests"], 2);
        assert_eq!(plan["targetRequests"], 0);
        let actions = plan["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        for (i, action) in actions.iter().enumerate() {
            assert_eq!(action["order"], i + 1);
            assert_eq!(action["role"], wanted[i]);
            assert_eq!(action["tokenCeiling"], 4000);
            assert_eq!(action["modelRequests"], 1);
            assert_eq!(action["maxOutputTokens"], 512);
            assert_eq!(action["targetRequests"], 0);
            assert_eq!(action["advisoryOnly"], true);
            assert_eq!(action["executionState"], "not_started");
            assert_eq!(
                action["previousActionId"],
                if i == 0 {
                    json!(null)
                } else {
                    actions[i - 1]["actionId"].clone()
                }
            );
        }
        assert_eq!(
            actions[1]["requiredPreviousState"],
            "valid_advisory_receipt"
        );
        let loaded =
            crate::agent_runtime::multi_agent::directive::list_open_drafts(&db, &lease.scan_id, 1)
                .unwrap();
        assert_eq!(loaded.iter().find(|d| d.id == draft.id), Some(&draft));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ordered_human_plan_confirm_persists_the_same_plan_without_bypassing_original_policy() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("ordered-confirm-contract", 20_000, 20);
    let db = db::open(&path).unwrap();
    let draft = ordered_human_draft(&db, &lease, "@investigator 然后 @mapper 评估已有证据");
    let plan = json!(draft)["readonlyAssessmentPlan"].clone();
    assert!(
        plan.is_object(),
        "actual fresh creation must return a typed plan"
    );
    let id = directive::confirm_draft(
        &db,
        &lease.scan_id,
        1,
        &lease.root_run_id,
        &lease.target_key,
        &draft.id,
        draft.revision,
        &draft.draft_hash,
    )
    .unwrap()
    .id;
    let raw: String = db
        .query_row(
            "SELECT payload_json FROM agent_user_directives WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let payload: JsonValue = serde_json::from_str(&raw).unwrap();
    assert_eq!(payload["readonlyAssessmentPlan"], plan);
    assert_eq!(
        payload["estimatedBudget"],
        json!({"tokens":8000,"requests":2})
    );
    let review = directive::human_review::receipt_for(&db, &draft.id)
        .unwrap()
        .unwrap();
    assert_eq!(review["executionCompleted"], false);
    for action in review["actions"].as_array().unwrap() {
        assert_eq!(action["capabilityState"], "existing_policy_required");
        assert_eq!(action["reasonCode"], "");
        assert_eq!(action["executionState"], "not_started");
        assert!(action["executionReceipt"].is_null());
    }
    directive::claim_pending_directives(&db, &lease, 20).unwrap();
    assert!(
        directive::proposals::prepare_next(&db, &lease, &json!({"observed":[]}))
            .unwrap()
            .is_none()
    );
    let state:(String,String,i64)=db.query_row("SELECT status,rejection_code,(SELECT COUNT(*) FROM agent_assignments WHERE trigger_code LIKE 'human_directive:%') FROM agent_user_directives WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        state,
        (
            "accepted".into(),
            "".into(),
            0
        )
    );
    assert_eq!(
        directive::human_review::receipt_for(&db, &draft.id)
            .unwrap()
            .unwrap(),
        review,
        "planning receipt is immutable"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ordered_human_plan_revision_requires_new_hash_and_confirmation() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("ordered-revision", 20_000, 20);
    let db = db::open(&path).unwrap();
    let original = ordered_human_draft(&db, &lease, "@investigator 然后 @mapper 评估已有证据");
    let result = directive::human_review::review(
        &db,
        &lease.scan_id,
        &original.id,
        original.revision,
        &original.draft_hash,
        "revise",
        "@mapper 然后 @investigator 评估已有证据",
    )
    .unwrap();
    assert_eq!(result["receipt"]["kind"], "revise");
    assert_eq!(result["receipt"]["executionCompleted"], false);
    assert_eq!(
        result["draft"]["readonlyAssessmentPlan"]["actions"][0]["role"],
        "spa_api_mapper"
    );
    assert_ne!(result["draft"]["draftHash"], original.draft_hash);
    assert_ne!(
        result["draft"]["readonlyAssessmentPlan"]["planHash"],
        json!(original)["readonlyAssessmentPlan"]["planHash"]
    );
    assert_eq!(result["draft"]["confirmationRequired"], true);
    assert_eq!(result["draft"]["confirmedDirectiveId"], "");
    let queued: i64 = db
        .query_row("SELECT COUNT(*) FROM agent_user_directives", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(queued, 0);
    assert!(directive::confirm_draft(
        &db,
        &lease.scan_id,
        1,
        &lease.root_run_id,
        &lease.target_key,
        &original.id,
        original.revision,
        &original.draft_hash
    )
    .is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ordered_human_plan_cannot_promote_unsupported_or_nonreadonly_requests() {
    let (root, path, _, lease) = multi_agent_test_root("ordered-failclosed", 20_000, 20);
    let db = db::open(&path).unwrap();
    for text in [
        "@mapper @reviewer 请看证据",
        "@mapper @source_analyst 请看源码",
        "@mapper @investigator 扩大范围到 https://outside.invalid",
        "@mapper @investigator 增加预算",
        "@mapper @investigator 暂停后续工作",
        "@mapper @investigator 修改远程配置并提供回滚",
        "@mapper @investigator ssh root@host",
        "@mapper @investigator api_key=secret0123456789abcdef",
    ] {
        let draft = ordered_human_draft(&db, &lease, text);
        assert!(
            json!(draft).get("readonlyAssessmentPlan").is_none(),
            "{text}"
        );
        assert!(
            !draft
                .reason_codes
                .iter()
                .any(|r| matches!(r.as_str(), "ordered_readonly_assessment_v2" | "ordered_readonly_assessment_v3")),
            "{text}"
        );
    }
    let one = ordered_human_draft(&db, &lease, "@mapper 请看已有证据");
    assert!(json!(one).get("readonlyAssessmentPlan").is_none());
    assert_eq!((one.estimated_tokens, one.estimated_requests), (4000, 1));
    let _ = fs::remove_dir_all(root);
}
