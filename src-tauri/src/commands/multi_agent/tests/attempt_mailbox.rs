#[test]
fn assignment_attempt_expiry_denies_worker_mailbox_send_delivery_and_ack() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{mailbox, scheduler},
    };
    for action in ["worker_send", "root_send", "delivery", "ack"] {
        let (root, path, id, lease) = multi_agent_test_root("worker-mailbox", 1000, 10);
        let db = db::open(&path).unwrap();
        let child = scheduler::schedule_child(
            &db,
            &lease,
            AgentRole::SpaApiMapper,
            AgentLane::ReadOnlyAnalysis,
            "worker-mailbox",
            &serde_json::json!({}),
            1,
            &["evidence.read".into()],
            100,
            1,
        )
        .unwrap();
        let payload = serde_json::json!({"summary":"assignment"});
        let message = mailbox::send(
            &db,
            &lease,
            &id,
            &child.run_id,
            "coordinator",
            "spa_api_mapper",
            "assignment",
            "before-expiry",
            &child.assignment_id,
            1,
            &payload,
        )
        .unwrap();
        if action == "ack" {
            mailbox::deliver_expected(&db, &lease, &child.run_id, &message).unwrap();
        }
        db.execute(
            "UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",
            [],
        )
        .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let rejected = match action {
            "worker_send" => mailbox::send(
                &db,
                &lease,
                &child.run_id,
                &id,
                "spa_api_mapper",
                "coordinator",
                "evidence_summary",
                "after-expiry",
                &child.assignment_id,
                1,
                &payload,
            )
            .is_err(),
            "root_send" => mailbox::send(
                &db,
                &lease,
                &id,
                &child.run_id,
                "coordinator",
                "spa_api_mapper",
                "assignment",
                "after-expiry",
                &child.assignment_id,
                1,
                &payload,
            )
            .is_err(),
            "delivery" => mailbox::deliver_expected(&db, &lease, &child.run_id, &message).is_err(),
            "ack" => mailbox::acknowledge(&db, &lease, &child.run_id, &message).is_err(),
            _ => unreachable!(),
        };
        assert!(
            rejected,
            "{action}: expired worker must not borrow Root authority"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{action}"
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_mailbox_send_postwrite_loss_rolls_back_even_in_autocommit() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{mailbox, scheduler},
    };
    for mutation in [
        "UPDATE agent_assignment_attempts SET expires_at='2000-01-01';",
        "UPDATE agent_runs SET cancel_requested_at='2000-01-01' WHERE role='coordinator';",
        "UPDATE agent_assignments SET evidence_revision=evidence_revision+1;",
        "UPDATE agent_messages SET payload_json='{}' WHERE id=NEW.id;",
        "DELETE FROM agent_messages WHERE id=NEW.id;",
    ] {
        let (root, path, id, lease) = multi_agent_test_root("worker-mailbox-postwrite", 1000, 10);
        let db = db::open(&path).unwrap();
        let child = scheduler::schedule_child(
            &db,
            &lease,
            AgentRole::SpaApiMapper,
            AgentLane::ReadOnlyAnalysis,
            "worker-mailbox",
            &serde_json::json!({}),
            1,
            &["evidence.read".into()],
            100,
            1,
        )
        .unwrap();
        db.execute_batch(&format!("CREATE TRIGGER damage_worker_message AFTER INSERT ON agent_messages BEGIN {mutation} END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            mailbox::send(
                &db,
                &lease,
                &id,
                &child.run_id,
                "coordinator",
                "spa_api_mapper",
                "assignment",
                "postwrite",
                &child.assignment_id,
                1,
                &serde_json::json!({"summary":"assignment"})
            )
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

#[test]
fn assignment_attempt_mailbox_delivery_and_ack_recheck_after_the_last_write() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{mailbox, scheduler},
    };
    for action in ["delivery", "ack"] {
        for mutation in [
            "UPDATE agent_assignment_attempts SET expires_at='2000-01-01';",
            "UPDATE agent_runs SET cancel_requested_at='2000-01-01' WHERE role='coordinator';",
            "UPDATE agent_messages SET payload_json='{}' WHERE id=NEW.id;",
        ] {
            let (root, path, id, lease) = multi_agent_test_root("worker-mailbox-consume", 1000, 10);
            let db = db::open(&path).unwrap();
            let child = scheduler::schedule_child(
                &db,
                &lease,
                AgentRole::SpaApiMapper,
                AgentLane::ReadOnlyAnalysis,
                "worker-mailbox",
                &serde_json::json!({}),
                1,
                &["evidence.read".into()],
                100,
                1,
            )
            .unwrap();
            let message = mailbox::send(
                &db,
                &lease,
                &id,
                &child.run_id,
                "coordinator",
                "spa_api_mapper",
                "assignment",
                "consume",
                &child.assignment_id,
                1,
                &serde_json::json!({"summary":"assignment"}),
            )
            .unwrap();
            if action == "ack" {
                mailbox::deliver_expected(&db, &lease, &child.run_id, &message).unwrap();
            }
            let column = if action == "ack" {
                "acknowledged_at"
            } else {
                "delivered_at"
            };
            db.execute_batch(&format!("CREATE TRIGGER damage_worker_consumption AFTER UPDATE OF {column} ON agent_messages BEGIN {mutation} END;")).unwrap();
            let before = super::tests::application_table_snapshot(&db);
            let rejected = if action == "ack" {
                mailbox::acknowledge(&db, &lease, &child.run_id, &message).is_err()
            } else {
                mailbox::deliver_expected(&db, &lease, &child.run_id, &message).is_err()
            };
            assert!(rejected, "{action}: {mutation}");
            assert_eq!(
                super::tests::application_table_snapshot(&db),
                before,
                "{action}: {mutation}"
            );
            drop(db);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
