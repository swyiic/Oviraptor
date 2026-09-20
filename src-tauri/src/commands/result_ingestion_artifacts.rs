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
        "completed" | "complete" | "finished" | "succeeded" | "success" | "done" => "completed",
        "failed" | "crashed" => "failed",
        // Native Strix does not distinguish a user pause from budget and
        // lifecycle interruption. Oviraptor's own paused state is preserved
        // separately; every other interrupted artifact is a resumable partial.
        "stopped" | "interrupted" | "cancelled" | "canceled" => "partial",
        _ => "scanning",
    }
}

fn sarif_result_is_security_finding(result: &JsonValue, rule: Option<&JsonValue>) -> bool {
    let rule_id = value_first(result, &["ruleId", "rule_id"]);
    if rule_id.starts_with("strix-coverage/") {
        return false;
    }
    let rule_is_coverage = rule
        .and_then(|value| value.get("properties"))
        .and_then(|value| value.get("tags"))
        .and_then(JsonValue::as_array)
        .is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case("coverage"))
            })
        });
    let result_is_coverage = result
        .pointer("/properties/strix/coverage_outcome")
        .is_some();
    if rule_is_coverage || result_is_coverage {
        return false;
    }
    let kind = value_first(result, &["kind"]).to_ascii_lowercase();
    if !kind.is_empty() {
        return kind == "fail";
    }
    !value_first(result, &["level"]).eq_ignore_ascii_case("none")
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
            // Strix 1.6 publishes coverage ledger entries in SARIF as pass,
            // notApplicable or open results. They describe what was checked;
            // importing them as vulnerabilities would create false positives.
            if !sarif_result_is_security_finding(result, rule) {
                continue;
            }
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

fn strix_coverage(dir: &Path) -> Option<JsonValue> {
    let bytes = fs::read(dir.join(STRIX_COVERAGE_ARTIFACT)).ok()?;
    let value = serde_json::from_slice::<JsonValue>(&bytes).ok()?;
    let object = value.as_object()?;
    let schema_supported = object
        .get("schema_version")
        .and_then(JsonValue::as_u64)
        .is_some_and(|version| version == 1);
    let has_ledger = object.get("entries").is_some_and(JsonValue::is_array)
        && object.get("gaps").is_some_and(JsonValue::is_array);
    (schema_supported && has_ledger).then_some(value)
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
            let coverage_stage = format!("strix_coverage:{safe_id}");
            let coverage_artifact_exists = dir.join(STRIX_COVERAGE_ARTIFACT).is_file();
            let coverage_checkpoint_exists: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sentinel_checkpoints WHERE scan_id=?1 AND stage=?2",
                    params![scan_id, coverage_stage],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if previous_signature.as_deref() == Some(artifact_signature.as_str())
                && checkpoint_exists > 0
                && (!coverage_artifact_exists || coverage_checkpoint_exists > 0)
            {
                continue;
            }
            let vulnerabilities = strix_vulnerabilities(&dir);
            let coverage = strix_coverage(&dir);
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
                && existing.as_ref().is_some_and(|item| {
                    item.4.as_str() == "web" || matches!(item.3.as_str(), "paused" | "pausing")
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
            connection
                .execute(
                    "DELETE FROM sentinel_checkpoints WHERE scan_id=?1 AND stage=?2",
                    params![scan_id, coverage_stage],
                )
                .map_err(|error| error.to_string())?;
            if let Some(coverage) = coverage.as_ref() {
                connection.execute(
                    "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,'*',?2,?3) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
                    params![scan_id, format!("strix_coverage:{safe_id}"), coverage.to_string()],
                ).map_err(|error| error.to_string())?;
            }
            let (requests, input_tokens, output_tokens, cached_tokens, total_tokens) =
                aggregate_strix_usage(connection, &scan_id)?;
            connection.execute(
                "UPDATE sentinel_scans SET llm_requests=MAX(llm_requests,?1),input_tokens=MAX(input_tokens,?2),output_tokens=MAX(output_tokens,?3),cached_tokens=MAX(cached_tokens,?4),total_tokens=MAX(total_tokens,?5) WHERE id=?6",
                params![requests,input_tokens,output_tokens,cached_tokens,total_tokens,scan_id],
            ).map_err(|error| error.to_string())?;
            let default_target = targets.first().cloned().unwrap_or_else(|| "*".into());
            let coverage_record_key = format!("coverage:{safe_id}");
            connection
                .execute(
                    "DELETE FROM sentinel_findings WHERE scan_id=?1 AND stage='strix-coverage' AND record_key=?2",
                    params![scan_id, coverage_record_key],
                )
                .map_err(|error| error.to_string())?;
            if let Some(coverage) = coverage.as_ref() {
                let mut normalized = coverage.as_object().cloned().unwrap_or_default();
                normalized.insert("source".into(), JsonValue::String("strix".into()));
                normalized.insert(
                    "title".into(),
                    JsonValue::String("Strix 覆盖与完整性".into()),
                );
                insert_finding(
                    connection,
                    &scan_id,
                    &default_target,
                    "strix-coverage",
                    "coverage_summary",
                    &format!("coverage:{safe_id}"),
                    "Strix 覆盖与完整性",
                    "",
                    &JsonValue::Object(normalized),
                )?;
            }
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
                "SELECT id FROM sentinel_scans WHERE scan_type='web' AND status IN ('partial','failed') AND current_checkpoint NOT LIKE '%；报错细节：%' AND EXISTS (SELECT 1 FROM sentinel_targets st WHERE st.scan_id=sentinel_scans.id AND st.status IN ('paused','partial','completed_with_gaps','protected_stop','limited','failed') AND trim(st.routing_reason)<>'') LIMIT 100",
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
