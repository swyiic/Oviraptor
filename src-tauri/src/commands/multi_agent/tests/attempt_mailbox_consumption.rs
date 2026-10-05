#[test]
fn assignment_attempt_internal_consumer_cannot_ack_expired_worker_work() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{mailbox, scheduler},
    };
    let (root, path, id, lease) = multi_agent_test_root("worker-internal-ack", 1000, 10);
    let db = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "internal-ack",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        100,
        1,
    )
    .unwrap();
    let payload = serde_json::json!({"summary":"assignment"});
    let id = mailbox::send(
        &db,
        &lease,
        &id,
        &child.run_id,
        "coordinator",
        "spa_api_mapper",
        "assignment",
        "internal-ack",
        &child.assignment_id,
        1,
        &payload,
    )
    .unwrap();
    db.execute(
        "UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",
        [],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(
        consume_proposal_mailbox(&db, &lease, &child.run_id, &id, "assignment", &payload).is_err()
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_internal_consumer_rechecks_authority_and_payload_after_ack() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{mailbox, scheduler},
    };
    for mutation in [
        "UPDATE agent_assignment_attempts SET expires_at='2000-01-01';",
        "UPDATE agent_messages SET payload_json='{}' WHERE id=NEW.id;",
        "UPDATE agent_runs SET cancel_requested_at='2000-01-01' WHERE role='coordinator';",
    ] {
        let (root, path, id, lease) =
            multi_agent_test_root("worker-internal-ack-postwrite", 1000, 10);
        let db = db::open(&path).unwrap();
        let child = scheduler::schedule_child(
            &db,
            &lease,
            AgentRole::SpaApiMapper,
            AgentLane::ReadOnlyAnalysis,
            "internal-ack",
            &serde_json::json!({}),
            1,
            &["evidence.read".into()],
            100,
            1,
        )
        .unwrap();
        let payload = serde_json::json!({"summary":"assignment"});
        let id = mailbox::send(
            &db,
            &lease,
            &id,
            &child.run_id,
            "coordinator",
            "spa_api_mapper",
            "assignment",
            "internal-ack",
            &child.assignment_id,
            1,
            &payload,
        )
        .unwrap();
        db.execute_batch(&format!("CREATE TRIGGER damage_internal_ack AFTER UPDATE OF acknowledged_at ON agent_messages BEGIN {mutation} END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            consume_proposal_mailbox(&db, &lease, &child.run_id, &id, "assignment", &payload)
                .is_err(),
            "{mutation}"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{mutation}"
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}
