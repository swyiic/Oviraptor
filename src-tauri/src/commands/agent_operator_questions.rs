// Structured human-in-the-loop: the agent asks the operator a question (with
// optional choices) and waits. The model cannot answer its own question; only
// IPC from the UI may resolve a row. Collaboration events notify the process UI.

const OPERATOR_QUESTIONS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_operator_questions (
    question_id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    run_id TEXT NOT NULL DEFAULT '',
    question TEXT NOT NULL,
    options_json TEXT NOT NULL DEFAULT '[]',
    context_note TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK(status IN ('pending','answered','dismissed')),
    answer TEXT NOT NULL DEFAULT '',
    selected_option TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    answered_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_agent_operator_questions_scan
    ON agent_operator_questions(scan_id, status, created_at);
"#;

fn ensure_operator_questions_storage(connection: &rusqlite::Connection) -> Result<(), String> {
    connection
        .execute_batch(OPERATOR_QUESTIONS_SCHEMA)
        .map_err(|_| "operator_question_storage_unavailable".into())
}

fn emit_operator_question_event(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    question_id: &str,
    payload: &JsonValue,
) {
    let _ = connection.execute(
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) \
         VALUES(?1,?2,'operator_question',?3,'operator_question',?4)",
        params![scan_id, attempt_number, question_id, payload.to_string()],
    );
}

fn parse_options(arguments: &JsonValue) -> Vec<String> {
    arguments
        .get("options")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::trim).filter(|s| !s.is_empty()))
                .take(8)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn agent_tool_ask_operator(
    context: &AgentRunContext,
    _runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let question = value_first(arguments, &["question"]);
    if question.trim().is_empty() {
        return agent_tool_error("缺少 question", "invalid_arguments");
    }
    let connection = match db::open(&context.db_path) {
        Ok(connection) => connection,
        Err(error) => return agent_tool_error(&error, "tool_authorization_unavailable"),
    };
    if ensure_operator_questions_storage(&connection).is_err() {
        return agent_tool_error("无法初始化 operator 问答存储", "operator_question_storage_unavailable");
    }

    if let Some(existing_id) = arguments
        .get("questionId")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        let row: Option<(String, String, String)> = connection
            .query_row(
                "SELECT status, answer, selected_option FROM agent_operator_questions \
                 WHERE question_id=?1 AND scan_id=?2 AND attempt_number=?3",
                params![existing_id, &context.scan_id, context.attempt_number],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .ok()
            .flatten();
        if let Some((status, answer, selected)) = row {
            return match status.as_str() {
                "answered" => serde_json::json!({
                    "status": "answered",
                    "questionId": existing_id,
                    "answer": answer,
                    "selectedOption": selected,
                }),
                "dismissed" => serde_json::json!({
                    "status": "dismissed",
                    "questionId": existing_id,
                    "code": "operator_question_dismissed",
                    "error": "操作者已关闭此问题，请改走其它路径或 finish_target",
                }),
                _ => serde_json::json!({
                    "status": "pending",
                    "questionId": existing_id,
                    "code": "operator_question_pending",
                    "hint": "操作者尚未回答；用相同 questionId 稍后重试，不要重复创建新问题",
                }),
            };
        }
    }

    let options = parse_options(arguments);
    let context_note = value_first(arguments, &["context"]);
    let question_id = uuid::Uuid::new_v4().to_string();
    let run_id = context
        .run
        .as_ref()
        .map(|run| run.run_id.as_str())
        .unwrap_or("");
    let options_json = serde_json::to_string(&options).unwrap_or_else(|_| "[]".into());
    if connection
        .execute(
            "INSERT INTO agent_operator_questions(question_id,scan_id,attempt_number,run_id,question,options_json,context_note) \
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                question_id,
                &context.scan_id,
                context.attempt_number,
                run_id,
                question.trim(),
                options_json,
                context_note.trim(),
            ],
        )
        .map_err(|error| error.to_string())
        .is_err()
    {
        return agent_tool_error("无法写入 operator 问题", "operator_question_write_failed");
    }
    emit_operator_question_event(
        &connection,
        &context.scan_id,
        context.attempt_number,
        &question_id,
        &serde_json::json!({
            "status": "pending",
            "question": question.trim(),
            "options": options,
        }),
    );
    serde_json::json!({
        "status": "pending",
        "questionId": question_id,
        "code": "operator_question_pending",
        "options": options,
        "hint": "等待操作者在扫描界面回答；用 questionId 轮询，勿重复提问",
    })
}

fn list_pending_operator_questions_in(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Vec<JsonValue>, String> {
    ensure_operator_questions_storage(connection)?;
    let mut statement = connection
        .prepare(
            "SELECT question_id, attempt_number, question, options_json, context_note, created_at \
             FROM agent_operator_questions WHERE scan_id=?1 AND status='pending' ORDER BY created_at",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([scan_id], |row| {
            let options_raw: String = row.get(3)?;
            let options: Vec<String> = serde_json::from_str(&options_raw).unwrap_or_default();
            Ok(serde_json::json!({
                "questionId": row.get::<_, String>(0)?,
                "attemptNumber": row.get::<_, i64>(1)?,
                "question": row.get::<_, String>(2)?,
                "options": options,
                "context": row.get::<_, String>(4)?,
                "createdAt": row.get::<_, String>(5)?,
            }))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn answer_operator_question_inner(
    db_path: &Path,
    question_id: &str,
    answer: &str,
    selected_option: Option<&str>,
) -> Result<(), String> {
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Err("operator_question_answer_empty".into());
    }
    let connection = db::open(db_path)?;
    ensure_operator_questions_storage(&connection)?;
    let meta: (String, i64) = connection
        .query_row(
            "SELECT scan_id, attempt_number FROM agent_operator_questions WHERE question_id=?1 AND status='pending'",
            [question_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "operator_question_not_pending".to_string())?;
    let selected = selected_option.unwrap_or("").trim();
    let changed = connection
        .execute(
            "UPDATE agent_operator_questions SET status='answered', answer=?2, selected_option=?3, \
             answered_at=datetime('now','localtime') WHERE question_id=?1 AND status='pending'",
            params![question_id, trimmed, selected],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("operator_question_not_pending".into());
    }
    emit_operator_question_event(
        &connection,
        &meta.0,
        meta.1,
        question_id,
        &serde_json::json!({"status":"answered","answer":trimmed,"selectedOption":selected}),
    );
    Ok(())
}

#[tauri::command]
pub fn list_pending_operator_questions(
    state: State<AppState>,
    scan_id: String,
) -> Result<Vec<JsonValue>, String> {
    let connection = db::open(&state.db_path)?;
    list_pending_operator_questions_in(&connection, &scan_id)
}

#[tauri::command]
pub fn answer_operator_question(
    state: State<AppState>,
    question_id: String,
    answer: String,
    selected_option: Option<String>,
) -> Result<(), String> {
    answer_operator_question_inner(
        &state.db_path,
        &question_id,
        &answer,
        selected_option.as_deref(),
    )
}

#[tauri::command]
pub fn dismiss_operator_question(state: State<AppState>, question_id: String) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    ensure_operator_questions_storage(&connection)?;
    let meta: (String, i64) = connection
        .query_row(
            "SELECT scan_id, attempt_number FROM agent_operator_questions WHERE question_id=?1 AND status='pending'",
            [&question_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "operator_question_not_pending".to_string())?;
    let changed = connection
        .execute(
            "UPDATE agent_operator_questions SET status='dismissed', answered_at=datetime('now','localtime') \
             WHERE question_id=?1 AND status='pending'",
            [&question_id],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("operator_question_not_pending".into());
    }
    emit_operator_question_event(
        &connection,
        &meta.0,
        meta.1,
        &question_id,
        &serde_json::json!({"status":"dismissed"}),
    );
    Ok(())
}

#[cfg(test)]
mod operator_question_tests {
    use super::{answer_operator_question_inner, ensure_operator_questions_storage, list_pending_operator_questions_in};
    use rusqlite::Connection;

    #[test]
    fn answer_marks_row_answered() {
        let db_path = std::env::temp_dir().join(format!("oq-{}.sqlite3", uuid::Uuid::new_v4()));
        {
            let c = Connection::open(&db_path).unwrap();
            c.execute_batch(
                "CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY);
                 INSERT INTO sentinel_scans(id) VALUES('s');
                 CREATE TABLE agent_collaboration_events(
                   id INTEGER PRIMARY KEY AUTOINCREMENT, scan_id TEXT, attempt_number INTEGER,
                   entity_type TEXT, entity_id TEXT, event_type TEXT, payload_json TEXT, sequence INTEGER DEFAULT 1);",
            )
            .unwrap();
            ensure_operator_questions_storage(&c).unwrap();
            c.execute(
                "INSERT INTO agent_operator_questions(question_id,scan_id,attempt_number,question) VALUES('q','s',1,'hi')",
                [],
            )
            .unwrap();
        }
        answer_operator_question_inner(&db_path, "q", "ok", None).unwrap();
        assert!(list_pending_operator_questions_in(&Connection::open(&db_path).unwrap(), "s")
            .unwrap()
            .is_empty());
    }
}
