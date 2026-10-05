#[test]
fn human_proposal_owned_sdk_terminal_delivery_trigger_cannot_write_business_or_other_root() {
    for body in [
        "UPDATE projects SET name='bad-result-publish'",
        "UPDATE agent_runs SET terminal_reason='bad-side-effect' WHERE id='other-terminal-root'",
    ] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(valid_proposal_text()),
            )
        }));
        let (f, id) = human_owned_fixture(
            "human-owned-delivery-guard",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let db = db::open(&f.context.db_path).unwrap();
        db.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status,root_run_id,orchestration_policy,plan_hash,plan_json)
            SELECT 'other-terminal-root',scan_id,attempt_number,target_url,backend,role,'terminal','other-terminal-root',orchestration_policy,plan_hash,plan_json
            FROM agent_runs WHERE id=?1",[&f.actor.root_run_id]).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        db.execute_batch(&format!("CREATE TRIGGER human_delivery_bad BEFORE INSERT ON agent_messages WHEN NEW.kind='human_assessment_result' BEGIN {body}; END;")).unwrap();
        let mut inbox = take_human_directives(&f.context).unwrap();
        assert!(
            apply_human_proposal_actions(&f.context, &mut inbox).is_err(),
            "{body}"
        );
        let after = super::tests::application_table_snapshot(&db);
        assert_eq!(
            before.iter().find(|(t, _)| t == "projects"),
            after.iter().find(|(t, _)| t == "projects")
        );
        let other: String = db
            .query_row(
                "SELECT terminal_reason FROM agent_runs WHERE id='other-terminal-root'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(other, "");
        assert_eq!(human_owned_acked(&db), 0);
        let states:(String,String,String)=db.query_row("SELECT p.state,c.state,a.state FROM agent_directive_proposals p JOIN agent_specialist_calls c ON c.child_run_id=p.child_run_id JOIN agent_assignments a ON a.id=p.assignment_id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(
            states,
            ("received".into(), "received".into(), "running".into())
        );
        db.execute_batch("DROP TRIGGER human_delivery_bad").unwrap();
        assert_eq!(
            apply_human_proposal_actions(&f.context, &mut inbox)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn human_proposal_owned_sdk_original_dispatch_replace_and_delete_are_rejected_with_recursive_triggers_off(
) {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(valid_proposal_text()),
        )
    }));
    let (f, _id) = human_owned_fixture(
        "human-owned-original-immutable",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let mut inbox = take_human_directives(&f.context).unwrap();
    assert_eq!(
        apply_human_proposal_actions(&f.context, &mut inbox)
            .unwrap()
            .len(),
        1
    );
    let db = db::open(&f.context.db_path).unwrap();
    db.pragma_update(None, "recursive_triggers", false).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        human_owned_calls(&db),
        1,
        "this assertion exercises the actual route, not a fabricated call"
    );
    for sql in [
        "INSERT OR REPLACE INTO agent_specialist_calls SELECT * FROM agent_specialist_calls",
        "INSERT OR REPLACE INTO agent_specialist_calls(assignment_id,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,state)
            SELECT assignment_id,child_run_id,root_run_id,role,lease_epoch,fencing_token,'{}',request_hash,'executing' FROM agent_specialist_calls",
        "DELETE FROM agent_specialist_calls",
    ] {assert!(db.execute_batch(sql).is_err(),"{sql}");assert!(super::tests::application_table_snapshot(&db)==before);}
    assert_eq!(seen.lock().unwrap().len(), 1);
}
