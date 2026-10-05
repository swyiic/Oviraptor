// Actual finish → actual checked scheduler. These must precede the writer fix.
// Source rows stay unchanged; their hash cannot substitute for writer authority.
#[test]
fn client_side_actual_finish_rejects_changed_canonical_grant_emitter_before_sdk() {
    for body in [
        "UPDATE projects SET name='forbidden canonical-emitter write';",
        "UPDATE agent_runs SET used_tokens=999 WHERE id='client-foreign-historical';",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
          VALUES((SELECT scan_id FROM agent_runs WHERE id=NEW.coordinator_run_id),
            (SELECT attempt_number FROM agent_runs WHERE id=NEW.coordinator_run_id),
            'assignment',NEW.id,'assignment',json_object('role','wrong_role','state',NEW.state));",
    ] {
        let mut f=client_hook_fixture("client-canonical-grant-emitter",60000,20);
        client_hook_add_foreign_historical_root(&f);
        f.db.execute_batch(&format!("DROP TRIGGER agent_collaboration_assignment_insert;
          CREATE TRIGGER agent_collaboration_assignment_insert AFTER INSERT ON agent_assignments
          WHEN NEW.role='client_side' BEGIN {body} END;")).unwrap();
        let before=client_hook_untouched_scope(&f);
        let costs=client_hook_target_costs(&f.db);
        let error=f.finish().unwrap_err();
        assert!(error.contains("client_side_grant_emitter_conflict"),"{body}: {error}");
        assert_eq!(client_hook_untouched_scope(&f),before);
        assert_eq!(client_hook_target_costs(&f.db),costs);
        client_hook_assert_supplier_closure(&f);
        client_hook_assert_no_new_grant(&f);
    }
}
#[test]
fn client_side_actual_finish_ignored_canonical_grant_event_rolls_back_all_new_authority() {
    let mut f = client_hook_fixture("client-grant-event-ignore", 60000, 20);
    f.db.execute_batch(
        "CREATE TRIGGER client_grant_event_ignore BEFORE INSERT ON agent_collaboration_events
        WHEN NEW.entity_type='assignment' AND EXISTS(SELECT 1 FROM agent_assignments
          WHERE id=NEW.entity_id AND role='client_side') BEGIN SELECT RAISE(IGNORE); END;",
    )
    .unwrap();
    let before = client_hook_untouched_scope(&f);
    let costs = client_hook_target_costs(&f.db);
    let error = f.finish().unwrap_err();
    assert!(
        error.contains("client_side_grant_event_count_conflict"),
        "{error}"
    );
    assert_eq!(client_hook_untouched_scope(&f), before);
    assert_eq!(client_hook_target_costs(&f.db), costs);
    client_hook_assert_supplier_closure(&f);
    client_hook_assert_no_new_grant(&f);
}
#[test]
fn client_side_actual_finish_keeps_original_finance_and_grant_on_saved_received_replay() {
    let mut f = client_hook_fixture("client-grant-saved-replay", 60000, 20);
    f.finish().unwrap();
    let before = client_hook_untouched_scope(&f);
    let physical = [
        "agent_runs",
        "agent_assignments",
        "agent_assignment_attempts",
        "agent_capability_leases",
        "agent_contract_owners",
        "agent_budget_ledger",
        "agent_budget_entries",
        "agent_specialist_calls",
        "agent_messages",
        "agent_root_budget_attempts",
        "agent_budget_clock_origins",
        "agent_budget_limits",
    ]
    .into_iter()
    .map(|t| (t, client_grant_physical_rows(&f.db, t)))
    .collect::<Vec<_>>();
    f.finish().unwrap();
    assert_eq!(client_hook_untouched_scope(&f), before);
    for (name, rows) in physical {
        assert_eq!(
            client_grant_physical_rows(&f.db, name),
            rows,
            "{name}: saved received replay is read only"
        );
    }
    assert_eq!(f.model_seen.lock().unwrap().len(), 1);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    client_hook_assert_supplier_closure(&f);
}

fn client_grant_physical_rows(db: &rusqlite::Connection, table: &str) -> String {
    assert!(matches!(
        table,
        "agent_runs"
            | "agent_assignments"
            | "agent_assignment_attempts"
            | "agent_capability_leases"
            | "agent_contract_owners"
            | "agent_budget_ledger"
            | "agent_budget_entries"
            | "agent_specialist_calls"
            | "agent_messages"
            | "agent_root_budget_attempts"
            | "agent_budget_clock_origins"
            | "agent_budget_limits"
    ));
    let mut q = db
        .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
        .unwrap();
    let n = q.column_count();
    let rows = q
        .query_map([], |r| {
            (0..n)
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    format!("{rows:?}")
}

#[test]
fn client_side_actual_finished_executor_replay_rejects_changed_original_receipt_and_events() {
    for (mutation, expected) in [
        ("UPDATE agent_messages SET payload_json='{}' WHERE from_agent='web_executor';", "executor_finished_replay_receipt_conflict"),
        ("UPDATE agent_messages SET correlation_id='changed-route' WHERE from_agent='web_executor';", "executor_finished_replay_receipt_conflict"),
        ("DELETE FROM agent_collaboration_events WHERE entity_type='mailbox_message' AND entity_id IN (SELECT id FROM agent_messages WHERE from_agent='web_executor');", "executor_finished_replay_event_conflict"),
    ] {
        let mut f = client_hook_fixture("client-finished-original-replay-fault", 60000, 20);
        f.finish().unwrap();
        let raw: String = f.db.query_row("SELECT payload_json FROM agent_messages WHERE from_agent='web_executor' AND assignment_id=?1",
            [&f.session.as_ref().unwrap().executor.assignment_id], |r| r.get(0)).unwrap();
        let payload: JsonValue = serde_json::from_str(&raw).unwrap();
        let summary = payload["summary"].as_str().unwrap();
        f.db.execute_batch(mutation).unwrap();
        let before = client_delivery_all_rows(&f.db);
        let session = f.session.as_ref().unwrap();
        let error = original_executor_finished_replay(&f.db, &session.lease, &session.executor, &payload, summary).unwrap_err();
        assert_eq!(error, expected);
        assert!(client_delivery_all_rows(&f.db) == before, "read-only refusal must preserve all original physical rows");
        assert_eq!(f.model_seen.lock().unwrap().len(), 1);
        assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    }
}
