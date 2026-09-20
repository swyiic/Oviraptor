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
#[allow(clippy::too_many_arguments)]
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
