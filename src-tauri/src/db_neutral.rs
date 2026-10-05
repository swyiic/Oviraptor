// Native knowledge schema and its built-in skill. Included from `db.rs`.
// Retired backend tables are never imported or changed on startup.

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

fn seed_builtin_agent_skill(connection: &mut Connection) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR IGNORE INTO agent_skills(name,description,instructions,builtin,enabled) VALUES('业务前端深度分析','按看功能、触发请求、还原参数、分析业务 JS、匹配本地知识和一次性保底发现的顺序执行；只把证据充分的高价值候选交给后续复核。',?1,1,1)",
            [DEFAULT_BUSINESS_FRONTEND_SKILL],
        )
        .map_err(|error| format!("无法写入内置技能：{error}"))?;
    Ok(())
}
