#[test]
fn source_broker_scope_candidate_write_cannot_commit_after_stop_or_identity_change() {
    for (mutation, expected) in [
        (
            "UPDATE sentinel_scans SET status='paused'",
            "agent_attempt_not_active",
        ),
        (
            "UPDATE agent_runs SET cancel_requested_at='requested'",
            "coordinator_not_executable",
        ),
        (
            "UPDATE agent_runs SET root_run_id='foreign'",
            "assignment_attempt_run_binding:Query returned no rows",
        ),
        (
            "UPDATE sentinel_scan_attempts SET status='paused'",
            "source_assignment_scope_unavailable:Query returned no rows",
        ),
    ] {
        let fault=format!("CREATE TRIGGER revoke_source_candidate AFTER INSERT ON agent_evidence_nodes BEGIN {mutation}; END;");
        let p = source_broker_original_probe(
            "diff",
            true,
            vec![vec![(
                "evidence.submit_candidate",
                json!({"title":"suspect","rationale":"not confirmed","path":"app.py","line":1}),
            )]],
            Some(&fault),
            |_, _| {},
        );
        assert_eq!(p.result.as_ref().unwrap_err(), expected, "{mutation}");
        assert_eq!(
            p.calls, 5,
            "no finish, next role or SDK after transactional revocation"
        );
        assert_eq!(
            p.db.query_row("SELECT count(*) FROM agent_evidence_nodes", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            p.db.query_row("SELECT count(*) FROM source_analysis_views", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            p.db.query_row(
                "SELECT status FROM sentinel_scans WHERE id=?1",
                [&p.record.scan_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "scanning",
            "failed candidate must roll back scan revocation"
        );
        assert_eq!(
            p.db.query_row(
                "SELECT count(*) FROM agent_runs WHERE root_run_id<>?1 OR cancel_requested_at<>''",
                [&p.actor.root_run_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0,
            "failed candidate must roll back identity changes"
        );
        assert!(
            crate::agent_runtime::multi_agent::source::historical_materials(
                &p.db,
                &p.record.scan_id,
                1
            )
            .is_ok()
        );
        p.cleanup();
    }
}

#[test]
fn source_broker_scope_candidate_write_rolls_back_if_view_receipt_is_lost() {
    let p=source_broker_original_probe("diff",true,vec![vec![("evidence.submit_candidate",json!({"title":"selected suspect","rationale":"requires review","path":"app.py","line":1}))]],Some("DROP TRIGGER analysis_view_no_delete; CREATE TRIGGER lose_view_on_candidate AFTER INSERT ON agent_evidence_nodes BEGIN DELETE FROM source_analysis_views; END;"),|_,_|{});
    assert!(p.result.is_ok(), "{:?}", p.result);
    assert_eq!(p.calls, 7);
    let answer = p.output(1, 0);
    assert_eq!(answer["code"], "source_analysis_integrity", "{answer}");
    assert_eq!(
        p.db.query_row("SELECT count(*) FROM agent_evidence_nodes", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        p.db.query_row("SELECT count(*) FROM source_analysis_views", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    p.cleanup();
}

#[test]
fn source_broker_scope_candidates_bind_real_locations_and_distinct_content() {
    let mut invalid = Vec::new();
    for location in [
        json!({}),
        json!({"path":"app.py"}),
        json!({"path":"app.py","line":0}),
        json!({"path":"app.py","line":99}),
    ] {
        let mut args = location;
        args["title"] = json!("suspect");
        args["rationale"] = json!("requires review");
        invalid.push(("evidence.submit_candidate", args));
    }
    let args = |path: &str| json!({"title":"same title","rationale":"requires independent review","path":path,"line":1});
    invalid.extend([
        ("evidence.submit_candidate", args("app.py")),
        ("evidence.submit_candidate", args("untouched.py")),
    ]);
    let p = source_broker_original_probe(
        "full",
        true,
        vec![invalid, vec![("evidence.submit_candidate", args("app.py"))]],
        None,
        |_, _| {},
    );
    assert!(p.result.is_ok(), "{:?}", p.result);
    assert_eq!(p.calls, 9);
    for i in 0..4 {
        assert!(p.output(1, i).get("error").is_some());
    }
    let first = p.output(1, 4);
    let second = p.output(1, 5);
    assert!(first["id"].is_string(), "{first}");
    assert!(second["id"].is_string(), "{second}");
    assert_ne!(first["id"], second["id"]);
    assert_eq!(p.output(2, 0)["id"], first["id"]);
    let raw: String =
        p.db.query_row(
            "SELECT payload_json FROM agent_evidence_nodes WHERE id=?1",
            [first["id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let payload: JsonValue = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        payload["contentHash"],
        crate::artifact_import::canonical::sha256_hex(b"print('changed')\n")
    );
    assert_eq!(
        payload["analysisManifestDigest"].as_str().unwrap().len(),
        64
    );
    assert_eq!(payload["attemptNumber"], 1);
    assert_eq!(payload["scanId"], p.record.scan_id);
    assert_eq!(payload["reviewState"], "candidate");
    assert_eq!(
        p.db.query_row(
            "SELECT count(*) FROM agent_evidence_nodes WHERE kind='candidate_finding'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        p.result.as_ref().unwrap()["independentCandidateReviewCompleted"],
        true,
        "real independent Reviewer SDK, not candidate self-confirmation"
    );
    p.cleanup();
}
