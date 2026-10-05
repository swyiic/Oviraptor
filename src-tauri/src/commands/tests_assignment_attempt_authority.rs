#[test]
fn assignment_attempt_expiry_blocks_source_tools_without_new_evidence() {
    let (root,db,context,_)=native_source_tool_fixture(crate::agent_runtime::contract::AgentRole::SourceAnalyst);
    db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",[]).unwrap();
    let before=application_table_snapshot(&db);
    assert!(agent_authorize_tool_on(&db,&context,"evidence.submit_candidate").is_err(),
        "live Coordinator/capability must not authorize an expired worker");
    let output=agent_execute_source_tool(&context,"evidence.submit_candidate",
        &json!({"title":"expired worker","rationale":"reject","path":"app.py","line":1}));
    assert!(output.get("error").is_some(),"{output}");
    assert_application_tables_unchanged(&db,&before);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_expiry_blocks_new_specialist_dispatch_and_cancels_transport() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist}};
    let (root,db,_,lease)=source_specialist_fixture();
    let role=AgentRole::RepoMapper;
    let slice=source::task_slice(&db,&lease,role).unwrap();
    let child=scheduler::prepare_readonly_child(&db,&lease,role,"source_results_ready",&slice,8_000).unwrap();
    db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",[]).unwrap();
    let before=application_table_snapshot(&db);
    assert!(specialist::start(&db,&lease,&child,&source_specialist_request(&db,&lease,role)).is_err(),
        "new model work must require the original worker lease");
    assert!(specialist_model_cancel_token(&root.join("oviraptor.sqlite3"),&lease,&child,None).is_cancelled());
    assert_application_tables_unchanged(&db,&before);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_malformed_deadline_never_authorizes_source_worker() {
    let (root,db,context,_)=native_source_tool_fixture(crate::agent_runtime::contract::AgentRole::SourceAnalyst);
    db.execute("UPDATE agent_assignment_attempts SET expires_at='not-a-deadline'",[]).unwrap();
    let before=application_table_snapshot(&db);
    assert!(agent_authorize_tool_on(&db,&context,"repo.inventory").is_err());
    assert_application_tables_unchanged(&db,&before);
    drop(db);fs::remove_dir_all(root).unwrap();
}
