fn collect_checkpoint_files(
    dir: &std::path::Path,
    found: &mut Vec<(String, JsonValue)>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect_checkpoint_files(&path, found)?;
            continue;
        }
        if path.file_name().and_then(|v| v.to_str()) == Some("meta.json")
            || path.extension().and_then(|v| v.to_str()) != Some("json")
        {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let stage = ["s1", "s2", "s3", "s4", "s5"]
            .iter()
            .find(|item| stem.starts_with(**item))
            .map(|item| item.to_string())
            .or_else(|| (stem == "summary").then(|| "summary".to_string()));
        if let Some(stage) = stage {
            if let Ok(value) =
                serde_json::from_slice::<JsonValue>(&fs::read(&path).map_err(|e| e.to_string())?)
            {
                found.push((stage, value));
            }
        }
    }
    Ok(())
}

fn value_text(value: Option<&JsonValue>) -> String {
    value.and_then(JsonValue::as_str).unwrap_or("").to_string()
}
fn value_key(value: &JsonValue, keys: &[&str]) -> String {
    let parts: Vec<String> = keys
        .iter()
        .filter_map(|key| {
            value.get(*key).and_then(|item| match item {
                JsonValue::String(text) if !text.trim().is_empty() => Some(text.to_string()),
                JsonValue::Number(number) => Some(number.to_string()),
                _ => None,
            })
        })
        .collect();
    parts.join("|")
}
fn value_first(value: &JsonValue, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            value.get(*key).and_then(|item| match item {
                JsonValue::String(text) if !text.trim().is_empty() => Some(text.to_string()),
                _ => None,
            })
        })
        .unwrap_or_default()
}
#[allow(clippy::too_many_arguments)]
fn insert_finding(
    connection: &rusqlite::Connection,
    scan_id: &str,
    target_url: &str,
    stage: &str,
    kind: &str,
    record_key: &str,
    title: &str,
    severity: &str,
    value: &JsonValue,
) -> Result<(), String> {
    connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')", params![scan_id,target_url,stage,kind,record_key,title,severity,value.to_string()]).map_err(|e| e.to_string())?;
    Ok(())
}

fn sarif_severity(level: &str) -> &'static str {
    match level.to_ascii_lowercase().as_str() {
        "error" | "critical" => "high",
        "warning" | "high" => "medium",
        "note" | "low" => "low",
        _ => "info",
    }
}

fn import_sarif_findings(
    connection: &rusqlite::Connection,
    scan_id: &str,
    path: &Path,
    engine: &str,
) -> Result<i64, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let document: JsonValue =
        serde_json::from_slice(&bytes).map_err(|e| format!("{engine} SARIF 无法解析：{e}"))?;
    let mut imported = 0;
    for run in document
        .get("runs")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let rules = run
            .get("tool")
            .and_then(|v| v.get("driver"))
            .and_then(|v| v.get("rules"));
        for result in run
            .get("results")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            let rule_id = result
                .get("ruleId")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown");
            let message = result
                .get("message")
                .and_then(|v| v.get("text"))
                .and_then(JsonValue::as_str)
                .unwrap_or("未提供规则描述");
            let level = result
                .get("level")
                .and_then(JsonValue::as_str)
                .unwrap_or("warning");
            let severity = sarif_severity(level);
            let rule = rules.and_then(JsonValue::as_array).and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("id").and_then(JsonValue::as_str) == Some(rule_id))
            });
            let title = rule
                .and_then(|v| v.get("shortDescription"))
                .and_then(|v| v.get("text"))
                .and_then(JsonValue::as_str)
                .unwrap_or(rule_id);
            for (index, location) in result
                .get("locations")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let physical = location.get("physicalLocation").unwrap_or(location);
                let file = physical
                    .get("artifactLocation")
                    .and_then(|v| v.get("uri"))
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                let region = physical
                    .get("region")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let mut record = serde_json::json!({"engine":engine,"rule_id":rule_id,"message":message,"level":level,"file":file,"start_line":region.get("startLine").cloned().unwrap_or(JsonValue::Null),"end_line":region.get("endLine").cloned().unwrap_or(JsonValue::Null),"sarif":result});
                if let Some(properties) = rule.and_then(|v| v.get("properties")) {
                    record["rule_properties"] = properties.clone();
                }
                let key = format!(
                    "{engine}:{rule_id}:{file}:{}",
                    region
                        .get("startLine")
                        .and_then(JsonValue::as_i64)
                        .unwrap_or(index as i64)
                );
                insert_finding(
                    connection,
                    scan_id,
                    "*",
                    "local-sast",
                    "vulnerability",
                    &key,
                    title,
                    severity,
                    &record,
                )?;
                imported += 1;
            }
        }
    }
    Ok(imported)
}
