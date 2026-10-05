#[test]
fn mailbox_replay_checks_content_and_corrupt_payload_is_never_delivered() {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::{mailbox, scheduler}};
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("mailbox-fail-closed", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis,
        "mailbox-fixture", &serde_json::json!({}), 1, &["evidence.read".into()], 1_000, 1,
    ).unwrap();
    let send = |payload: &serde_json::Value, destination: &str| mailbox::send(
        &connection, &lease, &root_run_id, destination, "coordinator", "spa_api_mapper",
        "execution_assignment", "same-correlation", &child.assignment_id, 1, payload,
    );
    let id = send(&serde_json::json!({"summary":"original"}), &child.run_id).unwrap();
    assert_eq!(send(&serde_json::json!({"summary":"original"}), &child.run_id).unwrap(), id);
    assert_eq!(send(&serde_json::json!({"summary":"changed"}), &child.run_id).unwrap_err(), "mailbox_replay_conflict");
    assert_eq!(send(&serde_json::json!({"summary":"original"}), &root_run_id).unwrap_err(), "mailbox_route_not_assignment_bound");
    connection.execute("UPDATE agent_messages SET payload_json='{' WHERE id=?1", [&id]).unwrap();
    assert!(mailbox::deliver(&connection, &lease, &child.run_id, 20).unwrap_err().contains("无法解码 agent mailbox"));
    let delivery: (String, String, i64) = connection.query_row(
        "SELECT delivered_at,acknowledged_at,delivery_attempts FROM agent_messages WHERE id=?1",
        [&id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(delivery, (String::new(), String::new(), 0));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mailbox_send_requires_assignment_bound_child_and_truthful_roles() {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::{mailbox, scheduler}};
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("mailbox-route-binding", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis,
        "route-fixture", &serde_json::json!({}), 1, &["evidence.read".into()], 1_000, 1,
    ).unwrap();
    let send = |from: &str, to: &str, from_role: &str, to_role: &str| mailbox::send(
        &connection, &lease, from, to, from_role, to_role,
        "assignment", "route-fixture", &child.assignment_id, 1,
        &serde_json::json!({"summary":"scope binding"}),
    );
    for (from, to, from_role, to_role) in [
        (root_run_id.as_str(), root_run_id.as_str(), "coordinator", "coordinator"),
        ("run-foreign", root_run_id.as_str(), "spa_api_mapper", "coordinator"),
        (child.run_id.as_str(), root_run_id.as_str(), "coordinator", "coordinator"),
        (root_run_id.as_str(), child.run_id.as_str(), "coordinator", "evidence_reviewer"),
    ] {
        assert_eq!(send(from, to, from_role, to_role).unwrap_err(), "mailbox_route_not_assignment_bound");
    }
    let messages: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_messages WHERE root_run_id=?1", [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(messages, 0);
    assert!(!send(&root_run_id, &child.run_id, "coordinator", "spa_api_mapper").unwrap().is_empty());
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_coordinator_cannot_send_deliver_or_ack_mailbox_messages() {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::{mailbox, scheduler}};
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("mailbox-fencing", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis,
        "mailbox-fencing-fixture", &serde_json::json!({}), 1, &["evidence.read".into()], 1_000, 1,
    ).unwrap();
    let id = mailbox::send(&connection, &lease, &root_run_id, &child.run_id,
        "coordinator", "spa_api_mapper", "assignment", "before-fence", &child.assignment_id, 1,
        &serde_json::json!({"summary":"valid"})).unwrap();
    assert_eq!(mailbox::deliver(&connection, &lease, &child.run_id, 20).unwrap().len(), 1);
    connection.execute(
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='successor' WHERE root_run_id=?1",
        [&root_run_id],
    ).unwrap();
    assert_eq!(mailbox::acknowledge(&connection, &lease, &child.run_id, &id).unwrap_err(), "stale_coordinator_fencing_token");
    assert_eq!(mailbox::deliver(&connection, &lease, &child.run_id, 20).unwrap_err(), "stale_coordinator_fencing_token");
    assert_eq!(mailbox::send(&connection, &lease, &root_run_id, &child.run_id,
        "coordinator", "spa_api_mapper", "assignment", "after-fence", &child.assignment_id, 1,
        &serde_json::json!({"summary":"stale"})).unwrap_err(), "stale_coordinator_fencing_token");
    let (acked, messages): (String, i64) = connection.query_row(
        "SELECT acknowledged_at,(SELECT COUNT(*) FROM agent_messages WHERE root_run_id=?2) FROM agent_messages WHERE id=?1",
        rusqlite::params![id, root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert!(acked.is_empty());
    assert_eq!(messages, 1);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn paused_attempt_cannot_publish_or_consume_mailbox_messages() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{mailbox, scheduler},
    };

    let (root, db_path, root_run_id, lease) =
        multi_agent_test_root("paused-mailbox", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis,
        "mailbox-pause", &serde_json::json!({}), 1, &["evidence.read".into()], 100, 1,
    ).unwrap();
    let send = |correlation: &str| mailbox::send(
        &connection, &lease, &child.run_id, &root_run_id, "spa_api_mapper", "coordinator",
        "evidence_summary", correlation, &child.assignment_id, 1,
        &serde_json::json!({"summary":"before pause"}),
    );
    let delivered = send("already-delivered").unwrap();
    let pending = send("pending").unwrap();
    mailbox::deliver_expected(&connection, &lease, &root_run_id, &delivered).unwrap();
    connection.execute(
        "UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&lease.scan_id],
    ).unwrap();

    assert_eq!(send("late-result").unwrap_err(), "agent_attempt_not_active");
    assert_eq!(send("already-delivered").unwrap_err(), "agent_attempt_not_active");
    assert_eq!(mailbox::deliver_expected(&connection, &lease, &root_run_id, &pending).unwrap_err(),
        "agent_attempt_not_active");
    assert_eq!(mailbox::deliver(&connection, &lease, &root_run_id, 20).unwrap_err(),
        "agent_attempt_not_active");
    assert_eq!(mailbox::acknowledge(&connection, &lease, &root_run_id, &delivered).unwrap_err(),
        "agent_attempt_not_active");
    let state: (i64, String, String) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM agent_messages WHERE root_run_id=?1), \
         (SELECT delivered_at FROM agent_messages WHERE id=?2), \
         (SELECT acknowledged_at FROM agent_messages WHERE id=?3)",
        rusqlite::params![root_run_id, pending, delivered],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(state, (2, String::new(), String::new()));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn expected_child_message_does_not_consume_another_agents_pending_message() {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::{mailbox, scheduler}};
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("mailbox-selective", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis,
        "mailbox-selective-fixture", &serde_json::json!({}), 1, &["evidence.read".into()], 1_000, 1,
    ).unwrap();
    let send = |correlation: &str, summary: &str| mailbox::send(
        &connection, &lease, &child.run_id, &root_run_id, "spa_api_mapper", "coordinator",
        "evidence_summary", correlation, &child.assignment_id, 1,
        &serde_json::json!({"summary":summary}),
    ).unwrap();
    let other = send("other-thread", "keep unread");
    let expected = send("this-thread", "consume");
    assert_eq!(receive_expected_child_message(&connection, &lease, &root_run_id,
        &expected, "evidence_summary").unwrap()["summary"], "consume");
    let (delivered, acknowledged, attempts): (String, String, i64) = connection.query_row(
        "SELECT delivered_at,acknowledged_at,delivery_attempts FROM agent_messages WHERE id=?1",
        [&other], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!((delivered, acknowledged, attempts), (String::new(), String::new(), 0));
    assert_eq!(mailbox::deliver(&connection, &lease, &root_run_id, 20).unwrap().len(), 1);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

