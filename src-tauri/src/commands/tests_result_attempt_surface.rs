#[test]
fn retired_attempt_rows_reject_preparation_without_mutation() {
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

    assert!(prepare_current_attempt_surface(&connection, "native-attempt", 2).is_err());

    let native_marker: Option<String> = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='agent-current-attempt:native-attempt'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(native_marker, None);
    let old_markers: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM app_settings WHERE key='strix-current-attempt:native-attempt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_markers, 1);
    let current_findings: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='native-attempt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(current_findings, 1);
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
fn native_attempt_marker_cannot_bypass_retired_data_rejection() {
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

    assert!(prepare_current_attempt_surface(&connection, "native-attempt", 2).is_err());

    let current: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id='native-attempt' AND record_key='current'", [], |row| row.get(0)).unwrap();
    assert_eq!(current, 1, "幂等调用不可删除本次已产生的结果");
    let old_markers: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM app_settings WHERE key='strix-current-attempt:native-attempt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_markers, 1);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

/// Preparation rejects retired data before mutating any current or old rows.
#[test]
fn resume_prepare_preserves_retired_checkpoints_and_learning_outcome() {
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

    assert!(prepare_current_attempt_surface(&connection, "resume-attempt", 2).is_err());

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
        vec!["frontend_recon".to_string(), "learning_outcome".to_string(), "strix_events:old".to_string(), "strix_run:old".to_string()],
        "未经批准不得删除旧检查点；扫描级学习结果与现行 recon 须保留"
    );
    assert!(prepare_current_attempt_surface(&connection, "resume-attempt", 2).is_err());
    let retained: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_checkpoints WHERE scan_id='resume-attempt' AND stage='learning_outcome'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, 1, "幂等准备也不能清除已记录的扫描级学习结果");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_surface_preparation_does_not_match_other_scan_signature_patterns() {
    let root = std::env::temp_dir().join(format!("oviraptor-surface-scope-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection.execute("INSERT INTO projects(id,name) VALUES(1,'test')", []).unwrap();
    connection.execute("INSERT INTO sentinel_scans(id,project_id,status,scan_type,attempt_count) VALUES('scan_%',1,'scanning','web',2)", []).unwrap();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status) VALUES('scan_%',2,'resume','scanning')", []).unwrap();
    connection.execute("INSERT INTO app_settings(key,value) VALUES('strix-result-signature:scan_AB:source','old')", []).unwrap();
    connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('scan_%','*','learning_outcome','{}'),('scan_%','*','frontend_recon','{}')", []).unwrap();
    for _ in 0..2 { prepare_current_attempt_surface(&connection, "scan_%", 2).unwrap(); }
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM app_settings WHERE key='strix-result-signature:scan_AB:source' AND value='old'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1, "scan IDs are exact data, never LIKE patterns");
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_checkpoints WHERE scan_id='scan_%'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 2);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn each_retired_surface_record_blocks_all_modes_before_current_writes() {
    for mode in ["initial", "fresh", "resume"] {
        for sql in [
            "INSERT INTO sentinel_findings(scan_id,stage,kind,record_key) VALUES('scope','strix','vulnerability','old')",
            "INSERT INTO sentinel_findings(scan_id,stage,kind,record_key) VALUES('scope','strix-coverage','coverage','old')",
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage) VALUES('scope','*','strix_run')",
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage) VALUES('scope','*','strix_events:old')",
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage) VALUES('scope','*','strix_coverage:old')",
            "INSERT INTO app_settings(key,value) VALUES('strix-current-attempt:scope','2')",
            "INSERT INTO app_settings(key,value) VALUES('strix-result-signature:scope','old')",
            "INSERT INTO app_settings(key,value) VALUES('strix-result-signature:scope:source','old')",
        ] {
            let root = std::env::temp_dir().join(format!("oviraptor-surface-reject-{}", Uuid::new_v4()));
            let db_path = db::initialize(&root).unwrap();
            let connection = db::open(&db_path).unwrap();
            connection.execute("INSERT INTO projects(id,name) VALUES(1,'test')", []).unwrap();
            connection.execute("INSERT INTO sentinel_scans(id,project_id,status,scan_type,attempt_count) VALUES('scope',1,'scanning','web',2)", []).unwrap();
            connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status) VALUES('scope',2,?1,'scanning')", [mode]).unwrap();
            connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage) VALUES('scope','*','learning_outcome')", []).unwrap();
            connection.execute("INSERT INTO sentinel_findings(scan_id,stage,kind,record_key) VALUES('scope','native-agent','vulnerability','current')", []).unwrap();
            connection.execute(sql, []).unwrap();
            let changes = connection.total_changes();
            assert_eq!(prepare_current_attempt_surface(&connection, "scope", 2).unwrap_err(), "retired_result_data_requires_confirmed_cleanup", "{mode}: {sql}");
            assert_eq!(connection.total_changes(), changes, "rejection must precede every write");
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn web_start_retired_result_refusal_rolls_back_publication_and_owned_files() {
    let (root, _, mut connection) = web_start_fixture();
    connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES('start-test','*','strix_run:old','{}')", []).unwrap();
    let before = web_start_snapshot(&connection);
    assert_eq!(run_web_start_fixture(&mut connection, &root, WebStartMode::Confirm).unwrap_err(), "retired_result_data_requires_confirmed_cleanup");
    assert_eq!(web_start_snapshot(&connection), before);
    assert!(!root.join("agent-jobs/start-test/attempt-0001").exists());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
