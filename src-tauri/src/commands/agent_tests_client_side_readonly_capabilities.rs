#[test]
fn client_side_readonly_never_borrows_target_lane_or_target_capabilities() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (directory, path, root, lease) =
        multi_agent_new_task_root("client-readonly-denied", 60_000, 20);
    let db = db::open(&path).unwrap();
    let content = b"<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'\">";
    let input_dir = fs::canonicalize(&directory).unwrap().join("private-client-input");
    fs::create_dir_all(&input_dir).unwrap();
    fs::write(input_dir.join("client-side-observed.html"), content).unwrap();
    let slice = json!({"schemaVersion":1,"phase":"client_side_readonly","rootRunId":root,
        "scanId":lease.scan_id,"attemptNumber":lease.attempt_number,"target":lease.target_key,
        "coordinatorEpoch":lease.lease_epoch,"coordinatorFence":lease.fencing_token,
        "evidenceRevision":1,"targetRequestsGranted":0,"browserActionsGranted":0,
        "observations":[{"id":"meta-csp-1","kind":"meta_csp","classification":"source-derived",
            "artifact":{"path":"client-side-observed.html","contentHash":crate::agent_runtime::store::artifact_id(content)},
            "value":"default-src 'self'"}],"toolsGranted":[]});
    // Existing refusal preservation: currently all ClientSide is unimplemented.
    // This must remain denied when the actual readonly feature is enabled.
    for (lane, extra) in [
        (AgentLane::TargetTouching, "evidence.read"),
        (AgentLane::ReadOnlyAnalysis, "replay_http"),
        (AgentLane::ReadOnlyAnalysis, "browser_action"),
        (AgentLane::ReadOnlyAnalysis, "upload"),
        (AgentLane::ReadOnlyAnalysis, "shell"),
    ] {
        let before = receipt_database_snapshot(&db);
        assert!(scheduler::schedule_child(
            &db,
            &lease,
            AgentRole::ClientSide,
            lane,
            "client-side-forbidden",
            &slice,
            1,
            &["evidence.read".into(), extra.into()],
            8_000,
            1
        )
        .is_err());
        assert_eq!(receipt_database_snapshot(&db), before);
    }
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
