#[test]
fn human_directive_requires_a_matching_confirmed_draft_before_queueing() {
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("directive-confirm", 200, 4);
    let connection = db::open(&db_path).unwrap();
    let draft = directive::create_draft(
        &connection,
        "scan-directive-confirm",
        1,
        &root_run_id,
        "https://authorized.example.test",
        "coordinator",
        "@Mapper 先评估 G-17，暂停目录发现",
        lease.lease_epoch,
        &lease.fencing_token,
    )
    .unwrap();
    assert_eq!(draft.status, "drafted");
    assert_eq!(draft.intent, "agent_proposal_request");
    assert!(draft.requested_roles.contains(&"spa_api_mapper".into()));
    // A conversational gap label is not a verified evidence-graph fact.
    assert!(draft.referenced_fact_ids.is_empty());
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM agent_user_directives WHERE scan_id=?1",
                ["scan-directive-confirm"],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        directive::confirm_draft(
            &connection,
            "scan-directive-confirm",
            1,
            &root_run_id,
            "https://authorized.example.test",
            &draft.id,
            draft.revision + 1,
            &draft.draft_hash,
        )
        .unwrap_err(),
        "directive_draft_stale_revision"
    );
    let confirmed = directive::confirm_draft(
        &connection,
        "scan-directive-confirm",
        1,
        &root_run_id,
        "https://authorized.example.test",
        &draft.id,
        draft.revision,
        &draft.draft_hash,
    )
    .unwrap();
    assert_eq!(confirmed.status, "pending");
    assert!(confirmed.text.contains("仅请求指定角色评估并提出提案"));
    let replay = directive::confirm_draft(
        &connection,
        "scan-directive-confirm",
        1,
        &root_run_id,
        "https://authorized.example.test",
        &draft.id,
        draft.revision,
        &draft.draft_hash,
    )
    .unwrap();
    assert_eq!(replay.id, confirmed.id);
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM agent_user_directives WHERE source_draft_id=?1",
                [&draft.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn chat_fact_references_require_current_root_evidence_and_revalidate_on_confirmation() {
    use crate::agent_runtime::evidence_graph::{
        contract::{EvidenceNode, EvidenceNodeKind, EvidenceProvenance},
        store::insert_evidence_node,
    };
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("directive-facts", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let fact_id = format!(
        "ev-{}",
        &crate::agent_runtime::store::stable_hash("directive-fact")[..32]
    );
    let missing = directive::create_draft(
        &connection, &lease.scan_id, 1, &root_run_id, &lease.target_key,
        "coordinator", &format!("请复核 {fact_id}"), lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    assert_eq!(missing.status, "rejected");
    assert_eq!(missing.referenced_fact_ids, vec![fact_id.clone()]);
    assert!(missing.reason_codes.contains(&"fact_reference_not_current".into()));

    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentRole, AgentRunStatus},
        store::{self, AgentRunRow},
    };
    let foreign_root = "foreign-directive-facts";
    let mut foreign_run = AgentRunRow::new(
        foreign_root, &lease.scan_id, 1, &lease.target_key,
        AgentBackendKind::Native, AgentRole::Coordinator, "plan", "evidence",
    );
    foreign_run.root_run_id = foreign_root.into();
    foreign_run.status = AgentRunStatus::Running;
    store::create_run(&connection, &foreign_run).unwrap();
    let foreign_id = "ev-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    insert_evidence_node(&connection, &EvidenceNode {
        id: foreign_id.into(), root_run_id: foreign_root.into(), revision: 1,
        kind: EvidenceNodeKind::RequestRecord, provenance: EvidenceProvenance::Observed,
        natural_key_hash: store::stable_hash("foreign-directive-fact"),
        payload: serde_json::json!({"summary":"other root"}), artifact_refs: vec![],
        created_by_run_id: foreign_root.into(), supersedes_id: String::new(), created_at: String::new(),
    }).unwrap();
    let foreign = directive::create_draft(
        &connection, &lease.scan_id, 1, &root_run_id, &lease.target_key,
        "coordinator", &format!("请复核 {foreign_id}"), lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    assert_eq!(foreign.status, "rejected");

    let hypothesis_id = "ev-cccccccccccccccccccccccccccccccc";
    insert_evidence_node(&connection, &EvidenceNode {
        id: hypothesis_id.into(), root_run_id: root_run_id.clone(), revision: 1,
        kind: EvidenceNodeKind::Hypothesis, provenance: EvidenceProvenance::Observed,
        natural_key_hash: store::stable_hash("chat-hypothesis-is-not-a-fact"),
        payload: serde_json::json!({"proposal":"needs verification"}), artifact_refs: vec![],
        created_by_run_id: root_run_id.clone(), supersedes_id: String::new(), created_at: String::new(),
    }).unwrap();
    let hypothesis = directive::create_draft(
        &connection, &lease.scan_id, 1, &root_run_id, &lease.target_key,
        "coordinator", &format!("请按事实 {hypothesis_id} 继续"), lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    assert_eq!(hypothesis.status, "rejected");

    let fact = EvidenceNode {
        id: fact_id.clone(), root_run_id: root_run_id.clone(), revision: 1,
        kind: EvidenceNodeKind::RequestRecord, provenance: EvidenceProvenance::Observed,
        natural_key_hash: crate::agent_runtime::store::stable_hash("directive-fact"),
        payload: serde_json::json!({"summary":"observed"}), artifact_refs: vec![],
        created_by_run_id: root_run_id.clone(), supersedes_id: String::new(), created_at: String::new(),
    };
    insert_evidence_node(&connection, &fact).unwrap();
    let draft = directive::create_draft(
        &connection, &lease.scan_id, 1, &root_run_id, &lease.target_key,
        "coordinator", &format!("请复核 {fact_id}"), lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    assert_eq!(draft.status, "drafted");
    assert_eq!(draft.referenced_fact_ids, vec![fact_id.clone()]);
    let confirmed = directive::confirm_draft(
        &connection, &lease.scan_id, 1, &root_run_id, &lease.target_key,
        &draft.id, draft.revision, &draft.draft_hash,
    ).unwrap();
    assert_eq!(confirmed.status, "pending");

    let pending = directive::create_draft(
        &connection, &lease.scan_id, 1, &root_run_id, &lease.target_key,
        "coordinator", &format!("再次复核 {fact_id}"), lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    let successor = EvidenceNode {
        id: "ev-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        revision: 3, supersedes_id: fact_id.clone(),
        natural_key_hash: crate::agent_runtime::store::stable_hash("directive-fact-successor"),
        ..fact
    };
    insert_evidence_node(&connection, &successor).unwrap();
    assert_eq!(
        directive::confirm_draft(
            &connection, &lease.scan_id, 1, &root_run_id, &lease.target_key,
            &pending.id, pending.revision, &pending.draft_hash,
        ).unwrap_err(),
        "directive_fact_reference_not_current"
    );
    assert!(directive::claim_pending_directives(&connection, &lease, 20).unwrap().is_empty());
    let rejected: (String, String) = connection.query_row(
        "SELECT status,rejection_code FROM agent_user_directives WHERE id=?1",
        [&confirmed.id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(rejected, ("rejected".into(), "directive_fact_reference_not_current".into()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn human_directive_thread_must_belong_to_the_current_root() {
    use crate::agent_runtime::multi_agent::directive;
    use crate::agent_runtime::{contract::{AgentBackendKind, AgentRole, AgentRunStatus}, store::{self, AgentRunRow}};

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("directive-thread", 200, 4);
    let connection = db::open(&db_path).unwrap();
    let target = "https://authorized.example.test";
    let denied = directive::create_draft_in_thread(
        &connection, "scan-directive-thread", 1, &root_run_id, target,
        "coordinator", "优先复核这个候选", "review-gap:foreign:1",
        lease.lease_epoch, &lease.fencing_token,
    );
    assert_eq!(denied.unwrap_err(), "directive_thread_not_in_current_root");
    let mut other = AgentRunRow::new(
        "other-directive-root", "scan-directive-thread", 1, target,
        AgentBackendKind::Native, AgentRole::Coordinator, "plan", "evidence",
    );
    other.root_run_id = other.id.clone();
    other.status = AgentRunStatus::Running;
    store::create_run(&connection, &other).unwrap();
    connection.execute(
        "INSERT INTO agent_messages(id,run_id,root_run_id,from_agent,to_agent,kind,correlation_id,dedup_key) \
         VALUES('foreign-thread-message',?1,?1,'coordinator','reviewer','review_request','real-foreign-thread','foreign-dedup')",
        [&other.id],
    ).unwrap();
    assert_eq!(directive::create_draft_in_thread(
        &connection, "scan-directive-thread", 1, &root_run_id, target,
        "coordinator", "优先复核这个候选", "real-foreign-thread",
        lease.lease_epoch, &lease.fencing_token,
    ).unwrap_err(), "directive_thread_not_in_current_root");
    let draft = directive::create_draft_in_thread(
        &connection, "scan-directive-thread", 1, &root_run_id, target,
        "coordinator", "优先复核当前目标", target,
        lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    assert_eq!(draft.thread_key, target);
    directive::confirm_draft(
        &connection, "scan-directive-thread", 1, &root_run_id, target,
        &draft.id, draft.revision, &draft.draft_hash,
    ).unwrap();
    let stored: (String, String) = connection.query_row(
        "SELECT d.thread_key,u.thread_key FROM agent_directive_drafts d \
         JOIN agent_user_directives u ON u.source_draft_id=d.id WHERE d.id=?1",
        [&draft.id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(stored, (target.into(), target.into()));
    let team = directive::create_draft(
        &connection, "scan-directive-thread", 1, &root_run_id, target,
        "coordinator", "优先复核当前目标", lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    connection.execute(
        "UPDATE agent_directive_drafts SET thread_key=?1 WHERE id=?2",
        rusqlite::params![target, team.id],
    ).unwrap();
    assert_eq!(directive::confirm_draft(
        &connection, "scan-directive-thread", 1, &root_run_id, target,
        &team.id, team.revision, &team.draft_hash,
    ).unwrap_err(), "directive_draft_integrity_failed");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pre_thread_binding_draft_hash_can_only_confirm_in_team_thread() {
    use crate::agent_runtime::multi_agent::directive;

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("legacy-thread-hash", 200, 4);
    let connection = db::open(&db_path).unwrap();
    let target = "https://authorized.example.test";
    let draft = directive::create_draft(
        &connection, "scan-legacy-thread-hash", 1, &root_run_id, target,
        "coordinator", "优先复核当前目标", lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    // Reconstruct the persisted pre-migration hash without the threadKey
    // member; this is the shape already stored in users' older databases.
    let old_material = serde_json::json!({
        "sourceMessageId": draft.source_message_id,
        "scanId": draft.scan_id,
        "attemptNumber": draft.attempt_number,
        "rootRunId": draft.root_run_id,
        "targetKey": draft.target_key,
        "recipientRole": draft.recipient_role,
        "text": draft.text,
        "intent": draft.intent,
        "requestedRoles": draft.requested_roles,
        "referencedFactIds": draft.referenced_fact_ids,
        "requestedContracts": draft.requested_contracts,
        "priorityChanges": draft.priority_changes,
        "proposedScopeChange": draft.proposed_scope_change,
        "estimatedTokens": draft.estimated_tokens,
        "estimatedRequests": draft.estimated_requests,
        "sideEffectClass": draft.side_effect_class,
        "requiredApprovals": draft.required_approvals,
        "validationResult": draft.validation_result,
        "reasonCodes": draft.reason_codes,
        "coordinatorDecision": draft.coordinator_decision,
        "safeExecutionText": draft.safe_execution_text,
        "revision": draft.revision,
        "boundLeaseEpoch": lease.lease_epoch,
        "boundFencingToken": lease.fencing_token,
    });
    let old_hash = crate::agent_runtime::store::stable_hash(&old_material.to_string());
    connection.execute(
        "UPDATE agent_directive_drafts SET draft_hash=?1 WHERE id=?2",
        rusqlite::params![old_hash, draft.id],
    ).unwrap();
    assert_eq!(directive::confirm_draft(
        &connection, "scan-legacy-thread-hash", 1, &root_run_id, target,
        &draft.id, draft.revision, &old_hash,
    ).unwrap().status, "pending");
    let another = directive::create_draft(
        &connection, "scan-legacy-thread-hash", 1, &root_run_id, target,
        "coordinator", "优先复核当前目标", lease.lease_epoch, &lease.fencing_token,
    ).unwrap();
    let mut another_old_material = old_material;
    another_old_material["sourceMessageId"] = serde_json::json!(another.source_message_id);
    let another_old_hash = crate::agent_runtime::store::stable_hash(&another_old_material.to_string());
    connection.execute(
        "UPDATE agent_directive_drafts SET draft_hash=?1,thread_key=?2 WHERE id=?3",
        rusqlite::params![another_old_hash, target, another.id],
    ).unwrap();
    assert_eq!(directive::confirm_draft(
        &connection, "scan-legacy-thread-hash", 1, &root_run_id, target,
        &another.id, another.revision, &another_old_hash,
    ).unwrap_err(), "directive_draft_integrity_failed");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

