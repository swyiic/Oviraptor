// Only reconstruct the previous Native declarations in a temporary fixture.
pub(super) fn namespace_restore_native_declaration(db: &rusqlite::Connection, table: &str) {
    let mut sql: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )
        .unwrap();
    if table == "agent_specialist_calls" {
        sql = sql
            .replace(
                "assignment_id TEXT NOT NULL REFERENCES",
                "assignment_id TEXT PRIMARY KEY REFERENCES",
            )
            .replace(
                "child_run_id TEXT NOT NULL PRIMARY KEY REFERENCES",
                "child_run_id TEXT NOT NULL UNIQUE REFERENCES",
            );
    } else {
        assert_eq!(table, "agent_capability_leases");
        sql = sql.replace(
            "UNIQUE(assignment_id,child_run_id,capability)",
            "UNIQUE(assignment_id,capability)",
        );
    }
    let mut objects=db.prepare("SELECT sql FROM sqlite_schema WHERE tbl_name=?1 AND sql IS NOT NULL AND type IN ('trigger','index') ORDER BY name").unwrap();
    let objects = objects
        .query_map([table], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut columns = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
    let columns = columns
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join(",");
    db.execute_batch(&format!("CREATE TEMP TABLE old_native_copy AS SELECT rowid AS saved_rowid,* FROM {table}; DROP TABLE {table}; {sql};
        INSERT INTO {table}(rowid,{columns}) SELECT saved_rowid,{columns} FROM old_native_copy; DROP TABLE old_native_copy;")).unwrap();
    for object in objects {
        db.execute_batch(&object).unwrap();
    }
}

#[test]
fn assignment_worker_namespace_native_upgrade_preserves_every_table_and_schema_object() {
    use crate::agent_runtime::multi_agent::specialist;
    let (root, context, lease, child) = specialist_journal_fixture();
    let mut db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(_) =
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    db.execute_batch("CREATE INDEX namespace_custom_call ON agent_specialist_calls(root_run_id,state);
        CREATE TRIGGER namespace_custom_guard BEFORE UPDATE ON agent_specialist_calls WHEN NEW.request_hash<>OLD.request_hash BEGIN SELECT RAISE(ABORT,'original_hash'); END;").unwrap();
    for table in ["agent_specialist_calls", "agent_capability_leases"] {
        namespace_restore_native_declaration(&db, table);
    }
    let before = super::tests::application_table_snapshot(&db);
    let old_row: i64 = db
        .query_row(
            "SELECT rowid FROM agent_specialist_calls WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get(0),
        )
        .unwrap();
    db::migrate_worker_namespaces_for_test(&mut db).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        db.query_row(
            "SELECT rowid FROM agent_specialist_calls WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        old_row
    );
    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('namespace_custom_call','namespace_custom_guard','agent_specialist_call_immutable','idx_agent_capability_child')",[],|r|r.get::<_,i64>(0)).unwrap(),4);
    db::migrate_worker_namespaces_for_test(&mut db).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    // Startup invokes the same migration and does not rewrite either journal.
    let own_before = before
        .into_iter()
        .filter(|(t, _)| {
            [
                "agent_specialist_calls",
                "agent_capability_leases",
                "agent_assignment_attempts",
                "agent_budget_entries",
                "agent_runs",
                "agent_assignments",
            ]
            .contains(&t.as_str())
        })
        .collect::<Vec<_>>();
    drop(db);
    db::initialize(&root).unwrap();
    let db = db::open(&context.db_path).unwrap();
    let own_after = super::tests::application_table_snapshot(&db)
        .into_iter()
        .filter(|(t, _)| own_before.iter().any(|(name, _)| name == t))
        .collect::<Vec<_>>();
    assert_eq!(own_before, own_after);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_worker_namespace_upgrade_unknown_schema_or_reference_rolls_back_both_tables() {
    for fault in ["extension", "reference", "invalid_parent"] {
        use crate::agent_runtime::multi_agent::specialist;
        let (root, context, lease, child) = specialist_journal_fixture();
        let mut db = db::open(&context.db_path).unwrap();
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap();
        for table in ["agent_specialist_calls", "agent_capability_leases"] {
            namespace_restore_native_declaration(&db, table);
        }
        match fault {
            "extension"=>db.execute_batch("ALTER TABLE agent_capability_leases ADD COLUMN unreviewed_extension TEXT").unwrap(),
            "reference"=>db.execute_batch("CREATE TABLE unreviewed_child(grant_id TEXT REFERENCES agent_capability_leases(id))").unwrap(),
            _=>{
                db.pragma_update(None,"foreign_keys",false).unwrap();
                db.execute("UPDATE agent_capability_leases SET child_run_id='missing-worker'",[]).unwrap();
                db.pragma_update(None,"foreign_keys",true).unwrap();
            }
        }
        let before = super::tests::application_table_snapshot(&db);
        let schema_before:String=db.query_row("SELECT group_concat(sql,';') FROM (SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name)",[],|r|r.get(0)).unwrap();
        assert!(
            db::migrate_worker_namespaces_for_test(&mut db).is_err(),
            "{fault}"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{fault}"
        );
        let schema_after:String=db.query_row("SELECT group_concat(sql,';') FROM (SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name)",[],|r|r.get(0)).unwrap();
        assert_eq!(schema_after, schema_before, "{fault}");
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_temp_schema WHERE name LIKE 'native_worker_copy_%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
