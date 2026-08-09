#[derive(Clone)]
struct AppSecCandidate {
    finding_id: i64,
    source_key: String,
    source_types: Vec<String>,
    engine: String,
    title: String,
    vulnerability_type: String,
    severity: String,
    confidence: String,
    url: String,
    method: String,
    parameter: String,
    file: String,
    symbol: String,
    start_line: i64,
    cwe: String,
    has_data_flow: bool,
    evidence: JsonValue,
}

fn normalized_endpoint(value: &str) -> String {
    let mut endpoint = value.trim().to_ascii_lowercase();
    if let Some(scheme) = endpoint.find("://") {
        let remainder = &endpoint[scheme + 3..];
        endpoint = remainder
            .find('/')
            .map(|index| remainder[index..].to_string())
            .unwrap_or_else(|| "/".to_string());
    }
    endpoint
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string()
}

fn canonical_vulnerability_type(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    for (needles, name) in [
        (
            &["sql injection", "sqli", "sql-injection"][..],
            "SQL Injection",
        ),
        (&["cross-site scripting", "xss"][..], "Cross-Site Scripting"),
        (
            &["command injection", "os command"][..],
            "Command Injection",
        ),
        (
            &["path traversal", "directory traversal"][..],
            "Path Traversal",
        ),
        (&["server-side request forgery", "ssrf"][..], "SSRF"),
        (
            &["insecure direct object", "idor", "broken access"][..],
            "Broken Access Control",
        ),
        (&["authentication", "auth bypass"][..], "Authentication"),
        (
            &["hardcoded secret", "secret", "credential"][..],
            "Secret Exposure",
        ),
        (
            &["dependency", "vulnerable package", "cve-"][..],
            "Vulnerable Dependency",
        ),
        (&["deserialization"][..], "Unsafe Deserialization"),
        (&["xml external", "xxe"][..], "XXE"),
        (&["open redirect"][..], "Open Redirect"),
    ] {
        if needles.iter().any(|needle| lower.contains(needle)) {
            return name.into();
        }
    }
    value.trim().chars().take(120).collect()
}

fn appsec_location(value: &JsonValue) -> (String, String, i64) {
    let location = value
        .get("code_locations")
        .and_then(JsonValue::as_array)
        .and_then(|items| items.first())
        .or_else(|| value.get("code_location"));
    let file = location
        .map(|item| value_first(item, &["file", "path", "uri"]))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| value_first(value, &["file", "path"]));
    let symbol = location
        .map(|item| value_first(item, &["method", "function", "symbol"]))
        .unwrap_or_default();
    let start_line = location
        .and_then(|item| item.get("start_line").or_else(|| item.get("startLine")))
        .and_then(JsonValue::as_i64)
        .or_else(|| {
            value
                .get("start_line")
                .or_else(|| value.get("startLine"))
                .and_then(JsonValue::as_i64)
        })
        .unwrap_or(0);
    (file, symbol, start_line)
}

fn appsec_candidates(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Vec<AppSecCandidate>, String> {
    let mut statement = connection.prepare("SELECT id,stage,kind,record_key,title,severity,target_url,record_json FROM sentinel_findings WHERE scan_id=?1 AND kind IN ('vulnerability','code_smell','security_hotspot','dependency','secret','sast','dast','iast') ORDER BY id").map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([scan_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut candidates = Vec::new();
    for (id, stage, kind, record_key, finding_title, finding_severity, target_url, raw) in rows {
        let evidence = json(raw);
        if kind == "dependency"
            && ![
                "cve",
                "cwe",
                "cvss",
                "advisory",
                "fixed_version",
                "affected_version",
            ]
            .iter()
            .any(|key| evidence.get(key).is_some())
            && evidence
                .get("dependency_metadata")
                .and_then(|value| value.get("fixed_version"))
                .is_none()
            && evidence.get("vulnerable").and_then(JsonValue::as_bool) != Some(true)
        {
            continue;
        }
        let (file, symbol, start_line) = appsec_location(&evidence);
        let engine = value_first(&evidence, &["engine", "source", "scanner"]);
        let mut url = value_first(&evidence, &["url", "endpoint", "target", "path"]);
        if url.is_empty()
            && (target_url.starts_with("http://") || target_url.starts_with("https://"))
        {
            url = target_url;
        }
        let method = value_first(&evidence, &["method", "http_method", "httpMethod"]);
        let parameter = value_first(&evidence, &["parameter", "param", "variable", "input"]);
        let title = value_first(&evidence, &["title", "message", "name"]);
        let title = if title.is_empty() {
            finding_title
        } else {
            title
        };
        let raw_type = value_first(&evidence, &["type", "finding_class", "category", "rule_id"]);
        let vulnerability_type = canonical_vulnerability_type(if raw_type.is_empty() {
            &title
        } else {
            &raw_type
        });
        let cwe = value_first(&evidence, &["cwe", "cwe_id"]);
        let has_runtime = !url.is_empty()
            || evidence.get("pocRequest").is_some()
            || evidence.get("payload").is_some()
            || evidence.get("request").is_some();
        let has_code = !file.is_empty()
            || ["semgrep", "codeql"].contains(&engine.to_ascii_lowercase().as_str())
            || matches!(
                kind.as_str(),
                "code_smell" | "security_hotspot" | "sast" | "secret"
            );
        let mut source_types = Vec::new();
        if kind == "dependency" {
            source_types.push("sca".into());
        } else if kind == "dast" {
            source_types.push("dast".into());
        } else if kind == "iast" {
            source_types.push("iast".into());
        } else {
            if has_code {
                source_types.push("sast".into());
            }
            if has_runtime {
                source_types.push("dast".into());
            }
        }
        if source_types.is_empty() {
            source_types.push(if stage == "strix" {
                "ai_validation".into()
            } else {
                "scanner".into()
            });
        }
        candidates.push(AppSecCandidate {
            finding_id: id,
            source_key: format!("{stage}:{kind}:{record_key}"),
            source_types,
            engine,
            title,
            vulnerability_type,
            severity: if finding_severity.trim().is_empty() {
                "info".into()
            } else {
                finding_severity.to_ascii_lowercase()
            },
            confidence: value_first(&evidence, &["confidence"]),
            url,
            method,
            parameter,
            file,
            symbol,
            start_line,
            cwe,
            has_data_flow: evidence.get("data_flow").is_some()
                || evidence.get("taint_flow").is_some()
                || evidence.get("call_chain").is_some(),
            evidence,
        });
    }
    Ok(candidates)
}

fn appsec_correlation(left: &AppSecCandidate, right: &AppSecCandidate) -> (i64, JsonValue) {
    let type_match = (!left.vulnerability_type.is_empty()
        && left
            .vulnerability_type
            .eq_ignore_ascii_case(&right.vulnerability_type))
        || (!left.cwe.is_empty() && left.cwe.eq_ignore_ascii_case(&right.cwe));
    let left_endpoint = normalized_endpoint(&left.url);
    let right_endpoint = normalized_endpoint(&right.url);
    let endpoint_match = !left_endpoint.is_empty() && left_endpoint == right_endpoint;
    let parameter_match =
        !left.parameter.is_empty() && left.parameter.eq_ignore_ascii_case(&right.parameter);
    let file_match = !left.file.is_empty()
        && left.file.eq_ignore_ascii_case(&right.file)
        && (left.start_line == 0 || right.start_line == 0 || left.start_line == right.start_line);
    let left_has_sast = left.source_types.iter().any(|value| value == "sast");
    let left_has_dast = left.source_types.iter().any(|value| value == "dast");
    let right_has_sast = right.source_types.iter().any(|value| value == "sast");
    let right_has_dast = right.source_types.iter().any(|value| value == "dast");
    let cross_runtime = (left_has_sast && right_has_dast) || (left_has_dast && right_has_sast);
    let data_flow_match =
        cross_runtime && type_match && (left.has_data_flow || right.has_data_flow);
    let mut score = 0;
    if type_match {
        score += 30;
    }
    if endpoint_match {
        score += 30;
    }
    if parameter_match {
        score += 20;
    }
    if data_flow_match {
        score += 20;
    }
    if file_match {
        score += 40;
    }
    score = score.min(100);
    (
        score,
        serde_json::json!({"type":{"matched":type_match,"weight":30},"url":{"matched":endpoint_match,"weight":30},"parameter":{"matched":parameter_match,"weight":20},"dataFlow":{"matched":data_flow_match,"weight":20},"codeLocation":{"matched":file_match,"weight":40}}),
    )
}

fn severity_rank(value: &str) -> i64 {
    match value.to_ascii_lowercase().as_str() {
        "critical" => 5,
        "high" => 4,
        "medium" => 3,
        "low" => 2,
        _ => 1,
    }
}

fn refresh_appsec_vulnerabilities(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<(), String> {
    let project_id: i64 = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|_| "扫描任务没有项目归属".to_string())?;
    let environment: String = connection
        .query_row(
            "SELECT environment FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan_id],
            |row| row.get(0),
        )
        .unwrap_or_default();
    let candidates = appsec_candidates(connection, scan_id)?;
    let mut groups: Vec<(Vec<AppSecCandidate>, i64, JsonValue)> = Vec::new();
    for candidate in candidates {
        let mut best: Option<(usize, i64, JsonValue)> = None;
        for (index, (members, _, _)) in groups.iter().enumerate() {
            for member in members {
                let (score, detail) = appsec_correlation(member, &candidate);
                if score >= 60 && best.as_ref().is_none_or(|value| score > value.1) {
                    best = Some((index, score, detail));
                }
            }
        }
        if let Some((index, score, detail)) = best {
            groups[index].0.push(candidate);
            if score > groups[index].1 {
                groups[index].1 = score;
                groups[index].2 = detail;
            }
        } else {
            let embedded_score = if candidate.source_types.contains(&"sast".to_string())
                && candidate.source_types.contains(&"dast".to_string())
            {
                100
            } else {
                0
            };
            groups.push((
                vec![candidate],
                embedded_score,
                serde_json::json!({"embeddedEvidence":embedded_score==100}),
            ));
        }
    }
    connection
        .execute(
            "DELETE FROM appsec_vulnerability_sources WHERE scan_id=?1",
            [scan_id],
        )
        .map_err(|error| error.to_string())?;
    for (members, score, detail) in groups {
        let representative = members
            .iter()
            .max_by_key(|item| severity_rank(&item.severity))
            .expect("group always contains a candidate");
        let file = members
            .iter()
            .find(|item| !item.file.is_empty())
            .map(|item| item.file.clone())
            .unwrap_or_default();
        let start_line = members
            .iter()
            .find(|item| item.start_line > 0)
            .map(|item| item.start_line)
            .unwrap_or(0);
        let url = members
            .iter()
            .find(|item| !item.url.is_empty())
            .map(|item| item.url.clone())
            .unwrap_or_default();
        let method = members
            .iter()
            .find(|item| !item.method.is_empty())
            .map(|item| item.method.clone())
            .unwrap_or_default();
        let parameter = members
            .iter()
            .find(|item| !item.parameter.is_empty())
            .map(|item| item.parameter.clone())
            .unwrap_or_default();
        let symbol = members
            .iter()
            .find(|item| !item.symbol.is_empty())
            .map(|item| item.symbol.clone())
            .unwrap_or_default();
        let mut fingerprint = format!(
            "{}|{}|{}|{}|{}|{}",
            representative.vulnerability_type.to_ascii_lowercase(),
            file.to_ascii_lowercase(),
            start_line,
            normalized_endpoint(&url),
            method.to_ascii_uppercase(),
            parameter.to_ascii_lowercase()
        );
        if file.is_empty() && url.is_empty() && parameter.is_empty() && start_line == 0 {
            fingerprint.push('|');
            fingerprint.push_str(&representative.source_key.to_ascii_lowercase());
        }
        connection.execute("INSERT INTO appsec_vulnerabilities(project_id,fingerprint,title,vulnerability_type,severity,confidence,asset,environment,url,http_method,parameter,file,symbol,start_line,correlation_score,correlation_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) ON CONFLICT(project_id,fingerprint) DO UPDATE SET title=excluded.title,severity=CASE WHEN CASE excluded.severity WHEN 'critical' THEN 5 WHEN 'high' THEN 4 WHEN 'medium' THEN 3 WHEN 'low' THEN 2 ELSE 1 END > CASE appsec_vulnerabilities.severity WHEN 'critical' THEN 5 WHEN 'high' THEN 4 WHEN 'medium' THEN 3 WHEN 'low' THEN 2 ELSE 1 END THEN excluded.severity ELSE appsec_vulnerabilities.severity END,confidence=excluded.confidence,asset=excluded.asset,environment=excluded.environment,url=excluded.url,http_method=excluded.http_method,parameter=excluded.parameter,file=excluded.file,symbol=excluded.symbol,start_line=excluded.start_line,correlation_score=excluded.correlation_score,correlation_json=excluded.correlation_json,last_seen=datetime('now','localtime')",params![project_id,fingerprint,representative.title,representative.vulnerability_type,representative.severity,representative.confidence,url,environment,url,method,parameter,file,symbol,start_line,score,detail.to_string()]).map_err(|error|error.to_string())?;
        let vulnerability_id: i64 = connection
            .query_row(
                "SELECT id FROM appsec_vulnerabilities WHERE project_id=?1 AND fingerprint=?2",
                params![project_id, fingerprint],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        for member in members {
            for source_type in &member.source_types {
                connection.execute("INSERT OR REPLACE INTO appsec_vulnerability_sources(vulnerability_id,scan_id,finding_id,source_type,source_key,engine,evidence_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![vulnerability_id,scan_id,member.finding_id,source_type,member.source_key,member.engine,member.evidence.to_string()]).map_err(|error|error.to_string())?;
            }
        }
    }
    Ok(())
}

fn appsec_context_row(row: &Row<'_>) -> rusqlite::Result<AppSecScanContext> {
    Ok(AppSecScanContext {
        scan_id: row.get(0)?,
        environment: row.get(1)?,
        auth_profile_name: row.get(2)?,
        auth_type: row.get(3)?,
        authenticated: row.get::<_, i64>(4)? != 0,
        ci_provider: row.get(5)?,
        repository_url: row.get(6)?,
        branch: row.get(7)?,
        commit_sha: row.get(8)?,
        build_id: row.get(9)?,
        policy: json(row.get(10)?),
        gate_status: row.get(11)?,
        gate_reason: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn evaluate_appsec_gate(connection: &rusqlite::Connection, scan_id: &str) -> Result<(), String> {
    let policy: Option<String> = connection
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(policy) = policy else {
        return Ok(());
    };
    let policy = json(policy);
    let max_critical = policy
        .get("maxCritical")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let max_high = policy
        .get("maxHigh")
        .and_then(JsonValue::as_i64)
        .unwrap_or(5);
    let block_release = policy
        .get("blockRelease")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let (critical, high): (i64,i64) = connection.query_row("SELECT COUNT(DISTINCT CASE WHEN v.severity='critical' THEN v.id END),COUNT(DISTINCT CASE WHEN v.severity='high' THEN v.id END) FROM appsec_vulnerabilities v JOIN appsec_vulnerability_sources s ON s.vulnerability_id=v.id WHERE s.scan_id=?1",[scan_id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|error|error.to_string())?;
    let exceeded = critical > max_critical || high > max_high;
    let status = if exceeded && block_release {
        "blocked"
    } else if exceeded {
        "warning"
    } else {
        "passed"
    };
    let reason = format!(
        "Critical {critical}/{max_critical} · High {high}/{max_high} · {}",
        if block_release {
            "超限时阻断发布"
        } else {
            "仅告警"
        }
    );
    connection.execute("UPDATE sentinel_scan_contexts SET gate_status=?1,gate_reason=?2,updated_at=datetime('now','localtime') WHERE scan_id=?3",params![status,reason,scan_id]).map_err(|error|error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn list_appsec_scan_result(
    state: State<AppState>,
    scan_id: String,
) -> Result<AppSecScanResult, String> {
    let connection = db::open(&state.db_path)?;
    refresh_appsec_vulnerabilities(&connection, &scan_id)?;
    evaluate_appsec_gate(&connection, &scan_id)?;
    let mut vulnerabilities_statement=connection.prepare("SELECT DISTINCT v.id,v.project_id,v.fingerprint,v.title,v.vulnerability_type,v.severity,v.status,v.confidence,v.asset,v.environment,v.url,v.http_method,v.parameter,v.file,v.symbol,v.start_line,v.correlation_score,v.correlation_json,v.first_seen,v.last_seen,v.owner FROM appsec_vulnerabilities v JOIN appsec_vulnerability_sources s ON s.vulnerability_id=v.id WHERE s.scan_id=?1 ORDER BY CASE v.severity WHEN 'critical' THEN 5 WHEN 'high' THEN 4 WHEN 'medium' THEN 3 WHEN 'low' THEN 2 ELSE 1 END DESC,v.id").map_err(|error|error.to_string())?;
    let vulnerabilities = vulnerabilities_statement
        .query_map([&scan_id], |row| {
            Ok(AppSecVulnerability {
                id: row.get(0)?,
                project_id: row.get(1)?,
                fingerprint: row.get(2)?,
                title: row.get(3)?,
                vulnerability_type: row.get(4)?,
                severity: row.get(5)?,
                status: row.get(6)?,
                confidence: row.get(7)?,
                asset: row.get(8)?,
                environment: row.get(9)?,
                url: row.get(10)?,
                http_method: row.get(11)?,
                parameter: row.get(12)?,
                file: row.get(13)?,
                symbol: row.get(14)?,
                start_line: row.get(15)?,
                correlation_score: row.get(16)?,
                correlation: json(row.get(17)?),
                first_seen: row.get(18)?,
                last_seen: row.get(19)?,
                owner: row.get(20)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut sources_statement=connection.prepare("SELECT id,vulnerability_id,scan_id,finding_id,source_type,source_key,engine,evidence_json,created_at FROM appsec_vulnerability_sources WHERE scan_id=?1 ORDER BY vulnerability_id,source_type").map_err(|error|error.to_string())?;
    let sources = sources_statement
        .query_map([&scan_id], |row| {
            Ok(AppSecVulnerabilitySource {
                id: row.get(0)?,
                vulnerability_id: row.get(1)?,
                scan_id: row.get(2)?,
                finding_id: row.get(3)?,
                source_type: row.get(4)?,
                source_key: row.get(5)?,
                engine: row.get(6)?,
                evidence: json(row.get(7)?),
                created_at: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let context=connection.query_row("SELECT scan_id,environment,auth_profile_name,auth_type,authenticated,ci_provider,repository_url,branch,commit_sha,build_id,policy_json,gate_status,gate_reason,created_at,updated_at FROM sentinel_scan_contexts WHERE scan_id=?1",[&scan_id],appsec_context_row).optional().map_err(|error|error.to_string())?;
    Ok(AppSecScanResult {
        vulnerabilities,
        sources,
        context,
    })
}

#[tauri::command]
pub async fn sentinel_overview_stats(
    state: State<'_, AppState>,
    project_id: Option<i64>,
) -> Result<SentinelOverviewStats, String> {
    let connection = db::open(&state.db_path)?;
    let count = |sql: &str| -> Result<i64, String> {
        connection
            .query_row(sql, params![project_id], |r| r.get(0))
            .map_err(|e| e.to_string())
    };
    Ok(SentinelOverviewStats {
        task_count: count("SELECT COUNT(*) FROM sentinel_scans WHERE (?1 IS NULL OR project_id=?1)")?,
        url_count: count("SELECT COUNT(DISTINCT url) FROM sentinel_targets WHERE (?1 IS NULL OR project_id=?1)")?,
        fingerprint_count: count("SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE (?1 IS NULL OR s.project_id=?1) AND f.kind IN ('fingerprint','wordpress','tech_stack')")?,
        api_count: count("SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE (?1 IS NULL OR s.project_id=?1) AND f.kind IN ('api','route','external_script','env_var','realtime_endpoint')")?,
        endpoint_count: count("SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE (?1 IS NULL OR s.project_id=?1) AND (f.kind LIKE 'endpoint%' OR f.kind LIKE 'directory_%' OR f.kind='login_endpoint')")?,
        vulnerability_count: count("SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE (?1 IS NULL OR s.project_id=?1) AND f.kind='vulnerability'")?,
        high_risk_count: count("SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id LEFT JOIN sentinel_validations v ON v.scan_id=f.scan_id AND v.url=f.target_url AND v.finding_key=(f.stage||':'||f.kind||':'||f.record_key) WHERE (?1 IS NULL OR s.project_id=?1) AND f.kind='vulnerability' AND COALESCE(CASE WHEN v.verdict='false_positive' THEN 'none' WHEN v.verdict<>'pending' AND v.severity<>'' THEN lower(v.severity) END,lower(f.severity)) IN ('high','critical')")?,
        validated_count: count("SELECT COUNT(*) FROM sentinel_validations v JOIN sentinel_scans s ON s.id=v.scan_id WHERE (?1 IS NULL OR s.project_id=?1) AND v.verdict <> 'pending'")?,
        pending_vulnerability_count: count("SELECT COUNT(*) FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE (?1 IS NULL OR s.project_id=?1) AND f.kind='vulnerability' AND NOT EXISTS (SELECT 1 FROM sentinel_validations v WHERE v.scan_id=f.scan_id AND v.url=f.target_url AND v.finding_key=(f.stage||':'||f.kind||':'||f.record_key) AND v.verdict<>'pending')")?,
        vulnerable_url_count: count("SELECT COUNT(*) FROM (SELECT DISTINCT f.scan_id,f.target_url FROM sentinel_findings f JOIN sentinel_scans s ON s.id=f.scan_id WHERE (?1 IS NULL OR s.project_id=?1) AND f.kind='vulnerability' AND trim(f.target_url)<>'' AND f.target_url<>'*')")?,
        active_fuse_count: count("SELECT COUNT(*) FROM sentinel_fuse_zone WHERE (?1 IS NULL OR project_id=?1) AND archived=0 AND verdict='pending'")?,
        opportunity_count: count("SELECT COUNT(*) FROM sentinel_opportunities WHERE (?1 IS NULL OR project_id=?1) AND status IN ('queued','ready','in_progress')")?,
        ready_opportunity_count: count("SELECT COUNT(*) FROM sentinel_opportunities WHERE (?1 IS NULL OR project_id=?1) AND status IN ('ready','in_progress') AND score>=65")?,
    })
}

#[tauri::command]
pub async fn list_sentinel_validations(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<Vec<SentinelValidation>, String> {
    let connection = db::open(&state.db_path)?;
    let mut stmt = connection.prepare("SELECT id,scan_id,url,finding_key,finding_kind,verdict,severity,note,evidence,created_at,updated_at FROM sentinel_validations WHERE scan_id=?1 ORDER BY updated_at DESC").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([scan_id], |r| {
            Ok(SentinelValidation {
                id: r.get(0)?,
                scan_id: r.get(1)?,
                url: r.get(2)?,
                finding_key: r.get(3)?,
                finding_kind: r.get(4)?,
                verdict: r.get(5)?,
                severity: r.get(6)?,
                note: r.get(7)?,
                evidence: r.get(8)?,
                created_at: r.get(9)?,
                updated_at: r.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_all_sentinel_validations(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<SentinelValidation>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(
        "SELECT v.id,v.scan_id,v.url,v.finding_key,v.finding_kind,v.verdict,v.severity,v.note,v.evidence,v.created_at,v.updated_at
         FROM sentinel_validations v JOIN sentinel_scans s ON s.id=v.scan_id
         WHERE (?1 IS NULL OR s.project_id=?1)
         ORDER BY v.updated_at DESC,v.id DESC LIMIT ?2"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![project_id, limit.unwrap_or(5000).clamp(100, 20_000)],
            |row| {
                Ok(SentinelValidation {
                    id: row.get(0)?,
                    scan_id: row.get(1)?,
                    url: row.get(2)?,
                    finding_key: row.get(3)?,
                    finding_kind: row.get(4)?,
                    verdict: row.get(5)?,
                    severity: row.get(6)?,
                    note: row.get(7)?,
                    evidence: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_sentinel_validation_work_items(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<SentinelValidationWorkItem>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT f.id,f.scan_id,s.project_id,s.project_name,s.task_name,f.target_url,
                (f.stage||':'||f.kind||':'||f.record_key),f.kind,f.title,f.severity,f.record_json,
                v.id,COALESCE(v.verdict,'pending'),COALESCE(v.severity,''),
                COALESCE(v.note,''),COALESCE(v.evidence,''),COALESCE(v.updated_at,f.updated_at)
         FROM sentinel_findings f
         JOIN sentinel_scans s ON s.id=f.scan_id
         LEFT JOIN sentinel_validations v
           ON v.scan_id=f.scan_id AND v.url=f.target_url
          AND v.finding_key=(f.stage||':'||f.kind||':'||f.record_key)
         WHERE f.kind='vulnerability' AND (?1 IS NULL OR s.project_id=?1)
         ORDER BY
           CASE WHEN v.id IS NULL OR v.verdict IN ('','pending','needs_more') THEN 0 ELSE 1 END,
           CASE lower(COALESCE(NULLIF(v.severity,''),f.severity))
             WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2
             WHEN 'low' THEN 3 ELSE 4 END,
           COALESCE(v.updated_at,f.updated_at) DESC,f.id DESC
         LIMIT ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![project_id, limit.unwrap_or(5000).clamp(100, 20_000)],
            |row| {
                Ok(SentinelValidationWorkItem {
                    finding_id: row.get(0)?,
                    scan_id: row.get(1)?,
                    project_id: row.get(2)?,
                    project_name: row.get(3)?,
                    task_name: row.get(4)?,
                    url: row.get(5)?,
                    finding_key: row.get(6)?,
                    finding_kind: row.get(7)?,
                    title: row.get(8)?,
                    original_severity: row.get(9)?,
                    record_json: row.get(10)?,
                    validation_id: row.get(11)?,
                    verdict: row.get(12)?,
                    confirmed_severity: row.get(13)?,
                    note: row.get(14)?,
                    evidence: row.get(15)?,
                    updated_at: row.get(16)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn save_sentinel_validation(
    state: State<AppState>,
    input: SentinelValidationInput,
) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    connection.execute("INSERT INTO sentinel_validations(scan_id,url,finding_key,finding_kind,verdict,severity,note,evidence) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(scan_id,url,finding_key) DO UPDATE SET finding_kind=excluded.finding_kind,verdict=excluded.verdict,severity=excluded.severity,note=excluded.note,evidence=excluded.evidence,updated_at=datetime('now','localtime')", params![input.scan_id,input.url,input.finding_key,input.finding_kind,input.verdict,input.severity,input.note,input.evidence]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn export_sentinel_results(state: State<AppState>, scan_id: String) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    let scan: JsonValue = connection.query_row("SELECT json_object('id',id,'projectId',project_id,'projectName',project_name,'status',status,'currentCheckpoint',current_checkpoint,'taskPath',task_path,'previousScanId',previous_scan_id,'llmRequests',llm_requests,'inputTokens',input_tokens,'outputTokens',output_tokens,'cachedTokens',cached_tokens,'totalTokens',total_tokens,'scanType',scan_type,'taskName',task_name,'sourcePath',source_path,'skillNames',skill_names,'attemptCount',attempt_count,'createdAt',created_at,'updatedAt',updated_at) FROM sentinel_scans WHERE id=?1", [&scan_id], |r| r.get::<_,String>(0)).map(|text| json(text)).map_err(|_| "任务不存在".to_string())?;
    let checkpoints: Vec<JsonValue> = {
        let mut s=connection.prepare("SELECT json_object('scanId',scan_id,'url',url,'stage',stage,'rawJson',raw_json,'updatedAt',updated_at) FROM sentinel_checkpoints WHERE scan_id=?1").map_err(|e|e.to_string())?;
        let rows = s
            .query_map([&scan_id], |r| Ok(json(r.get::<_, String>(0)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows?
    };
    let validations: Vec<JsonValue> = {
        let mut s=connection.prepare("SELECT json_object('id',id,'scanId',scan_id,'url',url,'findingKey',finding_key,'findingKind',finding_kind,'verdict',verdict,'severity',severity,'note',note,'evidence',evidence,'createdAt',created_at,'updatedAt',updated_at) FROM sentinel_validations WHERE scan_id=?1").map_err(|e|e.to_string())?;
        let rows = s
            .query_map([&scan_id], |r| Ok(json(r.get::<_, String>(0)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows?
    };
    let findings: Vec<JsonValue> = {
        let mut s=connection.prepare("SELECT json_object('id',id,'scanId',scan_id,'targetUrl',target_url,'stage',stage,'kind',kind,'recordKey',record_key,'title',title,'severity',severity,'recordJson',record_json,'updatedAt',updated_at) FROM sentinel_findings WHERE scan_id=?1").map_err(|e|e.to_string())?;
        let rows = s
            .query_map([&scan_id], |r| Ok(json(r.get::<_, String>(0)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows?
    };
    let opportunities: Vec<JsonValue> = {
        let mut s=connection.prepare("SELECT json_object('id',id,'projectId',project_id,'scanId',scan_id,'targetUrl',target_url,'opportunityKey',opportunity_key,'category',category,'title',title,'score',score,'status',status,'confidence',confidence,'whyJson',why_json,'evidenceJson',evidence_json,'recommendedActionJson',recommended_action_json,'source',source,'recordJson',record_json,'firstSeen',first_seen,'lastSeen',last_seen) FROM sentinel_opportunities WHERE scan_id=?1").map_err(|e|e.to_string())?;
        let rows = s
            .query_map([&scan_id], |r| Ok(json(r.get::<_, String>(0)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows?
    };
    fs::create_dir_all(&state.export_dir).map_err(|e| e.to_string())?;
    let path = state.export_dir.join(format!("sentinel-{}.json", scan_id));
    let bundle = serde_json::json!({"format":"oviraptor-sentinel-v1","exportedAt":chrono::Utc::now().to_rfc3339(),"scan":scan,"checkpoints":checkpoints,"findings":findings,"opportunities":opportunities,"validations":validations});
    fs::write(
        &path,
        serde_json::to_vec_pretty(&bundle).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn import_sentinel_results(state: State<AppState>, content: String) -> Result<i64, String> {
    let bundle: JsonValue =
        serde_json::from_str(&content).map_err(|e| format!("结果文件不是有效 JSON：{}", e))?;
    let scan = bundle.get("scan").ok_or("缺少 scan 数据")?;
    let id = scan
        .get("id")
        .and_then(JsonValue::as_str)
        .ok_or("缺少 scan.id")?;
    let connection = db::open(&state.db_path)?;
    connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,previous_scan_id,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,scan_type,task_name,source_path,skill_names,attempt_count,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,COALESCE(?18,datetime('now','localtime')),COALESCE(?19,datetime('now','localtime'))) ON CONFLICT(id) DO UPDATE SET project_name=excluded.project_name,status=excluded.status,current_checkpoint=excluded.current_checkpoint,task_path=excluded.task_path,previous_scan_id=excluded.previous_scan_id,llm_requests=excluded.llm_requests,input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cached_tokens=excluded.cached_tokens,total_tokens=excluded.total_tokens,scan_type=excluded.scan_type,task_name=excluded.task_name,source_path=excluded.source_path,skill_names=excluded.skill_names,attempt_count=excluded.attempt_count,updated_at=datetime('now','localtime')", params![id,scan.get("projectId").and_then(JsonValue::as_i64),scan.get("projectName").and_then(JsonValue::as_str).unwrap_or(""),scan.get("status").and_then(JsonValue::as_str).unwrap_or("imported"),scan.get("currentCheckpoint").and_then(JsonValue::as_str).unwrap_or(""),scan.get("taskPath").and_then(JsonValue::as_str).unwrap_or(""),scan.get("previousScanId").and_then(JsonValue::as_str).unwrap_or(""),scan.get("llmRequests").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("inputTokens").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("outputTokens").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("cachedTokens").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("totalTokens").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("scanType").and_then(JsonValue::as_str).unwrap_or("web"),scan.get("taskName").and_then(JsonValue::as_str).unwrap_or(""),scan.get("sourcePath").and_then(JsonValue::as_str).unwrap_or(""),scan.get("skillNames").and_then(JsonValue::as_str).unwrap_or(""),scan.get("attemptCount").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("createdAt").and_then(JsonValue::as_str),scan.get("updatedAt").and_then(JsonValue::as_str)]).map_err(|e|e.to_string())?;
    let mut imported = 1;
    if let Some(items) = bundle.get("checkpoints").and_then(JsonValue::as_array) {
        for item in items {
            let raw = item
                .get("rawJson")
                .and_then(JsonValue::as_str)
                .unwrap_or("{}");
            connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,?2,?3,?4) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')", params![id,item.get("url").and_then(JsonValue::as_str).unwrap_or("*"),item.get("stage").and_then(JsonValue::as_str).unwrap_or("imported"),raw]).map_err(|e|e.to_string())?;
            imported += 1;
        }
    }
    if let Some(items) = bundle.get("validations").and_then(JsonValue::as_array) {
        for item in items {
            connection.execute("INSERT INTO sentinel_validations(scan_id,url,finding_key,finding_kind,verdict,severity,note,evidence) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(scan_id,url,finding_key) DO UPDATE SET finding_kind=excluded.finding_kind,verdict=excluded.verdict,severity=excluded.severity,note=excluded.note,evidence=excluded.evidence,updated_at=datetime('now','localtime')", params![id,item.get("url").and_then(JsonValue::as_str).unwrap_or(""),item.get("findingKey").and_then(JsonValue::as_str).unwrap_or("url-summary"),item.get("findingKind").and_then(JsonValue::as_str).unwrap_or(""),item.get("verdict").and_then(JsonValue::as_str).unwrap_or("pending"),item.get("severity").and_then(JsonValue::as_str).unwrap_or(""),item.get("note").and_then(JsonValue::as_str).unwrap_or(""),item.get("evidence").and_then(JsonValue::as_str).unwrap_or("")]).map_err(|e|e.to_string())?;
            imported += 1;
        }
    }
    if let Some(items) = bundle.get("findings").and_then(JsonValue::as_array) {
        for item in items {
            connection.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')", params![id,item.get("targetUrl").and_then(JsonValue::as_str).unwrap_or(""),item.get("stage").and_then(JsonValue::as_str).unwrap_or(""),item.get("kind").and_then(JsonValue::as_str).unwrap_or(""),item.get("recordKey").and_then(JsonValue::as_str).unwrap_or(""),item.get("title").and_then(JsonValue::as_str).unwrap_or(""),item.get("severity").and_then(JsonValue::as_str).unwrap_or(""),item.get("recordJson").and_then(JsonValue::as_str).unwrap_or("{}")]).map_err(|e|e.to_string())?;
            imported += 1;
        }
    }
    if let Some(items) = bundle.get("opportunities").and_then(JsonValue::as_array) {
        for item in items {
            connection.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,why_json,evidence_json,recommended_action_json,source,record_json,first_seen,last_seen) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,COALESCE(?15,datetime('now','localtime')),COALESCE(?16,datetime('now','localtime'))) ON CONFLICT(scan_id,target_url,opportunity_key) DO UPDATE SET project_id=excluded.project_id,category=excluded.category,title=excluded.title,score=excluded.score,status=excluded.status,confidence=excluded.confidence,why_json=excluded.why_json,evidence_json=excluded.evidence_json,recommended_action_json=excluded.recommended_action_json,source=excluded.source,record_json=excluded.record_json,last_seen=excluded.last_seen", params![item.get("projectId").and_then(JsonValue::as_i64),id,item.get("targetUrl").and_then(JsonValue::as_str).unwrap_or(""),item.get("opportunityKey").and_then(JsonValue::as_str).unwrap_or(""),item.get("category").and_then(JsonValue::as_str).unwrap_or(""),item.get("title").and_then(JsonValue::as_str).unwrap_or(""),item.get("score").and_then(JsonValue::as_i64).unwrap_or(0),item.get("status").and_then(JsonValue::as_str).unwrap_or("queued"),item.get("confidence").and_then(JsonValue::as_str).unwrap_or(""),item.get("whyJson").and_then(JsonValue::as_str).unwrap_or("[]"),item.get("evidenceJson").and_then(JsonValue::as_str).unwrap_or("[]"),item.get("recommendedActionJson").and_then(JsonValue::as_str).unwrap_or("{}"),item.get("source").and_then(JsonValue::as_str).unwrap_or(""),item.get("recordJson").and_then(JsonValue::as_str).unwrap_or("{}"),item.get("firstSeen").and_then(JsonValue::as_str),item.get("lastSeen").and_then(JsonValue::as_str)]).map_err(|e|e.to_string())?;
            imported += 1;
        }
    }
    Ok(imported)
}

pub(crate) fn sentinel_project_bundle(
    state: &AppState,
    project_id: i64,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    let project: JsonValue = connection.query_row(
        "SELECT json_object('id',id,'name',name,'description',description,'createdAt',created_at,'updatedAt',updated_at) FROM projects WHERE id=?1",
        [project_id], |row| row.get::<_,String>(0),
    ).map(json).map_err(|_| "项目不存在".to_string())?;
    let collect = |sql: &str| -> Result<Vec<JsonValue>, String> {
        let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
        let result = statement
            .query_map([project_id], |row| Ok(json(row.get::<_, String>(0)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(result)
    };
    let scans=collect("SELECT json_object('id',id,'projectName',project_name,'status',status,'currentCheckpoint',current_checkpoint,'taskPath',task_path,'previousScanId',previous_scan_id,'llmRequests',llm_requests,'inputTokens',input_tokens,'outputTokens',output_tokens,'cachedTokens',cached_tokens,'totalTokens',total_tokens,'scanType',scan_type,'taskName',task_name,'sourcePath',source_path,'skillNames',skill_names,'attemptCount',attempt_count,'createdAt',created_at,'updatedAt',updated_at) FROM sentinel_scans WHERE project_id=?1 ORDER BY created_at")?;
    let targets=collect("SELECT json_object('scanId',scan_id,'company',company,'url',url,'status',status,'valueScore',value_score,'scanMode',scan_mode,'routingReason',routing_reason,'createdAt',created_at,'updatedAt',updated_at) FROM sentinel_targets WHERE project_id=?1 AND scan_id IS NOT NULL ORDER BY id")?;
    let checkpoints=collect("SELECT json_object('scanId',scan_id,'url',url,'stage',stage,'rawJson',raw_json,'updatedAt',updated_at) FROM sentinel_checkpoints WHERE scan_id IN(SELECT id FROM sentinel_scans WHERE project_id=?1)")?;
    let findings=collect("SELECT json_object('scanId',scan_id,'targetUrl',target_url,'stage',stage,'kind',kind,'recordKey',record_key,'title',title,'severity',severity,'recordJson',record_json,'updatedAt',updated_at) FROM sentinel_findings WHERE scan_id IN(SELECT id FROM sentinel_scans WHERE project_id=?1)")?;
    let opportunities=collect("SELECT json_object('scanId',scan_id,'targetUrl',target_url,'opportunityKey',opportunity_key,'category',category,'title',title,'score',score,'status',status,'confidence',confidence,'whyJson',why_json,'evidenceJson',evidence_json,'recommendedActionJson',recommended_action_json,'source',source,'recordJson',record_json,'firstSeen',first_seen,'lastSeen',last_seen) FROM sentinel_opportunities WHERE project_id=?1")?;
    let validations=collect("SELECT json_object('scanId',scan_id,'url',url,'findingKey',finding_key,'findingKind',finding_kind,'verdict',verdict,'severity',severity,'note',note,'evidence',evidence,'createdAt',created_at,'updatedAt',updated_at) FROM sentinel_validations WHERE scan_id IN(SELECT id FROM sentinel_scans WHERE project_id=?1)")?;
    let fuse_zone=collect("SELECT json_object('assetId',asset_id,'company',company,'url',url,'sourceScanId',source_scan_id,'reason',reason,'verdict',verdict,'note',note,'evidence',evidence,'archived',CASE WHEN archived=1 THEN json('true') ELSE json('false') END,'createdAt',created_at,'updatedAt',updated_at) FROM sentinel_fuse_zone WHERE project_id=?1 ORDER BY id")?;
    let bundle = serde_json::json!({"format":"oviraptor-sentinel-project-v2","exportedAt":chrono::Utc::now().to_rfc3339(),"project":project,"scans":scans,"targets":targets,"checkpoints":checkpoints,"findings":findings,"opportunities":opportunities,"validations":validations,"fuseZone":fuse_zone});
    Ok(bundle)
}

#[tauri::command]
pub fn export_sentinel_project(state: State<AppState>, project_id: i64) -> Result<String, String> {
    let bundle = sentinel_project_bundle(&state, project_id)?;
    fs::create_dir_all(&state.export_dir).map_err(|error| error.to_string())?;
    let safe_name = bundle
        .pointer("/project/name")
        .and_then(JsonValue::as_str)
        .unwrap_or("project")
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let path = state.export_dir.join(format!(
        "sentinel-project-{}-{}.json",
        safe_name,
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&bundle).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn import_sentinel_project(state: State<AppState>, path: String) -> Result<i64, String> {
    let source = PathBuf::from(path);
    let metadata = fs::metadata(&source).map_err(|error| format!("无法读取导入文件：{}", error))?;
    if metadata.len() > 512 * 1024 * 1024 {
        return Err("项目包超过 512 MB，请先拆分或压缩历史原始结果".into());
    }
    let bundle: JsonValue =
        serde_json::from_slice(&fs::read(&source).map_err(|error| error.to_string())?)
            .map_err(|error| format!("项目包不是有效 JSON：{}", error))?;
    if !matches!(
        bundle.get("format").and_then(JsonValue::as_str),
        Some("oviraptor-sentinel-project-v2" | "asset-atlas-sentinel-project-v2")
    ) {
        return Err("不是 Oviraptor Sentinel 项目包（v2）".into());
    }
    let project = bundle.get("project").ok_or("项目包缺少 project")?;
    let project_name = project
        .get("name")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("项目名称为空")?;
    let mut connection = db::open(&state.db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let project_id: i64 = transaction
        .query_row(
            "SELECT id FROM projects WHERE lower(name)=lower(?1) LIMIT 1",
            [project_name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| {
            let _ = transaction.execute(
                "INSERT INTO projects(name,description) VALUES(?1,?2)",
                params![
                    project_name,
                    project
                        .get("description")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("")
                ],
            );
            transaction.last_insert_rowid()
        });
    let mut imported = 0i64;
    for scan in bundle
        .get("scans")
        .and_then(JsonValue::as_array)
        .ok_or("项目包缺少 scans")?
    {
        let id = scan
            .get("id")
            .and_then(JsonValue::as_str)
            .ok_or("扫描任务缺少 id")?;
        transaction
            .execute("DELETE FROM sentinel_deleted_scans WHERE scan_id=?1", [id])
            .map_err(|error| error.to_string())?;
        transaction.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,previous_scan_id,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,scan_type,task_name,source_path,skill_names,attempt_count,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,COALESCE(?18,datetime('now','localtime')),COALESCE(?19,datetime('now','localtime'))) ON CONFLICT(id) DO UPDATE SET project_id=excluded.project_id,project_name=excluded.project_name,status=excluded.status,current_checkpoint=excluded.current_checkpoint,task_path=excluded.task_path,previous_scan_id=excluded.previous_scan_id,llm_requests=excluded.llm_requests,input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cached_tokens=excluded.cached_tokens,total_tokens=excluded.total_tokens,scan_type=excluded.scan_type,task_name=excluded.task_name,source_path=excluded.source_path,skill_names=excluded.skill_names,attempt_count=excluded.attempt_count,updated_at=excluded.updated_at",params![id,project_id,project_name,scan.get("status").and_then(JsonValue::as_str).unwrap_or("imported"),scan.get("currentCheckpoint").and_then(JsonValue::as_str).unwrap_or(""),scan.get("taskPath").and_then(JsonValue::as_str).unwrap_or(""),scan.get("previousScanId").and_then(JsonValue::as_str).unwrap_or(""),scan.get("llmRequests").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("inputTokens").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("outputTokens").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("cachedTokens").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("totalTokens").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("scanType").and_then(JsonValue::as_str).unwrap_or("web"),scan.get("taskName").and_then(JsonValue::as_str).unwrap_or(""),scan.get("sourcePath").and_then(JsonValue::as_str).unwrap_or(""),scan.get("skillNames").and_then(JsonValue::as_str).unwrap_or(""),scan.get("attemptCount").and_then(JsonValue::as_i64).unwrap_or(0),scan.get("createdAt").and_then(JsonValue::as_str),scan.get("updatedAt").and_then(JsonValue::as_str)]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    for item in bundle
        .get("targets")
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let scan_id = item.get("scanId").and_then(JsonValue::as_str).unwrap_or("");
        let url = item.get("url").and_then(JsonValue::as_str).unwrap_or("");
        if scan_id.is_empty() || url.is_empty() {
            continue;
        }
        transaction.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status,value_score,scan_mode,routing_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(project_id,scan_id,url) DO UPDATE SET company=excluded.company,status=excluded.status,value_score=excluded.value_score,scan_mode=excluded.scan_mode,routing_reason=excluded.routing_reason,updated_at=datetime('now','localtime')",params![project_id,scan_id,item.get("company").and_then(JsonValue::as_str).unwrap_or(""),url,item.get("status").and_then(JsonValue::as_str).unwrap_or("queued"),item.get("valueScore").and_then(JsonValue::as_i64).unwrap_or(0),item.get("scanMode").and_then(JsonValue::as_str).unwrap_or(""),item.get("routingReason").and_then(JsonValue::as_str).unwrap_or("")]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    for item in bundle
        .get("checkpoints")
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        transaction.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,?2,?3,?4) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",params![item.get("scanId").and_then(JsonValue::as_str).unwrap_or(""),item.get("url").and_then(JsonValue::as_str).unwrap_or("*"),item.get("stage").and_then(JsonValue::as_str).unwrap_or("imported"),item.get("rawJson").and_then(JsonValue::as_str).unwrap_or("{}")]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    for item in bundle
        .get("findings")
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        transaction.execute("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",params![item.get("scanId").and_then(JsonValue::as_str).unwrap_or(""),item.get("targetUrl").and_then(JsonValue::as_str).unwrap_or(""),item.get("stage").and_then(JsonValue::as_str).unwrap_or(""),item.get("kind").and_then(JsonValue::as_str).unwrap_or(""),item.get("recordKey").and_then(JsonValue::as_str).unwrap_or(""),item.get("title").and_then(JsonValue::as_str).unwrap_or(""),item.get("severity").and_then(JsonValue::as_str).unwrap_or(""),item.get("recordJson").and_then(JsonValue::as_str).unwrap_or("{}")]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    for item in bundle
        .get("validations")
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        transaction.execute("INSERT INTO sentinel_validations(scan_id,url,finding_key,finding_kind,verdict,severity,note,evidence) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(scan_id,url,finding_key) DO UPDATE SET finding_kind=excluded.finding_kind,verdict=excluded.verdict,severity=excluded.severity,note=excluded.note,evidence=excluded.evidence,updated_at=datetime('now','localtime')",params![item.get("scanId").and_then(JsonValue::as_str).unwrap_or(""),item.get("url").and_then(JsonValue::as_str).unwrap_or(""),item.get("findingKey").and_then(JsonValue::as_str).unwrap_or("url-summary"),item.get("findingKind").and_then(JsonValue::as_str).unwrap_or(""),item.get("verdict").and_then(JsonValue::as_str).unwrap_or("pending"),item.get("severity").and_then(JsonValue::as_str).unwrap_or(""),item.get("note").and_then(JsonValue::as_str).unwrap_or(""),item.get("evidence").and_then(JsonValue::as_str).unwrap_or("")]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    for item in bundle
        .get("opportunities")
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let scan_id = item.get("scanId").and_then(JsonValue::as_str).unwrap_or("");
        let key = item
            .get("opportunityKey")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        if scan_id.is_empty() || key.is_empty() {
            continue;
        }
        transaction.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,why_json,evidence_json,recommended_action_json,source,record_json,first_seen,last_seen) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,COALESCE(?15,datetime('now','localtime')),COALESCE(?16,datetime('now','localtime'))) ON CONFLICT(scan_id,target_url,opportunity_key) DO UPDATE SET project_id=excluded.project_id,category=excluded.category,title=excluded.title,score=excluded.score,status=excluded.status,confidence=excluded.confidence,why_json=excluded.why_json,evidence_json=excluded.evidence_json,recommended_action_json=excluded.recommended_action_json,source=excluded.source,record_json=excluded.record_json,last_seen=excluded.last_seen",params![project_id,scan_id,item.get("targetUrl").and_then(JsonValue::as_str).unwrap_or(""),key,item.get("category").and_then(JsonValue::as_str).unwrap_or(""),item.get("title").and_then(JsonValue::as_str).unwrap_or(""),item.get("score").and_then(JsonValue::as_i64).unwrap_or(0),item.get("status").and_then(JsonValue::as_str).unwrap_or("queued"),item.get("confidence").and_then(JsonValue::as_str).unwrap_or(""),item.get("whyJson").and_then(JsonValue::as_str).unwrap_or("[]"),item.get("evidenceJson").and_then(JsonValue::as_str).unwrap_or("[]"),item.get("recommendedActionJson").and_then(JsonValue::as_str).unwrap_or("{}"),item.get("source").and_then(JsonValue::as_str).unwrap_or(""),item.get("recordJson").and_then(JsonValue::as_str).unwrap_or("{}"),item.get("firstSeen").and_then(JsonValue::as_str),item.get("lastSeen").and_then(JsonValue::as_str)]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    for item in bundle
        .get("fuseZone")
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let url = item
            .get("url")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        if url.is_empty() {
            continue;
        }
        transaction.execute("INSERT INTO sentinel_fuse_zone(project_id,asset_id,company,url,normalized_url,source_scan_id,reason,verdict,note,evidence,archived) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(project_id,normalized_url) DO UPDATE SET asset_id=COALESCE(excluded.asset_id,sentinel_fuse_zone.asset_id),company=excluded.company,url=excluded.url,source_scan_id=excluded.source_scan_id,reason=excluded.reason,verdict=excluded.verdict,note=excluded.note,evidence=excluded.evidence,archived=excluded.archived,updated_at=datetime('now','localtime')",params![project_id,item.get("assetId").and_then(JsonValue::as_i64),item.get("company").and_then(JsonValue::as_str).unwrap_or(""),url,normalized_fuse_url(url),item.get("sourceScanId").and_then(JsonValue::as_str).unwrap_or(""),item.get("reason").and_then(JsonValue::as_str).unwrap_or(""),item.get("verdict").and_then(JsonValue::as_str).unwrap_or("pending"),item.get("note").and_then(JsonValue::as_str).unwrap_or(""),item.get("evidence").and_then(JsonValue::as_str).unwrap_or(""),item.get("archived").and_then(JsonValue::as_bool).unwrap_or(false) as i64]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(imported)
}
