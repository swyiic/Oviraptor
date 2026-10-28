//! Bounded global refinement context, excluding project-owned task knowledge.

use serde_json::Value as JsonValue;

pub(crate) fn agent_learning_catalog(
    connection: &rusqlite::Connection,
    exclude_skill_id: Option<i64>,
) -> Result<String, String> {
    let mut sections = Vec::new();
    let mut skills = connection
        .prepare("SELECT id,name,description,instructions,builtin FROM agent_skills WHERE enabled=1 AND (?1 IS NULL OR id<>?1) ORDER BY builtin DESC,updated_at DESC,id DESC LIMIT 24")
        .map_err(|error| error.to_string())?;
    let rows = skills
        .query_map([exclude_skill_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows.flatten() {
        sections.push(format!(
            "Skill #{} [{}] {}\n描述：{}\n指令：{}",
            row.0,
            if row.4 != 0 { "内置" } else { "自定义" },
            row.1,
            row.2.chars().take(700).collect::<String>(),
            row.3.chars().take(2600).collect::<String>()
        ));
    }
    let mut knowledge = connection
        // This is a global refinement catalog, not a task-scoped retrieval.
        // Per-project task cards must not leak into another project's prompt.
        .prepare("SELECT id,title,summary,patterns_json,skill_instructions FROM agent_knowledge_entries WHERE project_id IS NULL ORDER BY updated_at DESC,id DESC LIMIT 96")
        .map_err(|error| error.to_string())?;
    let rows = knowledge
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut knowledge_added = 0;
    for row in rows.flatten() {
        let patterns = serde_json::from_str::<JsonValue>(&row.3)
            .unwrap_or_else(|_| JsonValue::Object(Default::default()));
        let quality = patterns
            .get("qualityScore")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        if quality < 70 {
            continue;
        }
        let kind = patterns
            .get("knowledgeKind")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        let distinct_scans = patterns
            .pointer("/support/distinctScans")
            .and_then(JsonValue::as_i64)
            .unwrap_or_else(|| if kind == "aggregate" { 2 } else { 1 });
        if !matches!(kind, "aggregate" | "external_source")
            || distinct_scans < 2 && kind == "aggregate"
        {
            // A manually reviewed one-off can still be converted explicitly,
            // but task-derived cards never enter this cross-project catalog.
            continue;
        }
        sections.push(format!(
            "知识 #{} {}（质量 {}）\n摘要：{}\n方法：{}",
            row.0,
            row.1,
            quality,
            row.2.chars().take(700).collect::<String>(),
            row.4.chars().take(1800).collect::<String>()
        ));
        knowledge_added += 1;
        if knowledge_added >= 16 {
            break;
        }
    }
    let mut catalog = sections.join("\n\n");
    if catalog.chars().count() > 42_000 {
        catalog = catalog.chars().take(42_000).collect();
    }
    Ok(if catalog.is_empty() {
        "（暂无其它 Skill 或高质量知识）".into()
    } else {
        catalog
    })
}

