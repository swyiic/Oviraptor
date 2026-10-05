fn attempt_http_fact_fixture() -> (std::path::PathBuf, AgentRunContext, String, JsonValue) {
    let (root, context, _, _) = http_journal_fixture("http://127.0.0.1:9/", 0);
    let view = serde_json::json!({"requestId":"worker-fact","status":200,
        "bodySha256":format!("{:x}",Sha256::digest(b"saved response"))});
    let artifact = agent_write_http_record(
        &context,
        1,
        &serde_json::json!({"method":"GET",
        "url":context.target_url,"identity":"anonymous"}),
        &view,
        b"saved response",
    )
    .unwrap();
    (root, context, artifact, view)
}

#[test]
fn assignment_attempt_expiry_cannot_promote_a_saved_http_artifact() {
    for expiry in ["2000-01-01", "malformed-deadline"] {
        let (root, context, artifact, view) = attempt_http_fact_fixture();
        let db = db::open(&context.db_path).unwrap();
        db.execute(
            "UPDATE agent_assignment_attempts SET expires_at=?1",
            [expiry],
        )
        .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let bytes = std::fs::read(
            context
                .target_dir
                .join(AGENT_HTTP_DIRECTORY)
                .join(&artifact),
        )
        .unwrap();
        assert!(
            agent_record_http_observation(&context, "replay_http", &artifact, &view, "worker-fact")
                .is_err(),
            "{expiry}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(
            std::fs::read(
                context
                    .target_dir
                    .join(AGENT_HTTP_DIRECTORY)
                    .join(&artifact)
            )
            .unwrap(),
            bytes
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_http_fact_postwrite_loss_rolls_back_the_entire_graph() {
    for mutation in [
        "UPDATE agent_assignment_attempts SET expires_at='2000-01-01';",
        "UPDATE agent_capability_leases SET revoked_at='2000-01-01';",
        "UPDATE agent_assignments SET evidence_revision=evidence_revision+1;",
        "UPDATE agent_runs SET cancel_requested_at='2000-01-01' WHERE role='coordinator';",
        "DELETE FROM agent_evidence_nodes WHERE id=NEW.id;",
        "UPDATE agent_evidence_nodes SET payload_json='{}' WHERE id=NEW.id;",
    ] {
        let (root, context, artifact, view) = attempt_http_fact_fixture();
        let db = db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER damage_http_fact AFTER INSERT ON agent_evidence_nodes BEGIN {mutation} END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            agent_record_http_observation(&context, "replay_http", &artifact, &view, "worker-fact")
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
fn assignment_attempt_live_http_fact_is_attributable_and_replay_is_readonly() {
    let (root, context, artifact, view) = attempt_http_fact_fixture();
    let db = db::open(&context.db_path).unwrap();
    agent_record_http_observation(&context, "replay_http", &artifact, &view, "worker-fact")
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM agent_evidence_nodes WHERE created_by_run_id=?1",
            [&context.run.as_ref().unwrap().run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let before = super::tests::application_table_snapshot(&db);
    agent_record_http_observation(&context, "replay_http", &artifact, &view, "worker-fact")
        .unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}
