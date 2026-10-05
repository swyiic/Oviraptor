#[test]
fn human_directive_review_multirole_actions_stay_ordered_and_real_scheduler_defers() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db, record, lease) = source_specialist_fixture();
    let value = draft_scan_directive_in(
        &db,
        &record.scan_id,
        "@mapper @investigator 请分析现有证据缺口",
        "team",
    )
    .unwrap();
    assert!(value.get("readonlyAssessmentPlan").is_none(),
        "Source must retain its original blocked Web-role meaning");
    let draft = directive::list_open_drafts(&db, &record.scan_id, 1)
        .unwrap()
        .into_iter()
        .find(|d| d.id == value["id"])
        .unwrap();
    let native = human_review_native_fingerprint(&db);
    let confirmed = directive::confirm_bound_draft(
        &db,
        &record.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash,
    )
    .unwrap();
    let receipt = directive::human_review::receipt_for(&db, &draft.id)
        .unwrap()
        .expect("missing per-role human review actions");
    assert_eq!(receipt["actions"].as_array().unwrap().len(), 2);
    for (index, role) in ["spa_api_mapper", "deep_investigator"].iter().enumerate() {
        assert_eq!(receipt["actions"][index]["order"], index + 1);
        assert_eq!(receipt["actions"][index]["role"], *role);
        assert_eq!(receipt["actions"][index]["capabilityState"], "blocked");
        assert_eq!(
            receipt["actions"][index]["reasonCode"],
            "proposal_role_decomposition_required"
        );
        assert_eq!(
            receipt["actions"][index]["executionReceipt"],
            JsonValue::Null
        );
    }
    assert_eq!(receipt["executionCompleted"], false);
    let claimed = directive::claim_pending_directives(&db, &lease, 50).unwrap();
    assert_eq!(
        claimed
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        [&confirmed.id]
    );
    assert!(
        directive::proposals::prepare_next(&db, &lease, &json!({"observedEvidence":[]}))
            .unwrap()
            .is_none()
    );
    let state: (String, String) = db
        .query_row(
            "SELECT status,rejection_code FROM agent_user_directives WHERE id=?1",
            [&confirmed.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            "deferred".into(),
            "proposal_role_decomposition_required".into()
        )
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM agent_directive_proposals WHERE directive_id=?1",
            [&confirmed.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        human_review_native_fingerprint(&db),
        native,
        "defer must not dispatch roles/change Source JSON or budgets"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
