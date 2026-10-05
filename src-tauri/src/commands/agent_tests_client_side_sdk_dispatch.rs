// Dispatch-only actual producer RED. These do not claim receipt/uncertain writers fixed.
#[test]
fn client_side_actual_sdk_dispatch_rejects_business_foreign_and_invocation_before_io() {
    for write in [
        "UPDATE projects SET name='forbidden SDK dispatch business';",
        "UPDATE agent_runs SET used_tokens=999 WHERE id='client-foreign-historical';",
        "UPDATE tool_invocations SET error_class='forbidden SDK dispatch invocation' WHERE run_id='client-foreign-historical';",
    ] {
        let mut f=client_hook_fixture("client-sdk-dispatch-writer",60000,20);
        client_delivery_foreign_invocation(&f);
        f.db.execute_batch(&format!("CREATE TRIGGER client_sdk_dispatch_fault AFTER INSERT ON agent_specialist_calls
            WHEN NEW.role='client_side' BEGIN {write} END;")).unwrap();
        let before=client_hook_untouched_scope(&f);
        let costs=client_hook_target_costs(&f.db);
        let error=f.finish().unwrap_err();
        assert!(error.contains("client_side_sdk_dispatch_write_rejected:"),"{write}: {error}");
        assert_eq!(client_hook_untouched_scope(&f),before);
        assert_eq!(client_hook_target_costs(&f.db),costs);
        client_hook_assert_supplier_closure(&f);
        client_sdk_dispatch_assert_original_unsent_grant(&f);
    }
}
#[test]
fn client_side_actual_sdk_dispatch_ignore_never_enters_unprotected_cleanup() {
    let mut f = client_hook_fixture("client-sdk-dispatch-ignore-cleanup", 60000, 20);
    f.db.execute_batch(
        "CREATE TRIGGER client_sdk_dispatch_ignore BEFORE INSERT ON agent_specialist_calls
        WHEN NEW.role='client_side' BEGIN SELECT RAISE(IGNORE); END;
        CREATE TRIGGER client_sdk_dispatch_cleanup_business AFTER UPDATE OF status ON agent_runs
        WHEN NEW.role='client_side' AND NEW.status='paused'
        BEGIN SELECT RAISE(ABORT,'forbidden SDK rejection cleanup'); END;",
    )
    .unwrap();
    let before = client_hook_untouched_scope(&f);
    let costs = client_hook_target_costs(&f.db);
    let error = f.finish().unwrap_err();
    assert!(
        error.contains("client_side_sdk_dispatch_rejected:specialist_dispatch_persist_missing"),
        "{error}"
    );
    assert!(!error.contains("specialist_cleanup:"), "{error}");
    assert_eq!(client_hook_untouched_scope(&f), before);
    assert_eq!(client_hook_target_costs(&f.db), costs);
    client_hook_assert_supplier_closure(&f);
    client_sdk_dispatch_assert_original_unsent_grant(&f);
}
#[test]
fn client_side_actual_sdk_dispatch_private_writer_keeps_bounded_original_request() {
    let mut f = client_hook_fixture("client-sdk-dispatch-positive", 60000, 20);
    f.finish().unwrap();
    let (request,hash,root,epoch,fence):(String,String,String,i64,String)=f.db.query_row(
        "SELECT request_json,request_hash,root_run_id,lease_epoch,fencing_token FROM agent_specialist_calls WHERE role='client_side'",
        [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    let lease = &f.session.as_ref().unwrap().lease;
    assert_eq!(root, lease.root_run_id);
    assert_eq!(epoch, lease.lease_epoch);
    assert_eq!(fence, lease.fencing_token);
    assert_eq!(hash, crate::agent_runtime::store::stable_hash(&request));
    let json: JsonValue = serde_json::from_str(&request).unwrap();
    assert_eq!(json["tools"], json!([]));
    assert_eq!(json["maxOutputTokens"], 256);
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(f.model_seen.lock().unwrap().len(), 1);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
fn client_sdk_dispatch_assert_original_unsent_grant(f: &ClientHookFixture) {
    let (assignment, run): (String, String) =
        f.db.query_row(
            "SELECT id,child_run_id FROM agent_assignments
        WHERE coordinator_run_id=?1 AND role='client_side'",
            [&f.root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let lease = &f.session.as_ref().unwrap().lease;
    let worker =
        crate::agent_runtime::multi_agent::attempts::current(&f.db, lease, &assignment).unwrap();
    assert_eq!(worker.child_run_id, run);
    assert_eq!(worker.state, "running");
    assert_eq!(worker.finished_at, "");
    assert_eq!(worker.coordinator_epoch, lease.lease_epoch);
    assert_eq!(worker.coordinator_fencing_token, lease.fencing_token);
    assert!(f.db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        JOIN agent_lane_leases l ON l.assignment_id=a.id WHERE a.id=?1 AND r.id=?2 AND a.state='running'
        AND a.budget_settled_at='' AND a.reserved_tokens=8000 AND a.reserved_requests=1
        AND r.status='running' AND r.used_tokens=0 AND r.used_requests=0 AND r.finished_at='' AND l.lane='read_only_analysis')",
        params![assignment,run],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1",
            [&assignment],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''",
            [&run],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(f.db.query_row("SELECT count(*) FROM agent_budget_entries WHERE assignment_id=?1
        AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') AND kind IN ('consume','forfeit','release','reconcile')",
        [&assignment],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM agent_messages WHERE assignment_id=?1",
            [&assignment],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(f.model_seen.lock().unwrap().len(), 0);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
