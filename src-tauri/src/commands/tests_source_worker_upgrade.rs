fn source_namespace_restore_native_pair(db: &rusqlite::Connection) {
    let names = ["agent_source_model_rounds", "agent_source_tool_receipts"];
    let mut restore = Vec::new();
    for table in names {
        let sql: String = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name=?1 AND type='table'",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        let old = if table == names[0] {
            sql.replace(
                "PRIMARY KEY(assignment_id,child_run_id,round_number)",
                "PRIMARY KEY(assignment_id,round_number)",
            )
        } else {
            sql.replace("    child_run_id TEXT NOT NULL,\n", "")
                .replace(
                    "assignment_id,child_run_id,round_number",
                    "assignment_id,round_number",
                )
                .replace(
                    "assignment_id,child_run_id,call_id",
                    "assignment_id,call_id",
                )
        };
        let mut objects=db.prepare("SELECT sql FROM sqlite_schema WHERE tbl_name=?1 AND type IN ('trigger','index') AND sql IS NOT NULL ORDER BY name").unwrap();
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
            .into_iter()
            .filter(|n| !(table == names[1] && n == "child_run_id"))
            .collect::<Vec<_>>()
            .join(",");
        db.execute_batch(&format!(
            "CREATE TEMP TABLE saved_{table} AS SELECT rowid AS original_rowid,* FROM {table}"
        ))
        .unwrap();
        restore.push((table, old, columns, objects));
    }
    db.execute_batch("DROP TABLE agent_source_tool_receipts; DROP TABLE agent_source_model_rounds")
        .unwrap();
    for (table, sql, columns, objects) in restore {
        db.execute_batch(&format!("{sql}; INSERT INTO {table}(rowid,{columns}) SELECT original_rowid,{columns} FROM saved_{table}; DROP TABLE saved_{table}")).unwrap();
        for sql in objects {
            db.execute_batch(&sql.replace(
                " OR NEW.child_run_id<>OLD.child_run_id\n",
                if table == names[1] {
                    ""
                } else {
                    " OR NEW.child_run_id<>OLD.child_run_id\n"
                },
            ))
            .unwrap();
        }
    }
}

fn source_namespace_original_tool_values(
    db: &rusqlite::Connection,
) -> Vec<Vec<rusqlite::types::Value>> {
    let mut columns = db
        .prepare("PRAGMA table_info(agent_source_tool_receipts)")
        .unwrap();
    let names = columns
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .filter(|n| n != "child_run_id")
        .collect::<Vec<_>>()
        .join(",");
    let mut query = db
        .prepare(&format!(
            "SELECT rowid,{names} FROM agent_source_tool_receipts ORDER BY rowid"
        ))
        .unwrap();
    let count = query.column_count();
    let rows = query
        .query_map([], |r| {
            (0..count)
                .map(|n| r.get(n))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    rows
}

#[test]
fn source_worker_namespace_native_pair_upgrade_preserves_original_costs_events_and_receipts() {
    use crate::agent_runtime::multi_agent::source_rounds as rounds;
    let (root, mut db, context, lease, child, request) = source_round_fixture();
    let check = |db: &rusqlite::Connection| {
        agent_native_source_tool_authority(db, &context, "assignment.finish")
            .map(|_| ())
            .map_err(str::to_string)
    };
    let rounds::Start::Dispatch(call) =
        rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
    else {
        panic!()
    };
    rounds::record_received(
        &db,
        &call,
        &source_round_result("original-tool", "repo.inventory", json!({})),
    )
    .unwrap();
    rounds::execute_tool(&db, &call, 0, check, |_, _, _| {
        Ok(json!({"fixture":"inventory"}))
    })
    .unwrap();
    db.execute_batch(
        "CREATE INDEX namespace_source_custom ON agent_source_tool_receipts(tool_name,state)",
    )
    .unwrap();
    source_namespace_restore_native_pair(&db);
    let before = application_table_snapshot(&db);
    let original = source_namespace_original_tool_values(&db);
    db::migrate_worker_namespaces_for_test(&mut db).unwrap();
    for (table, rows) in before {
        if table != "agent_source_tool_receipts" {
            assert_eq!(
                application_table_snapshot(&db)
                    .iter()
                    .find(|(n, _)| n == &table)
                    .unwrap()
                    .1,
                rows,
                "{table}"
            );
        }
    }
    assert_eq!(source_namespace_original_tool_values(&db), original);
    assert_eq!(
        db.query_row(
            "SELECT child_run_id FROM agent_source_tool_receipts",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        child.run_id
    );
    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('namespace_source_custom','agent_source_round_immutable','agent_source_tool_receipt_immutable')",[],|r|r.get::<_,i64>(0)).unwrap(),3);
    assert!(db
        .execute(
            "UPDATE agent_source_tool_receipts SET child_run_id='different-owner'",
            []
        )
        .unwrap_err()
        .to_string()
        .contains("source_tool_receipt_immutable"));
    let stable = application_table_snapshot(&db);
    db::migrate_worker_namespaces_for_test(&mut db).unwrap();
    assert_eq!(application_table_snapshot(&db), stable);
    drop(db);
    db::initialize(&root).unwrap();
    let db = db::open(&context.db_path).unwrap();
    assert_eq!(source_namespace_original_tool_values(&db), original);
    assert_eq!(
        rounds::continuation(&db, &call).unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_worker_namespace_native_upgrade_faults_preserve_all_original_tables() {
    for fault in ["extension", "reference", "orphan", "mixed", "guard"] {
        use crate::agent_runtime::multi_agent::source_rounds as rounds;
        let (root, mut db, context, lease, child, request) = source_round_fixture();
        let check = |db: &rusqlite::Connection| {
            agent_native_source_tool_authority(db, &context, "assignment.finish")
                .map(|_| ())
                .map_err(str::to_string)
        };
        let rounds::Start::Dispatch(call) =
            rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
        else {
            panic!()
        };
        rounds::record_received(
            &db,
            &call,
            &source_round_result("original-tool", "repo.inventory", json!({})),
        )
        .unwrap();
        source_namespace_restore_native_pair(&db);
        for table in ["agent_specialist_calls", "agent_capability_leases"] {
            super::agent_tests::namespace_restore_native_declaration(&db, table);
        }
        match fault {
            "extension"=>db.execute_batch("ALTER TABLE agent_source_tool_receipts ADD COLUMN unreviewed_native_extension TEXT").unwrap(),
            "reference"=>db.execute_batch("CREATE TABLE extra_source_reference(a TEXT,b INTEGER,FOREIGN KEY(a,b) REFERENCES agent_source_model_rounds(assignment_id,round_number))").unwrap(),
            "orphan"=>{
                db.pragma_update(None,"foreign_keys",false).unwrap();db.execute_batch("DROP TRIGGER agent_source_tool_receipt_immutable").unwrap();
                db.execute("UPDATE agent_source_tool_receipts SET round_number=99",[]).unwrap();db.pragma_update(None,"foreign_keys",true).unwrap();
            },
            "mixed"=>db.execute_batch("ALTER TABLE agent_source_tool_receipts ADD COLUMN child_run_id TEXT NOT NULL DEFAULT ''").unwrap(),
            _=>db.execute_batch("DROP TRIGGER agent_source_tool_receipt_immutable; CREATE TRIGGER agent_source_tool_receipt_immutable BEFORE UPDATE ON agent_source_tool_receipts BEGIN SELECT RAISE(ABORT,'unreviewed_guard'); END").unwrap(),
        }
        let before = application_table_snapshot(&db);
        let schema = || {
            db.query_row("SELECT group_concat(sql,';') FROM (SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name)",[],|r|r.get::<_,String>(0)).unwrap()
        };
        let original = schema();
        assert!(
            db::migrate_worker_namespaces_for_test(&mut db).is_err(),
            "{fault}"
        );
        assert_eq!(application_table_snapshot(&db), before, "{fault}");
        let after:String=db.query_row("SELECT group_concat(sql,';') FROM (SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name)",[],|r|r.get(0)).unwrap();
        assert_eq!(after, original, "{fault}");
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_temp_schema WHERE name LIKE 'native_source_copy_%'",
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
