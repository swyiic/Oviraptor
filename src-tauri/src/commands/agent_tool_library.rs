// Agent tool library (design §4d / AGENT_TOP_LEVEL_REFACTOR §3 D3): the agent's OWN
// verified, reusable scripts (decryptors, carvers, parsers), kept GLOBAL across
// workspaces and recalled by target fingerprint. Unlike research cards, a tool is
// code the agent itself wrote and ran — NOT external material — so it is stored
// verbatim and is NOT poison-stripped (stripping would wrongly delete a target's
// own URLs/IPs the decrypt script legitimately needs).
//
// A tool is `pending` until the operator approves it. Approved tools are later
// surfaced to the agent as advisory "you have a reusable script for this" hints
// (same wall as memory/technique cards: advisory only, never an input to any
// auth / scope / settlement / capability gate). Before reuse on a new target the
// agent must re-read and re-run it in the sandbox to re-verify.

const AGENT_TOOL_LIBRARY_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_tool_library (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    language TEXT NOT NULL DEFAULT 'node' CHECK(language IN ('node','python')),
    code TEXT NOT NULL DEFAULT '',
    target_fingerprint TEXT NOT NULL DEFAULT '',
    algo_tags TEXT NOT NULL DEFAULT '[]',
    source_conversation_id TEXT NOT NULL DEFAULT '',
    verified INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','approved','rejected')),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    decided_at TEXT NOT NULL DEFAULT ''
);
"#;

fn agent_tool_library_exists(connection: &rusqlite::Connection) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='agent_tool_library')",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

fn agent_tool_language(language: &str) -> &'static str {
    if language == "python" { "python" } else { "node" }
}

/// Normalize the algo-tags input (a JSON array string, or empty) to a compact
/// JSON array string of trimmed, non-empty tags.
fn agent_tool_tags_json(algo_tags: Option<&str>) -> String {
    let parsed: Vec<String> = algo_tags
        .and_then(|raw| serde_json::from_str::<Vec<JsonValue>>(raw).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .take(24)
        .collect();
    serde_json::to_string(&parsed).unwrap_or_else(|_| "[]".to_string())
}

/// Store a tool (code passed in verbatim) as a pending library entry.
fn ingest_agent_tool_in(
    connection: &rusqlite::Connection,
    title: &str,
    language: &str,
    code: &str,
    target_fingerprint: &str,
    algo_tags: Option<&str>,
    source_conversation_id: &str,
) -> Result<JsonValue, String> {
    if code.trim().is_empty() {
        return Err("空脚本不入库".into());
    }
    connection
        .execute_batch(AGENT_TOOL_LIBRARY_SCHEMA)
        .map_err(|_| "agent_tool_library_storage_unavailable")?;
    let id = uuid::Uuid::new_v4().to_string();
    let lang = agent_tool_language(language);
    let tags = agent_tool_tags_json(algo_tags);
    connection
        .execute(
            "INSERT INTO agent_tool_library(id,title,language,code,target_fingerprint,algo_tags,source_conversation_id) \
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                title.trim(),
                lang,
                code,
                target_fingerprint.trim(),
                tags,
                source_conversation_id.trim()
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(agent_tool_json(
        &id,
        title.trim(),
        lang,
        code,
        target_fingerprint.trim(),
        &tags,
        source_conversation_id.trim(),
        false,
        "pending",
        "",
    ))
}

#[allow(clippy::too_many_arguments)]
fn agent_tool_json(
    id: &str,
    title: &str,
    language: &str,
    code: &str,
    target_fingerprint: &str,
    algo_tags_json: &str,
    source_conversation_id: &str,
    verified: bool,
    status: &str,
    created_at: &str,
) -> JsonValue {
    serde_json::json!({
        "id": id,
        "title": title,
        "language": language,
        "code": code,
        "targetFingerprint": target_fingerprint,
        "algoTags": serde_json::from_str::<JsonValue>(algo_tags_json).unwrap_or_else(|_| serde_json::json!([])),
        "sourceConversationId": source_conversation_id,
        "verified": verified,
        "status": status,
        "createdAt": created_at,
    })
}

fn list_agent_tools_in(
    connection: &rusqlite::Connection,
    status: Option<&str>,
) -> Result<Vec<JsonValue>, String> {
    if !agent_tool_library_exists(connection) {
        return Ok(vec![]);
    }
    let mut statement = connection
        .prepare(
            "SELECT id,title,language,code,target_fingerprint,algo_tags,source_conversation_id,verified,status,created_at \
             FROM agent_tool_library WHERE (?1 IS NULL OR status=?1) ORDER BY created_at DESC, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![status], |r| {
            Ok(agent_tool_json(
                &r.get::<_, String>(0)?,
                &r.get::<_, String>(1)?,
                &r.get::<_, String>(2)?,
                &r.get::<_, String>(3)?,
                &r.get::<_, String>(4)?,
                &r.get::<_, String>(5)?,
                &r.get::<_, String>(6)?,
                r.get::<_, i64>(7)? != 0,
                &r.get::<_, String>(8)?,
                &r.get::<_, String>(9)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Operator decision; only a pending tool can be decided.
fn decide_agent_tool_in(
    connection: &rusqlite::Connection,
    id: &str,
    approve: bool,
) -> Result<(), String> {
    if !agent_tool_library_exists(connection) {
        return Err("没有该工具".into());
    }
    let status = if approve { "approved" } else { "rejected" };
    let changed = connection
        .execute(
            "UPDATE agent_tool_library SET status=?2, decided_at=datetime('now','localtime') WHERE id=?1 AND status='pending'",
            params![id, status],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("该工具不存在或已不是待审状态".into());
    }
    Ok(())
}

fn delete_agent_tool_in(connection: &rusqlite::Connection, id: &str) -> Result<(), String> {
    if !agent_tool_library_exists(connection) {
        return Ok(());
    }
    connection
        .execute("DELETE FROM agent_tool_library WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Operator / agent: sink a verified script into the global tool library (pending).
#[tauri::command]
pub fn ingest_agent_tool(
    state: State<AppState>,
    title: String,
    language: String,
    code: String,
    target_fingerprint: Option<String>,
    algo_tags: Option<String>,
    source_conversation_id: Option<String>,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    ingest_agent_tool_in(
        &connection,
        &title,
        &language,
        &code,
        target_fingerprint.as_deref().unwrap_or(""),
        algo_tags.as_deref(),
        source_conversation_id.as_deref().unwrap_or(""),
    )
}

/// Operator: list tools, optionally filtered by status.
#[tauri::command]
pub fn list_agent_tools(
    state: State<AppState>,
    status: Option<String>,
) -> Result<Vec<JsonValue>, String> {
    let connection = db::open(&state.db_path)?;
    list_agent_tools_in(&connection, status.as_deref())
}

/// Operator: approve or reject one pending tool.
#[tauri::command]
pub fn decide_agent_tool(state: State<AppState>, id: String, approve: bool) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    decide_agent_tool_in(&connection, &id, approve)
}

/// Operator: delete a tool.
#[tauri::command]
pub fn delete_agent_tool(state: State<AppState>, id: String) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    delete_agent_tool_in(&connection, &id)
}

#[cfg(test)]
mod agent_tool_library_tests {
    use super::{decide_agent_tool_in, delete_agent_tool_in, ingest_agent_tool_in, list_agent_tools_in};
    use rusqlite::Connection;

    fn db() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn ingest_stores_code_verbatim_without_stripping() {
        let c = db();
        // a real decrypt script keeps the target's own URL/IP — must NOT be stripped
        let code = "const r=require('crypto');fetch('http://10.255.134.90/api/getKey');// SM4-CBC";
        let tool = ingest_agent_tool_in(
            &c, "SM4 解密器", "node", code, "国密SM4-CBC+RSA会话密钥",
            Some("[\"sm4\",\"cbc\",\"rsa\"]"), "conv-1",
        )
        .unwrap();
        assert_eq!(tool["code"], code); // verbatim, callback URL/IP intact
        assert_eq!(tool["language"], "node");
        assert_eq!(tool["status"], "pending");
        assert_eq!(tool["verified"], false);
        assert_eq!(tool["targetFingerprint"], "国密SM4-CBC+RSA会话密钥");
        assert_eq!(tool["algoTags"].as_array().unwrap().len(), 3);
        assert_eq!(tool["sourceConversationId"], "conv-1");
    }

    #[test]
    fn empty_code_is_rejected() {
        let c = db();
        assert!(ingest_agent_tool_in(&c, "t", "node", "   ", "", None, "").is_err());
    }

    #[test]
    fn list_filter_and_decide_roundtrip() {
        let c = db();
        let tool = ingest_agent_tool_in(&c, "t", "python", "print(1)", "", None, "").unwrap();
        let id = tool["id"].as_str().unwrap().to_string();
        assert_eq!(tool["language"], "python");
        assert_eq!(list_agent_tools_in(&c, Some("pending")).unwrap().len(), 1);
        decide_agent_tool_in(&c, &id, true).unwrap();
        assert_eq!(list_agent_tools_in(&c, Some("approved")).unwrap().len(), 1);
        assert_eq!(list_agent_tools_in(&c, Some("pending")).unwrap().len(), 0);
        assert_eq!(list_agent_tools_in(&c, None).unwrap().len(), 1);
        // a decided tool cannot be re-decided
        assert!(decide_agent_tool_in(&c, &id, false).is_err());
        delete_agent_tool_in(&c, &id).unwrap();
        assert_eq!(list_agent_tools_in(&c, None).unwrap().len(), 0);
    }

    #[test]
    fn unknown_language_falls_back_to_node() {
        let c = db();
        let tool = ingest_agent_tool_in(&c, "t", "ruby", "x", "", None, "").unwrap();
        assert_eq!(tool["language"], "node");
    }
}
