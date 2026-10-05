// Real IPC business helpers + production confirm store; no SFC/mocked receipts.
fn human_review_fixture() -> (
    PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::directive::HumanDirectiveDraft,
) {
    let (root, connection, record, _lease) = source_specialist_fixture();
    let value = draft_scan_directive_in(
        &connection,
        &record.scan_id,
        "@source_analyst 请关注权限检查",
        "team",
    )
    .unwrap();
    let draft = crate::agent_runtime::multi_agent::directive::list_open_drafts(
        &connection,
        &record.scan_id,
        1,
    )
    .unwrap()
    .into_iter()
    .find(|d| d.id == value["id"])
    .unwrap();
    (root, connection, draft)
}

fn human_review_native_fingerprint(db: &rusqlite::Connection) -> Vec<(String, Vec<String>)> {
    application_table_snapshot(db)
        .into_iter()
        .filter(|(table, _)| {
            !matches!(
                table.as_str(),
                "agent_directive_drafts"
                    | "agent_user_directives"
                    | "agent_directive_human_reviews"
                    | "agent_collaboration_events"
            )
        })
        .collect()
}

#[test]
fn human_directive_review_approve_writes_ordered_receipt_with_real_queue_atomically() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db, draft) = human_review_fixture();
    let native = human_review_native_fingerprint(&db);
    let confirmed = directive::confirm_bound_draft(
        &db,
        &draft.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash,
    )
    .unwrap();
    let receipt = directive::human_review::receipt_for(&db, &draft.id)
        .unwrap()
        .expect("approve has no durable human decision receipt");
    assert_eq!(receipt["kind"], "approve");
    assert_eq!(receipt["directiveId"], confirmed.id);
    assert_eq!(receipt["draftHash"], draft.draft_hash);
    assert_eq!(receipt["revision"], draft.revision);
    assert_eq!(receipt["rootRunId"], draft.root_run_id);
    assert_eq!(receipt["executionCompleted"], false);
    for (index, role) in draft.requested_roles.iter().enumerate() {
        assert_eq!(receipt["actions"][index]["order"], index + 1);
        assert_eq!(receipt["actions"][index]["role"], role.as_str());
        assert_eq!(receipt["actions"][index]["executionState"], "not_started");
        assert_eq!(
            receipt["actions"][index]["executionReceipt"],
            JsonValue::Null
        );
    }
    let saved = application_table_snapshot(&db);
    assert_eq!(
        directive::confirm_bound_draft(
            &db,
            &draft.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash
        )
        .unwrap()
        .id,
        confirmed.id
    );
    assert_eq!(
        application_table_snapshot(&db),
        saved,
        "approval replay must not append a second decision or queue row"
    );
    assert_eq!(
        human_review_native_fingerprint(&db),
        native,
        "full original Source Native JSON/business rows changed"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn human_directive_review_revise_freezes_old_snapshot_and_requires_new_confirmation() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db, draft) = human_review_fixture();
    let native = human_review_native_fingerprint(&db);
    let value = review_scan_directive_in(
        &db,
        &draft.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash,
        "revise",
        "@source_analyst 请关注路径边界",
    )
    .unwrap();
    let next = &value["draft"];
    assert_ne!(next["id"], draft.id);
    assert_ne!(next["draftHash"], draft.draft_hash);
    assert_eq!(next["revision"], draft.revision + 1);
    assert_eq!(next["confirmationRequired"], true);
    assert_eq!(next["rootRunId"], draft.root_run_id);
    assert_eq!(next["threadKey"], draft.thread_key);
    assert_eq!(value["receipt"]["kind"], "revise");
    assert_eq!(value["receipt"]["successorDraftId"], next["id"]);
    assert_eq!(value["receipt"]["executionCompleted"], false);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM agent_user_directives WHERE scan_id=?1",
            [&draft.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        directive::confirm_bound_draft(
            &db,
            &draft.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash
        )
        .is_err(),
        "old frozen approval was still usable"
    );
    assert!(directive::confirm_bound_draft(
        &db,
        &draft.scan_id,
        next["id"].as_str().unwrap(),
        draft.revision,
        &draft.draft_hash
    )
    .is_err());
    let stable = application_table_snapshot(&db);
    let replay = review_scan_directive_in(
        &db,
        &draft.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash,
        "revise",
        "@source_analyst 请关注路径边界",
    )
    .unwrap();
    assert_eq!(replay["receipt"], value["receipt"]);
    assert_eq!(application_table_snapshot(&db), stable);
    let queued = directive::confirm_bound_draft(
        &db,
        &draft.scan_id,
        next["id"].as_str().unwrap(),
        next["revision"].as_i64().unwrap(),
        next["draftHash"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(queued.status, "pending");
    assert_eq!(
        human_review_native_fingerprint(&db),
        native,
        "complete original Source JSON/evidence/budgets changed"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn human_directive_review_reject_persists_reason_and_terminal_receipt_without_queueing() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db, draft) = human_review_fixture();
    let native = human_review_native_fingerprint(&db);
    let reason = "证据不足，password=operator-secret";
    let value = review_scan_directive_in(
        &db,
        &draft.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash,
        "reject",
        reason,
    )
    .unwrap();
    assert_eq!(value["receipt"]["kind"], "reject");
    assert_eq!(value["receipt"]["terminal"], true);
    assert!(value["receipt"]["reason"]
        .as_str()
        .unwrap()
        .contains("证据不足"));
    assert!(!value.to_string().contains("operator-secret"));
    assert!(value["draft"].is_null());
    let raw: String = db
        .query_row(
            "SELECT receipt_json FROM agent_directive_human_reviews WHERE draft_id=?1",
            [&draft.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !raw.contains("operator-secret"),
        "secret reached the persisted terminal receipt"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM agent_directive_drafts WHERE id=?1",
            [&draft.id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "rejected"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM agent_user_directives WHERE source_draft_id=?1",
            [&draft.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(directive::confirm_bound_draft(
        &db,
        &draft.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash
    )
    .is_err());
    let stable = application_table_snapshot(&db);
    assert_eq!(
        review_scan_directive_in(
            &db,
            &draft.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash,
            "reject",
            reason
        )
        .unwrap(),
        value
    );
    assert_eq!(application_table_snapshot(&db), stable);
    assert_eq!(human_review_native_fingerprint(&db), native);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn human_directive_review_confirmed_or_queued_effects_cannot_be_revised_or_rejected() {
    use crate::agent_runtime::multi_agent::directive;
    for status in ["pending", "claimed", "assigned", "applied", "completed"] {
        let (root, db, draft) = human_review_fixture();
        let confirmed = directive::confirm_bound_draft(
            &db,
            &draft.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash,
        )
        .unwrap();
        db.execute(
            "UPDATE agent_user_directives SET status=?1 WHERE id=?2",
            params![status, confirmed.id],
        )
        .unwrap();
        let stable = application_table_snapshot(&db);
        for kind in ["revise", "reject"] {
            assert!(review_scan_directive_in(
                &db,
                &draft.scan_id,
                &draft.id,
                draft.revision,
                &draft.draft_hash,
                kind,
                "不能撤销已派发效果"
            )
            .is_err());
            assert_eq!(application_table_snapshot(&db), stable);
        }
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn human_directive_review_revised_blocked_role_or_host_request_stays_fail_closed() {
    use crate::agent_runtime::multi_agent::directive;
    for text in [
        "请用 SSH 登录主机验证",
        "@unknown_executor 扩大范围并跳过 reviewer",
    ] {
        let (root, db, draft) = human_review_fixture();
        let value = review_scan_directive_in(
            &db,
            &draft.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash,
            "revise",
            text,
        )
        .unwrap();
        let next = &value["draft"];
        assert_eq!(next["status"], "rejected");
        assert!(directive::confirm_bound_draft(
            &db,
            &draft.scan_id,
            next["id"].as_str().unwrap(),
            next["revision"].as_i64().unwrap(),
            next["draftHash"].as_str().unwrap()
        )
        .is_err());
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM agent_user_directives WHERE scan_id=?1",
                [&draft.scan_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

include!("tests_human_directive_review_faults.rs");

include!("tests_human_directive_review_roles.rs");
