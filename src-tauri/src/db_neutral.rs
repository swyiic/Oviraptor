// §Stage 3 — the neutral skill/knowledge/learning tables, their one-shot migration,
// and the freeze of Strix runs that can no longer be resumed. Included from `db.rs`.

/// The three neutral tables. Column-for-column compatible with the legacy tables so a
/// copy is a straight `INSERT … SELECT`, but every foreign key now points at a neutral
/// table (or at a platform table), which is what lets the legacy trio be retired later.
const NEUTRAL_KNOWLEDGE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_skills (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    instructions TEXT NOT NULL,
    builtin INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS agent_knowledge_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL UNIQUE,
    project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    patterns_json TEXT NOT NULL DEFAULT '{}',
    skill_instructions TEXT NOT NULL DEFAULT '',
    source_hash TEXT NOT NULL DEFAULT '',
    skill_id INTEGER REFERENCES agent_skills(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_agent_knowledge_project ON agent_knowledge_entries(project_id,updated_at);
CREATE TABLE IF NOT EXISTS agent_learning_candidates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    scan_type TEXT NOT NULL DEFAULT 'web',
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    candidate_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending',
    target_skill_id INTEGER REFERENCES agent_skills(id) ON DELETE SET NULL,
    source_hash TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    reviewed_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,source_hash)
);
CREATE INDEX IF NOT EXISTS idx_agent_learning_candidates_status ON agent_learning_candidates(status,updated_at);
CREATE INDEX IF NOT EXISTS idx_agent_learning_candidates_project ON agent_learning_candidates(project_id,updated_at);
"#;

/// Copy order matters: the skill table is the target of two foreign keys.
struct NeutralCopy {
    legacy: &'static str,
    neutral: &'static str,
    columns: &'static [&'static str],
    /// A column whose per-status distribution is verified separately, when present.
    status_column: Option<&'static str>,
}

const NEUTRAL_COPIES: &[NeutralCopy] = &[
    NeutralCopy {
        legacy: "strix_skills",
        neutral: "agent_skills",
        columns: &[
            "name",
            "description",
            "instructions",
            "builtin",
            "enabled",
            "created_at",
            "updated_at",
        ],
        status_column: None,
    },
    NeutralCopy {
        legacy: "strix_knowledge_entries",
        neutral: "agent_knowledge_entries",
        columns: &[
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
        ],
        status_column: None,
    },
    NeutralCopy {
        legacy: "strix_learning_candidates",
        neutral: "agent_learning_candidates",
        columns: &[
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
        ],
        status_column: Some("status"),
    },
];

const MIGRATION_MARK_KEY: &str = "neutral_knowledge_migration";

fn migration_marks(connection: &Connection) -> BTreeMap<String, i64> {
    let stored: String = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [MIGRATION_MARK_KEY],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "{}".into());
    serde_json::from_str::<BTreeMap<String, i64>>(&stored).unwrap_or_default()
}

fn copy_is_faithful(
    connection: &Connection,
    copy: &NeutralCopy,
    from_id: i64,
    to_id: i64,
) -> Result<(), String> {
    let bound = "id>?1 AND id<=?2";
    let source_rows = neutral_rows(connection, copy, copy.legacy, bound, from_id, to_id)?;
    let target_rows = neutral_rows(connection, copy, copy.neutral, bound, from_id, to_id)?;
    let orphans = match copy.neutral {
        "agent_knowledge_entries" => {
            Some("skill_id IS NOT NULL AND skill_id NOT IN (SELECT id FROM agent_skills)")
        }
        "agent_learning_candidates" => Some(
            "target_skill_id IS NOT NULL AND target_skill_id NOT IN (SELECT id FROM agent_skills) \
             OR scan_id NOT IN (SELECT id FROM sentinel_scans)",
        ),
        _ => None,
    };
    if let Some(orphans) = orphans {
        let found: i64 = connection
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM {} WHERE {bound} AND ({orphans})",
                    copy.neutral
                ),
                params![from_id, to_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法检查孤儿引用：{error}"))?;
        if found > 0 {
            return Err(format!(
                "中性知识表迁移的 orphan 校验失败：{} 有 {found} 行引用不存在",
                copy.neutral
            ));
        }
    }
    for (id, fields) in &source_rows {
        let Some(target) = target_rows.get(id) else {
            return Err(format!(
                "中性知识表迁移的 count 校验失败：{} #{id} 没有对应副本",
                copy.neutral
            ));
        };
        if digest_of(fields) == digest_of(target) {
            continue;
        }
        // The row differs; name the dimension that broke so an operator can act on it.
        for dimension in ["created_at", "updated_at", "reviewed_at"] {
            if let Some(index) = copy.columns.iter().position(|column| *column == dimension) {
                if fields.get(index) != target.get(index) {
                    return Err(format!(
                        "中性知识表迁移的 timestamp 校验失败：{} #{id} 的 {dimension} 与源行不同",
                        copy.neutral
                    ));
                }
            }
        }
        if let Some(status) = copy.status_column {
            if let Some(index) = copy.columns.iter().position(|column| *column == status) {
                if fields.get(index) != target.get(index) {
                    return Err(format!(
                        "中性知识表迁移的 status 校验失败：{} #{id} 的 {status} 与源行不同",
                        copy.neutral
                    ));
                }
            }
        }
        return Err(format!(
            "中性知识表迁移的 hash 校验失败：{} #{id} 与源行内容不一致",
            copy.neutral
        ));
    }
    let highest = source_rows.keys().next_back().copied().unwrap_or(0);
    let sequence: i64 = connection
        .query_row(
            "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name=?1),0)",
            [copy.neutral],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法读取 sqlite_sequence：{error}"))?;
    if highest > 0 && sequence < highest {
        return Err(format!(
            "中性知识表迁移的 sqlite_sequence 校验失败：{} 序列为 {sequence}，已复制到 {highest}",
            copy.neutral
        ));
    }
    Ok(())
}

/// One row per id: the copied columns as text, with NULL kept distinguishable.
fn neutral_rows(
    connection: &Connection,
    copy: &NeutralCopy,
    table: &str,
    bound: &str,
    from_id: i64,
    to_id: i64,
) -> Result<BTreeMap<i64, Vec<String>>, String> {
    // CAST keeps integers readable as text; COALESCE keeps NULL distinguishable from "".
    let projection = copy
        .columns
        .iter()
        .map(|column| format!("COALESCE(CAST({column} AS TEXT),'∅')"))
        .collect::<Vec<_>>()
        .join(",");
    let mut statement = connection
        .prepare(&format!(
            "SELECT id,{projection} FROM {table} WHERE {bound} ORDER BY id"
        ))
        .map_err(|error| format!("无法读取 {table}：{error}"))?;
    let width = copy.columns.len();
    let rows = statement
        .query_map(params![from_id, to_id], |row| {
            let id = row.get::<_, i64>(0)?;
            let mut fields = Vec::with_capacity(width);
            for index in 0..width {
                fields.push(row.get::<_, String>(index + 1)?);
            }
            Ok((id, fields))
        })
        .map_err(|error| format!("无法读取 {table}：{error}"))?;
    let mut parsed = BTreeMap::new();
    for row in rows {
        let (id, fields) = row.map_err(|error| format!("无法取回 {table} 行：{error}"))?;
        parsed.insert(id, fields);
    }
    Ok(parsed)
}

/// Field lengths are included so a value that only moves a boundary cannot collide.
fn digest_of(fields: &[String]) -> String {
    let joined = fields
        .iter()
        .map(|field| format!("{}:{}", field.chars().count(), field))
        .collect::<Vec<_>>()
        .join("\u{1}");
    sha256_hex(joined.as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// §Stage 3: built-in content is seeded into the neutral table only. The legacy trio is
/// kept for history but never written again, so an upgraded database shows exactly what
/// it had before the switch.
fn seed_builtin_agent_skill(connection: &mut Connection) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR IGNORE INTO agent_skills(name,description,instructions,builtin,enabled) VALUES('业务前端深度分析','按看功能、触发请求、还原参数、分析业务 JS、匹配本地知识和一次性保底发现的顺序执行；只把证据充分的高价值候选交给后续复核。',?1,1,1)",
            [DEFAULT_BUSINESS_FRONTEND_SKILL],
        )
        .map_err(|error| format!("无法写入内置技能：{error}"))?;
    Ok(())
}

/// Standalone verification over every overlapping id, used by the tests and by any
/// later audit. Startup only verifies what it just copied.
pub fn neutral_knowledge_copy_is_faithful(connection: &Connection) -> Result<(), String> {
    for copy in NEUTRAL_COPIES {
        if !table_exists(connection, copy.legacy)? {
            continue;
        }
        copy_is_faithful(connection, copy, 0, i64::MAX)?;
    }
    Ok(())
}

/// Loop3 (§11.4 prerequisite): read-only preview of the legacy knowledge trio.
/// No copy, no watermark advance, no DELETE. Operators use the exact counts and
/// max ids to approve a future directed cleanup; removal must still go through
/// the §0 step-4 preview/backup/confirm path.
/// Loop3/5 note: the only production caller so far is a future directed-cleanup
/// flow; today the inventory backs the Loop3 regression tests and operator
/// preview. The item-level allows below are staging-only and must be removed
/// when the first non-test caller lands.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyKnowledgeEntry {
    pub legacy: String,
    pub neutral: String,
    pub legacy_exists: bool,
    pub legacy_rows: i64,
    pub legacy_max_id: i64,
    pub neutral_rows: i64,
    pub migration_mark: i64,
}

#[allow(dead_code)]
pub fn legacy_knowledge_inventory(
    connection: &Connection,
) -> Result<Vec<LegacyKnowledgeEntry>, String> {
    let marks = migration_marks(connection);
    let mut out = Vec::with_capacity(NEUTRAL_COPIES.len());
    for copy in NEUTRAL_COPIES {
        let legacy_exists = table_exists(connection, copy.legacy)?;
        let (legacy_rows, legacy_max_id) = if legacy_exists {
            let rows: i64 = connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {}", copy.legacy),
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| format!("无法盘点 {}：{error}", copy.legacy))?;
            let max_id: i64 = connection
                .query_row(
                    &format!("SELECT COALESCE(MAX(id),0) FROM {}", copy.legacy),
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| format!("无法读取 {} 最大 id：{error}", copy.legacy))?;
            (rows, max_id)
        } else {
            (0, 0)
        };
        let neutral_rows: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {}", copy.neutral),
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法盘点 {}：{error}", copy.neutral))?;
        out.push(LegacyKnowledgeEntry {
            legacy: copy.legacy.to_string(),
            neutral: copy.neutral.to_string(),
            legacy_exists,
            legacy_rows,
            legacy_max_id,
            neutral_rows,
            migration_mark: marks.get(copy.neutral).copied().unwrap_or(0),
        });
    }
    Ok(out)
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法检查历史表 {table}：{error}"))
}

/// Build the neutral tables, copy whatever the legacy tables still hold, verify the
/// copy, and only then record the high-water mark. One transaction: any failed check
/// rolls the whole copy back and `initialize` reports the error rather than letting the
/// application run against an empty neutral store.
fn migrate_neutral_knowledge_tables(connection: &mut Connection) -> Result<(), String> {
    connection
        .execute_batch(NEUTRAL_KNOWLEDGE_SCHEMA)
        .map_err(|error| format!("无法创建中性知识表：{error}"))?;
    let marks = migration_marks(connection);
    let first_migration = marks.is_empty();
    let mut advanced = BTreeMap::<String, i64>::new();
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    for copy in NEUTRAL_COPIES {
        let from_id = marks.get(copy.neutral).copied().unwrap_or(0);
        if !table_exists(&transaction, copy.legacy)? {
            advanced.insert(copy.neutral.to_string(), from_id);
            continue;
        }
        let to_id: i64 = transaction
            .query_row(
                &format!("SELECT COALESCE(MAX(id),0) FROM {}", copy.legacy),
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法读取 {} 的最大 id：{error}", copy.legacy))?;
        if to_id > from_id {
            let columns = copy.columns.join(",");
            transaction
                .execute(
                    &format!(
                        "INSERT INTO {neutral}(id,{columns}) \
                         SELECT id,{columns} FROM {legacy} WHERE id>?1 ORDER BY id",
                        neutral = copy.neutral,
                        columns = columns,
                        legacy = copy.legacy
                    ),
                    [from_id],
                )
                .map_err(|error| format!("中性知识表迁移复制 {} 失败：{error}", copy.legacy))?;
            // A failed check returns out of the whole function; dropping the
            // transaction without committing is what rolls the copy back.
            copy_is_faithful(&transaction, copy, from_id, to_id)?;
        }
        advanced.insert(copy.neutral.to_string(), to_id.max(from_id));
    }
    // The very first migration is also audited over the whole overlap, so a database
    // that comes up neutral is known to be a faithful copy, not just a partial one.
    if first_migration {
        neutral_knowledge_copy_is_faithful(&transaction)?;
    }
    transaction
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![
                MIGRATION_MARK_KEY,
                serde_json::to_string(&advanced).unwrap_or_else(|_| "{}".into())
            ],
        )
        .map_err(|error| format!("无法登记中性知识迁移水位：{error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("中性知识表迁移提交失败：{error}"))?;
    Ok(())
}

/// The historical default of `agent_runs.backend`, and the neutral one it must become.
const AGENT_RUNS_LEGACY_DEFAULT: &str = "'strix'";
const AGENT_RUNS_NEUTRAL_DEFAULT: &str = "'native'";

fn backend_default_of(connection: &Connection) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT dflt_value FROM pragma_table_info('agent_runs') WHERE name='backend'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|error| format!("无法读取 agent_runs.backend 的默认值：{error}"))
}

/// Rewrites only the `backend` column's default in a stored CREATE TABLE statement.
/// The statement text comes from this repository's own schema, so the column sits on
/// its own line; anything else is refused rather than guessed at.
fn with_native_backend_default(stored: &str) -> Option<String> {
    let mut rewritten = String::with_capacity(stored.len());
    let mut changed = false;
    for line in stored.lines() {
        let trimmed = line.trim_start();
        let is_backend = trimmed
            .strip_prefix("backend")
            .is_some_and(|rest| rest.starts_with(' ') || rest.starts_with('\t'));
        if !is_backend || changed {
            rewritten.push_str(line);
            rewritten.push('\n');
            continue;
        }
        if let Some(at) = line.find(AGENT_RUNS_LEGACY_DEFAULT) {
            let mut next = String::new();
            next.push_str(&line[..at]);
            next.push_str(AGENT_RUNS_NEUTRAL_DEFAULT);
            next.push_str(&line[at + AGENT_RUNS_LEGACY_DEFAULT.len()..]);
            rewritten.push_str(&next);
        } else {
            // No default at all: append the neutral one, keeping a trailing comma.
            let head = line.trim_end();
            let (body, comma) = match head.strip_suffix(',') {
                Some(without) => (without, ","),
                None => (head, ""),
            };
            rewritten.push_str(&format!(
                "{body} DEFAULT {AGENT_RUNS_NEUTRAL_DEFAULT}{comma}"
            ));
        }
        rewritten.push('\n');
        changed = true;
    }
    changed.then_some(rewritten)
}

/// Rebuilds the table so one column default changes. Rows are copied by column and
/// keep their own `backend` value; indexes and triggers come back under their old
/// names. Foreign keys are switched off for the swap because `DROP TABLE` would
/// otherwise cascade into the child rows this migration must not touch.
fn migrate_agent_runs_backend_default(connection: &mut Connection) -> Result<(), String> {
    if backend_default_of(connection)?.as_deref() == Some(AGENT_RUNS_NEUTRAL_DEFAULT) {
        return Ok(());
    }
    let stored: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_runs'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法读取 agent_runs 的建表语句：{error}"))?;
    let rewritten = with_native_backend_default(&stored)
        .ok_or("agent_runs 定义里找不到可改写的 backend 默认值，请人工检查该表结构")?;
    let prefix_len = rewritten.find('(').ok_or("agent_runs 定义不完整")?;
    let body = &rewritten[prefix_len..];
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('agent_runs') ORDER BY cid")
        .map_err(|error| error.to_string())?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let rows_before: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let companions: Vec<(String, String)> = connection
        .prepare(
            "SELECT type, sql FROM sqlite_master WHERE tbl_name='agent_runs' \
             AND type IN ('index','trigger') AND sql IS NOT NULL ORDER BY name",
        )
        .map_err(|error| error.to_string())?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    // A PRAGMA is a no-op inside a transaction, so it has to move before the BEGIN.
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .map_err(|error| error.to_string())?;
    let outcome = rebuild_agent_runs_default(connection, body, &columns, &companions, rows_before);
    let restore = connection.pragma_update(None, "foreign_keys", "ON");
    outcome?;
    restore.map_err(|error| format!("无法恢复外键开关：{error}"))?;
    // Verify after the swap: the default moved, the rows did not, and nothing broke.
    if backend_default_of(connection)?.as_deref() != Some(AGENT_RUNS_NEUTRAL_DEFAULT) {
        return Err("agent_runs.backend 默认值改写后仍不是 native".to_string());
    }
    let rows_after: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if rows_after != rows_before {
        return Err(format!(
            "agent_runs 重建前后行数不同：{rows_before} → {rows_after}"
        ));
    }
    let violations = connection
        .prepare("SELECT * FROM pragma_foreign_key_check('agent_runs')")
        .map_err(|error| error.to_string())?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .count();
    if violations > 0 {
        return Err(format!("agent_runs 重建后留下 {violations} 处外键破损"));
    }
    Ok(())
}

fn rebuild_agent_runs_default(
    connection: &mut Connection,
    body: &str,
    columns: &[String],
    companions: &[(String, String)],
    rows_before: i64,
) -> Result<(), String> {
    const SWAP: &str = "agent_runs__neutral_default";
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(&format!("DROP TABLE IF EXISTS {SWAP}"), [])
        .map_err(|error| error.to_string())?;
    transaction
        .execute(&format!("CREATE TABLE {SWAP} {body}"), [])
        .map_err(|error| format!("无法建立 agent_runs 替换表：{error}"))?;
    let list = columns.join(",");
    transaction
        .execute(
            &format!("INSERT INTO {SWAP}({list}) SELECT {list} FROM agent_runs"),
            [],
        )
        .map_err(|error| format!("无法复制 agent_runs 行：{error}"))?;
    let copied: i64 = transaction
        .query_row(&format!("SELECT COUNT(*) FROM {SWAP}"), [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if copied != rows_before {
        return Err(format!(
            "agent_runs 复制行数不符：源 {rows_before}，副本 {copied}"
        ));
    }
    // SQLite reparses external triggers during ALTER TABLE. Temporarily remove
    // those referencing this table while its old name is absent, then restore
    // their exact SQL in the same transaction. Rollback restores both schema
    // and data if any step fails.
    let external_triggers: Vec<(String, String)> = transaction
        .prepare("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name<>'agent_runs' AND instr(lower(sql),'agent_runs')>0 ORDER BY name")
        .map_err(|error| error.to_string())?
        .query_map([], |row| Ok((row.get(0)?,row.get(1)?)))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>,_>>()
        .map_err(|error| error.to_string())?;
    for (name, _) in &external_triggers {
        transaction
            .execute(
                &format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\"")),
                [],
            )
            .map_err(|error| format!("无法暂存外部 trigger：{error}"))?;
    }
    transaction
        .execute("DROP TABLE agent_runs", [])
        .map_err(|error| format!("无法移除旧 agent_runs：{error}"))?;
    transaction
        .execute(&format!("ALTER TABLE {SWAP} RENAME TO agent_runs"), [])
        .map_err(|error| format!("无法换回 agent_runs：{error}"))?;
    for (kind, sql) in companions {
        transaction
            .execute(sql, [])
            .map_err(|error| format!("无法重建 agent_runs 的 {kind}：{error}"))?;
    }
    for (_, sql) in &external_triggers {
        transaction
            .execute(sql, [])
            .map_err(|error| format!("无法恢复外部 trigger：{error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("agent_runs 默认值迁移提交失败：{error}"))?;
    Ok(())
}

/// §Stage 3 item 5: a Strix run that was still open when the backend was retired is
/// sealed as terminal. Finished Strix rows keep their status and provenance so history
/// stays readable; nothing here rewrites a backend value.
pub fn retire_resumable_strix_runs(connection: &Connection) -> Result<usize, String> {
    let changed = connection
        .execute(
            "UPDATE agent_runs SET status='legacy_backend_removed',updated_at=datetime('now','localtime') \
             WHERE backend='strix' AND status IN ('prepared','running')",
            [],
        )
        .map_err(|error| format!("无法封口历史 Strix run：{error}"))?;
    Ok(changed)
}
