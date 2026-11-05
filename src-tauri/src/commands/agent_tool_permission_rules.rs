// Fine-grained allow / ask / deny overlay on operator tier (OpenCode-style glob,
// last match wins). Axis-2 capability authorization is unchanged; these rules only
// refine axis-1 behaviour and hard-deny specific tools.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ToolPermissionDecision {
    Default,
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
struct ToolPermissionRule {
    pattern: String,
    decision: String,
}

const DEFAULT_RULES_JSON: &str = r#"[
  {"pattern":"ask_operator","decision":"allow"},
  {"pattern":"call_mcp_tool","decision":"ask"},
  {"pattern":"inspect_evidence","decision":"allow"},
  {"pattern":"record_hypothesis_result","decision":"allow"},
  {"pattern":"finish_target","decision":"allow"},
  {"pattern":"probe_command_execution","decision":"ask"},
  {"pattern":"run_target_poc","decision":"ask"},
  {"pattern":"run_analysis_script","decision":"ask"}
]"#;

fn glob_match(pattern: &str, text: &str) -> bool {
    let pat = pattern.trim().to_ascii_lowercase();
    let hay = text.trim().to_ascii_lowercase();
    if pat.is_empty() {
        return false;
    }
    if !pat.contains('*') {
        return pat == hay;
    }
    let mut parts = pat.split('*').collect::<Vec<_>>();
    if parts.is_empty() {
        return true;
    }
    let mut cursor = hay.as_str();
    if !pat.starts_with('*') {
        let head = parts.remove(0);
        if head.is_empty() || !cursor.starts_with(head) {
            return false;
        }
        cursor = &cursor[head.len()..];
    }
    if !pat.ends_with('*') {
        let tail = parts.pop().unwrap_or("");
        if !tail.is_empty() && !cursor.ends_with(tail) {
            return false;
        }
        if !tail.is_empty() {
            cursor = &cursor[..cursor.len().saturating_sub(tail.len())];
        }
    }
    for part in parts {
        if part.is_empty() {
            continue;
        }
        let Some(index) = cursor.find(part) else {
            return false;
        };
        cursor = &cursor[index + part.len()..];
    }
    true
}

fn parse_rules(raw: &str) -> Vec<ToolPermissionRule> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn stored_rules(connection: &rusqlite::Connection) -> Vec<ToolPermissionRule> {
    let custom: Option<String> = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='agent_tool_permission_rules'",
            [],
            |row| row.get(0),
        )
        .ok();
    if let Some(json) = custom.filter(|value| !value.trim().is_empty()) {
        let parsed = parse_rules(&json);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    parse_rules(DEFAULT_RULES_JSON)
}

pub(crate) fn tool_permission_decision(
    connection: &rusqlite::Connection,
    tool_name: &str,
) -> ToolPermissionDecision {
    let mut decision = ToolPermissionDecision::Default;
    for rule in stored_rules(connection) {
        if glob_match(&rule.pattern, tool_name) {
            decision = match rule.decision.to_ascii_lowercase().as_str() {
                "allow" => ToolPermissionDecision::Allow,
                "ask" => ToolPermissionDecision::Ask,
                "deny" => ToolPermissionDecision::Deny,
                _ => ToolPermissionDecision::Default,
            };
        }
    }
    decision
}

#[tauri::command]
pub fn get_agent_tool_permission_rules(state: State<AppState>) -> Result<Vec<JsonValue>, String> {
    let connection = db::open(&state.db_path)?;
    Ok(stored_rules(&connection)
        .into_iter()
        .map(|rule| {
            serde_json::json!({
                "pattern": rule.pattern,
                "decision": rule.decision,
            })
        })
        .collect())
}

#[tauri::command]
pub fn set_agent_tool_permission_rules(
    state: State<AppState>,
    rules: Vec<JsonValue>,
) -> Result<(), String> {
    let mut normalized = Vec::new();
    for rule in rules {
        let pattern = rule
            .get("pattern")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or("permission_rule_pattern_invalid")?;
        let decision = rule
            .get("decision")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| matches!(*value, "allow" | "ask" | "deny"))
            .ok_or("permission_rule_decision_invalid")?;
        normalized.push(ToolPermissionRule {
            pattern: pattern.to_string(),
            decision: decision.to_string(),
        });
    }
    if normalized.is_empty() {
        return Err("permission_rules_empty".into());
    }
    let payload = serde_json::to_string(&normalized).map_err(|error| error.to_string())?;
    let connection = db::open(&state.db_path)?;
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES('agent_tool_permission_rules',?1) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [payload],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod permission_rule_tests {
    use super::{glob_match, tool_permission_decision, ToolPermissionDecision};
    use rusqlite::Connection;

    #[test]
    fn glob_star_matches() {
        assert!(glob_match("probe_*", "probe_sql_injection"));
        assert!(glob_match("*_script", "run_analysis_script"));
        assert!(!glob_match("probe_*", "replay_http"));
    }

    #[test]
    fn defaults_force_ask_on_poc() {
        let c = Connection::open_in_memory().unwrap();
        assert_eq!(
            tool_permission_decision(&c, "run_target_poc"),
            ToolPermissionDecision::Ask
        );
        assert_eq!(
            tool_permission_decision(&c, "inspect_evidence"),
            ToolPermissionDecision::Allow
        );
    }
}
