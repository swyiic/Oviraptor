#[test]
fn startup_existing_native_mailbox_adds_recipient_columns_before_index() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-startup-native-mailbox-{}",
        Uuid::new_v4()
    ));
    fs::create_dir(&root).unwrap();
    let path = root.join("oviraptor.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TABLE agent_messages (
        id TEXT PRIMARY KEY,run_id TEXT NOT NULL,
        from_agent TEXT NOT NULL DEFAULT '',to_agent TEXT NOT NULL DEFAULT '',kind TEXT NOT NULL,
        correlation_id TEXT NOT NULL DEFAULT '',dedup_key TEXT NOT NULL DEFAULT '',
        payload_json TEXT NOT NULL DEFAULT '{}',artifact_refs_json TEXT NOT NULL DEFAULT '[]',
        created_at TEXT NOT NULL DEFAULT '',delivered_at TEXT NOT NULL DEFAULT '',acknowledged_at TEXT NOT NULL DEFAULT '',
        UNIQUE(run_id,dedup_key));
        INSERT INTO agent_messages(id,run_id,kind,payload_json,created_at,delivered_at)
        VALUES('saved-message','saved-native-run','result','{\"summary\":\"retain original\"}','2026-09-01','2026-09-02');").unwrap();
    let original:(String,String,String)=connection.query_row(
        "SELECT payload_json,created_at,delivered_at FROM agent_messages WHERE id='saved-message'",[],
        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    drop(connection);
    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    for column in [
        "root_run_id",
        "from_run_id",
        "to_run_id",
        "assignment_id",
        "evidence_revision",
        "delivery_attempts",
    ] {
        assert!(column_exists(&connection, "agent_messages", column).unwrap());
    }
    assert_eq!(connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_agent_messages_recipient'",[],|row|row.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(connection.query_row("SELECT payload_json,created_at,delivered_at FROM agent_messages WHERE id='saved-message'",[],
        |row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).unwrap(),original);
    drop(connection);
    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(connection.query_row("SELECT payload_json,created_at,delivered_at FROM agent_messages WHERE id='saved-message'",[],
        |row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).unwrap(),original);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
