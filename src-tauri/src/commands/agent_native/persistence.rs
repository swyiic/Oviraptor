/// Native usage accumulates on the scan row; `sync_sentinel_attempt` derives the
/// per-attempt delta from the value captured at attempt start.
/// §5.2: usage is part of the commit boundary. If it cannot be written, the loop
/// must not start another model round it would be unable to bill or explain.
fn persist_agent_usage(db_path: &Path, scan_id: &str, usage: &AgentTokenUsage) -> Result<(), String> {
    let connection = db::open(db_path).map_err(|error| error.to_string())?;
    connection.execute(
        "UPDATE sentinel_scans SET llm_requests=llm_requests+?1,input_tokens=input_tokens+?2,output_tokens=output_tokens+?3,cached_tokens=cached_tokens+?4,total_tokens=total_tokens+?5,updated_at=datetime('now','localtime') WHERE id=?6",
        params![
            usage.model_requests,
            usage.input_tokens,
            usage.output_tokens,
            usage.cached_input_tokens,
            usage.total_tokens,
            scan_id
        ],
    )
    .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE sentinel_scans SET llm_requests=MAX(0,llm_requests) WHERE id=?1",
            [scan_id],
        )
        .map_err(|error| error.to_string())?;
    sync_sentinel_attempt(&connection, scan_id);
    Ok(())
}

fn agent_record_key(prefix: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("{prefix}:{}", digest.chars().take(24).collect::<String>())
}

/// Request/response evidence remains a candidate until the independent reviewer
/// confirms the frozen bundle; raw bytes stay under the target directory.
fn persist_agent_evidence(
    context: &AgentRunContext,
    summary: &JsonValue,
    tool: &str,
) -> Result<(), String> {
    let endpoint = format!(
        "{} {}",
        value_first(summary, &["method"]).to_ascii_uppercase(),
        value_first(summary, &["url", "endpoint"])
    );
    let status = summary.get("status").and_then(JsonValue::as_i64).unwrap_or(0);
    let key = agent_record_key(tool, &format!("{endpoint}|{status}"));
    stage_agent_finding(
        context,
        AGENT_EVIDENCE_STAGE,
        "evidence",
        &key,
        &agent_text_truncated(&endpoint, 200),
        "info",
        &serde_json::json!({
            "source": "native-agent",
            "tool": tool,
            "identity": value_first(summary, &["identity"]),
            "endpoint": endpoint,
            "status": status,
            "observedAt": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            "detail": summary,
        }),
    )
}

/// §8: only `confirmed` reaches the vulnerability contract, and it carries the
/// full evidence shape the UI and the appsec ledger already expect.
fn persist_agent_vulnerability(
    context: &AgentRunContext,
    arguments: &JsonValue,
    hypothesis_key: &str,
    bound: &ConfirmedPair,
) -> Result<(), String> {
    let severity = {
        let value = value_first(arguments, &["severity"]).to_ascii_lowercase();
        if value.is_empty() { "medium".to_string() } else { value }
    };
    let title = {
        let value = value_first(arguments, &["title"]);
        if value.is_empty() {
            format!("原生 Agent 确认：{hypothesis_key}")
        } else {
            value
        }
    };
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let record = serde_json::json!({
        "source": "native-agent",
        "title": title,
        "severity": severity,
        "cwe": value_first(arguments, &["cwe"]),
        "cvss": value_first(arguments, &["cvss"]),
        "confidence": arguments.get("confidence").and_then(JsonValue::as_f64).unwrap_or(0.9),
        "confidenceRationale": value_first(arguments, &["confidenceRationale"]),
        // §10.1: the finding is its two executed requests plus the difference
        // record between them, not a paragraph the model typed.
        "controlRequestId": bound.control_request_id,
        "testRequestId": bound.test_request_id,
        "responseDifferenceArtifactId": bound.difference_artifact_id,
        "controlRequest": bound.control_summary,
        "pocRequest": bound.test_summary,
        "testRequest": bound.test_summary,
        "responseDifference": format!("响应差异记录 {}", bound.difference_artifact_id),
        "impact": value_first(arguments, &["impact"]),
        "reproductionSteps": value_first(arguments, &["reproductionSteps"]),
        "counterEvidence": value_first(arguments, &["counterEvidenceCheck"]),
        "severityChangeConditions": value_first(arguments, &["severityChangeConditions"]),
        "recommendation": value_first(arguments, &["remediation"]),
        "fixVerification": value_first(arguments, &["fixVerification"]),
        "verdict": "confirmed",
        "url": context.target_url,
        "updatedAt": now,
        "updateHistory": [{
            "at": now,
            "status": "confirmed",
            "note": "原生 Agent 工具循环确认",
            "backend": "native",
        }],
    });
    stage_agent_finding(
        context,
        AGENT_VULNERABILITY_STAGE,
        "vulnerability",
        &agent_record_key("native", hypothesis_key),
        &agent_text_truncated(&title, 200),
        &severity,
        &record,
    )
}

// The coverage ledger is a finding row so the existing evidence UI can render
// it without a new table, and its write is part of the terminal boundary (§5.2).
// A deterministic observation finding from executed work is read-only analysis
// evidence, not a model-authored confirmation (§8/§10): it still cites the
// request id the chain really sent.
#[allow(clippy::too_many_arguments)]
fn persist_agent_observation(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    dedupe_key: &str,
    title: &str,
    severity: &str,
    cwe: &str,
    observation_kind: &str,
    request_id: &str,
    url: &str,
    evidence: &str,
    impact: &str,
    remediation: &str,
    family: &str,
) -> Result<bool, String> {
    if !runtime.observation_finding_keys.insert(dedupe_key.to_string()) {
        return Ok(false);
    }
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let record = serde_json::json!({
        "source": "native-agent",
        "title": title,
        "severity": severity,
        "cwe": cwe,
        "confidence": 0.9,
        "confidenceRationale": "由本轮已执行请求/静态特征直接观测，不依赖模型叙述",
        "controlRequestId": request_id,
        "testRequestId": request_id,
        "responseDifferenceArtifactId": "",
        "controlRequest": format!("观测请求 {request_id}"),
        "pocRequest": evidence,
        "testRequest": evidence,
        "responseDifference": evidence,
        "impact": impact,
        "reproductionSteps": format!("对 {url} 的已记录请求 {request_id} 复现观测：{evidence}"),
        "recommendation": remediation,
        "verdict": "observed",
        "observationKind": observation_kind,
        "url": context.target_url,
        "updatedAt": now,
        "updateHistory": [{
            "at": now,
            "status": "observed",
            "note": "原生 Agent 观测自动落库",
            "backend": "native",
        }],
    });
    stage_agent_finding(
        context,
        AGENT_VULNERABILITY_STAGE,
        "vulnerability",
        &agent_record_key("observe", dedupe_key),
        &agent_text_truncated(title, 200),
        severity,
        &record,
    )?;
    runtime.confirmed_findings += 1;
    if !family.is_empty() {
        runtime.note_coverage(
            family,
            "request",
            "",
            request_id,
            "covered",
            "observation_recorded",
        );
        runtime.families.insert(family.to_string());
    }
    Ok(true)
}

fn persist_agent_observation_finding(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    request_id: &str,
    url: &str,
    header: &str,
    value: &str,
) -> Result<bool, String> {
    persist_agent_observation(
        context,
        runtime,
        &format!("hdr:{header}:{value}"),
        &format!("响应头暴露技术栈：{header}"),
        "info",
        "CWE-200",
        "version_disclosure",
        request_id,
        url,
        &format!("{header}: {value}"),
        "暴露运行时与框架版本，便于攻击者检索已知漏洞与针对性利用",
        &format!("移除或模糊化生产环境响应头 {header}，避免返回具体版本号"),
        "information_disclosure",
    )
}

fn persist_agent_coverage(
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    state: &NativeAgentState,
    summary: &str,
) -> Result<(), String> {
    let covered: Vec<&str> = state.covered_families.iter().map(String::as_str).collect();
    let uncovered = agent_required_families()
        .into_iter()
        .filter(|family| !covered.contains(family))
        .map(|family| {
            runtime
                .uncovered_families
                .iter()
                .find(|row| value_first(row, &["family"]) == family)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({
                    "family": family,
                    "label": agent_coverage_family_label(family),
                    "status": "not_covered",
                    "reason": summary,
                }))
        })
        .collect::<Vec<_>>();
    stage_agent_finding(
        context,
        AGENT_COVERAGE_STAGE,
        "coverage",
        "coverage",
        &format!("原生 Agent 覆盖账本（{} 个族已覆盖）", covered.len()),
        "info",
        &serde_json::json!({
            "source": "native-agent",
            "coveredFamilies": covered,
            "uncoveredFamilies": uncovered,
            "coverageEvidence": runtime.coverage,
            "confirmedFindings": runtime.confirmed_findings,
            "exclusions": runtime.exclusions,
            "manualDeepDiveSuggestions": runtime.manual_suggestions,
            "modelRequests": state.token_usage.model_requests,
            "totalTokens": state.token_usage.total_tokens,
            "targetRequests": runtime.target_requests,
            "discoveryRounds": runtime.discovery_rounds,
            "turns": state.turns,
            "summary": summary,
        }),
    )
}
