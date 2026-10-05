#[test]
fn source_guidance_addressed_syntax_does_not_change_web_proposal_contracts() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db_path, _, lease) = multi_agent_test_root("source-focus-web-isolation", 500, 4);
    let connection = db::open(&db_path).unwrap();
    for (mention,role) in [("@reviewer","evidence_reviewer"),("@source_analyst","source_analyst"),("@repo_mapper","repo_mapper")] {
        let draft=directive::create_draft_in_thread(&connection,&lease.scan_id,lease.attempt_number,
            &lease.root_run_id,&lease.target_key,"coordinator",&format!("{mention} 请关注证据缺口"),"team",
            lease.lease_epoch,&lease.fencing_token).unwrap();
        assert_eq!(draft.intent,"agent_proposal_request");
        assert_eq!(draft.requested_roles,[role]);
        assert!(!draft.reason_codes.contains(&"source_focus_next_unfrozen_role_phase".into()));
    }
    let spoofed=directive::create_draft_in_thread(&connection,&lease.scan_id,lease.attempt_number,
        &lease.root_run_id,"source:untrusted-chat-target","coordinator","@source_analyst 请关注权限检查","team",
        lease.lease_epoch,&lease.fencing_token).unwrap();
    assert_eq!(spoofed.intent,"agent_proposal_request","a caller-supplied target is not a source root");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_guidance",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

fn directive_routing_second_root(
    connection: &rusqlite::Connection,
    first: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> crate::agent_runtime::multi_agent::lease::CoordinatorLease {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentRole, AgentRunStatus},
        multi_agent::lease,
        store::{self, AgentRunRow},
    };
    let mut run = AgentRunRow::new(
        "routing-second",
        &first.scan_id,
        first.attempt_number,
        "https://second.authorized.example.test",
        AgentBackendKind::Native,
        AgentRole::Coordinator,
        "plan",
        "evidence",
    );
    run.root_run_id = run.id.clone();
    run.status = AgentRunStatus::Running;
    store::create_run(connection, &run).unwrap();
    lease::acquire_coordinator_lease(
        connection,
        &first.scan_id,
        first.attempt_number,
        &run.target_url,
        &run.id,
        600,
    )
    .unwrap()
}

#[test]
fn directive_routing_target_assignment_and_message_resolve_without_latest_root_guessing() {
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-threads", 500, 4);
    let connection = db::open(&db_path).unwrap();
    assert_eq!(
        scan_directive_scope(&connection, &first.scan_id, "team")
            .unwrap()
            .1,
        first.root_run_id
    );
    let second = directive_routing_second_root(&connection, &first);
    connection
        .execute(
            "UPDATE agent_runs SET updated_at='2099-01-01' WHERE id=?1",
            [&second.root_run_id],
        )
        .unwrap();
    for lease in [&first, &second] {
        let assignment = format!("assignment-{}", lease.root_run_id);
        let correlation = format!("conversation-{}", lease.root_run_id);
        connection.execute(
            "INSERT INTO agent_assignments(id,coordinator_run_id,target_key,dedup_key) VALUES(?1,?2,?3,?1)",
            rusqlite::params![assignment, lease.root_run_id, lease.target_key],
        ).unwrap();
        connection.execute(
            "INSERT INTO agent_messages(id,run_id,root_run_id,kind,correlation_id,dedup_key) VALUES(?1,?2,?2,'assessment',?1,?1)",
            rusqlite::params![correlation, lease.root_run_id],
        ).unwrap();
        for thread in [&lease.target_key, &assignment, &correlation] {
            let scope = scan_directive_scope(&connection, &first.scan_id, thread).unwrap();
            assert_eq!(
                scope,
                (
                    1,
                    lease.root_run_id.clone(),
                    lease.target_key.clone(),
                    lease.lease_epoch,
                    lease.fencing_token.clone()
                )
            );
        }
    }
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_ambiguous_unknown_and_stale_threads_make_no_writes() {
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-ambiguity", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let second = directive_routing_second_root(&connection, &first);
    for lease in [&first, &second] {
        connection.execute(
            "INSERT INTO agent_messages(id,run_id,root_run_id,kind,correlation_id,dedup_key) VALUES(?1,?1,?1,'assessment','shared','shared')",
            [&lease.root_run_id],
        ).unwrap();
    }
    let before = connection.total_changes();
    for thread in ["team", "shared"] {
        assert_eq!(
            scan_directive_scope(&connection, &first.scan_id, thread).unwrap_err(),
            "directive_thread_coordinator_ambiguous"
        );
    }
    assert_eq!(
        scan_directive_scope(&connection, &first.scan_id, "foreign-assignment").unwrap_err(),
        "directive_thread_has_no_coordinator"
    );
    assert_eq!(connection.total_changes(), before);
    connection
        .execute(
            "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
            [&first.scan_id],
        )
        .unwrap();
    let before = connection.total_changes();
    for thread in ["team", "shared", first.target_key.as_str()] {
        assert_eq!(
            scan_directive_scope(&connection, &first.scan_id, thread).unwrap_err(),
            "directive_thread_has_no_coordinator"
        );
    }
    assert_eq!(connection.total_changes(), before);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_confirmation_follows_frozen_recipient_and_is_idempotent() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-confirm", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let draft = directive::create_draft_in_thread(
        &connection,
        &first.scan_id,
        1,
        &first.root_run_id,
        &first.target_key,
        "coordinator",
        "请优先检查权限控制",
        &first.target_key,
        first.lease_epoch,
        &first.fencing_token,
    )
    .unwrap();
    let second = directive_routing_second_root(&connection, &first);
    connection
        .execute(
            "UPDATE agent_runs SET updated_at='2099-01-01' WHERE id=?1",
            [&second.root_run_id],
        )
        .unwrap();
    let confirmed = directive::confirm_bound_draft(
        &connection,
        &first.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash,
    )
    .unwrap();
    let replay = directive::confirm_bound_draft(
        &connection,
        &first.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash,
    )
    .unwrap();
    assert_eq!(confirmed.id, replay.id);
    assert!(
        directive::claim_pending_directives(&connection, &second, 20)
            .unwrap()
            .is_empty()
    );
    let claimed = directive::claim_pending_directives(&connection, &first, 20).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, confirmed.id);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_user_directives", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_confirmation_rechecks_scope_fence_and_coordinator_state() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-revalidate", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let draft = directive::create_draft(
        &connection,
        &first.scan_id,
        1,
        &first.root_run_id,
        &first.target_key,
        "coordinator",
        "请优先检查权限控制",
        first.lease_epoch,
        &first.fencing_token,
    )
    .unwrap();
    let confirm = || {
        directive::confirm_bound_draft(
            &connection,
            &first.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash,
        )
    };
    assert!(directive::confirm_bound_draft(
        &connection,
        "foreign-scan",
        &draft.id,
        draft.revision,
        &draft.draft_hash
    )
    .is_err());
    connection
        .execute(
            "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
            [&first.scan_id],
        )
        .unwrap();
    assert_eq!(confirm().unwrap_err(), "directive_draft_scope_mismatch");
    connection
        .execute(
            "UPDATE sentinel_scans SET attempt_count=1 WHERE id=?1",
            [&first.scan_id],
        )
        .unwrap();
    for status in ["completed", "paused"] {
        connection
            .execute(
                "UPDATE agent_runs SET status=?2 WHERE id=?1",
                rusqlite::params![first.root_run_id, status],
            )
            .unwrap();
        assert_eq!(confirm().unwrap_err(), "coordinator_not_executable");
    }
    connection
        .execute(
            "UPDATE agent_runs SET status='running' WHERE id=?1",
            [&first.root_run_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE agent_coordinator_leases SET fencing_token='replacement' WHERE root_run_id=?1",
            [&first.root_run_id],
        )
        .unwrap();
    assert_eq!(
        confirm().unwrap_err(),
        "directive_draft_stale_fencing_token"
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_user_directives", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_unbound_history_cannot_be_confirmed_or_adopted_by_first_target() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-unbound", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let second = directive_routing_second_root(&connection, &first);
    let draft = directive::create_draft(
        &connection,
        &first.scan_id,
        1,
        "",
        "",
        "coordinator",
        "请优先检查权限控制",
        0,
        "",
    )
    .unwrap();
    assert_eq!(
        directive::confirm_bound_draft(
            &connection,
            &first.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash
        )
        .unwrap_err(),
        "directive_draft_recipient_not_bound"
    );
    assert_eq!(
        directive::confirm_draft(
            &connection,
            &first.scan_id,
            1,
            &first.root_run_id,
            &first.target_key,
            &draft.id,
            draft.revision,
            &draft.draft_hash
        )
        .unwrap_err(),
        "directive_draft_recipient_not_bound"
    );
    // Reconstruct a previously accepted unbound row without altering its hash.
    connection.execute(
        "INSERT INTO agent_user_directives(id,scan_id,attempt_number,text_redacted,source_draft_id,confirmed_revision,confirmed_hash,confirmation_at) VALUES('historical-unbound',?1,1,?2,?3,?4,?5,datetime('now','localtime'))",
        rusqlite::params![first.scan_id, draft.safe_execution_text, draft.id, draft.revision, draft.draft_hash],
    ).unwrap();
    connection.execute("UPDATE agent_directive_drafts SET status='confirmed',confirmed_directive_id='historical-unbound',confirmed_at=datetime('now','localtime') WHERE id=?1", [&draft.id]).unwrap();
    for lease in [&second, &first] {
        assert!(directive::claim_pending_directives(&connection, lease, 20)
            .unwrap()
            .is_empty());
    }
    let state: (String, String, String) = connection.query_row("SELECT status,root_run_id,claim_run_id FROM agent_user_directives WHERE id='historical-unbound'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!(state, ("rejected".into(), "".into(), "".into()));
    assert_eq!(
        directive::list_open_drafts(&connection, &first.scan_id, 1)
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM agent_directive_drafts WHERE id=?1",
                [&draft.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_confirmation_failure_rolls_back_queue_and_draft() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-rollback", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let draft = directive::create_draft(
        &connection,
        &first.scan_id,
        1,
        &first.root_run_id,
        &first.target_key,
        "coordinator",
        "请优先检查权限控制",
        first.lease_epoch,
        &first.fencing_token,
    )
    .unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_directive_confirmation BEFORE UPDATE OF status ON agent_directive_drafts WHEN NEW.status='confirmed' BEGIN SELECT RAISE(ABORT,'routing-test-rollback'); END;").unwrap();
    assert!(directive::confirm_bound_draft(
        &connection,
        &first.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash
    )
    .unwrap_err()
    .contains("routing-test-rollback"));
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_user_directives", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM agent_directive_drafts WHERE id=?1",
                [&draft.id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        draft.status
    );
    connection
        .execute_batch("DROP TRIGGER refuse_directive_confirmation;")
        .unwrap();
    assert!(directive::confirm_bound_draft(
        &connection,
        &first.scan_id,
        &draft.id,
        draft.revision,
        &draft.draft_hash
    )
    .is_ok());
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_empty_target_thread_is_projected_and_persisted_without_execution() {
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-empty-thread", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let second = directive_routing_second_root(&connection, &first);
    let before = connection.total_changes();
    let status = native_scan_status(&connection, &first.scan_id).unwrap();
    assert_eq!(connection.total_changes(), before);
    assert!(status["directiveRecipients"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |item| item["rootRunId"].as_str() == Some(second.root_run_id.as_str())
                && item["targetKey"].as_str() == Some(second.target_key.as_str())
        ));
    let input = AgentDialogViewInput {
        scan_id: first.scan_id.clone(),
        attempt_number: 1,
        expected_revision: 0,
        selected_thread: format!("coordinator:{}", second.root_run_id),
        mark_read_through: None,
        selected_thread_sequence: None,
    };
    let view = persist_dialog_view(&connection, &input).unwrap();
    assert_eq!(
        view.selected_thread,
        format!("coordinator:{}", second.root_run_id)
    );
    assert_eq!(view.all_read_sequence, 0);
    assert!(view.thread_read_sequences.is_empty());
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_user_directives", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    connection
        .execute(
            "UPDATE agent_runs SET status='completed' WHERE id=?1",
            [&second.root_run_id],
        )
        .unwrap();
    let status = native_scan_status(&connection, &first.scan_id).unwrap();
    assert!(!status["directiveRecipients"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["rootRunId"].as_str() == Some(second.root_run_id.as_str())));
    let input = AgentDialogViewInput {
        expected_revision: 1,
        ..input
    };
    assert_eq!(
        persist_dialog_view(&connection, &input).unwrap_err(),
        "dialog_view_thread_unavailable"
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_foreign_scan_and_deleted_scan_never_supply_recipients() {
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-foreign", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let second = directive_routing_second_root(&connection, &first);
    connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type,attempt_count) SELECT 'routing-foreign-scan',project_id,project_name,'scanning','web',1 FROM sentinel_scans WHERE id=?1", [&first.scan_id]).unwrap();
    connection
        .execute(
            "UPDATE agent_runs SET scan_id='routing-foreign-scan' WHERE id=?1",
            [&second.root_run_id],
        )
        .unwrap();
    connection.execute("INSERT INTO agent_messages(id,run_id,root_run_id,kind,correlation_id,dedup_key) VALUES('foreign-message',?1,?1,'assessment','foreign-thread','foreign-thread')", [&second.root_run_id]).unwrap();
    for thread in [second.target_key.as_str(), "foreign-thread"] {
        assert_eq!(
            scan_directive_scope(&connection, &first.scan_id, thread).unwrap_err(),
            "directive_thread_has_no_coordinator"
        );
    }
    assert_eq!(
        scan_directive_scope(&connection, &first.scan_id, "team")
            .unwrap()
            .1,
        first.root_run_id
    );
    connection
        .execute(
            "INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",
            [&first.scan_id],
        )
        .unwrap();
    let before = connection.total_changes();
    assert!(scan_directive_scope(&connection, &first.scan_id, "team").is_err());
    assert!(native_scan_status(&connection, &first.scan_id).is_err());
    assert_eq!(connection.total_changes(), before);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_draft_rechecks_recipients_after_waiting_for_writer() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WAITING: AtomicBool = AtomicBool::new(false);
    WAITING.store(false, Ordering::SeqCst);
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-writer", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let second = directive_routing_second_root(&connection, &first);
    connection
        .execute(
            "UPDATE agent_runs SET status='completed' WHERE id=?1",
            [&second.root_run_id],
        )
        .unwrap();
    let worker = db::open(&db_path).unwrap();
    worker
        .busy_handler(Some(|attempt| {
            WAITING.store(true, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(1));
            attempt < 5_000
        }))
        .unwrap();
    let tx =
        rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    let scan_id = first.scan_id.clone();
    let thread = std::thread::spawn(move || {
        draft_scan_directive_in(&worker, &scan_id, "请优先检查权限控制", "team")
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !WAITING.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let reached_writer = WAITING.load(Ordering::SeqCst);
    tx.execute(
        "UPDATE agent_runs SET status='running' WHERE id=?1",
        [&second.root_run_id],
    )
    .unwrap();
    tx.commit().unwrap();
    let result = thread.join().unwrap();
    assert!(
        reached_writer,
        "worker must actually reach SQLite write contention"
    );
    assert_eq!(
        result.unwrap_err(),
        "directive_thread_coordinator_ambiguous"
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_directive_drafts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let draft = draft_scan_directive_in(
        &connection,
        &first.scan_id,
        "请优先检查权限控制",
        &first.target_key,
    )
    .unwrap();
    assert_eq!(
        draft["rootRunId"].as_str(),
        Some(first.root_run_id.as_str())
    );
    assert_eq!(draft["targetKey"].as_str(), Some(first.target_key.as_str()));
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_user_directives", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_draft_postwrite_damage_rolls_back_every_insert() {
    let (root, db_path, _, first) = multi_agent_test_root("directive-routing-draft-damage", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let before = connection
        .query_row("SELECT count(*) FROM agent_collaboration_events", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    connection.execute_batch("CREATE TRIGGER damage_draft_route AFTER INSERT ON agent_directive_drafts BEGIN UPDATE agent_directive_drafts SET target_key='wrong-target' WHERE id=NEW.id; END;").unwrap();
    assert_eq!(
        draft_scan_directive_in(&connection, &first.scan_id, "请优先检查权限控制", "team")
            .unwrap_err(),
        "directive_draft_integrity_failed_after_insert"
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_directive_drafts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_collaboration_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_user_directives", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_routing_opaque_thread_survives_long_and_redacted_target_labels() {
    use crate::agent_runtime::multi_agent::directive;
    let secret = "fixture-routing-secret-value";
    let target = format!(
        "https://authorized.example.test/login?token={secret}&path={}",
        "a".repeat(240)
    );
    let (root, db_path, _, first) =
        multi_agent_test_root_for_target("directive-routing-redaction", 500, 4, &target);
    let connection = db::open(&db_path).unwrap();
    let status = native_scan_status(&connection, &first.scan_id).unwrap();
    let recipient = &status["directiveRecipients"][0];
    let thread = recipient["threadKey"].as_str().unwrap();
    assert_eq!(thread, format!("coordinator:{}", first.root_run_id));
    assert!(!recipient.to_string().contains(secret));
    let scope = scan_directive_scope(&connection, &first.scan_id, thread).unwrap();
    assert_eq!(scope.1, first.root_run_id);
    assert_eq!(scope.2, target);
    let draft =
        draft_scan_directive_in(&connection, &first.scan_id, "请优先检查权限控制", thread).unwrap();
    assert!(!draft.to_string().contains(secret));
    assert!(draft.get("boundFencingToken").is_none());
    let confirmed = directive::confirm_bound_draft(
        &connection,
        &first.scan_id,
        draft["id"].as_str().unwrap(),
        draft["revision"].as_i64().unwrap(),
        draft["draftHash"].as_str().unwrap(),
    )
    .unwrap();
    let claimed = directive::claim_pending_directives(&connection, &first, 20).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, confirmed.id);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}
