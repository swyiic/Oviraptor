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
            .execute(
                "DELETE FROM sentinel_checkpoints WHERE scan_id=?1",
                [scan_id],
            )
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
                "DELETE FROM sentinel_findings WHERE scan_id=?1 AND stage IN ('strix','strix-coverage') AND (target_url='*' OR target_url IN (SELECT url FROM sentinel_targets WHERE scan_id=?1 AND last_attempt_number=?2))",
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
            "DELETE FROM sentinel_checkpoints WHERE scan_id=?1 AND (stage IN ('strix_run','strix_events','strix_coverage','learning_outcome') OR stage LIKE 'strix_run:%' OR stage LIKE 'strix_events:%' OR stage LIKE 'strix_coverage:%')",
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
    // Keep the upstream snake_case contract intact while exposing stable
    // camelCase aliases to the UI. 1.6.x adds reviewer-facing evidence that
    // must not disappear merely because an older task was normalized first.
    for (source, alias) in [
        ("counterevidence", "counterEvidence"),
        ("confidence_rationale", "confidenceRationale"),
        ("severity_change_conditions", "severityChangeConditions"),
        ("fix_verification", "fixVerification"),
        ("update_history", "updateHistory"),
        ("updated_at", "updatedAt"),
    ] {
        if let Some(field) = value.get(source) {
            normalized.entry(alias).or_insert_with(|| field.clone());
        }
    }
    JsonValue::Object(normalized)
}
