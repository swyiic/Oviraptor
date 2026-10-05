fn source_result_material_rows(db: &rusqlite::Connection) -> SourceExitSnapshot {
    source_exit_snapshot(db)
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "source_analysis_results" | "source_analysis_views" | "import_record_revisions"
            )
        })
        .collect()
}

fn source_result_assert_material_error(change: &str, error: &str) {
    match change {
        "missing" => assert_eq!(error, "analysis_result_receipt_unavailable"),
        "records" => assert!(
            error.starts_with("analysis_result_receipt_invalid:missing field `bundleId`"),
            "{error}"
        ),
        "envelope" => assert_eq!(error, "accepted_result_envelope_changed"),
        _ => assert_eq!(error, "analysis_result_receipt_binding_changed"),
    }
}

#[test]
fn source_result_receipt_missing_or_changed_never_falls_back_to_history() {
    // Faults affect temporary original material only after actual SDK payment and
    // scoped execution. Post-delivery authorization must roll back output and
    // corruption together; an unrelated closed-Root rejection is insufficient.
    for (change, drop_guard, mutation) in [
        ("missing", "analysis_results_no_delete", "DELETE FROM source_analysis_results"),
        ("digest", "analysis_results_no_update", "UPDATE source_analysis_results SET receipt_digest=printf('%064d',0)"),
        ("view", "analysis_results_no_update", "UPDATE source_analysis_results SET analysis_digest=printf('%064d',0)"),
        ("attempt", "analysis_results_no_update", "UPDATE source_analysis_results SET receipt_json=json_set(receipt_json,'$.attemptNumber',2)"),
        ("records", "analysis_results_no_update", "UPDATE source_analysis_results SET receipt_json=json_set(receipt_json,'$.records',json('[{}]'))"),
        ("envelope", "import_revisions_are_append_only", "UPDATE import_record_revisions SET envelope_json='{}'"),
    ] {
        for tool in ["analyzer.list_results", "analyzer.get_result"] {
            let original = Arc::new(std::sync::Mutex::new(SourceExitSnapshot::new()));
            let material = original.clone();
            let fault = format!("DROP TRIGGER {drop_guard}; CREATE TRIGGER corrupt_delivered_source_material AFTER UPDATE ON agent_source_tool_receipts WHEN NEW.tool_name='{tool}' AND NEW.state='completed' BEGIN {mutation}; END;");
            let p = source_broker_original_probe_using("diff", true, move |root, db, record, report| {
                assert_eq!(report["sourceClaims"].as_array().unwrap().len(), 1);
                let actual = report["sourceClaims"][0]["key"].as_str().unwrap();
                let historical = source_result_import_history(root, db, &record.scan_id);
                assert_ne!(historical, actual);
                *material.lock().unwrap() = source_result_material_rows(db);
                vec![vec![(tool, if tool=="analyzer.list_results" {json!({})} else {json!({"key":historical})})]]
            }, Some(&fault), |_, _| {}, |engine, scratch| source_result_outcome(engine, scratch, "app.py", "actual"));
            let error=p.result.as_ref().unwrap_err();
            eprintln!("Original material {change}/{tool}: {error}");
            source_result_assert_material_error(change, error);
            assert_eq!(p.calls, 5, "no next SDK, finish or independent review after lost material");
            assert_eq!(source_result_material_rows(&p.db), *original.lock().unwrap(), "{change}/{tool}: rollback preserves original typed rows and rowids");
            let receipts:Vec<(String,String,i64)>=p.db.prepare("SELECT t.state,t.output_json,t.event_sequence FROM agent_source_tool_receipts t JOIN agent_runs r ON r.id=t.child_run_id WHERE r.root_run_id=?1 AND r.role='source_analyst' ORDER BY t.round_number,t.call_index").unwrap()
                .query_map([&p.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            assert_eq!(receipts, vec![("planned".into(), "{}".into(), 0)], "no original or historical output committed");
            let view=source_result_view(&p.db,&p.record);
            assert_eq!(source_claims_for_analysis(&p.db,&view).unwrap().len(),1);
            // Persist the same loss only in this temporary DB to exercise cold
            // analysis admission: never rerun an analyzer or mint new material.
            p.db.execute_batch(mutation).unwrap();
            let before=source_exit_snapshot(&p.db);
            source_result_assert_material_error(change, &source_claims_for_analysis(&p.db,&view).unwrap_err());
            let retry=analysis_view_run(&p.root,&p.record,|_,_,_,_,_,_|panic!("missing completed receipt must not be regenerated"));
            source_result_assert_material_error(change,&retry.unwrap_err());
            assert_eq!(source_exit_snapshot(&p.db),before,"cold rejection must retain all paid rows and avoid material regeneration");
            p.cleanup();
        }
    }
}
