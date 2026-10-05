// Post-R1 behavioral negatives. Admission baseline rejects ClientSide already;
// these are not additional claimed feature-red results.
fn client_readonly_negative_fixture(
    tag: &str,
) -> (
    PathBuf,
    PathBuf,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    AgentRunContext,
    JsonValue,
) {
    let (directory, path, root, lease) = multi_agent_new_task_root(tag, 60_000, 20);
    let db = db::open(&path).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger {
        db_path: path.clone(),
        run_id: root.clone(),
    });
    context.target_dir = fs::canonicalize(&directory).unwrap().join("client-readonly-private");
    fs::create_dir_all(&context.target_dir).unwrap();
    let bytes=b"<html><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'\"></html>";
    fs::write(context.target_dir.join("client-side-observed.html"), bytes).unwrap();
    let slice = json!({"schemaVersion":1,"phase":"client_side_readonly","rootRunId":root,
        "scanId":lease.scan_id,"attemptNumber":lease.attempt_number,"target":lease.target_key,
        "coordinatorEpoch":lease.lease_epoch,"coordinatorFence":lease.fencing_token,
        "evidenceRevision":1,"targetRequestsGranted":0,"browserActionsGranted":0,"toolsGranted":[],
        "observations":[{"id":"csp-1","kind":"meta_csp","classification":"source-derived",
            "artifact":{"path":"client-side-observed.html","contentHash":crate::agent_runtime::store::artifact_id(bytes)},
            "value":"default-src 'self'"}]});
    drop(db);
    (directory, path, root, lease, context, slice)
}
#[test]
fn client_side_readonly_invalid_actual_sdk_body_retains_original_fee_hash_without_semantic_body() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    let (directory, path, _, lease, mut context, slice) =
        client_readonly_negative_fixture("client-invalid-response");
    let raw = r#"{"summary":"invented impact","observationRefs":["csp-1"],"gaps":[],"candidates":[{"confirmed":true}]}"#;
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        (200, "application/json", proposal_model_response(raw))
    }));
    let _guard = ClientSideReadonlyTestGuard {
        root: directory.clone(),
        stop,
    };
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let db = db::open(&path).unwrap();
    let child = scheduler::prepare_readonly_child(
        &db,
        &lease,
        AgentRole::ClientSide,
        "client_side_frozen_observations_ready",
        &slice,
        8000,
    )
    .unwrap();
    let error = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "配置事实只读，无工具；不允许impact或confirmed",
        slice,
    )
    .unwrap_err();
    assert!(
        error.contains("client_side_readonly_assessment_insufficient"),
        "{error}"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let text:String=db.query_row("SELECT response_json FROM agent_specialist_calls WHERE assignment_id=?1 AND state='received'",[&child.assignment_id],|r|r.get(0)).unwrap();
    let response: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(response["text"], "");
    assert_eq!(
        response["rawTextHash"],
        crate::agent_runtime::store::artifact_id(raw.as_bytes())
    );
    assert_eq!(
        response["rejection"],
        "client_side_readonly_assessment_insufficient"
    );
    let worker: String = db
        .query_row(
            "SELECT id FROM agent_assignment_attempts WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE assignment_id=?1 AND lease_attempt_id=?2 AND dimension='model_requests' AND kind='consume' AND amount=1)",params![child.assignment_id,worker],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_messages WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn client_side_readonly_artifact_or_literal_change_denied_before_actual_sdk_and_dispatch_claim() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    for fault in [
        "artifact_hash",
        "invented_literal",
        "insufficient_reserved_tokens",
    ] {
        let (directory, path, _, lease, mut context, mut slice) =
            client_readonly_negative_fixture(&format!("client-input-{fault}"));
        if fault == "invented_literal" {
            slice["observations"][0]["value"] = json!("script-src *");
        }
        let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            panic!("ClientSide negative must fail before SDK")
        }));
        let _guard = ClientSideReadonlyTestGuard {
            root: directory.clone(),
            stop,
        };
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let db = db::open(&path).unwrap();
        let tokens = if fault == "insufficient_reserved_tokens" {
            1
        } else {
            8000
        };
        let child = scheduler::prepare_readonly_child(
            &db,
            &lease,
            AgentRole::ClientSide,
            "client_side_frozen_observations_ready",
            &slice,
            tokens,
        )
        .unwrap();
        if fault == "artifact_hash" {
            fs::write(
                context.target_dir.join("client-side-observed.html"),
                b"changed",
            )
            .unwrap();
        }
        let before = receipt_database_snapshot(&db);
        let error =
            multi_agent_child_round_transport(&context, &lease, &child, "独立只读配置专家", slice)
                .unwrap_err();
        let expected = match fault {
            "artifact_hash" => "client_side_artifact_hash_changed",
            "invented_literal" => "client_side_observation_not_in_artifact",
            _ => "client_side_model_reservation_insufficient",
        };
        assert!(error.contains(expected), "{fault}: {error}");
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(
            receipt_database_snapshot(&db),
            before,
            "full scope/rowid unchanged before I/O: {fault}"
        );
    }
}

#[test]
fn client_side_readonly_csp_absence_cannot_be_inferred_from_version_header_allowlist() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    let (directory, path, _, lease, mut context, mut slice) =
        client_readonly_negative_fixture("client-csp-allowlist-gap");
    let bytes=serde_json::to_vec(&json!({"response":{"responseHeaderNames":["content-type","content-security-policy"],"securityRelevantHeaders":[]}})).unwrap();
    fs::write(context.target_dir.join("client-side-observed.html"), &bytes).unwrap();
    slice["observations"][0]["kind"] = json!("http_csp_absent");
    slice["observations"][0]["value"] = json!("not_present");
    slice["observations"][0]["artifact"]["contentHash"] =
        json!(crate::agent_runtime::store::artifact_id(&bytes));
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        panic!("unknown captured header value cannot reach SDK")
    }));
    let _guard = ClientSideReadonlyTestGuard {
        root: directory,
        stop,
    };
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let db = db::open(&path).unwrap();
    let child = scheduler::prepare_readonly_child(
        &db,
        &lease,
        AgentRole::ClientSide,
        "client_side_frozen_observations_ready",
        &slice,
        8000,
    )
    .unwrap();
    let before = receipt_database_snapshot(&db);
    let error =
        multi_agent_child_round_transport(&context, &lease, &child, "只读已捕获配置", slice)
            .unwrap_err();
    assert!(error.contains("client_side_http_csp_ambiguous"), "{error}");
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(receipt_database_snapshot(&db), before);
}
