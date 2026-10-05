#[test]
fn directive_action_freezes_an_explicit_priority_before_confirmation() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-draft", 10_000, 20);
    let connection = db::open(&path).unwrap();
    for text in ["请优先检查权限控制", "prioritize authorization", "优先检查 authorization"] {
        let draft = directive::create_draft(
            &connection, &lease.scan_id, 1, &lease.root_run_id, &lease.target_key,
            "coordinator", text, lease.lease_epoch, &lease.fencing_token,
        ).unwrap();
        assert_eq!(draft.priority_changes, ["prioritize_family:authorization"]);
        assert_eq!(draft.estimated_requests, 0);
        assert_eq!(draft.estimated_tokens, 0);
        assert_eq!(draft.status, "drafted");
    }
    let _ = std::fs::remove_dir_all(root);
}

fn confirm_queue_directive(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    text: &str,
) -> String {
    use crate::agent_runtime::multi_agent::directive;
    let draft = directive::create_draft(
        connection, &lease.scan_id, lease.attempt_number, &lease.root_run_id, &lease.target_key,
        "coordinator", text, lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    directive::confirm_draft(
        connection, &lease.scan_id, lease.attempt_number, &lease.root_run_id, &lease.target_key,
        &draft.id, draft.revision, &draft.draft_hash,
    ).unwrap().id
}

fn priority_test_queue() -> Vec<String> {
    ["api:GET|/observed", "family:information_disclosure", "family:authorization",
     "contract:authorization|/owned", "family:business_flow", "api:GET|/observed"]
        .into_iter().map(str::to_string).collect()
}

#[test]
fn directive_action_replay_and_projection_reject_damaged_completion() {
    use crate::agent_runtime::multi_agent::directive;
    for damage in [
        "DELETE FROM agent_directive_queue_actions",
        "UPDATE agent_directive_drafts SET safe_execution_text='prioritize business_flow'",
        "UPDATE agent_user_directives SET claim_fencing_token='wrong-fence'",
        "UPDATE agent_user_directives SET payload_json=json_set(payload_json,'$.priorityChanges',json('[\"prioritize_family:business_flow\"]'))",
        "DELETE FROM agent_collaboration_events WHERE json_extract(payload_json,'$.status')='completed'",
        "DROP TRIGGER agent_directive_queue_action_immutable; UPDATE agent_directive_queue_actions SET family='business_flow'",
        "DROP TRIGGER agent_directive_queue_action_immutable; UPDATE agent_directive_queue_actions SET receipt_json=json_set(receipt_json,'$.targetRequests',1)",
        "DROP TRIGGER agent_directive_queue_action_immutable; UPDATE agent_directive_queue_actions SET receipt_json=json_set(receipt_json,'$.changedOrder',json('false'))",
        "DELETE FROM agent_directive_drafts",
    ] {
        let (root, path, _, lease) = multi_agent_test_root("directive-priority-damaged", 10_000, 20);
        let connection = db::open(&path).unwrap();
        let id = confirm_queue_directive(&connection, &lease, "prioritize authorization");
        directive::claim_pending_directives(&connection, &lease, 20).unwrap();
        directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).unwrap();
        connection.execute_batch(damage).unwrap();
        let before = connection.total_changes();
        let projection = native_scan_status_after(&connection, &lease.scan_id, None).unwrap();
        let item = projection["timeline"].as_array().unwrap().iter().find(|item| item["id"] == id).unwrap();
        assert!(item["queueAction"].is_null(), "{damage}: {item}");
        assert_eq!(item["deliveryState"], "receipt_unverified", "{damage}");
        assert!(item["reasonCodes"].as_array().unwrap().contains(&json!("queue_action_receipt_unverified")));
        assert_eq!(connection.total_changes(), before, "projection must not repair history");
        assert!(directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).is_err(), "{damage}");
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn directive_action_ignored_receipt_or_event_rolls_back_completion() {
    use crate::agent_runtime::multi_agent::directive;
    for trigger in [
        "CREATE TRIGGER ignore_action BEFORE INSERT ON agent_directive_queue_actions BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER ignore_action BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='user_directive' AND json_extract(NEW.payload_json,'$.status')='completed' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER ignore_action BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='user_directive' AND json_extract(NEW.payload_json,'$.status')='applied' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (root, path, _, lease) = multi_agent_test_root("directive-priority-ignored", 10_000, 20);
        let connection = db::open(&path).unwrap();
        let id = confirm_queue_directive(&connection, &lease, "prioritize authorization");
        directive::claim_pending_directives(&connection, &lease, 20).unwrap();
        directive::prepare_model_context(&connection, &lease).unwrap();
        connection.execute_batch(trigger).unwrap();
        assert!(directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).is_err(), "{trigger}");
        let state: (String, i64) = connection.query_row(
            "SELECT status,(SELECT COUNT(*) FROM agent_directive_queue_actions) FROM agent_user_directives WHERE id=?1",
            [&id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(state, ("accepted".into(), 0));
        connection.execute_batch("DROP TRIGGER ignore_action").unwrap();
        directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).unwrap();
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn directive_action_terminal_history_is_read_only_and_does_not_require_current_lease() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-history", 10_000, 20);
    let connection = db::open(&path).unwrap();
    let id = confirm_queue_directive(&connection, &lease, "prioritize authorization");
    directive::claim_pending_directives(&connection, &lease, 20).unwrap();
    directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).unwrap();
    connection.execute("UPDATE agent_runs SET status='terminal' WHERE id=?1", [&lease.root_run_id]).unwrap();
    connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'", []).unwrap();
    let before = connection.total_changes();
    let projection = native_scan_status_after(&connection, &lease.scan_id, None).unwrap();
    let item = projection["timeline"].as_array().unwrap().iter().find(|item| item["id"] == id).unwrap();
    assert_eq!(item["queueAction"]["family"], "authorization");
    assert_eq!(connection.total_changes(), before);
    assert!(directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_action_does_not_infer_negated_compound_or_specialist_commands() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-ambiguity", 10_000, 20);
    let connection = db::open(&path).unwrap();
    for text in ["不要优先检查权限控制", "请优先检查权限控制，然后暂停其他检查", "优先检查权限控制和业务流程",
        "@reviewer 请优先检查权限控制", "优先检查 https://other.example.test", "优先检查权限控制并提高预算",
        "优先检查SSH主机", "可能需要优先检查权限控制", "请优先复核已有证据"] {
        let draft = directive::create_draft(
            &connection, &lease.scan_id, 1, &lease.root_run_id, &lease.target_key,
            "coordinator", text, lease.lease_epoch, &lease.fencing_token,
        ).unwrap();
        assert!(!draft.priority_changes.iter().any(|action| action.starts_with("prioritize_family:")), "{text}");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_action_reorders_real_pending_work_and_replays_without_duplicate_receipts() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-replay", 10_000, 20);
    let connection = db::open(&path).unwrap();
    let id = confirm_queue_directive(&connection, &lease, "请优先检查权限控制");
    let normal_id = confirmed_directive_for_lease(&connection, &lease);
    directive::claim_pending_directives(&connection, &lease, 20).unwrap();
    let queue = priority_test_queue();
    let cursor: i64 = connection.query_row("SELECT MAX(sequence) FROM agent_collaboration_events", [], |row| row.get(0)).unwrap();
    let (remaining, ordered) = directive::apply_queue_actions(&connection, &lease, &queue).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, normal_id);
    assert_eq!(&ordered[..2], &["family:authorization", "contract:authorization|/owned"]);
    let mut before = queue.clone(); before.sort();
    let mut after = ordered.clone(); after.sort();
    assert_eq!(before, after, "do not drop duplicates, add work or change scope");
    let receipt: String = connection.query_row("SELECT receipt_json FROM agent_directive_queue_actions WHERE directive_id=?1", [&id], |row| row.get(0)).unwrap();
    let receipt: JsonValue = serde_json::from_str(&receipt).unwrap();
    assert_eq!(receipt["matchedItems"], 2);
    assert_eq!(receipt["changedOrder"], true);
    assert_eq!(receipt["targetRequests"], 0);
    assert_eq!(receipt["coverageVerified"], false);
    let projection = native_scan_status_after(&connection, &lease.scan_id, Some(cursor)).unwrap();
    let row = projection["timeline"].as_array().unwrap().iter().find(|row| row["id"] == id).unwrap();
    assert_eq!(row["status"], "completed");
    assert_eq!(row["queueAction"], receipt);
    assert_ne!(row["deliveryState"], "model_received");
    // Lose the memory/checkpoint order, then reopen the DB as a resumed process.
    drop(connection);
    let connection = db::open(&path).unwrap();
    let (_, replayed) = directive::apply_queue_actions(&connection, &lease, &queue).unwrap();
    assert_eq!(replayed, ordered);
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM agent_directive_queue_actions", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 1);
    assert!(connection.execute("UPDATE agent_directive_queue_actions SET family='business_flow'", []).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_action_receipt_and_completion_roll_back_if_timeline_fails() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-atomic", 10_000, 20);
    let connection = db::open(&path).unwrap();
    let id = confirm_queue_directive(&connection, &lease, "prioritize authorization");
    directive::claim_pending_directives(&connection, &lease, 20).unwrap();
    directive::prepare_model_context(&connection, &lease).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_priority_completion BEFORE INSERT ON agent_collaboration_events \
        WHEN NEW.event_type='user_directive' AND json_extract(NEW.payload_json,'$.status')='completed' \
        BEGIN SELECT RAISE(ABORT,'priority receipt unavailable'); END;").unwrap();
    let queue = priority_test_queue();
    let error = directive::apply_queue_actions(&connection, &lease, &queue).unwrap_err();
    assert!(error.contains("priority receipt unavailable"));
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM agent_directive_queue_actions", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
    let status: String = connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1", [&id], |row| row.get(0)).unwrap();
    assert_eq!(status, "accepted");
    connection.execute_batch("DROP TRIGGER refuse_priority_completion").unwrap();
    assert_eq!(directive::apply_queue_actions(&connection, &lease, &queue).unwrap().1[0], "family:authorization");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_action_without_matching_work_is_deferred_and_never_sent_as_executed() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-no-match", 10_000, 20);
    let connection = db::open(&path).unwrap();
    let id = confirm_queue_directive(&connection, &lease, "prioritize authorization");
    directive::claim_pending_directives(&connection, &lease, 20).unwrap();
    let queue = vec!["api:GET|/authorization".to_string(), "family:business_flow".to_string()];
    let (remaining, ordered) = directive::apply_queue_actions(&connection, &lease, &queue).unwrap();
    assert!(remaining.is_empty());
    assert_eq!(ordered, queue, "a path name is not a coverage category");
    let (status, reason): (String,String) = connection.query_row("SELECT status,rejection_code FROM agent_user_directives WHERE id=?1", [&id], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(status, "deferred");
    assert_eq!(reason, "priority_no_matching_pending_work");
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM agent_directive_queue_actions", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_action_latest_confirmed_preference_wins_and_new_work_is_reordered() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-precedence", 10_000, 20);
    let connection = db::open(&path).unwrap();
    confirm_queue_directive(&connection, &lease, "prioritize authorization");
    confirm_queue_directive(&connection, &lease, "prioritize business_flow");
    directive::claim_pending_directives(&connection, &lease, 20).unwrap();
    let (_, ordered) = directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).unwrap();
    assert_eq!(ordered[0], "family:business_flow");
    assert_eq!(ordered[1], "family:authorization");
    let mut new_work = priority_test_queue();
    new_work.push("contract:business_flow|/new-observed".into());
    let (_, ordered) = directive::apply_queue_actions(&connection, &lease, &new_work).unwrap();
    assert_eq!(&ordered[..2], &["family:business_flow", "contract:business_flow|/new-observed"]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_action_rechecks_integrity_and_active_attempt() {
    use crate::agent_runtime::multi_agent::directive;
    for paused in [false, true] {
        let (root, path, _, lease) = multi_agent_test_root("directive-priority-invalid", 10_000, 20);
        let connection = db::open(&path).unwrap();
        let id = confirm_queue_directive(&connection, &lease, "prioritize authorization");
        directive::claim_pending_directives(&connection, &lease, 20).unwrap();
        directive::prepare_model_context(&connection, &lease).unwrap();
        if paused {
            connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&lease.scan_id]).unwrap();
            assert!(directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).is_err());
        } else {
            connection.execute("UPDATE agent_user_directives SET text_redacted='tampered' WHERE id=?1", [&id]).unwrap();
            let (remaining, queue) = directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).unwrap();
            assert!(remaining.is_empty());
            assert_eq!(queue, priority_test_queue());
        }
        let count: i64 = connection.query_row("SELECT COUNT(*) FROM agent_directive_queue_actions", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn directive_action_schema_upgrades_an_existing_database() {
    let (root, path, _, _) = multi_agent_test_root("directive-priority-upgrade", 10_000, 20);
    let connection = db::open(&path).unwrap();
    connection.execute_batch("DROP TABLE agent_directive_queue_actions").unwrap();
    drop(connection);
    db::initialize(&root).unwrap();
    let connection = db::open(&path).unwrap();
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM agent_directive_queue_actions", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_action_cannot_apply_an_unconfirmed_draft() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-unconfirmed", 10_000, 20);
    let connection = db::open(&path).unwrap();
    directive::create_draft(&connection, &lease.scan_id, 1, &lease.root_run_id, &lease.target_key,
        "coordinator", "prioritize authorization", lease.lease_epoch, &lease.fencing_token).unwrap();
    assert!(directive::claim_pending_directives(&connection, &lease, 20).unwrap().is_empty());
    let (items, queue) = directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).unwrap();
    assert!(items.is_empty());
    assert_eq!(queue, priority_test_queue());
    let _ = std::fs::remove_dir_all(root);
}


#[test]
fn directive_action_claim_batches_follow_confirmation_order_not_random_ids() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-order", 10_000, 20);
    let connection = db::open(&path).unwrap();
    let first = confirm_queue_directive(&connection, &lease, "prioritize authorization");
    let second = confirm_queue_directive(&connection, &lease, "prioritize business_flow");
    // Simulate clock skew: durable insertion order is authoritative.
    connection.execute("UPDATE agent_user_directives SET created_at='2000-01-01' WHERE id=?1", [&second]).unwrap();
    assert_eq!(directive::claim_pending_directives(&connection, &lease, 1).unwrap()[0].id, first);
    directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).unwrap();
    assert_eq!(directive::claim_pending_directives(&connection, &lease, 1).unwrap()[0].id, second);
    assert_eq!(directive::apply_queue_actions(&connection, &lease, &priority_test_queue()).unwrap().1[0], "family:business_flow");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_action_fence_is_revalidated_after_real_writer_contention() {
    use crate::agent_runtime::multi_agent::directive;
    use std::sync::atomic::{AtomicBool, Ordering};
    static WAITING: AtomicBool = AtomicBool::new(false);
    WAITING.store(false, Ordering::SeqCst);
    let (root, path, _, lease) = multi_agent_test_root("directive-priority-fence", 10_000, 20);
    let connection = db::open(&path).unwrap();
    let id = confirm_queue_directive(&connection, &lease, "prioritize authorization");
    directive::claim_pending_directives(&connection, &lease, 20).unwrap();
    let worker = db::open(&path).unwrap();
    worker.busy_handler(Some(|attempt| {
        WAITING.store(true, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(1));
        attempt < 5_000
    })).unwrap();
    let transaction = rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate).unwrap();
    transaction.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='new-owner' WHERE root_run_id=?1", [&lease.root_run_id]).unwrap();
    let worker_lease = lease.clone();
    let handle = std::thread::spawn(move || directive::apply_queue_actions(&worker, &worker_lease, &priority_test_queue()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !WAITING.load(Ordering::SeqCst) && std::time::Instant::now() < deadline { std::thread::yield_now(); }
    let contended = WAITING.load(Ordering::SeqCst);
    transaction.commit().unwrap();
    let result = handle.join().unwrap();
    assert!(contended, "the worker must reach the held SQLite write lock");
    assert_eq!(result.unwrap_err(), "stale_coordinator_fencing_token");
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM agent_directive_queue_actions", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
    let status: String = connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1", [&id], |row| row.get(0)).unwrap();
    assert_eq!(status, "claimed");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directive_action_replay_is_target_scoped_and_does_not_spend_budget() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentRole, AgentRunStatus},
        multi_agent::{directive, lease}, store::{self, AgentRunRow},
    };
    let (root, path, _, first) = multi_agent_test_root("directive-priority-isolation", 10_000, 20);
    let connection = db::open(&path).unwrap();
    let target = "https://second.authorized.example.test";
    let mut run = AgentRunRow::new("root-priority-second", &first.scan_id, 1, target,
        AgentBackendKind::Native, AgentRole::Coordinator, "plan", "evidence");
    run.root_run_id = "root-priority-second".into();
    run.status = AgentRunStatus::Running;
    store::create_run(&connection, &run).unwrap();
    let second = lease::acquire_coordinator_lease(&connection, &first.scan_id, 1, target, &run.id, 600).unwrap();
    let budget_snapshot = || -> Vec<Vec<i64>> {
        let mut query = connection.prepare("SELECT total_tokens,reserved_tokens,spent_tokens,total_requests,reserved_requests,spent_requests FROM agent_budget_ledger ORDER BY root_run_id").unwrap();
        query.query_map([], |row| (0..6).map(|index| row.get(index)).collect::<Result<Vec<i64>,_>>()).unwrap()
            .collect::<Result<Vec<_>,_>>().unwrap()
    };
    let before = budget_snapshot();
    confirm_queue_directive(&connection, &first, "prioritize authorization");
    let second_id = confirm_queue_directive(&connection, &second, "prioritize business_flow");
    directive::claim_pending_directives(&connection, &first, 20).unwrap();
    directive::apply_queue_actions(&connection, &first, &priority_test_queue()).unwrap();
    let (_, ordered) = directive::apply_queue_actions(&connection, &second, &priority_test_queue()).unwrap();
    assert_eq!(ordered, priority_test_queue(), "the second target must not inherit the first target's rule");
    assert_eq!(directive::claim_pending_directives(&connection, &second, 20).unwrap()[0].id, second_id);
    let (_, ordered) = directive::apply_queue_actions(&connection, &second, &priority_test_queue()).unwrap();
    assert_eq!(ordered[0], "family:business_flow");
    assert_eq!(directive::apply_queue_actions(&connection, &first, &priority_test_queue()).unwrap().1[0], "family:authorization");
    assert_eq!(before, budget_snapshot());
    let _ = std::fs::remove_dir_all(root);
}

