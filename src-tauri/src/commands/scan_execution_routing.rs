fn update_batch_targets(db_path: &Path, scan_id: &str, urls: &[String], status: &str) {
    if let Ok(connection) = db::open(db_path) {
        for url in urls {
            let _ = connection.execute(
                "UPDATE sentinel_targets SET status=?1,updated_at=datetime('now','localtime') WHERE scan_id=?2 AND url=?3",
                params![status, scan_id, url],
            );
        }
    }
}

fn update_target_route(db_path: &Path, scan_id: &str, route: &FrontendRoute, status: &str) {
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "UPDATE sentinel_targets SET status=?1,value_score=?2,scan_mode=?3,routing_reason=?4,updated_at=datetime('now','localtime') WHERE scan_id=?5 AND url=?6",
            params![status, route.score, route.mode, route.reason_text(), scan_id, route.url],
        );
        let _ = connection.execute(
            "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,?2,'adaptive_routing',?3) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
            params![scan_id, route.url, route.as_json().to_string()],
        );
    }
}

/// Apply the persisted investigation decision after deterministic recon. Deep
/// validation still requires an evidence-backed hypothesis, while a separate
/// standard gate accepts concrete browser-observed API contracts for one
/// bounded read-only investigation. Static strings never open either gate.
fn investigation_model_gate_open(token_worthy: bool, decision: &JsonValue) -> bool {
    token_worthy
        && decision
            .pointer("/eligibleForModel")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        && decision
            .pointer("/readyHypotheses")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0)
            > 0
}

fn investigation_standard_gate_open(decision: &JsonValue) -> bool {
    decision
        .pointer("/standardInvestigationAllowed")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
        && (decision
            .pointer("/verifiedRuntimeApiCount")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0)
            > 0
            || decision
                .pointer("/sourceMappedReadOnlyApiCount")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0)
                > 0)
}

fn investigation_baseline_gate_open(decision: &JsonValue) -> bool {
    decision
        .pointer("/baselineInvestigationAllowed")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn apply_investigation_route_gate(
    db_path: &Path,
    scan_id: &str,
    route: &mut FrontendRoute,
) {
    let Some((gain, token_worthy, stop_reason, decision, requested_mode)) = db::open(db_path)
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT information_gain,token_worthy,stop_reason,decision_json,COALESCE((SELECT json_extract(policy_json,'$.webModeCeiling') FROM sentinel_scan_contexts WHERE scan_id=?1),'standard') FROM investigation_metrics WHERE scan_id=?1 AND target_url=?2",
                    params![scan_id, route.url],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? != 0, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)),
                )
                .optional()
                .ok()
                .flatten()
        })
    else {
        // Missing investigation metrics must fail closed. The route may still
        // be saved as deterministic recon, but it must never start Strix.
        route.mode = "skip".into();
        route.reasons.push(
            "模型门禁数据缺失：仅保存确定性前端侦察结果，未启动自动验证".into(),
        );
        return;
    };
    let decision = json(decision);
    let gate_open = investigation_model_gate_open(token_worthy, &decision);
    let standard_gate_open = investigation_standard_gate_open(&decision);
    let baseline_gate_open = investigation_baseline_gate_open(&decision);
    let source_guided = decision
        .pointer("/sourceGuidedInvestigationAllowed")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let requested_mode = normalized_web_scan_mode(Some(&requested_mode));
    route.score = route.score.max(gain);
    route.reasons.push(format!(
        "本地调查图谱信息增益 {gain}/100；{}",
        if gate_open {
            "存在明确允许交给模型的证据假设"
        } else if source_guided {
            "存在源码映射还原的高置信度只读接口，允许有界目标调查"
        } else if standard_gate_open {
            "存在真实运行时接口，允许一次有界标准调查"
        } else if baseline_gate_open {
            "浏览器已确认可交互页面，允许渐进式基础覆盖调查"
        } else {
            "没有满足模型门禁的新证据"
        }
    ));
    if gate_open {
        route.mode = requested_mode.into();
        route.reasons.push(format!(
            "任务要求上限为 {requested_mode}；风险证据按该模式预算执行"
        ));
        return;
    }
    if standard_gate_open {
        route.mode = if source_guided && requested_mode == "deep" {
            "deep".into()
        } else if requested_mode == "quick" {
            "quick".into()
        } else {
            "standard".into()
        };
        route.reasons.push(if source_guided {
            "自动验证源码映射中还原的准确只读调用，并使用目标模式的定向发现预算；不会执行仅由字符串拼出的写接口".into()
        } else {
            "标准扫描自动执行已观察请求和有界响应差异验证；按固定预算结束后直接形成终态，不要求再次点击继续".into()
        });
        return;
    }
    if baseline_gate_open {
        route.mode = if requested_mode == "deep" {
            "deep".into()
        } else {
            "standard".into()
        };
        route.reasons.push(
            "首轮没有捕获业务 API；执行低成本基线族与限定轮次的业务词定向发现，只有新增端点或响应差异时才扩容".into(),
        );
        return;
    }
    route.mode = "skip".into();
    route.reasons.push(format!(
        "模型门禁已关闭：{stop_reason}；已保存前端状态、动作、请求和 API 证据，未启动自动验证"
    ));
}

fn normalized_fuse_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn add_target_to_fuse_zone(db_path: &Path, scan_id: &str, url: &str, reason: &str) {
    let Ok(connection) = db::open(db_path) else {
        return;
    };
    let target = connection
        .query_row(
            "SELECT project_id,asset_id,company,url FROM sentinel_targets WHERE scan_id=?1 AND url=?2 LIMIT 1",
            params![scan_id, url],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
        )
        .optional()
        .ok()
        .flatten();
    let Some((project_id, asset_id, company, target_url)) = target else {
        return;
    };
    let normalized = normalized_fuse_url(&target_url);
    let _ = connection.execute(
        "INSERT INTO sentinel_fuse_zone(project_id,asset_id,company,url,normalized_url,source_scan_id,reason) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(project_id,normalized_url) DO UPDATE SET asset_id=COALESCE(excluded.asset_id,sentinel_fuse_zone.asset_id),company=excluded.company,url=excluded.url,source_scan_id=excluded.source_scan_id,reason=excluded.reason,verdict='pending',note='',evidence='',archived=0,updated_at=datetime('now','localtime')",
        params![project_id, asset_id, company, target_url, normalized, scan_id, reason],
    );
}
