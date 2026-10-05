#[test]
fn source_tool_before_transport_cancellation_saves_unsent_without_unknown_charge() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler, source},
    };
    let (root, db, _, result) = source_reviewer_execution_fixture_using_calls(
        "no_candidates",
        None,
        4,
        None,
        2,
        |root, db, record, lease| {
            source_reviewer_checkpoint_fixture(
                root,
                db,
                record,
                lease,
                "after_initial_assessments",
            );
            let database = root.join("oviraptor.sqlite3");
            let work = root.join("attempt-0001");
            let (model, runtime, _) =
                verify_source_runtime_contract(db, &record.scan_id, 1, &work).unwrap();
            let proxy = source_runtime_proxy(&runtime).unwrap();
            let context = SpecialistTransportContext { supervision: None,
                db_path: &database,
                scan_id: &record.scan_id,
                attempt_number: 1,
                target_key: &lease.target_key,
                run_id: &lease.root_run_id,
                environment: &model,
                proxy,
                usage_dir: &work,
                deadline: Some(std::time::Instant::now() + Duration::from_secs(180)),
            };
            let role = AgentRole::RepoMapper;
            crate::agent_runtime::multi_agent::directive::source_guidance::freeze(
                db, lease, role, true,
            )
            .unwrap();
            let revision:i64=db.query_row("SELECT COALESCE(MAX(revision),1) FROM agent_evidence_revisions WHERE root_run_id=?1",[&lease.root_run_id],|r|r.get(0)).unwrap();
            let slice = source::tool_task_slice(db, lease, role, revision).unwrap();
            let remaining:i64=db.query_row("SELECT total_tokens-spent_tokens-reserved_tokens FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|r.get(0)).unwrap();
            let child = scheduler::schedule_child(
                db,
                lease,
                role,
                AgentLane::ReadOnlyAnalysis,
                "source_tools_ready",
                &slice,
                revision,
                &source::tool_capabilities(role).unwrap(),
                remaining / 4,
                3,
            )
            .unwrap();
            scheduler::mark_child_running(db, lease, &child).unwrap();
            // Initial claim checks remain valid. Only the real gateway's pre-I/O
            // cancel checker tries to renew the near-deadline lease and sees failure.
            db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+60 seconds','localtime') WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
            db.execute_batch("CREATE TRIGGER source_unsent_renewal_failure BEFORE UPDATE OF lease_expires_at ON agent_coordinator_leases BEGIN SELECT RAISE(IGNORE); END").unwrap();
            let error =
                execute_source_tool_assignment(db, &context, lease, &child, &slice).unwrap_err();
            assert!(error.contains("user_cancelled"), "{error}");
            db.execute_batch("DROP TRIGGER source_unsent_renewal_failure")
                .unwrap();
            for dim in &budget::DIMENSIONS[..4] {
                assert_eq!(
                    budget::balance(db, &lease.root_run_id, Some(&child.assignment_id), dim)
                        .unwrap()
                        .indeterminate,
                    0,
                    "before-transport cancellation cannot invent an unknown bill"
                );
            }
            let (phase,code):(String,String)=db.query_row("SELECT phase,json_extract(fact_json,'$.code') FROM agent_model_cost_facts WHERE child_run_id=?1 AND family='source-round'",[&child.run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(
                (phase, code),
                ("unsent".into(), "model_cancelled_before_transport".into())
            );
            assert_eq!(db.query_row("SELECT json_array(state,response_json,usage_json,response_hash,event_sequence,failure_code,finished_at) FROM agent_source_model_rounds WHERE child_run_id=?1",[&child.run_id],|r|r.get::<_,String>(0)).unwrap(),json!(["executing","{}","{}","",0,"",""]).to_string());
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM agent_source_tool_receipts WHERE child_run_id=?1",
                    [&child.run_id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            let before = application_table_snapshot(db);
            assert!(
                execute_source_tool_assignment(db, &context, lease, &child, &slice)
                    .unwrap_err()
                    .contains("not_sent")
            );
            assert_eq!(application_table_snapshot(db), before);
            assert_eq!(failed_specialist_error(db, lease, &child, &error), error);
            for dim in budget::DIMENSIONS[..4]
                .iter()
                .chain(["concurrency_batches"].iter())
            {
                assert_eq!(
                    budget::balance(db, &lease.root_run_id, Some(&child.assignment_id), dim)
                        .unwrap(),
                    budget::Balance::default()
                );
            }
            Ok(JsonValue::Null)
        },
    );
    result.unwrap();
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
