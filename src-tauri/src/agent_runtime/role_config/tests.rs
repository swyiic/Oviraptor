/// The §8 execution-instance and collaboration-message views are kept beside the
/// draft type so the three concepts stay separate and comparable, but they are
/// only reachable from the contract test below: Stage 0 ships no scheduler or
/// mailbox consumer that could build them.
#[cfg(test)]
mod contract_views {
    use super::{CollaborationMessageRef, ExecutionInstanceRef};
    use crate::agent_runtime::contract::{AgentMessageKind, AgentRole};

    /// Map a live agent_runs row into an execution-instance view. This deliberately
    /// does not accept a RoleConfigDraft.
    pub fn execution_instance_from_run(
        run_id: &str,
        scan_id: &str,
        attempt_number: i64,
        role: AgentRole,
        status: &str,
        assignment_id: Option<String>,
    ) -> ExecutionInstanceRef {
        ExecutionInstanceRef {
            run_id: run_id.to_string(),
            scan_id: scan_id.to_string(),
            attempt_number,
            role: role.as_str().to_string(),
            status: status.to_string(),
            assignment_id,
        }
    }

    /// Map a mailbox row into a collaboration-message view.
    pub fn collaboration_message_ref(
        id: &str,
        run_id: &str,
        kind: AgentMessageKind,
        from_agent: &str,
        to_agent: &str,
        summary: &str,
    ) -> CollaborationMessageRef {
        CollaborationMessageRef {
            id: id.to_string(),
            run_id: run_id.to_string(),
            kind: kind.as_str().to_string(),
            from_agent: from_agent.to_string(),
            to_agent: to_agent.to_string(),
            summary: summary.to_string(),
        }
    }

    /// Refuse to treat a draft id as a run id or message id when looking up
    /// execution / collaboration surfaces.
    pub fn refuse_draft_as_execution(draft_id: &str, run_id: &str) -> Result<(), String> {
        if draft_id == run_id {
            return Err(format!("拒绝把角色配置草稿 `{draft_id}` 当作执行实例"));
        }
        Ok(())
    }

    pub fn refuse_draft_as_collaboration_message(
        draft_id: &str,
        message_id: &str,
    ) -> Result<(), String> {
        if draft_id == message_id {
            return Err(format!("拒绝把角色配置草稿 `{draft_id}` 当作协作消息"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::contract_views::*;
    use super::*;
    use crate::agent_runtime::contract::{AgentBackendKind, AgentMessageKind, AgentRunStatus};
    use crate::agent_runtime::store::{self, AgentRunRow};
    use uuid::Uuid;

    fn temp_db(tag: &str) -> (PathBuf, Connection) {
        let root = std::env::temp_dir().join(format!("oviraptor-role-{tag}-{}", Uuid::new_v4()));
        let path = crate::db::initialize(&root).unwrap();
        let connection = crate::db::open(&path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(8001,'RoleDraft')", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,scan_type,attempt_count) VALUES('role-scan',8001,'RoleDraft','scanning','测试','web',1)",
                [],
            )
            .unwrap();
        (root, connection)
    }

    #[test]
    fn loads_real_capability_directory_not_invented_names() {
        let dir = default_capability_directory();
        let bundles = load_capability_directory(&dir).expect("shipped directory");
        let ids: BTreeSet<_> = bundles.into_iter().map(|bundle| bundle.id).collect();
        assert!(ids.contains("evidence-read"));
        assert!(ids.contains("review-only"));
        assert!(ids.contains("frontend-map"));
        assert!(!ids.contains("shell-exec"));
        assert!(!ids.contains("arbitrary-tool"));
    }

    #[test]
    fn empty_or_missing_directory_is_rejected() {
        let missing =
            std::env::temp_dir().join(format!("oviraptor-caps-missing-{}", Uuid::new_v4()));
        assert!(load_capability_directory(&missing).is_err());
        let empty = std::env::temp_dir().join(format!("oviraptor-caps-empty-{}", Uuid::new_v4()));
        fs::create_dir_all(&empty).unwrap();
        assert!(load_capability_directory(&empty).is_err());
        let _ = fs::remove_dir_all(&empty);
    }

    #[test]
    fn draft_rejects_unknown_capability_and_stays_non_executable() {
        let (_root, connection) = temp_db("unknown-cap");
        let dir = default_capability_directory();
        let mut draft = RoleConfigDraft::new(
            "draft-unknown",
            "试写角色",
            AgentRole::DeepInvestigator,
            "只读分析",
            vec!["shell-exec".into()],
        );
        assert!(!draft.is_executable());
        assert_eq!(draft.status_label, DRAFT_STATUS_LABEL);
        let err = save_role_config_draft(&connection, &draft, &dir).unwrap_err();
        assert!(err.contains("不在真实目录"), "{err}");
        draft.capability_bundle_ids = vec!["evidence-read".into()];
        let saved = save_role_config_draft(&connection, &draft, &dir).unwrap();
        assert_eq!(saved.status, "draft");
        assert_eq!(saved.status_label, DRAFT_STATUS_LABEL);
        assert!(!saved.is_executable());
    }

    #[test]
    fn draft_instance_and_message_are_separate_tables() {
        let (_root, connection) = temp_db("separate");
        let dir = default_capability_directory();
        let draft = RoleConfigDraft::new(
            "draft-recon",
            "小王",
            AgentRole::SpaApiMapper,
            "整理接口",
            vec!["evidence-read".into(), "frontend-map".into()],
        );
        save_role_config_draft(&connection, &draft, &dir).unwrap();

        let mut run = AgentRunRow::new(
            "run-live-1",
            "role-scan",
            1,
            "https://app.example.invalid",
            AgentBackendKind::Native,
            AgentRole::Coordinator,
            "plan",
            "evidence",
        );
        run.status = AgentRunStatus::Running;
        store::create_run(&connection, &run).unwrap();

        store::append_message(
            &connection,
            "run-live-1",
            "coordinator",
            "spa_api_mapper",
            AgentMessageKind::GapProposed,
            "corr",
            "dedup-1",
            &serde_json::json!({"summary": "需要补证"}),
            &[],
        )
        .unwrap();
        let message_id: String = connection
            .query_row(
                "SELECT id FROM agent_messages WHERE dedup_key=?1",
                ["dedup-1"],
                |row| row.get(0),
            )
            .unwrap();

        let drafts = list_role_config_drafts(&connection).unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].id, "draft-recon");

        let draft_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM agent_role_config_drafts", [], |row| {
                row.get(0)
            })
            .unwrap();
        let run_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM agent_runs", [], |row| row.get(0))
            .unwrap();
        let message_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM agent_messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(draft_count, 1);
        assert_eq!(run_count, 1);
        assert_eq!(message_count, 1);

        // Draft id must not appear as a run or message.
        let as_run: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_runs WHERE id=?1",
                ["draft-recon"],
                |row| row.get(0),
            )
            .unwrap();
        let as_message: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_messages WHERE id=?1 OR dedup_key=?1",
                ["draft-recon"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(as_run, 0);
        assert_eq!(as_message, 0);

        let instance = execution_instance_from_run(
            &run.id,
            &run.scan_id,
            run.attempt_number,
            run.role,
            run.status.as_str(),
            None,
        );
        assert_ne!(instance.run_id, draft.id);
        refuse_draft_as_execution(&draft.id, &instance.run_id).unwrap();
        assert!(refuse_draft_as_execution(&draft.id, &draft.id).is_err());

        let message = collaboration_message_ref(
            &message_id,
            "run-live-1",
            AgentMessageKind::GapProposed,
            "coordinator",
            "spa_api_mapper",
            "需要补证",
        );
        assert_ne!(message.id, draft.id);
        assert!(refuse_draft_as_collaboration_message(&draft.id, &draft.id).is_err());
        refuse_draft_as_collaboration_message(&draft.id, &message.id).unwrap();
    }

    #[test]
    fn incompatible_role_capability_is_rejected() {
        let (_root, connection) = temp_db("incompat");
        let dir = default_capability_directory();
        let draft = RoleConfigDraft::new(
            "draft-bad-role",
            "错配",
            AgentRole::Coordinator,
            "不该拿复核包",
            vec!["review-only".into()],
        );
        let err = save_role_config_draft(&connection, &draft, &dir).unwrap_err();
        assert!(err.contains("不兼容"), "{err}");
    }

    #[test]
    fn unknown_role_cannot_be_silently_saved_as_coordinator() {
        let (_root, connection) = temp_db("unknown-role");
        let dir = default_capability_directory();
        let mut draft = RoleConfigDraft::new(
            "draft-unrecognized-role",
            "未知角色",
            AgentRole::Coordinator,
            "不应被升级为协调者",
            vec!["evidence-read".into()],
        );
        draft.role = "not_a_role".into();
        assert!(save_role_config_draft(&connection, &draft, &dir).is_err());
        let stored: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_role_config_drafts WHERE id=?1",
                [&draft.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, 0);
    }

    #[test]
    fn new_role_drafts_accept_only_canonical_names_not_legacy_aliases() {
        assert_eq!(parse_draft_role("spa_api_mapper").unwrap(), AgentRole::SpaApiMapper);
        for role in ["unknown", "evidence_triage", "COORDINATOR", " coordinator "] {
            assert!(parse_draft_role(role).is_err(), "{role}");
        }
        // Reading an old role alias remains a separate display compatibility
        // concern; a new draft never acquires authority from that alias.
        assert_eq!(AgentRole::try_parse("evidence_triage"), Some(AgentRole::SpaApiMapper));
    }
}
