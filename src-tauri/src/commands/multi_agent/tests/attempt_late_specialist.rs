#[test]
fn assignment_attempt_late_specialist_cost_does_not_publish_or_regrant() {
    use crate::agent_runtime::multi_agent::{budget, specialist};
    for uncertain in [false, true] {
        for different_root in [false, true] {
            let (root, context, lease, child) = specialist_journal_fixture();
            let db = db::open(&context.db_path).unwrap();
            let specialist::Start::Dispatch(call) =
                specialist::start(&db, &lease, &child, &serde_json::json!({"messages":[]}))
                    .unwrap()
            else {
                panic!()
            };
            let replacement = replace_target_cost_coordinator(&db, &lease, different_root);
            let before = super::tests::application_table_snapshot(&db);
            if uncertain {
                specialist::record_uncertain(&db, &call, "provider_outcome_unknown").unwrap();
            } else {
                assert!(specialist::record_received(
                    &db,
                    &call,
                    "late answer",
                    false,
                    &late_web_model_response().usage
                )
                .is_err());
            }
            let cost = budget::balance(
                &db,
                &lease.root_run_id,
                Some(&child.assignment_id),
                "model_requests",
            )
            .unwrap();
            assert_eq!(
                (cost.consumed, cost.indeterminate),
                if uncertain { (0, 1) } else { (1, 0) },
                "original provider cost was lost after Coordinator takeover"
            );
            let after = super::tests::application_table_snapshot(&db);
            for (table, rows) in &before {
                if !matches!(
                    table.as_str(),
                    "agent_budget_entries" | "agent_model_cost_facts"
                ) {
                    assert_eq!(
                        after
                            .iter()
                            .find(|(name, _)| name == table)
                            .map(|(_, values)| values),
                        Some(rows),
                        "{table}"
                    );
                }
            }
            if different_root {
                assert_eq!(
                    budget::balance(&db, &replacement.root_run_id, None, "model_requests").unwrap(),
                    budget::Balance::default()
                );
            }
            assert!(
                specialist::start(&db, &lease, &child, &serde_json::json!({"messages":[]}))
                    .is_err()
            );
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn assignment_attempt_late_specialist_cost_rechecks_original_dispatch_after_last_write() {
    use crate::agent_runtime::multi_agent::specialist;
    for mutation in [
        "DELETE FROM agent_specialist_calls WHERE child_run_id=NEW.child_run_id;",
        "UPDATE agent_specialist_calls SET state='uncertain',failure_code='damaged' WHERE child_run_id=NEW.child_run_id;",
        "UPDATE agent_runs SET role='evidence_reviewer',lane='review' WHERE id=NEW.child_run_id; UPDATE agent_assignments SET role='evidence_reviewer',lane='review' WHERE id=NEW.assignment_id;",
        "UPDATE agent_assignment_attempts SET failure_class='damaged' WHERE child_run_id=NEW.child_run_id;",
    ] {
        let (root,context,lease,child)=specialist_journal_fixture();
        let db=db::open(&context.db_path).unwrap();
        let specialist::Start::Dispatch(call)=specialist::start(&db,&lease,&child,&serde_json::json!({"messages":[]})).unwrap() else {panic!()};
        replace_target_cost_coordinator(&db,&lease,false);
        db.execute_batch(&format!("CREATE TRIGGER damage_original_specialist AFTER INSERT ON agent_budget_entries
            WHEN NEW.dimension='model_requests' AND NEW.kind='consume' BEGIN
            {} END;",mutation.replace("NEW.child_run_id",&format!("'{}'",child.run_id)))).unwrap();
        let before=super::tests::application_table_snapshot(&db);
        assert!(specialist::record_received(&db,&call,"bill only",false,&late_web_model_response().usage).is_err());
        assert_eq!(super::tests::application_table_snapshot(&db),before,"{mutation}");
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_late_specialist_terminal_facts_are_exact_private_and_immutable() {
    use crate::agent_runtime::multi_agent::{budget, specialist};
    for phase in ["received", "uncertain", "unsent"] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        let specialist::Start::Dispatch(call) =
            specialist::start(&db, &lease, &child, &serde_json::json!({"messages":[]})).unwrap()
        else {
            panic!()
        };
        replace_target_cost_coordinator(&db, &lease, true);
        let record = || match phase {
            "received" => {
                assert!(specialist::record_received(
                    &db,
                    &call,
                    "password=private-original-token",
                    false,
                    &late_web_model_response().usage
                )
                .is_err());
            }
            "uncertain" => {
                specialist::record_uncertain(&db, &call, "password=private-original-token").unwrap()
            }
            _ => specialist::record_not_sent(&db, &call, "user_cancelled").unwrap(),
        };
        record();
        let saved = super::tests::application_table_snapshot(&db);
        record();
        assert_eq!(super::tests::application_table_snapshot(&db), saved);
        let detail: String = db
            .query_row("SELECT fact_json FROM agent_model_cost_facts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!detail.contains("private-original-token"));
        assert!(db
            .execute("UPDATE agent_model_cost_facts SET phase='uncertain'", [])
            .is_err());
        assert!(db
            .execute("DELETE FROM agent_model_cost_facts", [])
            .is_err());
        if phase != "received" {
            assert!(specialist::record_received(
                &db,
                &call,
                "must not reconcile automatically",
                false,
                &late_web_model_response().usage
            )
            .is_err());
            assert_eq!(super::tests::application_table_snapshot(&db), saved);
        }
        let requests = budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_requests",
        )
        .unwrap();
        assert_eq!(
            (requests.reserved, requests.consumed, requests.indeterminate),
            match phase {
                "received" => (0, 1, 0),
                "uncertain" => (0, 0, 1),
                _ => (1, 0, 0),
            }
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_real_specialist_takeover_keeps_bill_and_stops_next_call() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, mut context, lease, child) = specialist_journal_fixture();
    let path = context.db_path.clone();
    let original = lease.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let db = db::open(&path).unwrap();
        replace_target_cost_coordinator(&db, &original, true);
        (
            200,
            "application/json",
            proposal_model_response("late result"),
        )
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    assert!(multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({})
    )
    .is_err());
    let db = db::open(&context.db_path).unwrap();
    assert_eq!(
        budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    assert!(multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        serde_json::json!({})
    )
    .is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(stop);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_late_unreported_cost_blocks_admission_even_without_token_reserve() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler, specialist},
    };
    let (root, path, id, lease) = multi_agent_test_root("late-unreported-no-token-grant", 0, 3);
    let db = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "unreported",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        0,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &lease, &child).unwrap();
    let specialist::Start::Dispatch(call) =
        specialist::start(&db, &lease, &child, &serde_json::json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    replace_target_cost_coordinator(&db, &lease, false);
    let usage = AgentTokenUsage {
        model_requests: 1,
        ..Default::default()
    };
    assert!(specialist::record_received_with_provenance(
        &db,
        &call,
        "estimated",
        false,
        &usage,
        false
    )
    .is_err());
    assert_eq!(
        budget::balance(&db, &id, Some(&child.assignment_id), "model_requests")
            .unwrap()
            .consumed,
        1
    );
    assert!(
        budget::admission::require_determinate(&db, &id).is_err(),
        "an unreported late bill must not permit new work"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
