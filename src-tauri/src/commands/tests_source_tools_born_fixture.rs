// Live Source tools/rounds require the genuine same-INSERT original issuer.
// Historical material fixtures and missing-owner transport negatives stay raw.
fn source_tool_true_born_fixture_lease(
    db: &rusqlite::Connection,
    record: &WorkbenchStartRecord,
) -> crate::agent_runtime::multi_agent::lease::CoordinatorLease {
    let work = Path::new(db.path().unwrap())
        .parent()
        .unwrap()
        .join("attempt-0001");
    let root =
        prepare_native_source_coordinator_fresh_schema_for_test(db, &record.scan_id, 1, &work, 1)
            .unwrap();
    let actor = native_source_fresh_finance::original_for_execution(db, &root.run_id).unwrap();
    assert_eq!(actor.lease_epoch, 1);
    assert_eq!(
        db.query_row(
            "SELECT hard_request_budget FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        8
    );
    actor
}
fn source_tool_true_born_fixture_model(
    model: Option<&ModelRuntimeEnv>,
) -> (
    PathBuf,
    rusqlite::Connection,
    WorkbenchStartRecord,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let (root, db, record) = analysis_view_fixture_configure("full", true, |db, record| {
        if let Some(model) = model {
            db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles',json(?1),'$.activeModelProfileId','source-fixture') WHERE id=(SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1)",
                [json!([{"id":"source-fixture","llm":model.llm,"apiKey":model.api_key,"apiBase":model.api_base,"deployment":model.deployment}]).to_string()]).unwrap();
            record.llm_policy = source_model_policy(model);
        }
    });
    analysis_view_run(&root, &record, |_, engine, _, _, scratch, _| {
        Ok(source_result_outcome(
            engine,
            scratch,
            "app.py",
            "source-analysis",
        ))
    })
    .unwrap();
    let actor = source_tool_true_born_fixture_lease(&db, &record);
    (root, db, record, actor)
}
