// Real ordinary original SDK history cannot be repaired by a local replay.
fn specialist_replay_missing_original_proof_case(known: bool) {
        let (root,mut context,actor,child)=specialist_financial_fixture();
        let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(move |_| if known {
            (200,"application/json",proposal_model_response("original paid ordinary assessment"))
        } else {(503,"application/json",r#"{"error":{"message":"original remote failure"}}"#.into())}));
        let _cleanup=ClientSideReadonlyTestGuard {root,stop};
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let first=multi_agent_child_round_transport(&context,&actor,&child,"original readonly",json!({}));
        assert_eq!(first.is_ok(),known);assert_eq!(seen.lock().unwrap().len(),1);
        let path=specialist_inflight_original_path(&context,&actor,&child.run_id);
        assert!(path.is_file());fs::remove_file(&path).unwrap();
        let db=db::open(&context.db_path).unwrap();let before=super::tests::application_table_snapshot(&db);
        let replay=multi_agent_child_round_transport(&context,&actor,&child,"original readonly",json!({}));
        assert!(replay.is_err(),"missing original SDK proof cannot be a received replay");
        assert!(!path.exists(),"local received/unknown replay must not CREATE historical exit proof");
        assert!(super::tests::application_table_snapshot(&db)==before);assert_eq!(seen.lock().unwrap().len(),1);
        assert!(finish_coordinator_run(&db,&actor,&AgentTargetOutcome::incomplete("original proof remains absent")).is_err());
        assert!(super::tests::application_table_snapshot(&db)==before);assert!(!path.exists());
}
#[test]
fn specialist_replay_missing_received_original_proof_never_creates_historical_exit_inode() {
    specialist_replay_missing_original_proof_case(true);
}
#[test]
fn specialist_replay_missing_unknown_original_proof_never_creates_historical_exit_inode() {
    specialist_replay_missing_original_proof_case(false);
}
#[test]
fn specialist_wrong_role_replay_cannot_manufacture_another_roles_missing_original_exit_proof() {
    let (root,context,actor,child,seen)=specialist_inflight_finished_original();
    let path=specialist_inflight_original_path(&context,&actor,&child.run_id);
    assert!(path.is_file());fs::remove_file(&path).unwrap();
    let db=db::open(&context.db_path).unwrap();let before=super::tests::application_table_snapshot(&db);
    let mut foreign=child.clone();foreign.role=crate::agent_runtime::contract::AgentRole::EvidenceReviewer;
    assert!(multi_agent_child_round_transport(&context,&actor,&foreign,"readonly",json!({})).is_err());
    assert!(!path.exists(),"caller role failure must occur before CREATE of original child proof");
    assert!(super::tests::application_table_snapshot(&db)==before);assert_eq!(seen.lock().unwrap().len(),1);
    drop(db);drop(context);fs::remove_dir_all(root).unwrap();
}

#[test]
fn specialist_replay_paid_original_inode_is_preserved_and_only_idle_received_result_replays() {
    let (root,mut context,actor,child)=specialist_financial_fixture();
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response("original paid readonly response"))));
    let _cleanup=ClientSideReadonlyTestGuard {root,stop};context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let original=multi_agent_child_round_transport(&context,&actor,&child,"original readonly",json!({})).unwrap();
    assert_eq!(seen.lock().unwrap().len(),1);
    let path=specialist_inflight_original_path(&context,&actor,&child.run_id);
    let metadata=fs::metadata(&path).unwrap();
    let db=db::open(&context.db_path).unwrap();let before=super::tests::application_table_snapshot(&db);
    // Hold the already existing original inode. This tests the OS replay probe,
    // not a second paid SDK or an SDK/worker join claim.
    let owner=crate::agent_runtime::execution_owner::claim_native_invocation(&context.db_path,&actor.scan_id,
        actor.attempt_number,"specialist-sdk",&child.run_id).unwrap();
    let error=multi_agent_child_round_transport(&context,&actor,&child,"original readonly",json!({})).unwrap_err();
    assert!(error.contains("specialist_transport_not_idle"),"{error}");
    assert!(super::tests::application_table_snapshot(&db)==before);drop(owner);
    let replay=multi_agent_child_round_transport(&context,&actor,&child,"original readonly",json!({})).unwrap();
    assert_eq!(replay,original);assert_eq!(seen.lock().unwrap().len(),1);
    assert!(super::tests::application_table_snapshot(&db)==before);
    assert_eq!(fs::metadata(&path).unwrap().created().unwrap(),metadata.created().unwrap());
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(&path).unwrap().ino(),metadata.ino());
    }
}
#[test]
fn specialist_replay_rejected_fresh_role_does_not_create_original_exit_lock_or_dispatch() {
    let (root,mut context,actor,child)=specialist_financial_fixture();
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_|panic!("wrong original role must refuse before SDK")));
    let _cleanup=ClientSideReadonlyTestGuard {root,stop};context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let db=db::open(&context.db_path).unwrap();let before=super::tests::application_table_snapshot(&db);
    let mut foreign=child.clone();foreign.role=crate::agent_runtime::contract::AgentRole::EvidenceReviewer;
    let path=specialist_inflight_original_path(&context,&actor,&child.run_id);assert!(!path.exists());
    let error=multi_agent_child_round_transport(&context,&actor,&foreign,"readonly",json!({})).unwrap_err();
    assert!(error.contains("specialist_binding_invalid"),"{error}");
    assert!(!path.exists());assert!(super::tests::application_table_snapshot(&db)==before);assert!(seen.lock().unwrap().is_empty());
}
