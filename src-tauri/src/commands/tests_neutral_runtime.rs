// §13 / Stage 3 — neutral runtime names, the agent-jobs directory, the native default
// and the frozen Strix resume.

#[test]
fn neutral_runtime_types_replace_the_strix_named_aliases() {
    // The four Stage 3 names must be the ones live code speaks.
    let mut budget = AgentBudgetSettings::from_json(&serde_json::json!({}));
    assert_eq!(budget.max_mode, "deep", "默认额度来自设置 JSON");
    budget.apply_web_policy(&serde_json::json!({"webModeCeiling": "standard"}));
    assert_eq!(budget.max_mode, "standard", "额度上限由策略夹取，走同一个中性类型");
    let env = ModelRuntimeEnv {
        llm: "openai/fixture-model".into(),
        api_key: "test-key".into(),
        api_base: "https://model.invalid/v1".into(),
        deployment: "local".into(),
        full_power: false,
        prompt_audit_mode: "off".into(),
    };
    assert_eq!(env.deployment, "local");
    assert_eq!(env.llm, "openai/fixture-model");
    let instruction = AgentInstruction::new("只读检查".to_string());
    assert_eq!(instruction.as_str(), "只读检查");
    assert!(!instruction.as_str().trim().is_empty());
    let input: WorkbenchScanInput = serde_json::from_value(serde_json::json!({
        "projectId": 1,
        "taskName": "源码扫描",
        "scanType": "code",
        "sourcePath": "/tmp/source",
        "instruction": "只看认证路径"
    }))
    .expect("WorkbenchScanInput 必须按历史 camelCase 字段名反序列化");
    assert_eq!(input.source_path, "/tmp/source");
    assert_eq!(input.instruction, "只看认证路径");
}

#[test]
fn new_scan_work_dirs_live_under_agent_jobs() {
    let root = std::env::temp_dir().join(format!("oviraptor-agent-jobs-{}", Uuid::new_v4()));
    let work = prepare_scan_work_dir(&root, "scan-1").unwrap();
    assert_eq!(
        work,
        root.join("agent-jobs").join("scan-1"),
        "新任务目录必须写在 agent-jobs 下"
    );
    assert!(work.is_dir(), "工作目录必须真的建出来");
    assert!(
        !root.join("strix-jobs").exists(),
        "新任务不得再创建 strix-jobs 目录"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn new_agent_runs_default_to_the_native_backend() {
    let root = std::env::temp_dir().join(format!("oviraptor-native-default-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(910,'默认')", []).unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) \
             VALUES('default-scan',910,'默认','completed','web')",
            [],
        )
        .unwrap();
    let default_value: Option<String> = connection
        .query_row(
            "SELECT dflt_value FROM pragma_table_info('agent_runs') WHERE name='backend'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        default_value.as_deref(),
        Some("'native'"),
        "agent_runs.backend 的建表默认值必须是 native"
    );
    connection
        .execute(
            "INSERT INTO agent_runs(id,scan_id) VALUES('implicit-backend','default-scan')",
            [],
        )
        .unwrap();
    let backend: String = connection
        .query_row(
            "SELECT backend FROM agent_runs WHERE id='implicit-backend'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(backend, "native", "未指明的新 run 必须落在 native");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// Retired rows stay unchanged on launch and must be rejected at the active boundary.
#[test]
fn startup_preserves_retired_rows_without_resuming_them() {
    let root = std::env::temp_dir().join(format!("oviraptor-seal-strix-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    seed_runs_for_sealing(&db_path);
    db::initialize(&root).expect("再次启动不得改变旧 run");
    let before: String = db::open(&db_path).unwrap().query_row(
        "SELECT status FROM agent_runs WHERE id='strix-running'", [], |row| row.get(0)
    ).unwrap();
    assert_eq!(before, "running", "启动不应再写入退役后端的 run");
    let connection = db::open(&db_path).unwrap();
    let status = |id: &str| {
        connection
            .query_row(
                "SELECT status || '|' || backend FROM agent_runs WHERE id=?1",
                [id],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default()
    };
    assert_eq!(status("strix-prepared"), "prepared|strix");
    assert_eq!(status("strix-running"), "running|strix");
    assert_eq!(
        status("strix-completed"),
        "terminal|strix",
        "已完成的 Strix 记录必须原样保留 provenance"
    );
    assert_eq!(status("native-running"), "running|native", "Native run 不受影响");
    assert!(retired_backend_resume_refusal(&connection, "seal-scan").unwrap().is_some());
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// A sealed run cannot be re-activated: neither by a status write nor by the
/// historical adapter re-opening the same lineage.
#[test]
fn a_sealed_run_cannot_become_active_again() {
    let root = std::env::temp_dir().join(format!("oviraptor-seal-reactive-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    seed_runs_for_sealing(&db_path);
    let connection = db::open(&db_path).unwrap();
    connection.execute("UPDATE agent_runs SET status='legacy_backend_removed' WHERE id='strix-running'", []).unwrap();
    let error = crate::agent_runtime::store::set_run_status(
        &connection,
        "strix-running",
        crate::agent_runtime::contract::AgentRunStatus::Running,
    )
    .expect_err("封口后的 run 不能再被置为 running");
    assert!(error.contains("legacy"), "错误要说明是封口导致：{error}");
    let status: String = connection
        .query_row(
            "SELECT status FROM agent_runs WHERE id='strix-running'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "legacy_backend_removed");
    let error = crate::agent_runtime::store::load_run(&connection, "strix-running")
        .expect_err("退役后端不能经运行时 reader 重新解析为可用后端");
    assert_eq!(error, "agent_runs_backend_unsupported");
    let report = crate::agent_runtime::runtime_adapter::BackendReport::new(
        "seal-scan",
        1,
        "https://example.invalid",
        crate::agent_runtime::contract::AgentBackendKind::LegacyRemoved,
    );
    let refusal = crate::agent_runtime::runtime_adapter::open_run_refusal(&connection, &report)
        .unwrap()
        .expect("必须拒绝把封口的 Strix lineage 注册成活动 run");
    assert!(refusal.contains("backend_not_executable"), "{refusal}");
    let native_report = crate::agent_runtime::runtime_adapter::BackendReport::new(
        "seal-scan", 1, "https://example.invalid",
        crate::agent_runtime::contract::AgentBackendKind::Native,
    );
    let refusal = crate::agent_runtime::runtime_adapter::open_run_refusal(&connection, &native_report)
        .unwrap().expect("改贴 Native 标签也不能重新打开已封口的 lineage");
    assert!(refusal.contains("后端已退役或不可识别"), "{refusal}");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resuming_a_retired_lineage_is_refused_while_native_still_resumes() {
    let root = std::env::temp_dir().join(format!("oviraptor-seal-resume-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    seed_runs_for_sealing(&db_path);
    let connection = db::open(&db_path).unwrap();
    let refusal = retired_backend_resume_refusal(&connection, "seal-scan")
        .unwrap()
        .expect("未经启动封口的退役 backend 也不能 resume");
    assert!(
        refusal.contains("fresh") || refusal.contains("全新"),
        "提示要告诉用户只能重新开始：{refusal}"
    );
    assert!(
        retired_backend_resume_refusal(&connection, "native-scan").unwrap().is_none(),
        "Native 任务的 resume 不受影响"
    );
    connection.execute("UPDATE agent_runs SET backend='native',status='legacy_backend_removed' WHERE id='strix-running'", []).unwrap();
    assert!(retired_backend_resume_refusal(&connection, "seal-scan").unwrap().is_some(), "退役状态不能通过改贴 Native 标签恢复");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resume_backend_lookup_failure_is_not_treated_as_safe() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    let error = retired_backend_resume_refusal(&connection, "missing-scan").unwrap_err();
    assert!(error.contains("拒绝恢复"), "查询失败须显式失败关闭：{error}");
}

/// §Stage 3 item 4 on an upgraded database: the column default may still read 'strix',
/// so what guarantees a Native default is the decision the code makes for a new run.
#[test]
fn an_unspecified_backend_decision_for_a_new_run_is_native() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-native-choice-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(912,'选择')", []).unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) \
             VALUES('choice-scan',912,'选择','scanning','web')",
            [],
        )
        .unwrap();
    drop(connection);
    let (choice, reason) = agent_backend_choice(
        &db_path,
        "choice-scan",
        1,
        "https://example.invalid",
        &serde_json::json!({}),
        true,
    );
    assert_eq!(
        choice,
        crate::agent_runtime::contract::AgentBackendKind::Native,
        "没有指定后端的新执行必须走 Native：{reason}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Gap 1: the sec_skills import upserts into `agent_skills`, so its conflict guard has
/// to read the same table. Before the fix the second import aborted the statement
/// because `strix_skills.builtin` does not exist in that statement's scope.
#[test]
fn import_sec_skill_knowledge_updates_the_same_table_and_spares_builtins() {
    let root = std::env::temp_dir().join(format!("oviraptor-sec-skill-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    let read = |name: &str| -> (String, String, i64, i64) {
        connection
            .query_row(
                "SELECT description, instructions, builtin, enabled FROM agent_skills WHERE name=?1",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    };

    upsert_sec_skill_package(&connection, "重复导入技能", "描述一", "第一版指令").unwrap();
    upsert_sec_skill_package(&connection, "重复导入技能", "描述二", "第二版指令").unwrap();
    let (description, instructions, builtin, _) = read("重复导入技能");
    assert_eq!(instructions, "第二版指令", "第二次导入必须更新内容");
    assert_eq!(description, "描述二");
    assert_eq!(builtin, 0);

    connection
        .execute(
            "INSERT INTO agent_skills(name,description,instructions,builtin,enabled) \
             VALUES('内置技能','原描述','原指令',1,0)",
            [],
        )
        .unwrap();
    upsert_sec_skill_package(&connection, "内置技能", "外来描述", "外来指令").unwrap();
    let (description, instructions, _, enabled) = read("内置技能");
    assert_eq!(instructions, "原指令", "内置技能不能被导入覆盖");
    assert_eq!(description, "原描述");
    assert_eq!(enabled, 0, "内置技能的停用状态也要保持");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// Runs used by the sealing and resume tests: one per interesting state.
fn seed_runs_for_sealing(db_path: &Path) {
    let connection = db::open(db_path).unwrap();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(911,'封口')", []).unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) \
             VALUES('seal-scan',911,'封口','paused','web'),('native-scan',911,'封口','paused','web')",
            [],
        )
        .unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,status)
            VALUES ('strix-prepared','seal-scan',1,'https://example.invalid','strix','prepared'),
                   ('strix-running','seal-scan',1,'https://other.invalid','strix','running'),
                   ('strix-completed','seal-scan',1,'https://done.invalid','strix','terminal'),
                   ('native-running','native-scan',1,'https://native.invalid','native','running');
            "#,
        )
        .unwrap();
    drop(connection);
}
#[test]
fn asset_summary_serializes_the_neutral_field_consumed_by_the_ui() {
    let summary = AssetSummary { sent_to_agent: 7, ..Default::default() };
    let value = serde_json::to_value(summary).unwrap();
    assert_eq!(value.get("sentToAgent").and_then(JsonValue::as_i64), Some(7));
    assert!(value.get("sentToStrix").is_none());
}
