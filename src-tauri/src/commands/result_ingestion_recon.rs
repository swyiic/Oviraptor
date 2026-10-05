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
        "SELECT id,title,summary,patterns_json,skill_id,skill_instructions FROM agent_knowledge_entries WHERE project_id IS NULL OR project_id=?1 ORDER BY updated_at DESC LIMIT 500",
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
                if kind == "js_file" {
                    if let Some((ver, script_url)) = agent_outdated_jquery_from_url(&title) {
                        let vuln_title = format!(
                            "jQuery {ver} 存在已知 XSS 风险（已停止维护/含公开漏洞）"
                        );
                        let vuln = serde_json::json!({
                            "source": "frontend-recon",
                            "title": vuln_title,
                            "severity": "low",
                            "cwe": "CWE-79",
                            "confidence": 0.9,
                            "verdict": "observed",
                            "observationKind": "outdated_jquery",
                            "scriptUrl": script_url,
                            "impact": "旧版 jQuery 含多个已公开 XSS 相关缺陷，可在页面上下文被利用",
                            "recommendation": "升级到 jQuery 3.5+ 或迁移至已维护的替代库，并移除页面中的旧副本",
                            "url": url,
                        });
                        insert_finding(
                            connection,
                            scan_id,
                            &url,
                            "native-agent",
                            "vulnerability",
                            &format!("observe:jquery:{ver}"),
                            &vuln_title,
                            "low",
                            &vuln,
                        )?;
                        count += 1;
                    }
                }
            }
        }
                if let Some(diagnostics) = target.get("runtimeDiagnostics").and_then(JsonValue::as_array) {
            if !diagnostics.is_empty() {
                let failed = diagnostics
                    .iter()
                    .filter(|item| value_first(item, &["captureStatus"]) != "complete")
                    .count();
                let title = if failed > 0 {
                    format!("运行时诊断 · {failed}/{} 个身份采集未完成", diagnostics.len())
                } else {
                    format!("运行时诊断 · {} 个身份采集完整", diagnostics.len())
                };
                let record = serde_json::json!({
                    "runtimeDiagnostics": diagnostics,
                    "collectionOutcome": target.get("collectionOutcome").cloned().unwrap_or(JsonValue::Null),
                    "analysisSummary": target.get("analysisSummary").cloned().unwrap_or(JsonValue::Null),
                });
                insert_finding(
                    connection,
                    scan_id,
                    &url,
                    "frontend-recon",
                    "runtime_diagnostics",
                    "runtime-diagnostics",
                    &title,
                    if failed > 0 { "medium" } else { "info" },
                    &record,
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
                let (eligible_for_agent, readiness_reason) = opportunity_agent_readiness(&record);
                if let Some(object) = record.as_object_mut() {
                    object.insert(
                        "verificationMode".into(),
                        JsonValue::String(
                            if eligible_for_agent {
                                "ai_auto"
                            } else {
                                "needs_evidence"
                            }
                            .into(),
                        ),
                    );
                    object.insert(
                        "humanReviewStage".into(),
                        JsonValue::String(
                            if eligible_for_agent {
                                "final_verdict_only"
                            } else {
                                "evidence_collection"
                            }
                            .into(),
                        ),
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
                let status = if eligible_for_agent {
                    "ready"
                } else {
                    "queued"
                };
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
