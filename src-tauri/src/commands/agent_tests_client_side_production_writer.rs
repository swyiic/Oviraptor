// Intentionally PENDING RED: sourceProof alone is not full application writer
// authority. These tests use actual finish/SDK, with no source-row mutation to
// accidentally make a business-only trigger appear protected. Do not weaken.
#[test]
fn client_side_actual_finish_business_only_grant_trigger_is_rejected_atomically() {
    for write in [
        "UPDATE projects SET name='forbidden unrelated write';",
        "UPDATE agent_runs SET used_tokens=999 WHERE id='client-foreign-historical';",
    ] {
        let mut f = client_hook_fixture("client-source-business-only-grant", 60000, 20);
        client_hook_add_foreign_historical_root(&f);
        f.db.execute_batch(&format!(
            "CREATE TRIGGER client_only_business_fault AFTER INSERT ON agent_assignments
        WHEN NEW.role='client_side' BEGIN {write} END;"
        ))
        .unwrap();
        let before = client_hook_untouched_scope(&f);
        let costs = client_hook_target_costs(&f.db);
        let error=f.finish().unwrap_err();
        assert!(error.contains("client_side_grant_write_rejected:"),"{write}: {error}");
        assert_eq!(client_hook_untouched_scope(&f), before);
        assert_eq!(client_hook_target_costs(&f.db), costs);
        client_hook_assert_supplier_closure(&f);
        client_hook_assert_no_new_grant(&f);
    }
}
#[test]
fn client_side_actual_finish_business_only_delivery_trigger_preserves_received_sdk_fee() {
    for write in [
        "UPDATE projects SET name='forbidden unrelated write';",
        "UPDATE agent_runs SET used_tokens=999 WHERE id='client-foreign-historical';",
    ] {
        for boundary in [
        "AFTER INSERT ON agent_messages WHEN NEW.from_agent='client_side'",
        "AFTER UPDATE OF acknowledged_at ON agent_messages WHEN NEW.from_agent='client_side' AND NEW.acknowledged_at<>''",
        "AFTER UPDATE OF status ON agent_runs WHEN NEW.role='client_side' AND NEW.terminal_state='completed'",
    ] {
        let mut f=client_hook_fixture("client-source-business-only-delivery",60000,20);
        client_hook_add_foreign_historical_root(&f);
        *f.model_boundary_sql.lock().unwrap() = Some(format!("CREATE TRIGGER client_only_business_fault {boundary}
            BEGIN {write} END;"));
        let before=client_hook_untouched_scope(&f);
        let costs=client_hook_target_costs(&f.db);
        assert!(f.finish().is_err(),"{boundary}: no source mutation may stand in for writer protection");
        assert_eq!(client_hook_untouched_scope(&f),before);
        assert_eq!(client_hook_target_costs(&f.db),costs);
        client_hook_assert_supplier_closure(&f);
        assert_eq!(f.model_seen.lock().unwrap().len(),1);
        assert_eq!(f.site_seen.lock().unwrap().len(),1);
        assert_eq!(f.db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE role='client_side' AND state='received'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert!(f.db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries e JOIN agent_assignments a ON a.id=e.assignment_id
            WHERE a.role='client_side' AND e.dimension='model_requests' AND e.kind='consume' AND e.amount=1)",[],|r|r.get::<_,bool>(0)).unwrap());
        assert_eq!(f.db.query_row("SELECT count(*) FROM agent_messages WHERE from_agent='client_side'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(f.db.query_row("SELECT count(*) FROM sentinel_findings",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    }
}
