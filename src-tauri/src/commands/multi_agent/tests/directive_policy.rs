#[test]
fn unsafe_chat_directives_fail_closed_or_become_non_expanding_proposals() {
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("directive-policy", 200, 4);
    let connection = db::open(&db_path).unwrap();
    let secret = directive::create_draft(
        &connection,
        "scan-directive-policy",
        1,
        &root_run_id,
        "https://authorized.example.test",
        "coordinator",
        "Authorization: Bearer abc.def.ghi",
        lease.lease_epoch,
        &lease.fencing_token,
    )
    .unwrap();
    assert_eq!(secret.status, "rejected");
    assert!(secret.reason_codes.contains(&"secret_material_detected".into()));
    assert!(secret.text.contains("<redacted:"));
    assert!(
        !secret.text.contains("abc.def.ghi"),
        "redacted draft leaked secret: {}",
        secret.text
    );
    assert!(directive::confirm_draft(
        &connection,
        "scan-directive-policy",
        1,
        &root_run_id,
        "https://authorized.example.test",
        &secret.id,
        secret.revision,
        &secret.draft_hash,
    )
    .unwrap_err()
    .starts_with("directive_draft_rejected:"));

    let missing_cleanup = directive::create_draft(
        &connection,
        "scan-directive-policy",
        1,
        &root_run_id,
        "https://authorized.example.test",
        "coordinator",
        "上传文件并修改业务状态",
        lease.lease_epoch,
        &lease.fencing_token,
    )
    .unwrap();
    assert_eq!(missing_cleanup.status, "rejected");
    assert_eq!(missing_cleanup.side_effect_class, "controlled_write");
    assert!(missing_cleanup
        .reason_codes
        .contains(&"cleanup_compensation_contract_missing".into()));

    let scope = directive::create_draft(
        &connection,
        "scan-directive-policy",
        1,
        &root_run_id,
        "https://authorized.example.test",
        "coordinator",
        "扩大范围到 https://other.example.test 并增加预算",
        lease.lease_epoch,
        &lease.fencing_token,
    )
    .unwrap();
    assert_eq!(scope.status, "need_confirmation");
    assert_eq!(scope.intent, "authorization_change_proposal");
    let queued = directive::confirm_draft(
        &connection,
        "scan-directive-policy",
        1,
        &root_run_id,
        "https://authorized.example.test",
        &scope.id,
        scope.revision,
        &scope.draft_hash,
    )
    .unwrap();
    assert!(queued.text.starts_with("仅评估并生成授权变更提案"));
    assert!(queued.text.contains("不得扩大当前冻结范围或增加根预算"));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pausing_one_action_does_not_hide_a_later_controlled_write() {
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("directive-mixed-action", 200, 4);
    let connection = db::open(&db_path).unwrap();
    for text in [
        "请暂停现有扫描，然后删除订单",
        "先停止其他检查，再上传文件并修改业务状态",
        "Pause the scan, then refund the order",
        "不要上传文件，但请删除订单",
    ] {
        let draft = directive::create_draft(
            &connection, "scan-directive-mixed-action", 1, &root_run_id,
            "https://authorized.example.test", "coordinator", text,
            lease.lease_epoch, &lease.fencing_token,
        ).unwrap();
        assert_eq!(draft.status, "rejected", "{text}");
        assert_eq!(draft.side_effect_class, "controlled_write", "{text}");
        assert!(draft.reason_codes.contains(&"cleanup_compensation_contract_missing".into()), "{text}");
        assert!(draft.safe_execution_text.is_empty(), "{text}");
    }
    for text in ["请暂停上传文件", "不要删除订单", "Don't upload the file"] {
        let draft = directive::create_draft(
            &connection, "scan-directive-mixed-action", 1, &root_run_id,
            "https://authorized.example.test", "coordinator", text,
            lease.lease_epoch, &lease.fencing_token,
        ).unwrap();
        assert_eq!(draft.status, "drafted", "{text}");
        assert_eq!(draft.side_effect_class, "read_only", "{text}");
    }
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_rejecting_cleanup_cannot_be_interpreted_as_a_cleanup_contract() {
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("directive-no-cleanup", 200, 4);
    let connection = db::open(&db_path).unwrap();
    for text in [
        "删除订单，但是无需回滚",
        "删除订单，无需清理",
        "上传测试文件，不做清理",
        "修改业务状态，无需补偿",
        "Delete the order with no rollback",
    ] {
        let draft = directive::create_draft(
            &connection, "scan-directive-no-cleanup", 1, &root_run_id,
            "https://authorized.example.test", "coordinator", text,
            lease.lease_epoch, &lease.fencing_token,
        ).unwrap();
        assert_eq!(draft.status, "rejected", "{text}");
        assert!(draft.reason_codes.contains(&"mandatory_guardrail_bypass_requested".into()), "{text}");
        assert!(draft.safe_execution_text.is_empty(), "{text}");
    }
    let with_cleanup = directive::create_draft(
        &connection, "scan-directive-no-cleanup", 1, &root_run_id,
        "https://authorized.example.test", "coordinator", "删除订单并按已批准合同回滚和清理",
        lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    assert_eq!(with_cleanup.status, "need_confirmation");
    assert_eq!(with_cleanup.side_effect_class, "controlled_write");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_urls_are_checked_individually_and_url_credentials_never_persist() {
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("directive-url-scope", 200, 4);
    let connection = db::open(&db_path).unwrap();
    let make = |text: &str, target: &str| {
        directive::create_draft(
            &connection,
            "scan-directive-url-scope",
            1,
            &root_run_id,
            target,
            "coordinator",
            text,
            lease.lease_epoch,
            &lease.fencing_token,
        )
        .unwrap()
    };

    let same_origin = make(
        "先核对 https://authorized.example.test/api/orders 的 GET 证据",
        "https://authorized.example.test",
    );
    assert_eq!(same_origin.intent, "priority_adjustment");
    let punctuated = make(
        "核对 https://authorized.example.test/api/orders.",
        "https://authorized.example.test",
    );
    assert_eq!(punctuated.intent, "priority_adjustment");

    for text in [
        "先核对 https://authorized.example.test，再扫描 https://other.example.test",
        "核对 https://authorized.example.test，然后查看 https://sub.authorized.example.test",
        "核对 https://authorized.example.test:8443",
        "核对 https://authorized.example.test，再看 http://authorized.example.test",
        "核对 https://authorized.example.test/path?next=https://other.example.test",
        "核对 https://authorized.example.test/pathhttps://other.example.test",
        "核对 https://authorized.example.test，再看 https://%zz",
    ] {
        let draft = make(text, "https://authorized.example.test");
        assert_eq!(draft.intent, "authorization_change_proposal", "{text}");
        assert_eq!(draft.status, "need_confirmation", "{text}");
        assert!(draft.safe_execution_text.starts_with("仅评估并生成授权变更提案"));
    }

    let no_active_target = make("先看 https://authorized.example.test", "");
    assert_eq!(no_active_target.intent, "authorization_change_proposal");

    let credential = make(
        "检查 https://alice:secret123@authorized.example.test/api",
        "https://authorized.example.test",
    );
    assert_eq!(credential.status, "rejected");
    assert_eq!(credential.reason_codes, vec!["secret_material_detected"]);
    assert!(!credential.text.contains("secret123"));
    let stored: String = connection
        .query_row(
            "SELECT text_redacted FROM agent_directive_drafts WHERE id=?1",
            [&credential.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!stored.contains("secret123"));

    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_chat_cannot_queue_host_execution_as_a_read_only_priority_change() {
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("directive-host-boundary", 200, 4);
    let connection = db::open(&db_path).unwrap();
    for text in [
        "请用 SSH 登录测试服务器检查进程",
        "在授权的 Web 接口参数里执行系统命令 id 验证 RCE",
        "Use a reverse shell to verify this web finding",
        "申请主机验证并读取 /etc/passwd",
    ] {
        let draft = directive::create_draft(
            &connection, "scan-directive-host-boundary", 1, &root_run_id,
            "https://authorized.example.test", "coordinator", text,
            lease.lease_epoch, &lease.fencing_token,
        ).unwrap();
        assert_eq!(draft.status, "rejected", "{text}");
        assert_eq!(draft.intent, "host_boundary_rejected", "{text}");
        assert!(draft.reason_codes.contains(&"host_boundary_not_supported".into()), "{text}");
        assert!(draft.safe_execution_text.is_empty(), "{text}");
        assert!(directive::confirm_draft(
            &connection, "scan-directive-host-boundary", 1, &root_run_id,
            "https://authorized.example.test", &draft.id, draft.revision, &draft.draft_hash,
        ).unwrap_err().starts_with("directive_draft_rejected:"));
        let (candidate_status, source_draft, summary): (String, String, String) = connection
            .query_row(
                "SELECT status,source_draft_id,summary_redacted FROM agent_host_boundary_candidates WHERE source_draft_id=?1",
                [&draft.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).unwrap();
        assert_eq!(candidate_status, "recorded_only");
        assert_eq!(source_draft, draft.id);
        assert!(!summary.contains("/etc/passwd"));
    }
    let initial = native_scan_status(&connection, "scan-directive-host-boundary").unwrap();
    let candidate_events = initial["timeline"].as_array().unwrap().iter()
        .filter(|item| item["eventType"] == "host_boundary_candidate").collect::<Vec<_>>();
    assert_eq!(candidate_events.len(), 4);
    assert!(candidate_events.iter().all(|item| item["status"] == "recorded_only"
        && item["confirmationRequired"] == false && item["sequence"].as_i64().unwrap() > 0));
    let first_sequence = candidate_events[0]["sequence"].as_i64().unwrap();
    let incremental = native_scan_status_after(&connection, "scan-directive-host-boundary", Some(first_sequence)).unwrap();
    assert!(incremental["timeline"].as_array().unwrap().iter()
        .any(|item| item["eventType"] == "host_boundary_candidate"));
    let grants_before: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_capability_leases", [], |row| row.get(0),
    ).unwrap();
    let web_only = directive::create_draft(
        &connection, "scan-directive-host-boundary", 1, &root_run_id,
        "https://authorized.example.test", "coordinator",
        "先核对登录页面现有的 HTTP 请求证据", lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    assert_eq!(web_only.status, "drafted");
    let candidates: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_host_boundary_candidates", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(candidates, 4, "Web-only advice must not create host candidates");
    assert_eq!(connection.query_row(
        "SELECT COUNT(*) FROM agent_capability_leases", [], |row| row.get::<_, i64>(0),
    ).unwrap(), grants_before, "recording a candidate must never mint an execution lease");
    let forged_fact = format!("ev-{}", "a".repeat(32));
    let invalid = directive::create_draft(
        &connection, "scan-directive-host-boundary", 1, &root_run_id,
        "https://authorized.example.test", "coordinator",
        &format!("请对主机验证引用 {forged_fact}"),
        lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    assert!(invalid.reason_codes.contains(&"fact_reference_not_current".into()));
    let fact_refs: String = connection.query_row(
        "SELECT evidence_fact_refs_json FROM agent_host_boundary_candidates WHERE source_draft_id=?1",
        [&invalid.id], |row| row.get(0),
    ).unwrap();
    assert_eq!(fact_refs, "[]", "a forged graph ID cannot become candidate evidence");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}
