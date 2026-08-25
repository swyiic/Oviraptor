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

fn enabled_rule_pack(connection: &rusqlite::Connection, engine: &str) -> Option<String> {
    connection.query_row("SELECT local_path FROM security_rule_packs WHERE engine=?1 AND enabled=1 AND status='ready' AND local_path<>'' ORDER BY builtin DESC,id LIMIT 1",[engine],|row|row.get(0)).optional().ok().flatten()
}

fn run_local_security_engines(
    db_path: &Path,
    scan_id: &str,
    work_dir: &Path,
    source_path: &str,
) -> String {
    if source_path.trim().is_empty() {
        return "未提供源码路径，跳过本地规则引擎".into();
    }
    let connection = match db::open(db_path) {
        Ok(value) => value,
        Err(error) => return format!("本地规则引擎数据库不可用：{error}"),
    };
    let mut notes = Vec::new();
    let source = Path::new(source_path);
    if let Some(config) = enabled_rule_pack(&connection, "semgrep") {
        let output = work_dir.join("semgrep.sarif");
        let result = Command::new("semgrep")
            .args([
                "--config",
                &config,
                "--sarif",
                "--output",
                &output.to_string_lossy(),
                source_path,
            ])
            .output();
        match result {
            Ok(value) if value.status.success() => {
                match import_sarif_findings(&connection, scan_id, &output, "semgrep") {
                    Ok(count) => notes.push(format!("Semgrep {count} 条")),
                    Err(error) => notes.push(error),
                }
            }
            Ok(value) => notes.push(format!(
                "Semgrep 未完成：{}",
                String::from_utf8_lossy(&value.stderr).trim()
            )),
            Err(_) => notes.push("Semgrep CLI 未安装".into()),
        }
    } else {
        notes.push("Semgrep 规则库未同步".into());
    }
    if let Some(config) = enabled_rule_pack(&connection, "codeql") {
        let codeql_db = work_dir.join("codeql-db");
        let sarif = work_dir.join("codeql.sarif");
        let codeql_target = if source.join("package.json").exists() {
            Some(("javascript-typescript", "javascript"))
        } else if source.join("pom.xml").exists()
            || source.join("build.gradle").exists()
            || source.join("build.gradle.kts").exists()
        {
            Some(("java-kotlin", "java"))
        } else if source.join("go.mod").exists() {
            Some(("go", "go"))
        } else if source.join("pyproject.toml").exists() || source.join("requirements.txt").exists()
        {
            Some(("python", "python"))
        } else if source.join("Gemfile").exists() {
            Some(("ruby", "ruby"))
        } else {
            None
        };
        let Some((language, query_folder)) = codeql_target else {
            notes.push("CodeQL 跳过：仓库主语言没有可用的 CodeQL 提取器".into());
            return notes.join("；");
        };
        let query_path = [
            Path::new(&config).join(query_folder).join("ql/src"),
            Path::new(&config).join("qlpacks/codeql").join(query_folder),
            Path::new(&config).join(query_folder),
            Path::new(&config).to_path_buf(),
        ]
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(config.clone()));
        let query_path_text = query_path.to_string_lossy().to_string();
        let create = Command::new("codeql")
            .args([
                "database",
                "create",
                &codeql_db.to_string_lossy(),
                "--language",
                language,
                "--source-root",
                source_path,
                "--overwrite",
            ])
            .output();
        match create {
            Ok(value) if value.status.success() => {
                let analyze = Command::new("codeql")
                    .args([
                        "database",
                        "analyze",
                        &codeql_db.to_string_lossy(),
                        &query_path_text,
                        "--format=sarif-latest",
                        "--output",
                        &sarif.to_string_lossy(),
                    ])
                    .output();
                match analyze {
                    Ok(value) if value.status.success() => {
                        match import_sarif_findings(&connection, scan_id, &sarif, "codeql") {
                            Ok(count) => notes.push(format!("CodeQL {count} 条")),
                            Err(error) => notes.push(error),
                        }
                    }
                    Ok(value) => notes.push(format!(
                        "CodeQL 分析失败：{}",
                        String::from_utf8_lossy(&value.stderr).trim()
                    )),
                    Err(_) => notes.push("CodeQL CLI 未安装".into()),
                }
            }
            Ok(value) => notes.push(format!(
                "CodeQL 数据库创建失败：{}",
                String::from_utf8_lossy(&value.stderr).trim()
            )),
            Err(_) => notes.push("CodeQL CLI 未安装".into()),
        }
    } else {
        notes.push("CodeQL 查询库未同步".into());
    }
    notes.join("；")
}
fn parse_finding_array(
    connection: &rusqlite::Connection,
    scan_id: &str,
    target_url: &str,
    stage: &str,
    kind: &str,
    value: Option<&JsonValue>,
    key_fields: &[&str],
    title_field: &str,
    severity_field: &str,
) -> Result<i64, String> {
    let mut count = 0;
    if let Some(JsonValue::Array(items)) = value {
        for (index, item) in items.iter().enumerate() {
            let key = value_key(item, key_fields);
            let key = if key.is_empty() {
                index.to_string()
            } else {
                key
            };
            let title = value_text(item.get(title_field));
            let severity = value_text(item.get(severity_field));
            let item_target = value_first(item, &["target", "targetUrl", "baseTarget"]);
            let item_target = if item_target == "ALL" || item_target == "all" {
                "*"
            } else if item_target.is_empty() {
                target_url
            } else {
                &item_target
            };
            insert_finding(
                connection,
                scan_id,
                item_target,
                stage,
                kind,
                &key,
                &title,
                &severity,
                item,
            )?;
            count += 1;
        }
    }
    Ok(count)
}
fn parse_checkpoint_findings(
    connection: &rusqlite::Connection,
    scan_id: &str,
    stage: &str,
    value: &JsonValue,
) -> Result<i64, String> {
    if stage == "s1" {
        if let Some(JsonValue::Array(items)) = value.get("targets") {
            let mut total = 0;
            for item in items {
                total += parse_checkpoint_findings(connection, scan_id, stage, item)?;
            }
            return Ok(total);
        }
    }
    let value = match stage {
        "s2" => value.pointer("/data/jsAnalysis").unwrap_or(value),
        "s3" => value.pointer("/data/endpointDiscovery").unwrap_or(value),
        "s4" => value.pointer("/data/parameterAnalysis").unwrap_or(value),
        "s5" => value.get("data").unwrap_or(value),
        _ => value,
    };
    let target_url = value_first(value, &["url", "target", "host"]);
    let mut count = 0;
    match stage {
        "s1" => {
            for (kind, key) in [
                ("fingerprint", "fingerprint"),
                ("wordpress", "wordpress"),
                ("tech_stack", "techStack"),
                ("meta_tags", "metaTags"),
                ("links", "links"),
            ] {
                if let Some(item) = value.get(key) {
                    insert_finding(
                        connection,
                        scan_id,
                        &target_url,
                        stage,
                        kind,
                        "root",
                        key,
                        "",
                        item,
                    )?;
                    count += 1;
                }
            }
            if let Some(JsonValue::Object(headers)) = value.get("securityHeaders") {
                for (name, item) in headers {
                    let title = name.clone();
                    let severity = value_text(item.get("risk"));
                    insert_finding(
                        connection,
                        scan_id,
                        &target_url,
                        stage,
                        "security_header",
                        name,
                        &title,
                        &severity,
                        item,
                    )?;
                    count += 1;
                }
            }
            for (key, kind, fields) in [
                ("cookies", "cookie", &["name"][..]),
                ("openPorts", "open_port", &["port", "service"][..]),
                ("infoDisclosure", "info_disclosure", &["key", "type"][..]),
                ("externalServices", "external_service", &["name", "url"][..]),
            ] {
                count += parse_finding_array(
                    connection,
                    scan_id,
                    &target_url,
                    stage,
                    kind,
                    value.get(key),
                    fields,
                    "name",
                    "risk",
                )?;
            }
        }
        "s2" => {
            for (key, kind, fields) in [
                ("jsFiles", "js_file", &["url"][..]),
                ("apis", "api", &["path", "method"][..]),
                ("routes", "route", &["path"][..]),
                (
                    "registrationEntrypoints",
                    "registration_endpoint",
                    &["url", "method"][..],
                ),
                ("sensitiveInfo", "sensitive_info", &["file", "value"][..]),
                ("envVars", "env_var", &["key", "file"][..]),
                ("externalScripts", "external_script", &["url"][..]),
            ] {
                count += parse_finding_array(
                    connection,
                    scan_id,
                    &target_url,
                    stage,
                    kind,
                    value.get(key),
                    fields,
                    "title",
                    "severity",
                )?;
            }
        }
        "s3" => {
            for (key, kind, fields) in [
                ("verified", "endpoint", &["url", "method"][..]),
                (
                    "abbreviationExpanded",
                    "endpoint_expanded",
                    &["url", "method"][..],
                ),
                ("directoryFinds", "directory_find", &["url"][..]),
                ("restEndpoints", "rest_endpoint", &["pattern"][..]),
                ("loginEndpoints", "login_endpoint", &["url", "method"][..]),
            ] {
                count += parse_finding_array(
                    connection,
                    scan_id,
                    &target_url,
                    stage,
                    kind,
                    value.get(key),
                    fields,
                    "title",
                    "risk",
                )?;
            }
            if let Some(item) = value.get("fixed404") {
                insert_finding(
                    connection,
                    scan_id,
                    &target_url,
                    stage,
                    "fixed_404",
                    "root",
                    "404特征",
                    "",
                    item,
                )?;
                count += 1;
            }
        }
        "s4" => {
            for (key, kind) in [
                ("jsonEndpoints", "parameter_json"),
                ("xmlEndpoints", "parameter_xml"),
                ("formEndpoints", "parameter_form"),
                ("uploadEndpoints", "parameter_upload"),
                ("pathParams", "parameter_path"),
                ("queryParams", "parameter_query"),
            ] {
                count += parse_finding_array(
                    connection,
                    scan_id,
                    &target_url,
                    stage,
                    kind,
                    value.get(key),
                    &["url", "method", "pattern"],
                    "title",
                    "severity",
                )?;
            }
        }
        "s5" => {
            count += parse_finding_array(
                connection,
                scan_id,
                &target_url,
                stage,
                "vulnerability",
                value.get("vulnerabilities"),
                &["id", "type", "title", "url"],
                "title",
                "severity",
            )?;
            count += parse_finding_array(
                connection,
                scan_id,
                &target_url,
                stage,
                "poc_test",
                value.get("pocTests"),
                &["name", "url"],
                "name",
                "result",
            )?;
            count += parse_finding_array(
                connection,
                scan_id,
                &target_url,
                stage,
                "login_endpoint",
                value.get("loginEndpoints"),
                &["url", "method"],
                "title",
                "risk",
            )?;
        }
        "summary" => {
            if let Some(item) = value.get("riskSummary") {
                let level = value_text(item.get("level"));
                insert_finding(
                    connection,
                    scan_id,
                    &target_url,
                    stage,
                    "risk_summary",
                    "root",
                    "风险汇总",
                    &level,
                    item,
                )?;
                count += 1;
            }
            if let Some(JsonValue::Array(items)) = value.get("targets") {
                for (index, item) in items.iter().enumerate() {
                    let url = value_first(item, &["url", "target"]);
                    let key = if url.is_empty() {
                        index.to_string()
                    } else {
                        url.clone()
                    };
                    let title = value_text(item.get("status"));
                    let severity = value_first(item, &["risk", "severity"]);
                    insert_finding(
                        connection,
                        scan_id,
                        &url,
                        stage,
                        "summary_target",
                        &key,
                        &title,
                        &severity,
                        item,
                    )?;
                    count += 1;
                }
            }
        }
        _ => {}
    }
    Ok(count)
}

fn expand_home_path(value: &str, home: &std::path::Path) -> PathBuf {
    let trimmed = value.trim();
    if trimmed == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        return home.join(rest);
    }
    PathBuf::from(trimmed)
}

fn platform_user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            Some(PathBuf::from(drive).join(path))
        })
}

fn strix_run_roots(connection: &rusqlite::Connection, state: &AppState) -> Vec<PathBuf> {
    let settings: String = connection
        .query_row(
            "SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "{}".into());
    let settings = json(settings);
    let home = platform_user_home().unwrap_or_else(|| {
        state
            .app_data_dir
            .parent()
            .unwrap_or(&state.app_data_dir)
            .to_path_buf()
    });
    let configured = settings
        .get("strixRunsDirectory")
        .and_then(JsonValue::as_str)
        .unwrap_or("~/strix_runs");
    let app_uses_user_home = state.app_data_dir.starts_with(&home);
    let mut roots: Vec<PathBuf> = configured
        .split([';', '\n'])
        .filter(|value| {
            app_uses_user_home
                || !(value.trim() == "~"
                    || value.trim().starts_with("~/")
                    || value.trim().starts_with("~\\"))
        })
        .map(|value| expand_home_path(value, &home))
        .filter(|path| !path.as_os_str().is_empty())
        .collect();
    if app_uses_user_home {
        roots.push(home.join("strix_runs"));
        roots.push(home.join(".strix/strix_runs"));
    }
    roots.push(state.app_data_dir.join("strix_runs"));
    roots.push(state.app_data_dir.join("strix-jobs"));
    roots.sort();
    roots.dedup();
    roots
}

fn strix_run_dirs(root: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    fn walk(path: &Path, depth: usize, result: &mut Vec<PathBuf>) -> Result<(), String> {
        if path.join(STRIX_RUN_ARTIFACT).is_file() {
            result.push(path.to_path_buf());
            return Ok(());
        }
        if !path.is_dir() || depth == 0 {
            return Ok(());
        }
        for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
            let child = entry.map_err(|error| error.to_string())?.path();
            if child.is_dir() {
                walk(&child, depth - 1, result)?;
            }
        }
        Ok(())
    }
    // Oviraptor 自适应任务目录为 scan/batches/batch/target/strix_runs/run。
    // 保留足够深度，同时只在发现兼容层声明的运行产物时停止继续下钻。
    walk(root, 8, &mut result)?;
    Ok(result)
}

fn oviraptor_scan_id_for_run(dir: &Path) -> Option<String> {
    let mut current = Some(dir);
    for _ in 0..5 {
        let path = current?;
        let marker = [".oviraptor-scan-id", ".asset-atlas-scan-id"]
            .into_iter()
            .map(|name| path.join(name))
            .find(|candidate| candidate.is_file());
        if let Some(marker) = marker {
            if let Ok(value) = fs::read_to_string(marker) {
                let value = value.trim();
                if !value.is_empty()
                    && !value.contains('/')
                    && !value.contains('\\')
                    && !value.contains("..")
                {
                    return Some(value.to_string());
                }
            }
        }
        current = path.parent();
    }
    None
}

fn latest_scan_attempt_work_dir(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Option<(i64, PathBuf)>, String> {
    connection
        .query_row(
            "SELECT attempt_number,work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number DESC LIMIT 1",
            [scan_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    PathBuf::from(row.get::<_, String>(1)?),
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn strix_result_signature_key(scan_id: &str, dir: &Path) -> String {
    let source_identity = dir.to_string_lossy();
    let source_hash = format!("{:x}", Sha256::digest(source_identity.as_bytes()));
    format!("strix-result-signature:{scan_id}:{}", &source_hash[..24])
}

fn prepare_latest_strix_attempt(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<(), String> {
    let marker_key = format!("strix-current-attempt:{scan_id}");
    let expected = attempt_number.to_string();
    let previous = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [&marker_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if previous.as_deref() == Some(expected.as_str()) {
        return Ok(());
    }

    let (scan_type, execution_mode): (String, String) = connection
        .query_row(
            "SELECT s.scan_type,COALESCE(a.execution_mode,'initial') FROM sentinel_scans s LEFT JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=?2 WHERE s.id=?1",
            params![scan_id, attempt_number],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;

    if scan_type == "web" && execution_mode == "fresh" {
        // A full rerun owns a new current-result surface. Confirmed human
        // validations and task-scoped login sessions remain durable, while
        // machine-generated evidence is rebuilt from the new browser/Strix
        // artifacts. The immutable attempt directory remains the audit copy.
        for table in [
            "investigation_identity_diffs",
            "investigation_metrics",
            "investigation_edges",
            "investigation_nodes",
            "investigation_actions",
            "investigation_api_models",
            "investigation_hypotheses",
        ] {
            connection
                .execute(&format!("DELETE FROM {table} WHERE scan_id=?1"), [scan_id])
                .map_err(|error| error.to_string())?;
        }
        connection
            .execute("DELETE FROM sentinel_checkpoints WHERE scan_id=?1", [scan_id])
            .map_err(|error| error.to_string())?;
        connection
            .execute("DELETE FROM sentinel_findings WHERE scan_id=?1", [scan_id])
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM sentinel_opportunities WHERE scan_id=?1 AND status IN ('queued','ready')",
                [scan_id],
            )
            .map_err(|error| error.to_string())?;
    } else if scan_type == "web" && execution_mode == "resume" {
        // A continuation inherits deterministic recon and already-confirmed
        // evidence, but stale model results for the URLs entering this attempt
        // must not be presented as if they were produced by the current run.
        connection
            .execute(
                "DELETE FROM sentinel_findings WHERE scan_id=?1 AND stage='strix' AND (target_url='*' OR target_url IN (SELECT url FROM sentinel_targets WHERE scan_id=?1 AND last_attempt_number=?2))",
                params![scan_id, attempt_number],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM sentinel_opportunities WHERE scan_id=?1 AND status IN ('queued','ready') AND target_url IN (SELECT url FROM sentinel_targets WHERE scan_id=?1 AND last_attempt_number=?2)",
                params![scan_id, attempt_number],
            )
            .map_err(|error| error.to_string())?;
    }
    if scan_type == "web" && matches!(execution_mode.as_str(), "fresh" | "resume") {
        connection
            .execute(
                "DELETE FROM strix_learning_candidates WHERE scan_id=?1 AND status='pending'",
                [scan_id],
            )
            .map_err(|error| error.to_string())?;
    }

    // Current-result checkpoints are attempt-local. Attempt history remains in
    // sentinel_scan_attempts and immutable work directories, but must never be
    // folded back into the live result graph after a retry.
    connection
        .execute(
            "DELETE FROM sentinel_checkpoints WHERE scan_id=?1 AND (stage IN ('strix_run','strix_events','learning_outcome') OR stage LIKE 'strix_run:%' OR stage LIKE 'strix_events:%')",
            [scan_id],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "DELETE FROM app_settings WHERE key=?1 OR key LIKE ?2",
            params![
                format!("strix-result-signature:{scan_id}"),
                format!("strix-result-signature:{scan_id}:%")
            ],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![marker_key, expected],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn frontend_recon_for_run(dir: &Path) -> Option<JsonValue> {
    let mut current = Some(dir);
    for _ in 0..5 {
        let path = current?;
        let candidate = ["oviraptor_recon.json", "asset_atlas_recon.json"]
            .into_iter()
            .map(|name| path.join(name))
            .find(|candidate| candidate.is_file());
        if let Some(candidate) = candidate {
            if let Ok(bytes) = fs::read(candidate) {
                if let Ok(value) = serde_json::from_slice(&bytes) {
                    return Some(value);
                }
            }
        }
        current = path.parent();
    }
    None
}

fn strix_target_urls(run: &JsonValue) -> Vec<String> {
    let mut urls = Vec::new();
    for key in ["target_url", "targetUrl", "url"] {
        if let Some(value) = run.get(key).and_then(JsonValue::as_str) {
            if !value.trim().is_empty() {
                urls.push(value.to_string());
            }
        }
    }
    let target_arrays = ["targets_info", "targets", "scope", "target_urls"];
    for key in target_arrays {
        let Some(targets) = run.get(key).and_then(JsonValue::as_array) else {
            continue;
        };
        for target in targets {
            let target_object = if target.is_object() {
                target
            } else {
                &JsonValue::Null
            };
            let details = target_object.get("details").unwrap_or(&JsonValue::Null);
            let value = if let Some(value) = target.as_str() {
                value.to_string()
            } else {
                value_first(
                    target_object,
                    &[
                        "original",
                        "target",
                        "url",
                        "target_url",
                        "targetUrl",
                        "host",
                    ],
                )
            };
            let value = if value.is_empty() {
                value_first(
                    details,
                    &[
                        "target_url",
                        "targetUrl",
                        "url",
                        "target_ip",
                        "target_repo",
                        "target_path",
                        "path",
                    ],
                )
            } else {
                value
            };
            if !value.trim().is_empty() && !urls.iter().any(|item| item == &value) {
                urls.push(value);
            }
        }
    }
    urls
}

fn is_web_target_url(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with("http://") || value.starts_with("https://")
}

fn web_targets_for_scan(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT url FROM sentinel_targets WHERE scan_id=?1 AND (lower(trim(url)) LIKE 'http://%' OR lower(trim(url)) LIKE 'https://%') ORDER BY id",
        )
        .map_err(|error| error.to_string())?;
    let targets = statement
        .query_map([scan_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(targets)
}

fn repair_web_target_pollution(connection: &rusqlite::Connection) -> Result<(), String> {
    // Strix 1.5 can report Oviraptor's staged evidence directory as
    // targets_info.original. Older sync code treated that local directory as a
    // second Web URL, which produced a fake "未提供公司" group. Findings on a
    // single-URL task can be rebound deterministically; multi-URL artifacts are
    // retained at task scope instead of being attached to a fabricated target.
    connection
        .execute(
            "UPDATE sentinel_findings AS finding SET target_url=COALESCE((SELECT CASE WHEN COUNT(*)=1 THEN MIN(target.url) ELSE '*' END FROM sentinel_targets AS target WHERE target.scan_id=finding.scan_id AND (lower(trim(target.url)) LIKE 'http://%' OR lower(trim(target.url)) LIKE 'https://%')),'*'),updated_at=datetime('now','localtime') WHERE finding.target_url<>'*' AND lower(trim(finding.target_url)) NOT LIKE 'http://%' AND lower(trim(finding.target_url)) NOT LIKE 'https://%' AND (finding.target_url LIKE '%/strix-jobs/%' OR finding.target_url LIKE '%strix-evidence-input%') AND EXISTS (SELECT 1 FROM sentinel_scans AS scan WHERE scan.id=finding.scan_id AND scan.scan_type='web')",
            [],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "DELETE FROM sentinel_targets WHERE lower(trim(url)) NOT LIKE 'http://%' AND lower(trim(url)) NOT LIKE 'https://%' AND EXISTS (SELECT 1 FROM sentinel_scans AS scan WHERE scan.id=sentinel_targets.scan_id AND scan.scan_type='web')",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn asset_match_keys(value: &str) -> Vec<String> {
    let normalized = value.trim().trim_end_matches('/').to_ascii_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut keys = vec![normalized.clone()];
    let without_scheme = normalized
        .strip_prefix("https://")
        .or_else(|| normalized.strip_prefix("http://"))
        .unwrap_or(&normalized);
    keys.push(without_scheme.to_string());
    if let Some((host, _)) = without_scheme.split_once('/') {
        keys.push(host.to_string());
        if normalized.starts_with("https://") {
            keys.push(format!("https://{host}"));
        } else if normalized.starts_with("http://") {
            keys.push(format!("http://{host}"));
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

fn project_for_strix_targets(
    connection: &rusqlite::Connection,
    run_name: &str,
    urls: &[String],
) -> Result<Option<(i64, String)>, String> {
    if !run_name.trim().is_empty() {
        let exact = connection
            .query_row(
                "SELECT id,name FROM projects WHERE lower(name)=lower(?1) LIMIT 1",
                [run_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if exact.is_some() {
            return Ok(exact);
        }
    }
    for url in urls {
        for key in asset_match_keys(url) {
            let matched = connection
                .query_row(
                    "SELECT pa.project_id,p.name FROM project_assets pa JOIN assets a ON a.id=pa.asset_id JOIN projects p ON p.id=pa.project_id WHERE pa.is_deleted=0 AND (lower(rtrim(a.link,'/'))=?1 OR lower(rtrim(a.host,'/'))=?1 OR lower(rtrim(a.domain,'/'))=?1) ORDER BY pa.last_seen DESC LIMIT 1",
                    [key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if matched.is_some() {
                return Ok(matched);
            }
        }
    }
    Ok(None)
}

fn company_for_strix_target(
    connection: &rusqlite::Connection,
    project_id: i64,
    target: &str,
) -> Result<String, String> {
    for key in asset_match_keys(target) {
        let company = connection
            .query_row(
                "SELECT a.company FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=?1 AND pa.is_deleted=0 AND (lower(rtrim(a.link,'/'))=?2 OR lower(rtrim(a.host,'/'))=?2 OR lower(rtrim(a.domain,'/'))=?2) ORDER BY pa.last_seen DESC LIMIT 1",
                params![project_id, key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(company) = company {
            return Ok(company);
        }
    }
    Ok(String::new())
}

fn safe_strix_log_line(line: &str) -> String {
    // Oviraptor is an internal, local-only workstation. Preserve the original
    // evidence line for reproducibility; only cap pathological line length.
    line.chars().take(1200).collect()
}

fn strix_event_tail(dir: &std::path::Path) -> JsonValue {
    let structured_path = [
        dir.join("events.jsonl"),
        dir.join("events.ndjson"),
        dir.join(".state/events.jsonl"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .or_else(|| {
        fs::read_dir(dir.join("events"))
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.is_file()
                    && matches!(
                        path.extension().and_then(|value| value.to_str()),
                        Some("jsonl" | "ndjson")
                    )
            })
    });
    let (path, structured) = if let Some(path) = structured_path {
        (path, true)
    } else if dir.join("strix.log").is_file() {
        (dir.join("strix.log"), false)
    } else if dir.join("oviraptor-runner.log").is_file() {
        (dir.join("oviraptor-runner.log"), false)
    } else if dir.join("asset-atlas-runner.log").is_file() {
        (dir.join("asset-atlas-runner.log"), false)
    } else {
        return serde_json::json!({"source":"strix","available":false});
    };
    let Ok(mut file) = File::open(&path) else {
        return serde_json::json!({"source":"strix","available":false});
    };
    let size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let start = size.saturating_sub(64 * 1024);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return serde_json::json!({"source":"strix","available":true,"bytes":size});
    }
    let mut tail = String::new();
    let _ = file.read_to_string(&mut tail);
    let mut recent: Vec<JsonValue> = tail
        .lines()
        .filter_map(|line| {
            if structured {
                serde_json::from_str::<JsonValue>(line).ok()
            } else if line.trim().is_empty() {
                None
            } else {
                Some(serde_json::json!({"type":"log","message":safe_strix_log_line(line)}))
            }
        })
        .rev()
        .take(20)
        .collect();
    recent.reverse();
    let last = recent.last().cloned().unwrap_or(JsonValue::Null);
    serde_json::json!({"source":"strix","available":true,"path":path.to_string_lossy(),"bytes":size,"lastEvent":last,"recentEvents":recent})
}

fn normalize_strix_vulnerability(value: &JsonValue) -> JsonValue {
    let mut normalized = value.as_object().cloned().unwrap_or_default();
    normalized.insert("source".into(), JsonValue::String("strix".into()));
    let finding_type = value_first(
        value,
        &["type", "finding_class", "category", "rule_id", "ruleId"],
    );
    if !finding_type.is_empty() {
        normalized.insert("type".into(), JsonValue::String(finding_type));
    }
    let endpoint = value_first(
        value,
        &["endpoint", "target", "target_url", "targetUrl", "url"],
    );
    if !endpoint.is_empty() {
        normalized.insert("url".into(), JsonValue::String(endpoint));
    }
    if let Some(remediation) = ["remediation_steps", "remediation", "recommendation", "fix"]
        .into_iter()
        .find_map(|key| value.get(key))
    {
        normalized.insert("recommendation".into(), remediation.clone());
    }
    let poc_description = value_first(
        value,
        &["poc_description", "poc", "poc_request", "pocRequest"],
    );
    let poc_code = value_first(
        value,
        &["poc_script_code", "poc_script", "pocScript", "script"],
    );
    let poc = [poc_description, poc_code]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !poc.is_empty() {
        normalized.insert("pocRequest".into(), JsonValue::String(poc));
    }
    let title = value_first(value, &["title", "name", "message", "rule_id", "ruleId"]);
    if !title.is_empty() {
        normalized
            .entry("title")
            .or_insert_with(|| JsonValue::String(title));
    }
    let severity = value_first(value, &["severity", "level", "priority"]);
    if !severity.is_empty() {
        normalized
            .entry("severity")
            .or_insert_with(|| JsonValue::String(severity));
    }
    JsonValue::Object(normalized)
}

fn opportunity_knowledge_matches(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
    opportunity: &JsonValue,
) -> Result<Vec<JsonValue>, String> {
    let mut terms = opportunity
        .get("productSignals")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .flat_map(|value| {
            let value = value.trim().to_ascii_lowercase();
            let product = value.split_whitespace().next().unwrap_or("").to_string();
            [value, product]
        })
        .filter(|value| value.len() >= 3 && value != "unknown")
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "SELECT id,title,summary,patterns_json,skill_id,skill_instructions FROM strix_knowledge_entries WHERE project_id IS NULL OR project_id=?1 ORDER BY updated_at DESC LIMIT 500",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([project_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut matches = Vec::new();
    for row in rows.flatten() {
        let patterns = json(row.3.clone());
        let quality = patterns
            .get("qualityScore")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let kind = patterns
            .get("knowledgeKind")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        let distinct_scans = patterns
            .pointer("/support/distinctScans")
            .and_then(JsonValue::as_i64)
            .unwrap_or_else(|| if kind == "aggregate" { 2 } else { 1 });
        if quality < 70 || (kind == "task_candidate" && distinct_scans < 2) {
            continue;
        }
        let haystack = format!("{}\n{}\n{}", row.1, row.2, row.3).to_ascii_lowercase();
        let matched_terms = terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if matched_terms.is_empty() {
            continue;
        }
        matches.push(serde_json::json!({
            "id": row.0,
            "title": row.1,
            "summary": row.2,
            "skillId": row.4,
            "method": row.5.chars().take(1400).collect::<String>(),
            "qualityScore": quality,
            "canonicalKey": patterns.get("canonicalKey").cloned().unwrap_or_default(),
            "support": patterns.get("support").cloned().unwrap_or_default(),
            "matchedTerms": matched_terms,
        }));
        if matches.len() >= 12 {
            break;
        }
    }
    Ok(matches)
}

fn insert_frontend_recon(
    connection: &rusqlite::Connection,
    scan_id: &str,
    recon: &JsonValue,
) -> Result<i64, String> {
    let mut count = 0i64;
    let Some(targets) = recon.get("targets").and_then(JsonValue::as_array) else {
        return Ok(0);
    };
    let project_id = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .flatten();
    for (target_index, target) in targets.iter().enumerate() {
        let url = value_first(target, &["url", "finalUrl"]);
        if url.trim().is_empty() {
            continue;
        }
        // 前端结果按 URL 增量替换。不能清空整个任务，否则同步后续批次时会
        // 抹掉已经完成的前序 URL。
        connection
            .execute(
                "DELETE FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage='frontend-recon'",
                params![scan_id, url],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM sentinel_opportunities WHERE scan_id=?1 AND target_url=?2 AND status IN ('queued','ready')",
                params![scan_id, url],
            )
            .map_err(|error| error.to_string())?;
        connection.execute(
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,?2,'frontend_recon',?3) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
            params![scan_id, url, target.to_string()],
        ).map_err(|error| error.to_string())?;
        for (key, kind, title) in [
            ("fingerprint", "fingerprint", "技术指纹"),
            ("techStack", "tech_stack", "技术栈"),
            ("metaTags", "meta_tags", "Meta 标签"),
            (
                "headerIntelligence",
                "request_header_intelligence",
                "请求头情报",
            ),
        ] {
            if let Some(value) = target.get(key).filter(|value| !value.is_null()) {
                insert_finding(
                    connection,
                    scan_id,
                    &url,
                    "frontend-recon",
                    kind,
                    key,
                    title,
                    "info",
                    value,
                )?;
                count += 1;
            }
        }
        let endpoint = serde_json::json!({
            "url": target.get("finalUrl").and_then(JsonValue::as_str).unwrap_or(&url),
            "method": "GET",
            "statusCode": target.get("statusCode").cloned().unwrap_or(JsonValue::Null),
            "responseTime": target.get("durationMs").cloned().unwrap_or(JsonValue::Null),
            "source": "frontend-recon",
            "note": "入口页面响应；不代表漏洞"
        });
        insert_finding(
            connection,
            scan_id,
            &url,
            "frontend-recon",
            "endpoint",
            "entry-page",
            "入口页面",
            "info",
            &endpoint,
        )?;
        count += 1;
        for (json_key, kind, title_key) in [
            ("jsFiles", "js_file", "url"),
            ("apis", "api", "path"),
            ("routes", "route", "path"),
            ("features", "runtime_feature", "title"),
            ("registrationEntrypoints", "registration_endpoint", "title"),
            ("realtimeEndpoints", "realtime_endpoint", "url"),
            ("sensitiveInfo", "sensitive_info", "type"),
            ("cryptoSignals", "crypto_signal", "algorithm"),
        ] {
            let Some(records) = target.get(json_key).and_then(JsonValue::as_array) else {
                continue;
            };
            for (index, record) in records.iter().enumerate() {
                // 0.5.8 的旧侦察结果曾把所有 URL/href 当作敏感信息，并把静态资源
                // 当作 API。同步旧任务时在入库边界清掉这些记录；真正的 0.5.9
                // 敏感记录必定包含 value 字段。
                if kind == "sensitive_info" && record.get("value").is_none() {
                    continue;
                }
                if kind == "api" {
                    let candidate = value_first(record, &["url", "path"]);
                    let path = candidate
                        .split(['?', '#'])
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let static_resource = [
                        ".avif", ".bmp", ".css", ".eot", ".gif", ".ico", ".jpeg", ".jpg", ".js",
                        ".map", ".mp3", ".mp4", ".pdf", ".png", ".svg", ".ttf", ".webp", ".woff",
                        ".woff2",
                    ]
                    .iter()
                    .any(|extension| path.ends_with(extension));
                    if candidate.contains('#') || static_resource {
                        continue;
                    }
                }
                let title = value_first(record, &[title_key, "url", "type"]);
                let key = value_first(record, &["sha256", "url", "path", "stateId", "id"]);
                let key = if key.trim().is_empty() {
                    format!("{target_index}-{json_key}-{index}")
                } else {
                    key
                };
                let severity = if kind == "sensitive_info" {
                    value_first(record, &["severity"])
                } else {
                    "info".into()
                };
                insert_finding(
                    connection,
                    scan_id,
                    &url,
                    "frontend-recon",
                    kind,
                    &key,
                    &title,
                    &severity,
                    record,
                )?;
                count += 1;
            }
        }
        if let Some(signals) = target.get("runtimeSignals").and_then(JsonValue::as_array) {
            for (index, signal) in signals.iter().enumerate() {
                let title = value_first(signal, &["label", "type"]);
                let key = format!("runtime-{}-{}", value_first(signal, &["type"]), index);
                insert_finding(
                    connection,
                    scan_id,
                    &url,
                    "frontend-recon",
                    "runtime_signal",
                    &key,
                    &title,
                    "info",
                    signal,
                )?;
                count += 1;
            }
        }
        if let Some(scripts) = target.get("externalScripts").and_then(JsonValue::as_array) {
            for (index, script) in scripts.iter().enumerate() {
                let value = script.as_str().unwrap_or_default();
                let record = serde_json::json!({"url":value,"source":"frontend-recon"});
                insert_finding(
                    connection,
                    scan_id,
                    &url,
                    "frontend-recon",
                    "external_script",
                    &format!("external-{index}"),
                    value,
                    "info",
                    &record,
                )?;
                count += 1;
            }
        }
        if let Some(links) = target.get("links").and_then(JsonValue::as_array) {
            let record = serde_json::json!({"count":links.len(),"items":links});
            insert_finding(
                connection,
                scan_id,
                &url,
                "frontend-recon",
                "links",
                "discovered-links",
                "页面链接",
                "info",
                &record,
            )?;
            count += 1;
        }
        if let Some(exploration) = target.get("runtimeExploration") {
            if let Some(actions) = exploration.get("actions").and_then(JsonValue::as_array) {
                for (index, action) in actions.iter().enumerate() {
                    let mut key = value_first(action, &["id"]);
                    if key.is_empty() {
                        key = format!("action-{index}");
                    }
                    let title = value_first(action, &["label", "role"]);
                    insert_finding(
                        connection,
                        scan_id,
                        &url,
                        "frontend-recon",
                        "runtime_action",
                        &key,
                        &title,
                        "info",
                        action,
                    )?;
                    count += 1;
                }
            }
            if let Some(blocked) = exploration
                .get("blockedRequests")
                .and_then(JsonValue::as_array)
            {
                for (index, request) in blocked.iter().enumerate() {
                    insert_finding(
                        connection,
                        scan_id,
                        &url,
                        "frontend-recon",
                        "observed_mutation",
                        &format!("blocked-{index}-{}", value_first(request, &["method"])),
                        &format!(
                            "{} {}",
                            value_first(request, &["method"]),
                            value_first(request, &["url"])
                        ),
                        "info",
                        request,
                    )?;
                    count += 1;
                }
            }
        }
        if let Some(opportunities) = target.get("opportunities").and_then(JsonValue::as_array) {
            for (index, opportunity) in opportunities.iter().enumerate() {
                // This is a second ingestion-side guard. Older workers or
                // imported JSON must not reintroduce OPTIONS/telemetry into
                // the high-value inbox after the frontend recon has filtered it.
                if opportunity_is_low_value(opportunity)
                    || opportunity_is_unresolved_static_clue(opportunity)
                    || opportunity
                        .pointer("/riskEvidence/present")
                        .and_then(JsonValue::as_bool)
                        != Some(true)
                {
                    continue;
                }
                let mut record = opportunity.clone();
                let knowledge_matches =
                    opportunity_knowledge_matches(connection, project_id, opportunity)?;
                if let Some(object) = record.as_object_mut() {
                    object.insert(
                        "knowledgeMatches".into(),
                        JsonValue::Array(knowledge_matches.clone()),
                    );
                    if !knowledge_matches.is_empty() {
                        object.insert(
                            "knowledgeMatchCount".into(),
                            JsonValue::Number((knowledge_matches.len() as i64).into()),
                        );
                    }
                }
                let existing_stage = record
                    .pointer("/readiness/stage")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("needs_contract")
                    .to_string();
                let (eligible_for_agent, readiness_reason) =
                    opportunity_agent_readiness(&record);
                if let Some(object) = record.as_object_mut() {
                    object.insert(
                        "verificationMode".into(),
                        JsonValue::String(if eligible_for_agent { "ai_auto" } else { "needs_evidence" }.into()),
                    );
                    object.insert(
                        "humanReviewStage".into(),
                        JsonValue::String(if eligible_for_agent { "final_verdict_only" } else { "evidence_collection" }.into()),
                    );
                    object.insert(
                        "readiness".into(),
                        serde_json::json!({
                            "stage": if eligible_for_agent { "agent_ready" } else { existing_stage.as_str() },
                            "reason": readiness_reason
                        }),
                    );
                }
                let score = opportunity
                    .get("score")
                    .and_then(JsonValue::as_i64)
                    .unwrap_or(0);
                let status = if eligible_for_agent { "ready" } else { "queued" };
                let confidence = value_first(opportunity, &["confidence"]);
                let mut why = opportunity
                    .get("whyValuable")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]));
                if !knowledge_matches.is_empty() {
                    if let Some(items) = why.as_array_mut() {
                        items.push(JsonValue::String(format!(
                            "命中 {} 条本地知识，仅用于选择验证方法；不会代替当前目标的请求/响应证据，也不会自动晋升为可验证",
                            knowledge_matches.len()
                        )));
                    }
                }
                let key = value_first(opportunity, &["opportunityKey"]);
                let key = if key.trim().is_empty() {
                    format!("opportunity-{target_index}-{index}")
                } else {
                    key
                };
                connection.execute(
                    "INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,why_json,evidence_json,recommended_action_json,source,record_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) ON CONFLICT(scan_id,target_url,opportunity_key) DO UPDATE SET project_id=excluded.project_id,category=excluded.category,title=excluded.title,score=excluded.score,status=CASE WHEN sentinel_opportunities.status IN ('in_progress','validated','dismissed','exhausted') THEN sentinel_opportunities.status ELSE excluded.status END,confidence=excluded.confidence,why_json=excluded.why_json,evidence_json=excluded.evidence_json,recommended_action_json=excluded.recommended_action_json,source=excluded.source,record_json=excluded.record_json,last_seen=datetime('now','localtime')",
                    params![
                        project_id,
                        scan_id,
                        url,
                        key,
                        value_first(opportunity, &["category"]),
                        value_first(opportunity, &["title"]),
                        score,
                        status,
                        confidence,
                        why.to_string(),
                        opportunity.get("evidenceRefs").cloned().unwrap_or_else(|| serde_json::json!([])).to_string(),
                        opportunity.get("recommendedAction").cloned().unwrap_or_else(|| serde_json::json!({})).to_string(),
                        value_first(opportunity, &["source"]),
                        record.to_string(),
                    ],
                ).map_err(|error| error.to_string())?;
            }
        }
        // Preserve the deterministic browser/AST evidence as an investigation
        // graph. This also computes the incremental baseline and the local
        // information-gain decision that gates later model work.
        persist_investigation_graph(connection, project_id, scan_id, &url, target)?;
    }
    Ok(count)
}

fn bind_strix_target(target: &str, targets: &[String]) -> String {
    let key = asset_match_keys(target);
    for candidate in targets {
        let candidate_keys = asset_match_keys(candidate);
        if key.iter().any(|item| candidate_keys.contains(item))
            || candidate_keys.iter().any(|item| key.contains(item))
        {
            return candidate.clone();
        }
    }
    if targets.len() == 1 && is_web_target_url(&targets[0]) && !is_web_target_url(target) {
        return targets[0].clone();
    }
    target.to_string()
}

fn aggregate_strix_usage(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<(i64, i64, i64, i64, i64), String> {
    let mut statement = connection.prepare("SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND (stage='strix_run' OR stage LIKE 'strix_run:%')").map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([scan_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let mut totals = (0, 0, 0, 0, 0);
    for raw in rows {
        let run: JsonValue =
            serde_json::from_str(&raw.map_err(|error| error.to_string())?).unwrap_or_default();
        let usage = run.get("llm_usage").unwrap_or(&JsonValue::Null);
        totals.0 += usage_request_count(usage);
        totals.1 += usage_input_tokens(usage);
        totals.2 += usage_output_tokens(usage);
        totals.3 += usage_cached_tokens(usage);
        totals.4 += usage_total_tokens(usage);
    }
    Ok(totals)
}

fn bounded_checkpoint_text(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(max_chars).collect::<String>())
    }
}

fn checkpoint_failure_suffix(
    connection: &rusqlite::Connection,
    scan_id: &str,
    checkpoint: &str,
) -> String {
    if let Some((_, details)) = checkpoint.split_once("；报错细节：") {
        let details = details.trim();
        if !details.is_empty() {
            return format!("；报错细节：{}", bounded_checkpoint_text(details, 2_400));
        }
    }
    let Ok(mut statement) = connection.prepare(
        "SELECT url,routing_reason FROM sentinel_targets WHERE scan_id=?1 AND status IN ('partial','limited','failed') AND trim(routing_reason)<>'' ORDER BY CASE status WHEN 'failed' THEN 0 WHEN 'partial' THEN 1 ELSE 2 END,updated_at DESC LIMIT 5",
    ) else {
        return String::new();
    };
    let Ok(rows) = statement.query_map([scan_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return String::new();
    };
    let mut details = Vec::new();
    for row in rows.flatten() {
        let (url, reason) = row;
        let reason = [
            "本地模型资源策略需要调整；前端证据已保留，可重试未完成阶段：",
            "Strix 模型服务不可用或配置错误，自动流程无法继续；已保留完整前端侦察结果：",
            "确认拦截并熔断：",
        ]
        .iter()
        .find_map(|marker| reason.rsplit_once(marker).map(|(_, tail)| tail))
        .or_else(|| reason.rsplit('；').find(|part| !part.trim().is_empty()))
        .unwrap_or(&reason)
        .trim();
        if !reason.is_empty() {
            details.push(format!(
                "{}：{}",
                bounded_checkpoint_text(url.trim(), 240),
                bounded_checkpoint_text(reason, 720)
            ));
        }
    }
    if details.is_empty() {
        String::new()
    } else {
        format!("；报错细节：{}", details.join("；"))
    }
}

fn repair_associated_scan_state(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<(), String> {
    let scan: Option<(String, String, String)> = connection
        .query_row(
            "SELECT status,current_checkpoint,scan_type FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((status, checkpoint, scan_type)) = scan else {
        return Ok(());
    };
    let terminal = matches!(
        status.as_str(),
        "completed" | "recon_only" | "partial" | "failed" | "paused" | "cancelled"
    );
    if !terminal {
        return Ok(());
    }

    // 0.8.0 的同步器曾将 URL 的自适应路由状态覆盖成任务总状态。
    // 路由原因是本地确定性证据，可用于一次性修复已有数据。
    connection
        .execute(
            "UPDATE sentinel_targets SET status=CASE WHEN routing_reason LIKE '%自动熔断：%' THEN 'limited' WHEN scan_mode='skip' THEN 'recon_only' WHEN scan_mode='manual_review' THEN 'manual_review' ELSE status END,updated_at=datetime('now','localtime') WHERE scan_id=?1 AND (routing_reason LIKE '%自动熔断：%' OR scan_mode IN ('skip','manual_review'))",
            [scan_id],
        )
        .map_err(|error| error.to_string())?;

    // 1.1.49 briefly classified a budget/no-progress stop as completed even
    // when its own route reason explicitly said that no target HTTP
    // request/response had been obtained. Repair only that exact contradictory
    // signature; genuine bounded completions with tool evidence stay complete.
    connection
        .execute(
            "UPDATE sentinel_targets SET status='partial',routing_reason=CASE WHEN routing_reason LIKE '%历史修复：未取得目标工具证据%' THEN routing_reason ELSE routing_reason || '；历史修复：未取得目标工具证据，不计入自动验证完成' END,updated_at=datetime('now','localtime') WHERE scan_id=?1 AND status='completed' AND routing_reason LIKE '%自动验证已按边界收口（本轮未形成新的工具证据）%' AND (routing_reason LIKE '%没有取得目标请求/响应%' OR routing_reason LIKE '%没有形成可用工具结果%' OR routing_reason LIKE '%没有形成任何工具证据%' OR routing_reason LIKE '%只读取了本地证据%')",
            [scan_id],
        )
        .map_err(|error| error.to_string())?;

    // Older runs could launch Strix after the investigation gate had already
    // decided that no hypothesis was model-eligible. Repair those records as
    // recon-only instead of presenting a false partial/limited failure. The
    // evidence files and any findings remain intact; only the route outcome is
    // corrected to match the persisted investigation contract.
    connection
        .execute(
            "UPDATE sentinel_targets SET status='recon_only',scan_mode='skip',routing_reason=CASE WHEN routing_reason LIKE '%历史修复：调查门禁关闭%' THEN routing_reason ELSE routing_reason || '；历史修复：调查门禁关闭，未启动 Strix' END,updated_at=datetime('now','localtime') WHERE scan_id=?1 AND status IN ('queued','frontend_recon','routed','scanning','partial','limited') AND EXISTS (SELECT 1 FROM investigation_metrics im WHERE im.scan_id=sentinel_targets.scan_id AND im.target_url=sentinel_targets.url AND COALESCE(im.token_worthy,0)=0 AND COALESCE(json_extract(im.decision_json,'$.eligibleForModel'),0)=0 AND COALESCE(json_extract(im.decision_json,'$.standardInvestigationAllowed'),0)=0)",
            [scan_id],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT OR IGNORE INTO sentinel_fuse_zone(project_id,asset_id,company,url,normalized_url,source_scan_id,reason) SELECT project_id,asset_id,company,url,lower(rtrim(trim(url),'/')),COALESCE(scan_id,''),routing_reason FROM sentinel_targets WHERE scan_id=?1 AND status='limited' AND trim(url)<>''",
            [scan_id],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute("DELETE FROM sentinel_processes WHERE scan_id=?1", [scan_id])
        .map_err(|error| error.to_string())?;

    // Recompute every terminal adaptive web pipeline from target rows. Do not
    // key this on one historical checkpoint prefix: frontend pipelines use
    // several checkpoints, and stale summaries were the reason the UI showed
    // partial/static counts that did not match sentinel_targets.
    if scan_type == "web" {
        let counts: (i64, i64, i64, i64, i64, i64, i64, i64) = connection
            .query_row(
                "SELECT COALESCE(SUM(CASE WHEN status='completed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='partial' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='recon_only' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='manual_review' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='limited' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status NOT IN ('completed','partial','recon_only','manual_review','limited','failed') THEN 1 ELSE 0 END),0),COUNT(*) FROM sentinel_targets WHERE scan_id=?1",
                [scan_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
            )
            .unwrap_or((0, 0, 0, 0, 0, 0, 0, 0));
        let (completed, partial, recon_only, manual_review, limited, failed, deferred, total) = counts;
        if total > 0 {
            // scan_execution persists the provider/target reason. The result
            // synchronizer must aggregate counts without erasing that reason;
            // if an older checkpoint already lost it, recover a concise reason
            // from the affected target row.
            let failure_suffix = checkpoint_failure_suffix(connection, scan_id, &checkpoint);
            let latest_attempt: Option<(i64, String)> = connection
                .query_row(
                    "SELECT attempt_number,COALESCE(NULLIF(trim(stop_reason),''),checkpoint) FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number DESC LIMIT 1",
                    [scan_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();
            let has_errors = partial + limited + failed + deferred > 0;
            let derived_status = if matches!(status.as_str(), "paused" | "cancelled") {
                status.as_str()
            } else if has_errors {
                // A limited/retryable target is retained as a partial pipeline
                // result even when it is the only target. It is not equivalent
                // to a hard execution failure and remains eligible for resume.
                if completed + partial + recon_only + manual_review + limited > 0 {
                    "partial"
                } else {
                    "failed"
                }
            } else {
                "completed"
            };
            let repaired = if has_errors && failed == 0 && limited == 0 && deferred == 0 {
                format!(
                    "任务累计状态：自动验证 {completed}，待补充验证 {partial}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}；无执行失败，待补充项未计入自动验证完成{failure_suffix}"
                )
            } else if has_errors {
                format!(
                    "任务累计状态：自动验证 {completed}，待补充验证 {partial}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}，熔断 {limited}，执行失败 {failed}，未处理 {deferred}{failure_suffix}"
                )
            } else if deferred == 0 {
                format!(
                    "任务累计状态：全部目标已收口；自动验证 {completed}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}"
                )
            } else {
                format!(
                    "任务累计状态：自动验证 {completed}，待补充验证 {partial}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}，熔断 {limited}，执行失败 {failed}，未处理 {deferred}"
                )
            };
            let display_checkpoint = latest_attempt
                .filter(|(_, reason)| !reason.trim().is_empty())
                .map(|(number, reason)| {
                    format!("最新第 {number} 次执行：{}；{repaired}", reason.trim())
                })
                .unwrap_or(repaired);
            connection
                .execute(
                    "UPDATE sentinel_scans SET status=?1,current_checkpoint=?2,updated_at=datetime('now','localtime') WHERE id=?3",
                    params![derived_status, display_checkpoint, scan_id],
                )
                .map_err(|error| error.to_string())?;
        } else if checkpoint.starts_with("Strix 实时 ·") {
            // Preserve the old checkpoint for non-adaptive scans with no URL
            // rows; there is nothing to aggregate.
            return Ok(());
        }
    }
    Ok(())
}

fn strix_json_records(value: &JsonValue) -> Vec<JsonValue> {
    if let Some(items) = value.as_array() {
        return items.clone();
    }
    for key in ["vulnerabilities", "findings", "results", "items"] {
        if let Some(items) = value.get(key).and_then(JsonValue::as_array) {
            return items.clone();
        }
    }
    if value.is_object() {
        return vec![value.clone()];
    }
    Vec::new()
}

fn imported_strix_run_status(source_status: &str) -> &'static str {
    match source_status {
        "completed" | "complete" | "finished" => "completed",
        "failed" | "crashed" => "failed",
        // Native Strix does not distinguish a user pause from budget and
        // lifecycle interruption. Oviraptor's own paused state is preserved
        // separately; every other interrupted artifact is a resumable partial.
        "stopped" | "interrupted" | "cancelled" | "canceled" => "partial",
        _ => "scanning",
    }
}

fn strix_sarif_records(path: &Path) -> Vec<JsonValue> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    let Ok(document) = serde_json::from_slice::<JsonValue>(&bytes) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for run in document
        .get("runs")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let rules = run
            .get("tool")
            .and_then(|value| value.get("driver"))
            .and_then(|value| value.get("rules"))
            .and_then(JsonValue::as_array);
        for result in run
            .get("results")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            let rule_id = value_first(result, &["ruleId", "rule_id"]);
            let message = result
                .get("message")
                .and_then(|value| value.get("text"))
                .and_then(JsonValue::as_str)
                .unwrap_or("未提供规则描述")
                .to_string();
            let level = value_first(result, &["level", "severity"]);
            let rule = rules.and_then(|items| {
                items.iter().find(|item| {
                    value_first(item, &["id", "ruleId"]) == rule_id && !rule_id.is_empty()
                })
            });
            let title = rule
                .and_then(|value| value.get("shortDescription"))
                .and_then(|value| value.get("text"))
                .and_then(JsonValue::as_str)
                .unwrap_or(if rule_id.is_empty() {
                    "Strix SARIF finding"
                } else {
                    &rule_id
                })
                .to_string();
            let target = result
                .get("locations")
                .and_then(JsonValue::as_array)
                .and_then(|locations| locations.first())
                .and_then(|location| {
                    location
                        .get("physicalLocation")
                        .unwrap_or(location)
                        .get("artifactLocation")
                })
                .and_then(|value| value.get("uri"))
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_string();
            let mut record = serde_json::json!({
                "source": "strix",
                "id": if rule_id.is_empty() { "sarif-finding" } else { &rule_id },
                "rule_id": rule_id,
                "title": title,
                "severity": sarif_severity(if level.is_empty() { "warning" } else { &level }),
                "description": message,
                "target": target,
                "sarif": result,
            });
            if let Some(rule_properties) = rule.and_then(|value| value.get("properties")) {
                record["rule_properties"] = rule_properties.clone();
            }
            records.push(record);
        }
    }
    records
}

fn strix_csv_records(path: &Path) -> Vec<JsonValue> {
    let Ok(mut reader) = ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_path(path)
    else {
        return Vec::new();
    };
    let Ok(headers) = reader.headers().cloned() else {
        return Vec::new();
    };
    reader
        .records()
        .filter_map(Result::ok)
        .enumerate()
        .map(|(index, row)| {
            let mut object = serde_json::Map::new();
            for (column, value) in headers.iter().zip(row.iter()) {
                if !value.trim().is_empty() {
                    object.insert(column.to_string(), JsonValue::String(value.to_string()));
                }
            }
            if !object.contains_key("id") {
                object.insert(
                    "id".into(),
                    JsonValue::String(format!("csv-finding-{index:04}")),
                );
            }
            JsonValue::Object(object)
        })
        .collect()
}

fn strix_markdown_records(dir: &Path) -> Vec<JsonValue> {
    let Ok(entries) = fs::read_dir(dir.join("vulnerabilities")) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("md"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let body = fs::read_to_string(&path).ok()?;
            let title = body
                .lines()
                .find_map(|line| line.strip_prefix("# ").or_else(|| line.strip_prefix("## ")))
                .unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("Strix finding")
                })
                .trim()
                .to_string();
            Some(serde_json::json!({
                "source": "strix",
                "id": path.file_stem().and_then(|value| value.to_str()).unwrap_or("markdown-finding"),
                "title": title,
                "description": body,
                "evidence_file": path.to_string_lossy(),
            }))
        })
        .collect()
}

fn strix_vulnerabilities(dir: &Path) -> Vec<JsonValue> {
    if let Ok(bytes) = fs::read(dir.join(STRIX_VULNERABILITIES_ARTIFACT)) {
        if let Ok(value) = serde_json::from_slice::<JsonValue>(&bytes) {
            let records = strix_json_records(&value);
            if !records.is_empty() {
                return records;
            }
        }
    }
    let sarif = strix_sarif_records(&dir.join(STRIX_SARIF_ARTIFACT));
    if !sarif.is_empty() {
        return sarif;
    }
    let csv = strix_csv_records(&dir.join(STRIX_CSV_ARTIFACT));
    if !csv.is_empty() {
        return csv;
    }
    strix_markdown_records(dir)
}

fn sync_strix_results(connection: &rusqlite::Connection, state: &AppState) -> Result<i64, String> {
    let mut synced = 0;
    for root in strix_run_roots(connection, state) {
        for dir in strix_run_dirs(&root)? {
            let run_path = dir.join(STRIX_RUN_ARTIFACT);
            let run: JsonValue =
                serde_json::from_slice(&fs::read(&run_path).map_err(|error| error.to_string())?)
                    .map_err(|error| format!("{}：{}", run_path.display(), error))?;
            let raw_run_id = value_first(&run, &["run_id", "run_name"]);
            let fallback_id = dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("run");
            let raw_run_id = if raw_run_id.trim().is_empty() {
                fallback_id.to_string()
            } else {
                raw_run_id
            };
            let safe_id = raw_run_id.replace(['/', '\\'], "_").replace("..", "_");
            let associated_scan_id = oviraptor_scan_id_for_run(&dir);
            let is_associated = associated_scan_id.is_some();
            let native_exists: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sentinel_scans WHERE id=?1",
                    [associated_scan_id.as_deref().unwrap_or(&safe_id)],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let scan_id = if let Some(scan_id) = associated_scan_id {
                scan_id
            } else if native_exists > 0 {
                safe_id.clone()
            } else {
                format!("strix-{safe_id}")
            };
            let deleted: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sentinel_deleted_scans WHERE scan_id=?1",
                    [&scan_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if deleted > 0 {
                continue;
            }
            let existing: Option<(Option<i64>, String, String, String, String)> = connection
                .query_row(
                    "SELECT project_id,project_name,task_path,status,scan_type FROM sentinel_scans WHERE id=?1",
                    [&scan_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let existing_web_scan = existing
                .as_ref()
                .is_some_and(|item| item.4.as_str() == "web");
            if is_associated && existing_web_scan {
                if let Some((attempt_number, work_dir)) =
                    latest_scan_attempt_work_dir(connection, &scan_id)?
                {
                    // Old migrated tasks can have an empty work_dir. Keep their
                    // compatibility behavior, but for all native attempts only
                    // the newest immutable directory may feed current state.
                    if !work_dir.as_os_str().is_empty() {
                        if !dir.starts_with(&work_dir) {
                            continue;
                        }
                        prepare_latest_strix_attempt(connection, &scan_id, attempt_number)?;
                    }
                }
            }
            let artifact_signature = sentinel_result_signature(&dir)?;
            let signature_key = strix_result_signature_key(&scan_id, &dir);
            let previous_signature = connection
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [&signature_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let checkpoint_stage = format!("strix_run:{safe_id}");
            let checkpoint_exists: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sentinel_checkpoints WHERE scan_id=?1 AND stage=?2",
                    params![scan_id, checkpoint_stage],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if previous_signature.as_deref() == Some(artifact_signature.as_str())
                && checkpoint_exists > 0
            {
                continue;
            }
            let vulnerabilities = strix_vulnerabilities(&dir);
            let artifact_targets = strix_target_urls(&run);
            let mut targets = if existing_web_scan {
                artifact_targets
                    .into_iter()
                    .filter(|target| is_web_target_url(target))
                    .collect::<Vec<_>>()
            } else {
                artifact_targets
            };
            if existing_web_scan {
                for target in web_targets_for_scan(connection, &scan_id)? {
                    if !targets.iter().any(|item| item == &target) {
                        targets.push(target);
                    }
                }
            }
            let run_name = value_first(&run, &["run_name"]);
            let inferred = if existing.as_ref().and_then(|item| item.0).is_none() {
                project_for_strix_targets(connection, &run_name, &targets)?
            } else {
                None
            };
            let project_id = existing
                .as_ref()
                .and_then(|item| item.0)
                .or_else(|| inferred.as_ref().map(|item| item.0));
            let project_name = existing
                .as_ref()
                .map(|item| item.1.clone())
                .filter(|value| !value.trim().is_empty())
                .or_else(|| inferred.as_ref().map(|item| item.1.clone()))
                .unwrap_or_else(|| run_name.clone());
            let source_status = value_first(&run, &["status"]).to_ascii_lowercase();
            let run_status = imported_strix_run_status(&source_status);
            let preserve_associated_state = is_associated
                && existing
                    .as_ref()
                    .is_some_and(|item| {
                        item.4.as_str() == "web"
                            || matches!(item.3.as_str(), "paused" | "pausing")
                    });
            let status = if preserve_associated_state {
                existing
                    .as_ref()
                    .map(|item| item.3.as_str())
                    .unwrap_or(run_status)
            } else {
                run_status
            };
            let event_summary = strix_event_tail(&dir);
            let event_bytes = event_summary
                .get("bytes")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0);
            let usage = run.get("llm_usage").unwrap_or(&JsonValue::Null);
            let total_tokens = usage_total_tokens(usage);
            let checkpoint = format!(
                "Strix 实时 · {:.1} KB 事件 · {} 个漏洞 · {} Token",
                event_bytes as f64 / 1024.0,
                vulnerabilities.len(),
                total_tokens
            );
            connection.execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET project_id=COALESCE(excluded.project_id,sentinel_scans.project_id),project_name=CASE WHEN excluded.project_name='' THEN sentinel_scans.project_name ELSE excluded.project_name END,status=CASE WHEN ?7=1 THEN sentinel_scans.status ELSE excluded.status END,current_checkpoint=CASE WHEN ?7=1 THEN sentinel_scans.current_checkpoint ELSE excluded.current_checkpoint END,task_path=CASE WHEN sentinel_scans.task_path='' THEN excluded.task_path ELSE sentinel_scans.task_path END,updated_at=datetime('now','localtime')",
                params![scan_id, project_id, project_name, status, checkpoint, dir.to_string_lossy(), preserve_associated_state as i64],
            ).map_err(|error| error.to_string())?;
            // Oviraptor-owned Web tasks have an independent deterministic
            // frontend-recon synchronizer. Importing the parent recon again
            // from whichever Strix run happens to be visited last can roll the
            // A/B matrix back to an older attempt.
            if !preserve_associated_state {
                if let Some(recon) = frontend_recon_for_run(&dir) {
                    let _ = insert_frontend_recon(connection, &scan_id, &recon)?;
                }
            }
            connection.execute(
                "DELETE FROM sentinel_checkpoints WHERE scan_id=?1 AND stage IN ('strix_run','strix_events')",
                [&scan_id],
            ).map_err(|error| error.to_string())?;
            connection.execute(
                "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,'*',?2,?3) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
                params![scan_id, format!("strix_run:{safe_id}"), run.to_string()],
            ).map_err(|error| error.to_string())?;
            connection.execute(
                "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,'*',?2,?3) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
                params![scan_id, format!("strix_events:{safe_id}"), event_summary.to_string()],
            ).map_err(|error| error.to_string())?;
            let (requests, input_tokens, output_tokens, cached_tokens, total_tokens) =
                aggregate_strix_usage(connection, &scan_id)?;
            connection.execute(
                "UPDATE sentinel_scans SET llm_requests=MAX(llm_requests,?1),input_tokens=MAX(input_tokens,?2),output_tokens=MAX(output_tokens,?3),cached_tokens=MAX(cached_tokens,?4),total_tokens=MAX(total_tokens,?5) WHERE id=?6",
                params![requests,input_tokens,output_tokens,cached_tokens,total_tokens,scan_id],
            ).map_err(|error| error.to_string())?;
            let default_target = targets.first().cloned().unwrap_or_else(|| "*".into());
            for (index, vulnerability) in vulnerabilities.iter().enumerate() {
                let target = value_first(
                    vulnerability,
                    &["target", "target_url", "targetUrl", "url", "endpoint"],
                );
                let target = if target.trim().is_empty() {
                    default_target.clone()
                } else {
                    bind_strix_target(&target, &targets)
                };
                let record_key = value_first(
                    vulnerability,
                    &["id", "finding_id", "rule_id", "ruleId", "fingerprint"],
                );
                let record_key = if record_key.trim().is_empty() {
                    format!("vuln-{index:04}")
                } else {
                    record_key
                };
                let record_key = format!("{safe_id}:{record_key}");
                let title = value_first(
                    vulnerability,
                    &["title", "name", "message", "rule_id", "ruleId"],
                );
                let severity = value_first(vulnerability, &["severity", "level", "priority"]);
                let normalized = normalize_strix_vulnerability(vulnerability);
                insert_finding(
                    connection,
                    &scan_id,
                    &target,
                    "strix",
                    "vulnerability",
                    &record_key,
                    &title,
                    &severity,
                    &normalized,
                )?;
            }
            if let Some(summary) = run.get("scan_results") {
                insert_finding(
                    connection,
                    &scan_id,
                    "*",
                    "strix",
                    "risk_summary",
                    &format!("executive:{safe_id}"),
                    "Strix 扫描总结",
                    "",
                    summary,
                )?;
            }
            if let Some(project_id) = project_id {
                for target in &targets {
                    let company = company_for_strix_target(connection, project_id, target)?;
                    connection.execute(
                        "INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(project_id,scan_id,url) DO UPDATE SET company=CASE WHEN excluded.company='' THEN sentinel_targets.company ELSE excluded.company END,status=CASE WHEN ?6=1 THEN sentinel_targets.status ELSE excluded.status END,updated_at=datetime('now','localtime')",
                        params![project_id, scan_id, company, target, status, preserve_associated_state as i64],
                    ).map_err(|error| error.to_string())?;
                }
            }
            if is_associated {
                repair_associated_scan_state(connection, &scan_id)?;
            }
            connection.execute(
                "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![signature_key, artifact_signature],
            ).map_err(|error| error.to_string())?;
            synced += 1;
        }
    }
    // A previously imported artifact is skipped by its immutable signature.
    // Still repair legacy terminal Web checkpoints once when an older version
    // already replaced their useful target error with aggregate counts.
    let legacy_failure_scans = {
        let mut statement = connection
            .prepare(
                "SELECT id FROM sentinel_scans WHERE scan_type='web' AND status IN ('partial','failed') AND current_checkpoint NOT LIKE '%；报错细节：%' AND EXISTS (SELECT 1 FROM sentinel_targets st WHERE st.scan_id=sentinel_scans.id AND st.status IN ('partial','limited','failed') AND trim(st.routing_reason)<>'') LIMIT 100",
            )
            .map_err(|error| error.to_string())?;
        let scan_ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .flatten()
            .collect::<Vec<_>>();
        scan_ids
    };
    for scan_id in legacy_failure_scans {
        repair_associated_scan_state(connection, &scan_id)?;
    }
    Ok(synced)
}

fn frontend_recon_signature_key(scan_id: &str, root: &Path, path: &Path) -> String {
    let source_identity = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy();
    let source_hash = format!("{:x}", Sha256::digest(source_identity.as_bytes()));
    format!(
        "frontend-recon-signature:{scan_id}:{}",
        &source_hash[..24]
    )
}

fn sync_pending_frontend_recon(
    connection: &rusqlite::Connection,
    state: &AppState,
) -> Result<i64, String> {
    let root = state.app_data_dir.join("strix-jobs");
    if !root.is_dir() {
        return Ok(0);
    }
    let mut synced = 0;
    let mut recon_files = Vec::new();
    fn collect_recon(path: &Path, depth: usize, found: &mut Vec<PathBuf>) -> Result<(), String> {
        if !path.is_dir() || depth == 0 {
            return Ok(());
        }
        for name in ["oviraptor_recon.json", "asset_atlas_recon.json"] {
            let candidate = path.join(name);
            if candidate.is_file() {
                found.push(candidate);
                break;
            }
        }
        for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
            let child = entry.map_err(|error| error.to_string())?.path();
            if child.is_dir() {
                collect_recon(&child, depth - 1, found)?;
            }
        }
        Ok(())
    }
    collect_recon(&root, 5, &mut recon_files)?;
    // A scan can contain several immutable attempt directories. Filesystem
    // iteration order is undefined; without sorting, an older attempt may be
    // ingested after the newest one and overwrite its complete A/B CDP matrix.
    // Attempt directories are zero padded, so path order is also chronological
    // within each scan and the newest evidence deterministically wins.
    recon_files.sort();
    for path in recon_files {
        let Some(dir) = path.parent() else { continue };
        let Some(scan_id) = oviraptor_scan_id_for_run(dir) else {
            continue;
        };
        let deleted = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_deleted_scans WHERE scan_id=?1",
                [&scan_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        if deleted > 0 {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let signature = format!("{:x}", Sha256::digest(&bytes));
        // Track each immutable recon source independently. The previous
        // scan-level key alternated between attempt signatures on every app
        // startup, causing all historical attempts to be re-imported in an
        // arbitrary order.
        let signature_key = frontend_recon_signature_key(&scan_id, &root, &path);
        let previous = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [&signature_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if previous.as_deref() == Some(signature.as_str()) {
            continue;
        }
        let Ok(recon) = serde_json::from_slice::<JsonValue>(&bytes) else {
            continue;
        };
        insert_frontend_recon(connection, &scan_id, &recon)?;
        connection.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![signature_key, signature],
        ).map_err(|error| error.to_string())?;
        synced += 1;
    }
    Ok(synced)
}

fn sentinel_result_signature(dir: &Path) -> Result<String, String> {
    fn visit(
        path: &Path,
        root: &Path,
        depth: usize,
        hasher: &mut DefaultHasher,
    ) -> Result<(), String> {
        if depth == 0 || !path.is_dir() {
            return Ok(());
        }
        let mut entries = fs::read_dir(path)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        entries.sort();
        for child in entries {
            if child.is_dir() {
                visit(&child, root, depth - 1, hasher)?;
                continue;
            }
            if !matches!(
                child.extension().and_then(|value| value.to_str()),
                Some("json" | "jsonl")
            ) || child.file_name().and_then(|value| value.to_str()) == Some("meta.json")
            {
                continue;
            }
            let metadata = fs::metadata(&child).map_err(|error| error.to_string())?;
            child.strip_prefix(root).unwrap_or(&child).hash(hasher);
            metadata.len().hash(hasher);
            metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_nanos())
                .unwrap_or_default()
                .hash(hasher);
        }
        Ok(())
    }
    let mut hasher = DefaultHasher::new();
    visit(dir, dir, 8, &mut hasher)?;
    Ok(format!("{:016x}", hasher.finish()))
}

#[tauri::command]
pub async fn sync_sentinel_results(state: State<'_, AppState>) -> Result<i64, String> {
    let root = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .join(".trae-cn/scan-results");
    let connection = db::open(&state.db_path)?;
    repair_web_target_pollution(&connection)?;
    let mut count = 0;
    if root.exists() {
        for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
            let dir = entry.map_err(|e| e.to_string())?.path();
            if !dir.is_dir() {
                continue;
            }
            let meta_path = dir.join("meta.json");
            if !meta_path.exists() {
                continue;
            }
            let meta: JsonValue =
                serde_json::from_slice(&fs::read(&meta_path).map_err(|e| e.to_string())?)
                    .unwrap_or_default();
            let scan_id = meta
                .get("scanId")
                .and_then(JsonValue::as_str)
                .unwrap_or_else(|| dir.file_name().and_then(|v| v.to_str()).unwrap_or(""));
            if scan_id.is_empty() {
                continue;
            }
            let deleted: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sentinel_deleted_scans WHERE scan_id=?1",
                    [scan_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if deleted > 0 {
                // 墓碑优先：即使旧版本进程曾把目录重新同步回来，也立即清掉残留行。
                let _ = connection.execute("DELETE FROM sentinel_scans WHERE id=?1", [scan_id]);
                continue;
            }
            let summary: JsonValue = fs::read(dir.join("summary.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            let existing: Option<(Option<i64>, String, String)> = connection
                .query_row(
                    "SELECT project_id,project_name,status FROM sentinel_scans WHERE id=?1",
                    [scan_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            // Agent 可能一直不回写 meta.status。summary.completedAt 表示结果已封盘，
            // 其完成态优先于 meta.json 中滞留的 scanning。
            let completed = meta
                .get("completedAt")
                .and_then(JsonValue::as_str)
                .is_some_and(|v| !v.is_empty())
                || summary
                    .get("completedAt")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|v| !v.is_empty())
                || summary
                    .get("status")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|v| matches!(v, "completed" | "complete" | "finished"));
            let status = if completed {
                "completed"
            } else {
                meta.get("status")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("scanning")
            };
            let status_changed = existing
                .as_ref()
                .map_or(true, |(_, _, previous_status)| previous_status != status);
            // Completed result directories are immutable. Re-parsing every
            // checkpoint on each page mount deleted and reinserted thousands of
            // rows even when nothing had changed.
            if completed
                && existing
                    .as_ref()
                    .is_some_and(|(_, _, old_status)| old_status == "completed")
            {
                continue;
            }
            // projectName 是 Oviraptor 项目归属；yakitProject 只是扫描器工作区名。
            let mut project_name = meta
                .get("projectName")
                .and_then(JsonValue::as_str)
                .filter(|v| !v.trim().is_empty())
                .or_else(|| {
                    existing
                        .as_ref()
                        .map(|item| item.1.as_str())
                        .filter(|v| !v.trim().is_empty())
                })
                .or_else(|| meta.get("yakitProject").and_then(JsonValue::as_str))
                .unwrap_or("")
                .to_string();
            let mut project_id = existing.as_ref().and_then(|item| item.0);
            if project_id.is_none() && !project_name.is_empty() {
                project_id = connection
                    .query_row(
                        "SELECT id FROM projects WHERE lower(name)=lower(?1) LIMIT 1",
                        [&project_name],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
            }
            if project_id.is_none() {
                if let Some(task_id) = meta.get("taskId").and_then(JsonValue::as_str) {
                    project_id = connection
                        .query_row(
                            "SELECT project_id FROM sentinel_scans WHERE id=?1",
                            [task_id],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|error| error.to_string())?;
                }
            }
            // Agent 既未写公司也未写 Atlas 项目时，用结果 URL 反查本地资产归属。
            if project_id.is_none() {
                let target_arrays = [
                    meta.get("targets"),
                    summary.get("perTarget"),
                    summary.get("targets"),
                ];
                'outer: for array in target_arrays.into_iter().flatten() {
                    if let Some(items) = array.as_array() {
                        for item in items {
                            let url = item
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| value_first(item, &["url", "target", "host"]));
                            let key = url.trim().trim_end_matches('/').to_ascii_lowercase();
                            if key.is_empty() {
                                continue;
                            }
                            let matched: Option<(i64,String)>=connection.query_row("SELECT pa.project_id,p.name FROM project_assets pa JOIN assets a ON a.id=pa.asset_id JOIN projects p ON p.id=pa.project_id WHERE pa.is_deleted=0 AND (lower(rtrim(a.link,'/'))=?1 OR lower(rtrim(a.host,'/'))=?1) ORDER BY pa.last_seen DESC LIMIT 1",[key],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|error|error.to_string())?;
                            if let Some((id, name)) = matched {
                                project_id = Some(id);
                                project_name = name;
                                break 'outer;
                            }
                        }
                    }
                }
            }
            let checkpoint = meta
                .get("currentCheckpoint")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET project_id=COALESCE(excluded.project_id,sentinel_scans.project_id),project_name=CASE WHEN excluded.project_name='' THEN sentinel_scans.project_name ELSE excluded.project_name END,status=excluded.status,current_checkpoint=excluded.current_checkpoint,task_path=excluded.task_path,updated_at=datetime('now','localtime')", params![scan_id,project_id,project_name,status,checkpoint,dir.to_string_lossy()]).map_err(|e| e.to_string())?;
            let artifact_signature = sentinel_result_signature(&dir)?;
            let signature_key = format!("sentinel-result-signature:{scan_id}");
            let previous_signature = connection
                .query_row(
                    "SELECT value FROM app_settings WHERE key=?1",
                    [&signature_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if previous_signature.as_deref() == Some(artifact_signature.as_str()) {
                // meta/status is still refreshed above, but immutable findings
                // are not deleted and reconstructed on every polling tick.
                if status_changed {
                    count += 1;
                }
                continue;
            }
            connection
                .execute(
                    "DELETE FROM sentinel_findings WHERE scan_id=?1 AND stage NOT IN ('local-inventory','local-sast')",
                    [scan_id],
                )
                .map_err(|e| e.to_string())?;
            connection
                .execute(
                    "DELETE FROM sentinel_checkpoints WHERE scan_id=?1",
                    [scan_id],
                )
                .map_err(|e| e.to_string())?;
            let mut checkpoint_files = Vec::new();
            collect_checkpoint_files(&dir, &mut checkpoint_files)?;
            // 根目录 S1-S5 是聚合摘要，URL 子目录文件通常更完整。先解析聚合，
            // 再用同 URL、同阶段的详细文件整体替换，避免重复漏洞和重复端点。
            checkpoint_files.sort_by_key(|(_, value)| {
                value
                    .get("url")
                    .and_then(JsonValue::as_str)
                    .filter(|url| !url.trim().is_empty())
                    .map(|_| 1)
                    .unwrap_or(0)
            });
            for (stage, value) in checkpoint_files {
                let url = value_first(&value, &["url", "target", "host"]);
                let url = if url.is_empty() { "*".to_string() } else { url };
                if url != "*" && matches!(stage.as_str(), "s1" | "s2" | "s3" | "s4" | "s5") {
                    connection
                    .execute(
                        "DELETE FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage=?3",
                        params![scan_id, url, stage],
                    )
                    .map_err(|error| error.to_string())?;
                }
                connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,?2,?3,?4) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')", params![scan_id,url,stage,value.to_string()]).map_err(|e| e.to_string())?;
                parse_checkpoint_findings(&connection, &scan_id, &stage, &value)?;
            }
            if let Some(project_id) = project_id {
                let mut company_by_url: HashMap<String, String> = HashMap::new();
                let mut asset_statement = connection.prepare("SELECT a.company,a.link,a.host,a.domain,a.ip,a.port FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=?1 AND pa.is_deleted=0").map_err(|error| error.to_string())?;
                let asset_rows = asset_statement
                    .query_map([project_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    })
                    .map_err(|error| error.to_string())?;
                for row in asset_rows {
                    let (company, link, host, domain, ip, port) =
                        row.map_err(|error| error.to_string())?;
                    for raw in [
                        link,
                        host,
                        domain,
                        if ip.is_empty() {
                            String::new()
                        } else if port.is_empty() {
                            ip
                        } else {
                            format!("{}:{}", ip, port)
                        },
                    ] {
                        let key = raw.trim().trim_end_matches('/').to_ascii_lowercase();
                        if !key.is_empty() {
                            company_by_url
                                .entry(key.clone())
                                .or_insert_with(|| company.clone());
                            company_by_url
                                .entry(
                                    key.trim_start_matches("https://")
                                        .trim_start_matches("http://")
                                        .to_string(),
                                )
                                .or_insert_with(|| company.clone());
                        }
                    }
                }
                let mut target_map: HashMap<String, (String, String)> = HashMap::new();
                let mut add_targets = |value: &JsonValue| {
                    if let Some(items) = value.as_array() {
                        for item in items {
                            let (url, company, target_status) = if let Some(url) = item.as_str() {
                                (url.to_string(), String::new(), String::new())
                            } else {
                                (
                                    value_first(item, &["url", "target", "host"]),
                                    value_first(item, &["company", "organization", "org"]),
                                    value_first(item, &["status", "state"]),
                                )
                            };
                            if !is_web_target_url(&url) {
                                continue;
                            }
                            let key = url.trim().trim_end_matches('/').to_ascii_lowercase();
                            let current = target_map
                                .entry(key)
                                .or_insert((String::new(), String::new()));
                            if !company.is_empty() {
                                current.0 = company
                            }
                            if !target_status.is_empty() {
                                current.1 = target_status
                            }
                        }
                    }
                };
                add_targets(meta.get("targets").unwrap_or(&JsonValue::Null));
                add_targets(summary.get("perTarget").unwrap_or(&JsonValue::Null));
                add_targets(summary.get("targets").unwrap_or(&JsonValue::Null));
                let mut finding_statement=connection.prepare("SELECT DISTINCT target_url FROM sentinel_findings WHERE scan_id=?1 AND target_url<>'' AND target_url<>'*'").map_err(|error| error.to_string())?;
                let finding_urls = finding_statement
                    .query_map([scan_id], |row| row.get::<_, String>(0))
                    .map_err(|error| error.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                for url in finding_urls {
                    if !is_web_target_url(&url) {
                        continue;
                    }
                    target_map
                        .entry(url.trim().trim_end_matches('/').to_ascii_lowercase())
                        .or_insert((String::new(), String::new()));
                }
                for (url, (mut company, target_status)) in target_map {
                    if company.trim().is_empty() {
                        let without_scheme = url
                            .trim_start_matches("https://")
                            .trim_start_matches("http://");
                        company = company_by_url
                            .get(&url)
                            .or_else(|| company_by_url.get(without_scheme))
                            .cloned()
                            .unwrap_or_default();
                    }
                    connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(project_id,scan_id,url) DO UPDATE SET company=CASE WHEN excluded.company='' THEN sentinel_targets.company ELSE excluded.company END,status=CASE WHEN excluded.status='' THEN sentinel_targets.status ELSE excluded.status END,updated_at=datetime('now','localtime')",params![project_id,scan_id,company,url,target_status]).map_err(|error| error.to_string())?;
                }
            }
            connection.execute(
                "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![signature_key, artifact_signature],
            ).map_err(|error| error.to_string())?;
            count += 1;
        }
    }
    count += sync_pending_frontend_recon(&connection, &state)?;
    count += sync_strix_results(&connection, &state)?;
    Ok(count)
}
