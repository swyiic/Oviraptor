// §13 / Stage 3 — the neutral skill/knowledge/learning tables and their migration.

#[test]
fn creates_only_neutral_knowledge_tables_for_a_fresh_database() {
    let root = std::env::temp_dir().join(format!("oviraptor-neutral-schema-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = Connection::open(&path).unwrap();
    for (table, columns) in [
        (
            "agent_skills",
            [
                "id",
                "name",
                "description",
                "instructions",
                "builtin",
                "enabled",
                "created_at",
                "updated_at",
            ]
            .as_slice(),
        ),
        (
            "agent_knowledge_entries",
            [
                "id",
                "scan_id",
                "project_id",
                "title",
                "summary",
                "patterns_json",
                "skill_instructions",
                "source_hash",
                "skill_id",
                "created_at",
                "updated_at",
            ]
            .as_slice(),
        ),
        (
            "agent_learning_candidates",
            [
                "id",
                "scan_id",
                "project_id",
                "scan_type",
                "title",
                "summary",
                "candidate_json",
                "status",
                "target_skill_id",
                "source_hash",
                "created_at",
                "reviewed_at",
                "updated_at",
            ]
            .as_slice(),
        ),
    ] {
        let present: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(present, 1, "{table} 必须存在");
        let actual: Vec<String> = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for column in columns {
            assert!(
                actual.iter().any(|seen| seen == column),
                "{table} 缺少列 {column}：{actual:?}"
            );
        }
    }
    // Fresh databases must not recreate retired backend tables. Existing upgraded
    // databases keep them in place so their history can still be copied read-only.
    for legacy in [
        "strix_skills",
        "strix_knowledge_entries",
        "strix_learning_candidates",
    ] {
        let kept: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [legacy],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, 0, "新库不得创建 {legacy}");
    }
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// Seeding a built-in skill is a live write, so it must land in the neutral table and
/// leave the historical one empty.
#[test]
fn builtin_skill_seeds_into_the_neutral_table_only() {
    let root = std::env::temp_dir().join(format!("oviraptor-neutral-seed-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = Connection::open(&path).unwrap();
    let neutral: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_skills", [], |row| row.get(0))
        .unwrap();
    assert_eq!(neutral, 1, "内置技能要写进 agent_skills");
    let legacy_exists: i64 = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='strix_skills')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_exists, 0, "新库不得创建 strix_skills");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn copies_legacy_rows_by_id_and_keeps_the_source_untouched() {
    let root = std::env::temp_dir().join(format!("oviraptor-neutral-copy-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    seed_legacy_history(&path);
    initialize(&root).expect("第二次启动必须成功完成迁移");
    let connection = Connection::open(&path).unwrap();
    for (legacy, neutral) in [
        ("strix_skills", "agent_skills"),
        ("strix_knowledge_entries", "agent_knowledge_entries"),
        ("strix_learning_candidates", "agent_learning_candidates"),
    ] {
        let matched: i64 = connection
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM {neutral} n JOIN {legacy} o ON o.id=n.id \
                     WHERE n.created_at=o.created_at AND n.updated_at=o.updated_at"
                ),
                [],
                |row| row.get(0),
            )
            .unwrap();
        let sources: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {legacy}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            matched, sources,
            "{legacy} 的每一行都必须按同一 id、同一时间戳出现在 {neutral}"
        );
    }
    // 状态与 hash 也必须一致。
    let status: String = connection
        .query_row(
            "SELECT n.status FROM agent_learning_candidates n
             JOIN strix_learning_candidates o ON o.id=n.id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "pending");
    let hash: String = connection
        .query_row(
            "SELECT n.source_hash FROM agent_knowledge_entries n
             JOIN strix_knowledge_entries o ON o.id=n.id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hash, "hash-1");
    // AUTOINCREMENT 序列必须跟上，否则下一次插入会撞 id。
    let sequence: i64 = connection
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='agent_learning_candidates'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        sequence >= 13,
        "agent_learning_candidates 的 sqlite_sequence 至少要覆盖已复制的最大 id：{sequence}"
    );
    connection
        .execute(
            "INSERT INTO agent_learning_candidates(scan_id,title) VALUES('mig-scan','新候选')",
            [],
        )
        .unwrap();
    let inserted: i64 = connection
        .query_row(
            "SELECT id FROM agent_learning_candidates ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        inserted > 13,
        "序列没跟上时这条插入会撞上已复制的 id：{inserted}"
    );
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// A second launch must not copy twice, and must not undo an edit made in the neutral
/// table after the switch.
#[test]
fn re_running_the_migration_is_a_no_op() {
    let root = std::env::temp_dir().join(format!("oviraptor-neutral-rerun-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    seed_legacy_history(&path);
    initialize(&root).expect("第二次启动必须成功完成迁移");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE agent_skills SET instructions='用户改过' WHERE name='甲'",
            [],
        )
        .unwrap();
    connection
        .execute("DELETE FROM agent_skills WHERE name='乙'", [])
        .unwrap();
    let before: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_skills", [], |row| row.get(0))
        .unwrap();
    drop(connection);
    initialize(&root).expect("第二次启动必须成功完成迁移");
    let connection = Connection::open(&path).unwrap();
    let after: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_skills", [], |row| row.get(0))
        .unwrap();
    assert_eq!(after, before, "重跑迁移不得复活被删行，也不得重复插入");
    let edited: String = connection
        .query_row(
            "SELECT instructions FROM agent_skills WHERE name='甲'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(edited, "用户改过", "重跑迁移不能覆盖切换后的编辑");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// The copy, the checks and the high-water mark share one transaction: any failure
/// leaves the neutral tables exactly as they were.
#[test]
fn a_failing_migration_rolls_the_whole_copy_back() {
    let root = std::env::temp_dir().join(format!("oviraptor-neutral-rollback-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    seed_legacy_history(&path);
    // A name collision planted in the target table makes the skills copy fail halfway.
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO agent_skills(id,name,instructions) VALUES(9001,'甲','占位')",
                [],
            )
            .unwrap();
        let candidates: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_learning_candidates",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(candidates, 0);
        drop(connection);
    }
    let failure =
        initialize(&root).expect_err("迁移失败必须让 initialize 报错，而不是悄悄留下半套数据");
    let connection = Connection::open(&path).unwrap();
    assert!(
        failure.contains("迁移"),
        "错误要能指认是迁移失败：{failure}"
    );
    for table in ["agent_knowledge_entries", "agent_learning_candidates"] {
        let rows: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "{table} 不能留下半截拷贝");
    }
    // The water mark row already exists from the first launch; what must not have
    // moved is its value, otherwise a later launch would trust a half-finished copy.
    let marker: String = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='neutral_knowledge_migration'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        marker.contains(":0") && !marker.contains("7") && !marker.contains("8"),
        "校验未通过就不能推进高水位：{marker}"
    );
    let placeholder: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_skills WHERE id=9001",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(placeholder, 1, "回滚只撤销迁移自己的写入");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// The verification step is what makes the copy trustworthy; it has to notice a
/// hand-corrupted row.
#[test]
fn the_copy_check_detects_a_mismatched_row() {
    let root = std::env::temp_dir().join(format!("oviraptor-neutral-check-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    seed_legacy_history(&path);
    initialize(&root).expect("第二次启动必须成功完成迁移");
    let connection = Connection::open(&path).unwrap();
    assert!(
        neutral_knowledge_copy_is_faithful(&connection).is_ok(),
        "正常迁移后必须通过校验"
    );
    connection
        .execute(
            "UPDATE agent_learning_candidates SET status='tampered' WHERE id=13",
            [],
        )
        .unwrap();
    let error = neutral_knowledge_copy_is_faithful(&connection)
        .expect_err("状态被改动必须被 status 校验发现");
    assert!(error.contains("status"), "错误要指认维度：{error}");
    connection
        .execute(
            "UPDATE agent_learning_candidates SET status='pending' WHERE id=13",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE agent_skills SET updated_at='2099-01-01' WHERE name='甲'",
            [],
        )
        .unwrap();
    let error = neutral_knowledge_copy_is_faithful(&connection)
        .expect_err("时间戳被改动必须被 timestamp 校验发现");
    assert!(error.contains("timestamp"), "错误要指认维度：{error}");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// Orphans are the reason the copy order matters: a knowledge row may only point at a
/// skill that exists in the neutral table.
#[test]
fn the_copy_check_detects_an_orphaned_reference() {
    let root = std::env::temp_dir().join(format!("oviraptor-neutral-orphan-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    seed_legacy_history(&path);
    initialize(&root).expect("第二次启动必须成功完成迁移");
    // The audit has to notice a dangling reference even though the database normally
    // refuses to create one, so this connection turns its own enforcement off.
    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    connection
        .execute("UPDATE agent_knowledge_entries SET skill_id=4242", [])
        .unwrap();
    let touched: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_knowledge_entries WHERE skill_id=4242",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(touched, 1, "UPDATE 必须落到已复制的行上");
    let error = neutral_knowledge_copy_is_faithful(&connection)
        .expect_err("指向不存在技能的行必须被 orphan 校验发现");
    assert!(error.contains("orphan"), "错误要指认维度：{error}");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// §Stage 3 缺口 2: `CREATE TABLE IF NOT EXISTS` never edits an existing table, so an
/// upgraded library can still carry `DEFAULT 'strix'` on `agent_runs.backend`. The
/// migration has to fix that default and nothing else.
#[test]
fn upgrades_the_agent_runs_backend_default_without_touching_rows() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-agent-runs-default-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    // Simulate the upgraded install: the real parent tables are there, but agent_runs
    // still carries the historical 'strix' default and one Strix row.
    {
        let old = Connection::open(&path).unwrap();
        old.pragma_update(None, "foreign_keys", "OFF").unwrap();
        // Keep the real column set and change only the default: that is exactly what an
        // upgraded library looks like.
        let ddl: String = old
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_runs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let historic = ddl.replace("DEFAULT 'native'", "DEFAULT 'strix'");
        assert!(
            historic != ddl,
            "夹具必须能在新建表上找到被改写过默认值的那一列"
        );
        old.execute("DELETE FROM agent_runs", []).unwrap();
        old.execute("DROP TABLE agent_runs", []).unwrap();
        old.execute_batch(&historic).unwrap();
        old.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_agent_runs_legacy_scan ON agent_runs(scan_id,status);
            INSERT OR IGNORE INTO projects(id,name) VALUES(930,'默认值');
            INSERT OR IGNORE INTO sentinel_scans(id,project_id,project_name,status,scan_type)
            VALUES('scan-1',930,'默认值','completed','web');
            INSERT INTO agent_runs(id,scan_id,backend) VALUES('legacy-row','scan-1','strix');
            CREATE TRIGGER external_run_guard BEFORE INSERT ON agent_user_directives
            WHEN NEW.root_run_id<>'' AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE id=NEW.root_run_id)
            BEGIN SELECT RAISE(ABORT,'external_run_missing'); END;
            "#,
        )
        .unwrap();
        let default: Option<String> = old
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('agent_runs') WHERE name='backend'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            default.as_deref(),
            Some("'strix'"),
            "夹具要先真的是旧默认值"
        );
        drop(old);
    }
    let path = initialize(&root).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    let default: Option<String> = connection
        .query_row(
            "SELECT dflt_value FROM pragma_table_info('agent_runs') WHERE name='backend'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        default.as_deref(),
        Some("'native'"),
        "升级库的列默认值必须变成 native"
    );
    let legacy: String = connection
        .query_row(
            "SELECT backend FROM agent_runs WHERE id='legacy-row'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy, "strix", "历史行必须保持它写入时的 backend");
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1, "迁移前后行数必须相等");
    let external_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='external_run_guard'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(external_sql.contains("SELECT 1 FROM agent_runs"));
    assert!(connection.execute(
        "INSERT INTO agent_user_directives(id,scan_id,attempt_number,root_run_id,text_redacted) VALUES('bad','scan-1',1,'missing','test')", [],
    ).unwrap_err().to_string().contains("external_run_missing"));
    let closure_guard: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name='agent_directive_proposal_state_guard_v2'",[],|row|row.get(0),
    ).unwrap();
    assert_eq!(
        closure_guard, 1,
        "cross-table cancellation guard must survive the rebuild"
    );
    let index_kept: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_agent_runs_legacy_scan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(index_kept, 1, "旧索引必须回来");
    // The foreign key still points at sentinel_scans, so a dangling row is refused and
    // a real scan still cascades.
    let orphan = connection.execute(
        "INSERT INTO agent_runs(id,scan_id) VALUES('orphan','no-such-scan')",
        [],
    );
    assert!(orphan.is_err(), "外键必须仍然生效：{orphan:?}");
    connection
        .execute(
            "INSERT INTO agent_runs(id,scan_id) VALUES('fresh-row','scan-1')",
            [],
        )
        .unwrap();
    let fresh: String = connection
        .query_row(
            "SELECT backend FROM agent_runs WHERE id='fresh-row'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fresh, "native", "省略列的新插入才拿到新默认值");
    let violations: i64 = connection
        .prepare("SELECT * FROM pragma_foreign_key_check('agent_runs')")
        .unwrap()
        .query_map([], |_| Ok(()))
        .unwrap()
        .count() as i64;
    assert_eq!(violations, 0, "重建不能留下外键破损");
    connection
        .execute("DELETE FROM sentinel_scans WHERE id='scan-1'", [])
        .unwrap();
    let cascaded: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE id='fresh-row'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cascaded, 0, "ON DELETE CASCADE 必须还在");
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

/// The rewrite helper, tested directly so the "column had no default at all" branch is
/// not dead code with no proof.
#[test]
fn rewrites_only_the_backend_default_of_a_stored_statement() {
    let stored = "CREATE TABLE agent_runs (\n    id TEXT PRIMARY KEY,\n    backend TEXT NOT NULL DEFAULT 'strix',\n    status TEXT NOT NULL\n)";
    let rewritten = with_native_backend_default(stored).expect("要能定位 backend 的默认值");
    assert!(
        rewritten.contains("backend TEXT NOT NULL DEFAULT 'native'"),
        "{rewritten}"
    );
    assert!(rewritten.contains("id TEXT PRIMARY KEY"), "其他列必须原样");
    assert!(rewritten.contains("status TEXT NOT NULL"), "其他列必须原样");
    let bare = "CREATE TABLE agent_runs (\n    id TEXT PRIMARY KEY,\n    backend TEXT NOT NULL,\n    status TEXT NOT NULL\n)";
    let rewritten = with_native_backend_default(bare).expect("没有默认值时也要补上");
    assert!(
        rewritten.contains("backend TEXT NOT NULL DEFAULT 'native',"),
        "{rewritten}"
    );
    assert!(
        rewritten.contains("status TEXT NOT NULL"),
        "补默认值不能吃掉后面的逗号"
    );
    assert!(
        with_native_backend_default("CREATE TABLE agent_runs (id TEXT PRIMARY KEY)").is_none(),
        "找不到 backend 列时必须拒绝重建",
    );
}

/// Drops legacy history into a freshly initialized database, exactly as an upgraded
/// install would arrive.
fn seed_legacy_history(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE strix_skills (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL DEFAULT '',
                instructions TEXT NOT NULL,
                builtin INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE TABLE strix_knowledge_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id TEXT NOT NULL UNIQUE,
                project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                patterns_json TEXT NOT NULL DEFAULT '{}',
                skill_instructions TEXT NOT NULL DEFAULT '',
                source_hash TEXT NOT NULL DEFAULT '',
                skill_id INTEGER REFERENCES strix_skills(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE TABLE strix_learning_candidates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
                scan_type TEXT NOT NULL DEFAULT 'web',
                title TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                candidate_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending',
                target_skill_id INTEGER REFERENCES strix_skills(id) ON DELETE SET NULL,
                source_hash TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                reviewed_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(scan_id,source_hash)
            );
            INSERT INTO strix_skills(id,name,description,instructions,builtin,enabled,created_at,updated_at)
            VALUES (7,'甲','','老指令一',0,1,'2024-01-01 00:00:00','2024-01-02 00:00:00'),
                   (8,'乙','','老指令二',0,1,'2024-01-01 00:00:00','2024-01-02 00:00:00');
            INSERT OR IGNORE INTO projects(id,name) VALUES (901,'迁移');
            INSERT OR IGNORE INTO sentinel_scans(id,project_id,project_name,status,scan_type)
            VALUES ('mig-scan',901,'迁移','completed','web');
            INSERT INTO strix_knowledge_entries(id,scan_id,project_id,title,summary,patterns_json,skill_instructions,source_hash,skill_id,created_at,updated_at)
            VALUES (11,'mig-scan',901,'知识','摘要','{}','','hash-1',7,'2024-01-03 00:00:00','2024-01-04 00:00:00');
            INSERT INTO strix_learning_candidates(id,scan_id,project_id,scan_type,title,summary,candidate_json,status,target_skill_id,source_hash,created_at,reviewed_at,updated_at)
            VALUES (13,'mig-scan',901,'web','候选','','{"a":1}','pending',7,'hash-2','2024-01-05 00:00:00','','2024-01-06 00:00:00');
            "#,
        )
        .unwrap();
    drop(connection);
}

/// Loop3 (§11.4): the inventory is read-only preview, never a cleanup.
/// Fresh databases report no legacy tables; seeded ones report exact counts and
/// watermarks, and a second read returns the same rows.
#[test]
fn legacy_inventory_reports_exact_counts_without_copying_or_cleaning() {
    // Fresh database: nothing legacy to preview.
    let fresh = std::env::temp_dir().join(format!(
        "oviraptor-neutral-inventory-fresh-{}",
        Uuid::new_v4()
    ));
    let fresh_path = initialize(&fresh).unwrap();
    let connection = Connection::open(&fresh_path).unwrap();
    let entries = legacy_knowledge_inventory(&connection).unwrap();
    assert_eq!(entries.len(), 3);
    for entry in &entries {
        assert!(!entry.legacy_exists, "{} 不应存在", entry.legacy);
        assert_eq!(entry.legacy_rows, 0);
        assert_eq!(entry.legacy_max_id, 0);
    }
    drop(connection);
    let _ = fs::remove_dir_all(fresh);

    // Upgraded database: exact preview of the three legacy tables.
    let root = std::env::temp_dir().join(format!("oviraptor-neutral-inventory-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    seed_legacy_history(&path);
    initialize(&root).expect("第二次启动必须成功完成迁移");
    let connection = Connection::open(&path).unwrap();
    let first = legacy_knowledge_inventory(&connection).unwrap();
    let by_legacy = |name: &str| {
        first
            .iter()
            .find(|entry| entry.legacy == name)
            .unwrap()
            .clone()
    };
    assert_eq!(
        (
            by_legacy("strix_skills").legacy_rows,
            by_legacy("strix_skills").legacy_max_id
        ),
        (2, 8)
    );
    assert_eq!(
        (
            by_legacy("strix_knowledge_entries").legacy_rows,
            by_legacy("strix_knowledge_entries").legacy_max_id
        ),
        (1, 11)
    );
    assert_eq!(
        (
            by_legacy("strix_learning_candidates").legacy_rows,
            by_legacy("strix_learning_candidates").legacy_max_id
        ),
        (1, 13)
    );
    for entry in &first {
        assert!(entry.legacy_exists);
        assert!(
            entry.neutral_rows >= entry.legacy_rows,
            "{} 中性行数不得少于旧行数",
            entry.neutral
        );
        assert_eq!(
            entry.migration_mark, entry.legacy_max_id,
            "{} 水位必须等于旧表最大 id",
            entry.neutral
        );
    }
    // Read-only: a second inventory returns identical rows and the faithful-copy
    // check still holds; nothing was deleted or advanced.
    let second = legacy_knowledge_inventory(&connection).unwrap();
    assert_eq!(first, second, "只读盘点不得推进水位或改变行数");
    neutral_knowledge_copy_is_faithful(&connection).unwrap();
    for legacy in [
        "strix_skills",
        "strix_knowledge_entries",
        "strix_learning_candidates",
    ] {
        let kept: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [legacy],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "{legacy} 在清理批准前必须保留");
    }
    drop(connection);
    let _ = fs::remove_dir_all(root);
}
