// Positive live-tool supervision uses the actual same-INSERT Source issuer.
// The shared historical v1 material helper deliberately remains unowned.
fn source_specialist_true_born_model_fixture(
    model: &ModelRuntimeEnv,
) -> (
    PathBuf,
    rusqlite::Connection,
    WorkbenchStartRecord,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let (root, db, record) = source_dispatch_fixture(model, None);
    analysis_view_run(&root, &record, |_, engine, _, _, scratch, _| {
        Ok(source_result_outcome(
            engine,
            scratch,
            "app.py",
            "source-analysis",
        ))
    })
    .unwrap();
    let registered = prepare_native_source_coordinator_fresh_schema_for_test(
        &db,
        &record.scan_id,
        1,
        &root.join("attempt-0001"),
        1,
    )
    .unwrap();
    let actor =
        native_source_fresh_finance::original_for_execution(&db, &registered.run_id).unwrap();
    let plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let plan: JsonValue = serde_json::from_str(&plan).unwrap();
    assert_eq!(plan["schemaVersion"], 1);
    assert_eq!(
        plan["modelRequestLimit"], 8,
        "preserve original v1 ceilings"
    );
    assert!(plan.get("sourceReviewPhaseVersion").is_none());
    (root, db, record, actor)
}
