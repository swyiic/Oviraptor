#[test]
fn matching_legacy_attempt_marker_does_not_skip_native_surface_migration() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-legacy-attempt-marker-{}",
        Uuid::new_v4()
    ));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
        .unwrap();
    connection.execute("INSERT INTO sentinel_scans(id,project_id,status,scan_type,attempt_count) VALUES('native-attempt',1,'scanning','web',2)", []).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status) VALUES('native-attempt',2,'fresh','scanning')", []).unwrap();
    connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) VALUES('native-attempt','https://example.invalid','strix','vulnerability','old','old')", []).unwrap();
    connection.execute("INSERT INTO sentinel_validations(scan_id,url,finding_key,verdict) VALUES('native-attempt','https://example.invalid','old','confirmed')", []).unwrap();
    connection.execute("INSERT INTO app_settings(key,value) VALUES('strix-current-attempt:native-attempt','2')", []).unwrap();

    prepare_current_attempt_surface(&connection, "native-attempt", 2).unwrap();

    let native_marker: Option<String> = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='agent-current-attempt:native-attempt'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(native_marker.as_deref(), Some("2"));
    let old_markers: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM app_settings WHERE key='strix-current-attempt:native-attempt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_markers, 0);
    let current_findings: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='native-attempt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(current_findings, 0);
    let confirmed_history: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_validations WHERE scan_id='native-attempt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(confirmed_history, 1, "旧的人工作业记录不可随当前投影删除");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_attempt_marker_retains_current_results_while_retiring_legacy_marker() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-native-attempt-idempotent-{}",
        Uuid::new_v4()
    ));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
        .unwrap();
    connection.execute("INSERT INTO sentinel_scans(id,project_id,status,scan_type,attempt_count) VALUES('native-attempt',1,'scanning','web',2)", []).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status) VALUES('native-attempt',2,'fresh','scanning')", []).unwrap();
    connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) VALUES('native-attempt','https://example.invalid','frontend-recon','api','current','current')", []).unwrap();
    connection.execute("INSERT INTO app_settings(key,value) VALUES('agent-current-attempt:native-attempt','2'),('strix-current-attempt:native-attempt','2')", []).unwrap();

    prepare_current_attempt_surface(&connection, "native-attempt", 2).unwrap();

    let current: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='native-attempt' AND record_key='current'", [], |row| row.get(0)).unwrap();
    assert_eq!(current, 1, "幂等调用不可删除本次已产生的结果");
    let old_markers: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM app_settings WHERE key='strix-current-attempt:native-attempt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_markers, 0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

/// Loop4：锁定 resume 路径的 checkpoint 清理语义（`result_ingestion_runs.rs:99`）。
/// 该行此前零覆盖：旧 `strix_*` 检查点必须清除；现行 `frontend_recon` 检查点
/// 必须保留。`learning_outcome` 当前同样被清除——写端
/// （`knowledge_learning_candidates.rs:242`）把它当作扫描级耐久结果，而读端
/// 准备函数把它当作轮次内临时结果，两处注释互相矛盾，本测试先锁定现状，
/// 耐久性裁决记为待人工确认（见审计 doc），本轮不改语义。
#[test]
fn resume_prepare_clears_retired_checkpoints_but_keeps_current_ones() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-resume-checkpoints-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection
        .execute("INSERT INTO projects(id,name) VALUES(1,'test')", [])
        .unwrap();
    connection.execute("INSERT INTO sentinel_scans(id,project_id,status,scan_type,attempt_count) VALUES('resume-attempt',1,'scanning','web',2)", []).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status) VALUES('resume-attempt',2,'resume','scanning')", []).unwrap();
    for stage in [
        "strix_run:old",
        "strix_events:old",
        "learning_outcome",
        "frontend_recon",
    ] {
        connection.execute(
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('resume-attempt','*',?1,'{}')",
            [stage],
        ).unwrap();
    }

    prepare_current_attempt_surface(&connection, "resume-attempt", 2).unwrap();

    let remaining: Vec<String> = connection
        .prepare(
            "SELECT stage FROM sentinel_checkpoints WHERE scan_id='resume-attempt' ORDER BY stage",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        remaining,
        vec!["frontend_recon".to_string()],
        "旧检查点须清除，现行检查点须保留；learning_outcome 现状随旧行清除（待裁决）"
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
