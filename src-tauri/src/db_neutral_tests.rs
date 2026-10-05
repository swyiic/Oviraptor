// Native knowledge stays independent of retired backend storage.

#[test]
fn fresh_database_creates_only_native_knowledge_tables() {
    let root = std::env::temp_dir().join(format!("oviraptor-native-schema-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = Connection::open(&path).unwrap();
    for table in ["agent_skills", "agent_knowledge_entries", "agent_learning_candidates"] {
        let exists: i64 = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table], |row| row.get(0)
        ).unwrap();
        assert_eq!(exists, 1, "缺少 Native 知识表 {table}");
    }
    for table in ["strix_skills", "strix_knowledge_entries", "strix_learning_candidates"] {
        let exists: i64 = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table], |row| row.get(0)
        ).unwrap();
        assert_eq!(exists, 0, "新数据库不能创建退役表 {table}");
    }
    let (builtin, enabled): (i64, i64) = connection.query_row(
        "SELECT builtin,enabled FROM agent_skills WHERE name='业务前端深度分析'",
        [], |row| Ok((row.get(0)?, row.get(1)?))
    ).unwrap();
    assert_eq!((builtin, enabled), (1, 1));
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn startup_does_not_import_or_mutate_retired_knowledge() {
    let root = std::env::temp_dir().join(format!("oviraptor-native-no-import-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(
        "CREATE TABLE strix_skills(id INTEGER PRIMARY KEY, name TEXT, instructions TEXT);\
         CREATE TABLE strix_knowledge_entries(id INTEGER PRIMARY KEY, title TEXT);\
         CREATE TABLE strix_learning_candidates(id INTEGER PRIMARY KEY, title TEXT);\
         INSERT INTO strix_skills VALUES(71,'旧技能','历史原文');\
         INSERT INTO strix_knowledge_entries VALUES(72,'旧知识');\
         INSERT INTO strix_learning_candidates VALUES(73,'旧候选');"
    ).unwrap();
    drop(connection);
    initialize(&root).unwrap();
    let connection = Connection::open(&path).unwrap();
    for (legacy, native, id) in [
        ("strix_skills", "agent_skills", 71),
        ("strix_knowledge_entries", "agent_knowledge_entries", 72),
        ("strix_learning_candidates", "agent_learning_candidates", 73),
    ] {
        let old: i64 = connection.query_row(
            &format!("SELECT COUNT(*) FROM {legacy} WHERE id=?1"), [id], |row| row.get(0)
        ).unwrap();
        let imported: i64 = connection.query_row(
            &format!("SELECT COUNT(*) FROM {native} WHERE id=?1"), [id], |row| row.get(0)
        ).unwrap();
        assert_eq!((old, imported), (1, 0), "启动不得复制或清理 {legacy}");
    }
    let marker: i64 = connection.query_row(
        "SELECT COUNT(*) FROM app_settings WHERE key='neutral_knowledge_migration'",
        [], |row| row.get(0)
    ).unwrap();
    assert_eq!(marker, 0, "不能推进退役数据导入水位");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}
