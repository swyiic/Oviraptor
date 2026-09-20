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
        "SELECT url,routing_reason FROM sentinel_targets WHERE scan_id=?1 AND status IN ('paused','partial','completed_with_gaps','protected_stop','limited','failed','persistence_failure') AND trim(routing_reason)<>'' ORDER BY CASE status WHEN 'persistence_failure' THEN 0 WHEN 'failed' THEN 1 WHEN 'paused' THEN 2 WHEN 'partial' THEN 2 ELSE 3 END,updated_at DESC LIMIT 5",
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
            "UPDATE sentinel_targets SET status='recon_only',scan_mode='skip',routing_reason=CASE WHEN routing_reason LIKE '%历史修复：调查门禁关闭%' THEN routing_reason ELSE routing_reason || '；历史修复：调查门禁关闭，未启动自动验证' END,updated_at=datetime('now','localtime') WHERE scan_id=?1 AND status IN ('queued','frontend_recon','routed','scanning','partial','limited') AND EXISTS (SELECT 1 FROM investigation_metrics im WHERE im.scan_id=sentinel_targets.scan_id AND im.target_url=sentinel_targets.url AND COALESCE(im.token_worthy,0)=0 AND COALESCE(json_extract(im.decision_json,'$.eligibleForModel'),0)=0 AND COALESCE(json_extract(im.decision_json,'$.standardInvestigationAllowed'),0)=0 AND COALESCE(json_extract(im.decision_json,'$.baselineInvestigationAllowed'),0)=0)",
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
        let counts: (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) = connection
            .query_row(
                "SELECT COALESCE(SUM(CASE WHEN status='completed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status IN ('partial','paused') THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='recon_only' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='manual_review' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status IN ('limited','protected_stop') THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status NOT IN ('completed','completed_with_gaps','partial','paused','recon_only','manual_review','limited','protected_stop','failed','resume_incompatible','persistence_failure') THEN 1 ELSE 0 END),0),COUNT(*),COALESCE(SUM(CASE WHEN status='resume_incompatible' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='persistence_failure' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='completed_with_gaps' THEN 1 ELSE 0 END),0) FROM sentinel_targets WHERE scan_id=?1",
                [scan_id],
                |row| Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                )),
            )
            .unwrap_or((0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0));
        let (
            completed,
            partial,
            recon_only,
            manual_review,
            limited,
            failed,
            deferred,
            total,
            resume_incompatible,
            persistence_failure,
            completed_with_gaps,
        ) = counts;
        let resume_incompatible = resume_incompatible > 0;
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
            // A target whose continuation was rejected is an error the user must
            // see, but never as a generic model failure (§11). A target that stopped
            // because its local record could not be written is its own error (§5.2).
            let has_errors = partial + limited + failed + deferred + persistence_failure > 0
                || resume_incompatible;
            let persistence_note = if persistence_failure > 0 {
                format!(
                    "，本地记录失败 {persistence_failure}；本地记录失败，已停止以避免重复消耗"
                )
            } else {
                String::new()
            };
            // §10: a target that closed inside its bounds with declared holes is
            // finished work, not a clean completion, and it is not "待补充验证" either.
            let gap_note = if completed_with_gaps > 0 {
                format!("，带覆盖缺口完成 {completed_with_gaps}")
            } else {
                String::new()
            };
            let derived_status = if matches!(status.as_str(), "paused" | "cancelled") {
                status.as_str()
            } else if has_errors {
                // A limited/retryable target is retained as a partial pipeline
                // result even when it is the only target. It is not equivalent
                // to a hard execution failure and remains eligible for resume.
                // A persistence failure is not retryable, so a run that only
                // stopped on one is reported as a failure, not as a partial.
                if completed + partial + recon_only + manual_review + limited > 0 {
                    "partial"
                } else {
                    "failed"
                }
            } else {
                "completed"
            };
            let repaired = if has_errors && failed == 0 && limited == 0 && deferred == 0 && persistence_failure == 0 {
                format!(
                    "任务累计状态：自动验证 {completed}{gap_note}，待补充验证 {partial}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}；无执行失败，待补充与缺口项未计入自动验证完成{failure_suffix}"
                )
            } else if has_errors {
                format!(
                    "任务累计状态：自动验证 {completed}{gap_note}，待补充验证 {partial}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}，熔断 {limited}，执行失败 {failed}，未处理 {deferred}{persistence_note}{failure_suffix}"
                )
            } else if deferred == 0 {
                format!(
                    "任务累计状态：全部目标已收口；自动验证 {completed}{gap_note}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}"
                )
            } else {
                format!(
                    "任务累计状态：自动验证 {completed}{gap_note}，待补充验证 {partial}，确定性侦察收口 {recon_only}，复杂前端自动收口 {manual_review}，熔断 {limited}，执行失败 {failed}，未处理 {deferred}"
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
