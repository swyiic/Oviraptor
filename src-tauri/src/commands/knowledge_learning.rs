fn knowledge_row(row: &Row<'_>) -> rusqlite::Result<StrixKnowledgeEntry> {
    Ok(StrixKnowledgeEntry {
        id: row.get(0)?,
        scan_id: row.get(1)?,
        project_id: row.get(2)?,
        title: row.get(3)?,
        summary: row.get(4)?,
        patterns: json(row.get(5)?),
        source_hash: row.get(6)?,
        skill_id: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

const KNOWLEDGE_COLUMNS: &str =
    "id,scan_id,project_id,title,summary,patterns_json,source_hash,skill_id,created_at,updated_at";

const LEARNING_CANDIDATE_COLUMNS: &str =
    "id,scan_id,project_id,scan_type,title,summary,candidate_json,status,target_skill_id,source_hash,created_at,reviewed_at,updated_at";

fn learning_candidate_row(row: &Row<'_>) -> rusqlite::Result<StrixLearningCandidate> {
    Ok(StrixLearningCandidate {
        id: row.get(0)?,
        scan_id: row.get(1)?,
        project_id: row.get(2)?,
        scan_type: row.get(3)?,
        title: row.get(4)?,
        summary: row.get(5)?,
        candidate: json(row.get(6)?),
        status: row.get(7)?,
        target_skill_id: row.get(8)?,
        source_hash: row.get(9)?,
        created_at: row.get(10)?,
        reviewed_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn learning_candidate_source_hash(
    trace: &StrixTraceSummary,
    findings: &[(String, String)],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(trace.scan_id.as_bytes());
    hasher.update(trace.instruction_hash.as_bytes());
    hasher.update(trace.scan_type.as_bytes());
    for tool in &trace.tools {
        hasher.update(tool.name.as_bytes());
        hasher.update(tool.calls.to_le_bytes());
        hasher.update(tool.results.to_le_bytes());
    }
    for (title, severity) in findings {
        hasher.update(title.as_bytes());
        hasher.update(severity.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn llm_content(response: &JsonValue) -> Option<String> {
    let content = response
        .pointer("/choices/0/message/content")
        .or_else(|| response.pointer("/output/0/content/0/text"));
    match content {
        Some(JsonValue::String(value)) => Some(value.clone()),
        Some(JsonValue::Array(values)) => Some(
            values
                .iter()
                .filter_map(|value| value.get("text").and_then(JsonValue::as_str))
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

fn parse_json_object(text: &str) -> Option<JsonValue> {
    let trimmed = text.trim().trim_matches('`').trim();
    serde_json::from_str(trimmed).ok().or_else(|| {
        let start = trimmed.find('{')?;
        let end = trimmed.rfind('}')?;
        serde_json::from_str(&trimmed[start..=end]).ok()
    })
}

fn call_learning_llm_request(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    include_response_format: bool,
) -> Result<JsonValue, String> {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [
            {"role":"system","content":"你是 AppSec 技能工程师。只输出严格 JSON，不要 Markdown，不要复述目标凭据、Cookie、Token、项目名或一次性 URL。将一次扫描中可复用的方法提炼成可审核的候选，明确证据、置信度、冗余步骤和 Skill 补丁。低置信度、一次性现象、未经复现的猜测不得升级为规则。"},
            {"role":"user","content":prompt}
        ],
        "temperature": 0.1,
        "max_tokens": 5000,
        "stream": false
    });
    if include_response_format {
        body["response_format"] = serde_json::json!({"type":"json_object"});
    }
    let request_body = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(request_body)
        .send()
        .map_err(|error| format!("学习提炼模型请求失败：{error}"))?;
    let status = response.status();
    let text = response
        .text()
        .map_err(|error| format!("学习提炼模型响应读取失败：{error}"))?;
    if !status.is_success() {
        return Err(format!(
            "学习提炼模型 HTTP {}：{}",
            status.as_u16(),
            text.chars().take(600).collect::<String>()
        ));
    }
    let response: JsonValue = serde_json::from_str(&text)
        .map_err(|error| format!("学习提炼模型响应不是 JSON：{error}"))?;
    let content = llm_content(&response).ok_or("学习提炼模型未返回 content")?;
    parse_json_object(&content).ok_or_else(|| "学习提炼模型 content 不是合法 JSON".into())
}

fn call_learning_llm(environment: &StrixRuntimeEnv, prompt: &str) -> Result<JsonValue, String> {
    let base = if environment.api_base.trim().is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        environment.api_base.trim_end_matches('/').to_string()
    };
    let endpoint = format!("{base}/chat/completions");
    let model = openai_chat_completion_model(&environment.llm);
    if model.is_empty() {
        return Err("当前 Strix 模型名为空，无法生成学习候选".into());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| error.to_string())?;
    match call_learning_llm_request(
        &client,
        &endpoint,
        &environment.api_key,
        &model,
        prompt,
        true,
    ) {
        Ok(value) => Ok(value),
        Err(first_error)
            if first_error.contains("HTTP 400")
                || first_error.contains("HTTP 401")
                || first_error.contains("HTTP 404")
                || first_error.contains("HTTP 422")
                || first_error.contains("content 不是合法 JSON")
                || first_error.contains("未返回 content") =>
        {
            call_learning_llm_request(
                &client,
                &endpoint,
                &environment.api_key,
                &model,
                prompt,
                false,
            )
            .map_err(|retry_error| {
                format!("{first_error}；去掉 response_format 后重试仍失败：{retry_error}")
            })
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone)]
struct LearningQualityGate {
    disposition: &'static str,
    score: i64,
    evidence_count: usize,
    reusable_signal: bool,
    generic_only: bool,
    duplicate_tool_calls: usize,
    repeated_results: usize,
    cve_classes: HashMap<String, usize>,
    reasons: Vec<String>,
}

enum LearningGenerationOutcome {
    Candidate(StrixLearningCandidate),
    Skipped(LearningQualityGate),
}

fn normalized_trace_fragment(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(1600)
        .collect()
}

fn finding_has_evidence(record_json: &str) -> bool {
    let value = json(record_json.to_string());
    fn has_signal(value: &JsonValue) -> bool {
        match value {
            JsonValue::Object(values) => values.iter().any(|(key, value)| {
                let key = key.to_ascii_lowercase().replace('_', "");
                let interesting = [
                    "evidence",
                    "proof",
                    "poc",
                    "reproduction",
                    "request",
                    "response",
                    "impact",
                    "file",
                    "line",
                    "dataflow",
                    "stack",
                    "payload",
                    "affectedendpoint",
                ]
                .iter()
                .any(|needle| key.contains(needle));
                (interesting
                    && match value {
                        JsonValue::String(text) => !text.trim().is_empty() && text != "{}",
                        JsonValue::Array(items) => !items.is_empty(),
                        JsonValue::Object(items) => !items.is_empty(),
                        JsonValue::Bool(value) => *value,
                        JsonValue::Number(_) => true,
                        JsonValue::Null => false,
                    })
                    || has_signal(value)
            }),
            JsonValue::Array(values) => values.iter().any(has_signal),
            _ => false,
        }
    }
    has_signal(&value)
}

fn classify_finding_signal(title: &str, kind: &str, record_json: &str) -> &'static str {
    let normalized = format!(
        "{} {} {}",
        title.to_ascii_lowercase(),
        kind.to_ascii_lowercase(),
        record_json.to_ascii_lowercase()
    );
    let has_evidence = finding_has_evidence(record_json);
    if normalized.contains("cve-") || normalized.contains("cwe-") {
        if has_evidence {
            "confirmed"
        } else if normalized.contains("version")
            || normalized.contains("dependency")
            || normalized.contains("package")
        {
            "dependency_signal"
        } else {
            "needs_verification"
        }
    } else if has_evidence {
        "confirmed"
    } else if normalized.contains("banner")
        || normalized.contains("fingerprint")
        || normalized.contains("version")
        || normalized.contains("discovered")
        || normalized.contains("endpoint")
        || normalized.contains("status")
    {
        "info"
    } else {
        "needs_verification"
    }
}

fn assess_learning_quality(
    trace: &StrixTraceSummary,
    events: &[StrixTraceEvent],
    findings: &[(String, String, String, String)],
) -> LearningQualityGate {
    let mut call_fragments = HashMap::<String, usize>::new();
    let mut result_fragments = HashMap::<String, usize>::new();
    for event in events {
        let fragment = normalized_trace_fragment(&event.detail);
        if fragment.is_empty() {
            continue;
        }
        if event.event_type == "function_call" {
            *call_fragments
                .entry(format!("{}:{}", event.name, fragment))
                .or_default() += 1;
        } else if event.event_type == "function_call_output" {
            *result_fragments.entry(fragment).or_default() += 1;
        }
    }
    let duplicate_tool_calls = call_fragments.values().filter(|count| **count > 1).sum();
    let repeated_results = result_fragments.values().filter(|count| **count > 1).sum();
    let evidence_count = findings
        .iter()
        .filter(|(_, _, _, record_json)| finding_has_evidence(record_json))
        .count();
    let mut cve_classes = HashMap::new();
    let mut confirmed_count = 0usize;
    let mut generic_count = 0usize;
    for (title, kind, _, record_json) in findings {
        let class = classify_finding_signal(title, kind, record_json);
        *cve_classes.entry(class.to_string()).or_default() += 1;
        if class == "confirmed" {
            confirmed_count += 1;
        }
        if class == "info" {
            generic_count += 1;
        }
    }
    let reusable_signal = confirmed_count > 0
        || (trace.reasoning_count > 0 && trace.tool_call_count >= 2 && evidence_count > 0)
        || events.iter().any(|event| {
            let detail = event.detail.to_ascii_lowercase();
            detail.contains("reproduction")
                || detail.contains("impact")
                || detail.contains("source location")
                || detail.contains("data flow")
        });
    let generic_only =
        !findings.is_empty() && evidence_count == 0 && generic_count == findings.len();
    let no_progress = duplicate_tool_calls >= 3 && repeated_results > 0;
    let mut score = trace_quality_score(trace, findings.len());
    if evidence_count > 0 {
        score += 15;
    }
    if reusable_signal {
        score += 10;
    }
    if no_progress {
        score -= 20;
    }
    if generic_only {
        score -= 25;
    }
    score = score.clamp(0, 100);
    let disposition = if generic_only {
        "no_learning_value"
    } else if cve_classes.contains_key("dependency_signal")
        || cve_classes.contains_key("needs_verification")
        || no_progress
    {
        "needs_verification"
    } else if !reusable_signal && evidence_count == 0 {
        "no_learning_value"
    } else {
        "reusable_candidate"
    };
    let mut reasons = Vec::new();
    if generic_only {
        reasons.push("当前发现只有指纹、版本、路径或状态类信息，没有可复现安全证据".into());
    }
    if evidence_count == 0 {
        reasons.push("没有发现请求/响应、代码位置、数据流、复现或影响证据".into());
    }
    if no_progress {
        reasons.push("检测到重复工具调用和重复结果，后续步骤没有带来新事实".into());
    }
    if cve_classes.contains_key("dependency_signal") {
        reasons
            .push("CVE 仅由版本或依赖匹配推断，必须验证真实可达组件、受影响路径和前置条件".into());
    }
    if reusable_signal {
        reasons.push("至少存在一条可跨目标复用的证据链或验证思路".into());
    }
    LearningQualityGate {
        disposition,
        score,
        evidence_count,
        reusable_signal,
        generic_only,
        duplicate_tool_calls,
        repeated_results,
        cve_classes,
        reasons,
    }
}

fn enforce_learning_quality(candidate: &mut JsonValue, gate: &LearningQualityGate) {
    if let Some(object) = candidate.as_object_mut() {
        object.insert(
            "qualityGate".into(),
            serde_json::json!({
                "disposition": gate.disposition,
                "score": gate.score,
                "evidenceCount": gate.evidence_count,
                "reusableSignal": gate.reusable_signal,
                "genericOnly": gate.generic_only,
                "duplicateToolCalls": gate.duplicate_tool_calls,
                "repeatedResults": gate.repeated_results,
                "findingClasses": gate.cve_classes,
                "reasons": gate.reasons,
            }),
        );
        if gate.disposition != "reusable_candidate" {
            object.insert(
                "summary".into(),
                JsonValue::String(format!(
                    "质量门禁：{}（{} 分）。该结果仅作待验证线索，不得直接沉淀为 Skill。",
                    gate.reasons.join("；"),
                    gate.score
                )),
            );
            object.insert("newIdeas".into(), JsonValue::Array(Vec::new()));
            object.insert(
                "skillPatch".into(),
                serde_json::json!({"addSections":[],"replaceSections":[],"removeSections":[],"keepSections":[],"instructions":""}),
            );
        }
    }
}

fn canonical_learning_text(value: &str) -> String {
    value
        .split_whitespace()
        .map(|part| {
            let trimmed = part.trim_matches(|character: char| {
                matches!(
                    character,
                    ',' | ';' | ':' | '(' | ')' | '[' | ']' | '"' | '\''
                )
            });
            if trimmed.contains("://") {
                "{url}".to_string()
            } else if trimmed.len() >= 16
                && trimmed
                    .chars()
                    .filter(|character| character.is_ascii_hexdigit() || *character == '-')
                    .count()
                    * 4
                    >= trimmed.len() * 3
            {
                "{id}".to_string()
            } else {
                trimmed.to_ascii_lowercase()
            }
        })
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(600)
        .collect()
}

fn canonical_candidate_item_key(value: &JsonValue) -> String {
    if let Some(text) = value.as_str() {
        return canonical_learning_text(text);
    }
    ["title", "name", "problem", "action", "step", "reason"]
        .iter()
        .filter_map(|key| value.get(key).and_then(JsonValue::as_str))
        .map(canonical_learning_text)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("|")
}

fn canonicalize_learning_candidate(
    candidate: &mut JsonValue,
    trace: &StrixTraceSummary,
    findings: &[(String, String, String, String)],
    environment: &StrixRuntimeEnv,
    prompt: &str,
) {
    let Some(object) = candidate.as_object_mut() else {
        return;
    };
    for key in [
        "newIdeas",
        "redundantSteps",
        "weakSteps",
        "externalKnowledgeRequests",
    ] {
        let mut seen = HashSet::new();
        let mut values = object
            .get(key)
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|value| {
                let marker = canonical_candidate_item_key(value);
                !marker.is_empty() && seen.insert(marker)
            })
            .take(40)
            .collect::<Vec<_>>();
        values.sort_by_key(canonical_candidate_item_key);
        object.insert(key.into(), JsonValue::Array(values));
    }
    let canonical_findings = findings
        .iter()
        .map(|(title, severity, kind, record)| {
            serde_json::json!({
                "key": canonical_learning_text(title),
                "severity": severity.to_ascii_lowercase(),
                "kind": kind.to_ascii_lowercase(),
                "classification": classify_finding_signal(title, kind, record),
                "hasEvidence": finding_has_evidence(record),
            })
        })
        .collect::<Vec<_>>();
    let canonical_tools = trace
        .tools
        .iter()
        .map(|tool| serde_json::json!({"name":tool.name.to_ascii_lowercase(),"calls":tool.calls,"results":tool.results}))
        .collect::<Vec<_>>();
    let mut prompt_hasher = Sha256::new();
    prompt_hasher.update(prompt.as_bytes());
    let prompt_hash = format!("{:x}", prompt_hasher.finalize());
    let facts = serde_json::json!({
        "scanType": trace.scan_type,
        "instructionHash": trace.instruction_hash,
        "findings": canonical_findings,
        "tools": canonical_tools,
        "evidenceTaskCount": 1,
    });
    let patch_titles = object
        .get("skillPatch")
        .and_then(|patch| patch.get("addSections"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .chain(
            object
                .get("skillPatch")
                .and_then(|patch| patch.get("replaceSections"))
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten(),
        )
        .map(canonical_candidate_item_key)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let mut canonical_hasher = Sha256::new();
    canonical_hasher.update(trace.scan_type.as_bytes());
    canonical_hasher.update(facts.to_string().as_bytes());
    canonical_hasher.update(patch_titles.join("|").as_bytes());
    object.insert("schemaVersion".into(), JsonValue::Number(2.into()));
    object.insert(
        "normalizerVersion".into(),
        JsonValue::String("learning-canonical-v2".into()),
    );
    object.insert(
        "canonicalKey".into(),
        JsonValue::String(format!("{:x}", canonical_hasher.finalize())),
    );
    object.insert("canonicalFacts".into(), facts);
    object.insert(
        "producer".into(),
        serde_json::json!({
            "model": openai_chat_completion_model(&environment.llm),
            "deployment": environment.deployment,
            "promptHash": prompt_hash,
            "instructionHash": trace.instruction_hash,
        }),
    );
    object.insert(
        "learningPolicy".into(),
        serde_json::json!({
            "factsAreDeterministic": true,
            "modelOutputIsProposal": true,
            "sameScanDifferentModelAddsSupport": false,
            "applyMode": "reviewed-canonical-markdown-patch",
        }),
    );
}

fn cached_external_knowledge_context(connection: &rusqlite::Connection) -> String {
    let mut statement = match connection.prepare(
        "SELECT title,patterns_json FROM strix_knowledge_entries WHERE patterns_json LIKE '%external_source%' ORDER BY updated_at DESC,id DESC LIMIT 6",
    ) {
        Ok(statement) => statement,
        Err(_) => return "- 当前没有已缓存的公开来源方法卡片。".into(),
    };
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|(title, patterns_json)| {
            let patterns = json(patterns_json);
            let score = patterns
                .get("qualityScore")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0);
            if score < 70 {
                return None;
            }
            let methods = patterns
                .get("methodCards")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .take(3)
                .filter_map(|card| card.get("method").and_then(JsonValue::as_str))
                .collect::<Vec<_>>();
            if methods.is_empty() {
                None
            } else {
                Some(format!("- {}（{}）：{}", title, score, methods.join("；")))
            }
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        "- 当前没有达到 70 分的已缓存公开来源方法卡片。".into()
    } else {
        rows.join("\n")
    }
}

fn fallback_learning_candidate(
    trace: &StrixTraceSummary,
    findings: &[(String, String)],
) -> JsonValue {
    let finding_titles = findings
        .iter()
        .map(|(title, _)| title)
        .cloned()
        .collect::<Vec<_>>();
    serde_json::json!({
        "summary": format!("扫描完成：{} 次工具调用、{} 个工具结果、{} 类安全问题。仅保留可复用的验证原则，等待人工审核。", trace.tool_call_count, trace.tool_result_count, findings.len()),
        "newIdeas": [{"title":"证据优先与停止条件","problem":"历史流程容易在无新增证据时继续调用工具","evidence":finding_titles,"confidence":0.55,"action":"review"}],
        "redundantSteps": [],
        "weakSteps": [],
        "externalKnowledgeRequests": [],
        "skillPatch": {"addSections": ["每个候选必须绑定新任务证据；无新增证据时停止并切换候选"], "replaceSections": [], "removeSections": [], "instructions": ""},
        "llmFallback": true
    })
}

#[tauri::command]
pub fn list_strix_knowledge(state: State<AppState>) -> Result<Vec<StrixKnowledgeEntry>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(&format!("SELECT {KNOWLEDGE_COLUMNS} FROM strix_knowledge_entries ORDER BY updated_at DESC,id DESC"))
        .map_err(|error| error.to_string())?;
    let entries = statement
        .query_map([], knowledge_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(entries)
}

fn generate_learning_candidate_with_environment(
    db_path: &Path,
    scan_id: &str,
    environment: &StrixRuntimeEnv,
) -> Result<LearningGenerationOutcome, String> {
    let connection = db::open(db_path)?;
    let (trace, events) = collect_strix_trace(&connection, scan_id, true, true)?;
    if !["completed", "partial", "failed"].contains(&trace.status.as_str()) {
        return Err("扫描尚未结束，暂不生成学习候选".into());
    }
    let mut statement = connection
        .prepare("SELECT DISTINCT title,severity,kind,record_json FROM sentinel_findings WHERE scan_id=?1 AND trim(title)<>'' ORDER BY severity,title LIMIT 40")
        .map_err(|error| error.to_string())?;
    let findings = statement
        .query_map([scan_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .flatten()
        .collect::<Vec<_>>();
    drop(statement);
    let finding_pairs = findings
        .iter()
        .map(|(title, severity, _, _)| (title.clone(), severity.clone()))
        .collect::<Vec<_>>();
    let source_hash = learning_candidate_source_hash(&trace, &finding_pairs);
    let existing = connection
        .query_row(
            &format!("SELECT {LEARNING_CANDIDATE_COLUMNS} FROM strix_learning_candidates WHERE scan_id=?1 AND source_hash=?2"),
            params![scan_id, source_hash],
            learning_candidate_row,
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(existing) =
        existing.filter(|candidate| matches!(candidate.status.as_str(), "accepted" | "applied"))
    {
        return Ok(LearningGenerationOutcome::Candidate(existing));
    }
    let event_digest = events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "function_call" | "function_call_output" | "message"
            )
        })
        .take(80)
        .map(|event| {
            format!(
                "{} {} {}",
                event.event_type,
                event.name,
                retained_trace_text(&event.detail)
                    .chars()
                    .take(800)
                    .collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let quality_gate = assess_learning_quality(&trace, &events, &findings);
    // Learning is a post-scan optimization, not part of scan success. If the
    // deterministic gate already knows there is no reusable evidence, do not
    // spend another model call asking an LLM to reach the same conclusion.
    if quality_gate.disposition != "reusable_candidate" {
        return Ok(LearningGenerationOutcome::Skipped(quality_gate));
    }
    let cached_knowledge = cached_external_knowledge_context(&connection);
    let findings_json = findings
        .iter()
        .map(|(title, severity, kind, record_json)| {
            serde_json::json!({
                "title":title,
                "severity":severity,
                "kind":kind,
                "classification":classify_finding_signal(title, kind, record_json),
                "hasEvidence":finding_has_evidence(record_json),
            })
        })
        .collect::<Vec<_>>();
    let prompt = format!(
        "请分析以下一次 {scan_type} 扫描，并输出 JSON：\n\n扫描概要：{trace_json}\n\n安全发现：{findings}\n\n调用摘要：{events}\n\n本地质量预检：{quality}\n\n已缓存的公开来源方法卡片（仅作思路索引，不等于本次证据）：\n{cached_knowledge}\n\n输出字段必须包含 summary、newIdeas（数组，每项含 title/problem/evidence/confidence/action）、redundantSteps、weakSteps、externalKnowledgeRequests、skillPatch、qualityGate。skillPatch 使用 addSections/replaceSections/removeSections/keepSections/reasoning；只有可跨目标复用、证据充分且不会引入误报的内容才放入 addSections。若只是普通探测、版本匹配、无复现 CVE 或没有新增证据，qualityGate.disposition 必须为 no_learning_value 或 needs_verification，并将 skillPatch 留空。CVE 必须区分 confirmed、needs_verification、dependency_signal、info、invalid_or_duplicate，禁止把 NVD/版本匹配直接写成已确认漏洞。",
        scan_type = trace.scan_type,
        trace_json = serde_json::to_string(&serde_json::json!({
            "taskName":trace.task_name,"project":trace.project_name,"status":trace.status,"model":trace.model,
            "runCount":trace.run_count,"agentCount":trace.agent_count,"messageCount":trace.message_count,
            "toolCalls":trace.tool_call_count,"toolResults":trace.tool_result_count,"tokens":trace.total_tokens,
            "tools":trace.tools.iter().map(|tool| serde_json::json!({"name":tool.name,"calls":tool.calls,"results":tool.results})).collect::<Vec<_>>()
        })).unwrap_or_default(),
        findings = serde_json::to_string(&findings_json).unwrap_or_default(),
        events = event_digest,
        cached_knowledge = cached_knowledge,
        quality = serde_json::json!({
            "disposition":quality_gate.disposition,
            "score":quality_gate.score,
            "evidenceCount":quality_gate.evidence_count,
            "reusableSignal":quality_gate.reusable_signal,
            "genericOnly":quality_gate.generic_only,
            "duplicateToolCalls":quality_gate.duplicate_tool_calls,
            "repeatedResults":quality_gate.repeated_results,
            "findingClasses":quality_gate.cve_classes,
            "reasons":quality_gate.reasons,
        }),
    );
    let mut candidate_json = call_learning_llm(environment, &prompt)
        .unwrap_or_else(|_| fallback_learning_candidate(&trace, &finding_pairs));
    enforce_learning_quality(&mut candidate_json, &quality_gate);
    canonicalize_learning_candidate(&mut candidate_json, &trace, &findings, environment, &prompt);
    let title = candidate_json
        .get("title")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if trace.task_name.trim().is_empty() {
                "扫描后的学习候选"
            } else {
                trace.task_name.as_str()
            }
        })
        .chars()
        .take(120)
        .collect::<String>();
    let summary = candidate_json
        .get("summary")
        .and_then(JsonValue::as_str)
        .unwrap_or("模型未提供摘要；请查看候选详情")
        .chars()
        .take(2000)
        .collect::<String>();
    let project_id = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .unwrap_or(None);
    connection
        .execute(
            "INSERT INTO strix_learning_candidates(scan_id,project_id,scan_type,title,summary,candidate_json,status,source_hash) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7) ON CONFLICT(scan_id,source_hash) DO UPDATE SET title=excluded.title,summary=excluded.summary,candidate_json=excluded.candidate_json,status='pending',reviewed_at='',updated_at=datetime('now','localtime') WHERE strix_learning_candidates.status IN ('pending','rejected')",
        params![scan_id, project_id, trace.scan_type, title, summary, candidate_json.to_string(), source_hash],
        )
        .map_err(|error| error.to_string())?;
    let candidate = connection
        .query_row(
            &format!("SELECT {LEARNING_CANDIDATE_COLUMNS} FROM strix_learning_candidates WHERE scan_id=?1 AND source_hash=?2"),
            params![scan_id, source_hash],
            learning_candidate_row,
        )
        .map_err(|error| error.to_string())?;
    Ok(LearningGenerationOutcome::Candidate(candidate))
}

fn scan_supports_automatic_learning(db_path: &Path, scan_id: &str) -> bool {
    db::open(db_path)
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT status IN ('completed','partial') FROM sentinel_scans WHERE id=?1",
                    [scan_id],
                    |row| row.get::<_, bool>(0),
                )
                .ok()
        })
        .unwrap_or(false)
}

fn schedule_learning_candidate(db_path: PathBuf, scan_id: String, environment: StrixRuntimeEnv) {
    thread::spawn(move || {
        let result = generate_learning_candidate_with_environment(&db_path, &scan_id, &environment);
        if let Ok(connection) = db::open(&db_path) {
            let outcome = match result {
                Ok(LearningGenerationOutcome::Candidate(candidate)) => {
                    let disposition = candidate
                        .candidate
                        .pointer("/qualityGate/disposition")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("unknown");
                    let idea_count = candidate
                        .candidate
                        .get("newIdeas")
                        .and_then(JsonValue::as_array)
                        .map(|items| items.len())
                        .unwrap_or(0);
                    serde_json::json!({
                        "status":"candidate_ready",
                        "disposition":disposition,
                        "candidateId":candidate.id,
                        "ideaCount":idea_count,
                        "summary":format!("已生成 {idea_count} 条通过质量预检的待审核学习候选"),
                    })
                }
                Ok(LearningGenerationOutcome::Skipped(gate)) => serde_json::json!({
                    "status":"skipped",
                    "disposition":gate.disposition,
                    "score":gate.score,
                    "evidenceCount":gate.evidence_count,
                    "reasons":gate.reasons,
                    "summary":"本次扫描没有形成可跨目标复用的证据链，已跳过学习模型调用；扫描结果不受影响",
                }),
                Err(error) => serde_json::json!({
                    "status":"error",
                    "summary":"扫描结果已保留，但后台学习分析未完成",
                    "error":error,
                }),
            };
            // Keep the terminal scan summary intact. Learning has its own
            // checkpoint and can never turn a completed scan into a perceived
            // failure or hide the original failure reason.
            let _ = connection.execute(
                "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,'*','learning_outcome',?2) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')",
                params![scan_id, outcome.to_string()],
            );
        }
    });
}

fn patch_section(value: &JsonValue) -> Option<(String, String)> {
    let (title, body) = if let Some(text) = value.as_str() {
        let mut lines = text.lines();
        let first = lines.next().unwrap_or("").trim();
        if first.starts_with('#') {
            (
                first.trim_start_matches('#').trim().to_string(),
                lines.collect::<Vec<_>>().join("\n").trim().to_string(),
            )
        } else {
            ("学习补丁".to_string(), format!("- {}", text.trim()))
        }
    } else if let Some(object) = value.as_object() {
        let title = object
            .get("title")
            .or_else(|| object.get("name"))
            .and_then(JsonValue::as_str)
            .unwrap_or("学习补丁")
            .trim()
            .to_string();
        let body = object
            .get("content")
            .or_else(|| object.get("instructions"))
            .or_else(|| object.get("body"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        (title, body)
    } else {
        return None;
    };
    if title.is_empty() || body.is_empty() {
        None
    } else {
        Some((title, body))
    }
}

fn normalize_section_title(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase()
}

fn parse_skill_sections(instructions: &str) -> (String, Vec<(String, String)>) {
    let mut preamble = Vec::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in instructions.lines() {
        let trimmed = line.trim();
        let is_heading = trimmed.starts_with("## ") || trimmed.starts_with("### ");
        if is_heading {
            if let Some((title, body)) = current.take() {
                sections.push((title, body.join("\n").trim().to_string()));
            }
            current = Some((
                trimmed.trim_start_matches('#').trim().to_string(),
                Vec::new(),
            ));
        } else if let Some((_, body)) = current.as_mut() {
            body.push(line.to_string());
        } else {
            preamble.push(line.to_string());
        }
    }
    if let Some((title, body)) = current {
        sections.push((title, body.join("\n").trim().to_string()));
    }
    (preamble.join("\n").trim().to_string(), sections)
}

fn apply_skill_patch(base: &str, patch: &JsonValue) -> String {
    let (preamble, mut sections) = parse_skill_sections(base);
    let mut add_without_heading = Vec::new();

    if let Some(items) = patch.get("replaceSections").and_then(JsonValue::as_array) {
        for item in items {
            if let Some((title, body)) = patch_section(item) {
                let key = normalize_section_title(&title);
                if let Some(existing) = sections
                    .iter_mut()
                    .find(|(old_title, _)| normalize_section_title(old_title) == key)
                {
                    existing.1 = body;
                } else {
                    sections.push((title, body));
                }
            }
        }
    }

    if let Some(items) = patch.get("removeSections").and_then(JsonValue::as_array) {
        let removals = items
            .iter()
            .filter_map(JsonValue::as_str)
            .map(normalize_section_title)
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        sections.retain(|(title, _)| !removals.contains(&normalize_section_title(title)));
    }

    if let Some(items) = patch.get("addSections").and_then(JsonValue::as_array) {
        for item in items {
            if let Some((title, body)) = patch_section(item) {
                if normalize_section_title(&title) == normalize_section_title("学习补丁")
                    && !item
                        .as_str()
                        .is_some_and(|value| value.trim_start().starts_with('#'))
                {
                    add_without_heading.push(body);
                    continue;
                }
                let key = normalize_section_title(&title);
                if let Some(existing) = sections
                    .iter_mut()
                    .find(|(old_title, _)| normalize_section_title(old_title) == key)
                {
                    if !existing.1.contains(&body) {
                        existing.1 = format!("{}\n{}", existing.1.trim_end(), body);
                    }
                } else {
                    sections.push((title, body));
                }
            }
        }
    }
    if let Some(instructions) = patch.get("instructions").and_then(JsonValue::as_str) {
        if !instructions.trim().is_empty() {
            add_without_heading.push(instructions.trim().to_string());
        }
    }
    if !add_without_heading.is_empty() {
        if let Some(existing) = sections.iter_mut().find(|(title, _)| {
            normalize_section_title(title) == normalize_section_title("学习补丁")
        }) {
            for body in add_without_heading {
                if !existing.1.contains(&body) {
                    existing.1 = format!("{}\n{}", existing.1.trim_end(), body);
                }
            }
        } else {
            sections.push(("学习补丁".to_string(), add_without_heading.join("\n")));
        }
    }

    let mut output = Vec::new();
    if !preamble.is_empty() {
        output.push(preamble);
    }
    output.extend(
        sections
            .into_iter()
            .filter(|(_, body)| !body.trim().is_empty())
            .map(|(title, body)| format!("## {title}\n{}", body.trim())),
    );
    output.join("\n\n").trim().to_string()
}

fn skill_patch_has_content(patch: &JsonValue) -> bool {
    patch
        .get("instructions")
        .and_then(JsonValue::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        || ["addSections", "replaceSections", "removeSections"]
            .iter()
            .any(|key| {
                patch
                    .get(key)
                    .and_then(JsonValue::as_array)
                    .is_some_and(|items| !items.is_empty())
            })
}

fn skill_compare_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn strix_learning_catalog(
    connection: &rusqlite::Connection,
    exclude_skill_id: Option<i64>,
) -> Result<String, String> {
    let mut sections = Vec::new();
    let mut skills = connection
        .prepare("SELECT id,name,description,instructions,builtin FROM strix_skills WHERE (?1 IS NULL OR id<>?1) ORDER BY builtin DESC,updated_at DESC,id DESC")
        .map_err(|error| error.to_string())?;
    let rows = skills
        .query_map([exclude_skill_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows.flatten() {
        sections.push(format!(
            "Skill #{} [{}] {}\n描述：{}\n指令：{}",
            row.0,
            if row.4 != 0 { "内置" } else { "自定义" },
            row.1,
            row.2.chars().take(700).collect::<String>(),
            row.3.chars().take(2600).collect::<String>()
        ));
    }
    let mut knowledge = connection
        .prepare("SELECT id,title,summary,patterns_json,skill_instructions FROM strix_knowledge_entries ORDER BY updated_at DESC,id DESC")
        .map_err(|error| error.to_string())?;
    let rows = knowledge
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows.flatten() {
        let patterns = json(row.3);
        let quality = patterns
            .get("qualityScore")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        if quality < 70 {
            continue;
        }
        let kind = patterns
            .get("knowledgeKind")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        let distinct_scans = patterns
            .pointer("/support/distinctScans")
            .and_then(JsonValue::as_i64)
            .unwrap_or_else(|| if kind == "aggregate" { 2 } else { 1 });
        if kind == "task_candidate" && distinct_scans < 2 {
            // A manually reviewed one-off can still be converted explicitly,
            // but it must not influence later model refinement automatically.
            continue;
        }
        sections.push(format!(
            "知识 #{} {}（质量 {}）\n摘要：{}\n方法：{}",
            row.0,
            row.1,
            quality,
            row.2.chars().take(700).collect::<String>(),
            row.4.chars().take(1800).collect::<String>()
        ));
    }
    let mut catalog = sections.join("\n\n");
    if catalog.chars().count() > 42_000 {
        catalog = catalog.chars().take(42_000).collect();
    }
    Ok(if catalog.is_empty() {
        "（暂无其它 Skill 或高质量知识）".into()
    } else {
        catalog
    })
}

fn refine_learning_patch_for_apply(
    environment: &StrixRuntimeEnv,
    candidate: &JsonValue,
    base_instructions: &str,
    catalog: &str,
) -> Result<JsonValue, String> {
    let prompt = format!(
        "请对一个已经通过人工审核的 AppSec 学习候选做最终 Skill 精炼。必须和其它已有 Skill、历史高质量知识进行全局去重。若候选只是重复内容，decision=merge_existing 并给出 targetSkillId；若只是补充，合并到最匹配的 Skill；只有确实形成独立可复用能力时才 decision=create_new；没有新增价值时 decision=discard。删除一次性目标信息、重复步骤和无证据猜测，保留目标 Skill 未涉及的稳定章节。只输出 JSON，结构为 {{\"decision\":\"merge_existing|create_new|discard\",\"targetSkillId\":null,\"reasoning\":\"\",\"skillPatch\":{{\"addSections\":[],\"replaceSections\":[],\"removeSections\":[],\"keepSections\":[],\"reasoning\":\"\"}}}}。replaceSections 项优先使用 {{\"title\":\"章节名\",\"content\":\"内容\"}}。\n\n已审核候选：{}\n\n目标 Skill 当前内容：{}\n\n已有 Skill 与知识目录：{}",
        serde_json::to_string(candidate).unwrap_or_default(),
        base_instructions.chars().take(16_000).collect::<String>(),
        catalog
    );
    let refined = call_learning_llm(environment, &prompt)?;
    let patch = refined
        .get("skillPatch")
        .cloned()
        .unwrap_or_else(|| refined.clone());
    let mut output = patch;
    if let Some(object) = output.as_object_mut() {
        if let Some(value) = refined.get("decision") {
            object.insert("decision".into(), value.clone());
        }
        if let Some(value) = refined.get("targetSkillId") {
            object.insert("targetSkillId".into(), value.clone());
        }
        if let Some(value) = refined.get("reasoning") {
            object.insert("globalReasoning".into(), value.clone());
        }
    }
    if refined.get("decision").and_then(JsonValue::as_str) == Some("discard") {
        return Err("全局去重后没有发现新增可复用价值".into());
    }
    if skill_patch_has_content(&output) {
        Ok(output)
    } else {
        Err("最终精炼没有返回可应用的 Skill 补丁".into())
    }
}

#[tauri::command]
pub fn list_strix_learning_candidates(
    state: State<AppState>,
    status: Option<String>,
) -> Result<Vec<StrixLearningCandidate>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(&format!("SELECT {LEARNING_CANDIDATE_COLUMNS} FROM strix_learning_candidates WHERE (?1 IS NULL OR status=?1) ORDER BY updated_at DESC,id DESC"))
        .map_err(|error| error.to_string())?;
    let result = statement
        .query_map([status.as_deref()], learning_candidate_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string());
    result
}

#[tauri::command]
pub fn generate_strix_learning_candidate(
    state: State<AppState>,
    scan_id: String,
) -> Result<StrixLearningCandidate, String> {
    let settings = sentinel_settings(&db::open(&state.db_path)?);
    let home = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .to_path_buf();
    let environment = strix_runtime_env(&settings, &home)?;
    match generate_learning_candidate_with_environment(&state.db_path, &scan_id, &environment)? {
        LearningGenerationOutcome::Candidate(candidate) => Ok(candidate),
        LearningGenerationOutcome::Skipped(gate) => Err(format!(
            "本次扫描没有可沉淀的学习候选（{}，{}/100）：{}。扫描结果本身不受影响，也没有消耗额外的学习模型调用",
            gate.disposition,
            gate.score,
            if gate.reasons.is_empty() {
                "没有形成可跨目标复用的证据链".to_string()
            } else {
                gate.reasons.join("；")
            }
        )),
    }
}

#[tauri::command]
pub fn review_strix_learning_candidate(
    state: State<AppState>,
    candidate_id: i64,
    decision: String,
    target_skill_id: Option<i64>,
) -> Result<StrixLearningCandidate, String> {
    let decision = decision.trim().to_ascii_lowercase();
    if !["accepted", "rejected", "pending"].contains(&decision.as_str()) {
        return Err("候选审核状态必须是 accepted、rejected 或 pending".into());
    }
    let connection = db::open(&state.db_path)?;
    let changed = connection
        .execute("UPDATE strix_learning_candidates SET status=?1,target_skill_id=COALESCE(?2,target_skill_id),reviewed_at=CASE WHEN ?1='pending' THEN '' ELSE datetime('now','localtime') END,updated_at=datetime('now','localtime') WHERE id=?3", params![decision,target_skill_id,candidate_id])
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("学习候选不存在".into());
    }
    connection
        .query_row(
            &format!(
                "SELECT {LEARNING_CANDIDATE_COLUMNS} FROM strix_learning_candidates WHERE id=?1"
            ),
            [candidate_id],
            learning_candidate_row,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_strix_learning_candidate(
    state: State<AppState>,
    candidate_id: i64,
) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let deleted = connection
        .execute(
            "DELETE FROM strix_learning_candidates WHERE id=?1",
            [candidate_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("学习候选不存在".into());
    }
    Ok(())
}

#[tauri::command]
pub fn apply_strix_learning_candidate(
    state: State<AppState>,
    candidate_id: i64,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let (status, target_skill_id, candidate_json, scan_id): (String, Option<i64>, String, String) = connection
        .query_row("SELECT status,target_skill_id,candidate_json,scan_id FROM strix_learning_candidates WHERE id=?1", [candidate_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))
        .map_err(|_| "学习候选不存在".to_string())?;
    if status != "accepted" {
        return Err("请先审核接受该候选，再沉淀为 Skill".into());
    }
    let mut candidate = json(candidate_json);
    let quality_disposition = candidate
        .pointer("/qualityGate/disposition")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    if quality_disposition != "reusable_candidate" {
        return Err(format!(
            "该候选未通过质量门禁（{quality_disposition}），请先补充复现/影响证据后再沉淀"
        ));
    }
    let original_patch = candidate.get("skillPatch").cloned().unwrap_or_default();
    let mut skill_id = target_skill_id;
    let mut name = String::new();
    let description = candidate
        .get("summary")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();
    let mut base_instructions = String::new();
    if let Some(id) = skill_id {
        let (builtin, old_name, old_instructions): (i64, String, String) = connection
            .query_row(
                "SELECT builtin,name,instructions FROM strix_skills WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "目标 Skill 不存在".to_string())?;
        if builtin != 0 {
            skill_id = None;
            name = format!("{} · 增强版", old_name);
            base_instructions = old_instructions;
        } else {
            name = old_name;
            base_instructions = old_instructions;
        }
    }
    // Applying an accepted candidate is deterministic. A second model call used
    // to rewrite the already-reviewed patch here, so changing providers could
    // silently produce a different Skill and spend more tokens. Explicit
    // "refine with latest knowledge" remains available as a separate action.
    let patch = original_patch;
    if !skill_patch_has_content(&patch) {
        return Err("候选没有可应用的规范化 Markdown 补丁".into());
    }
    let refinement_status = "reviewed_canonical_patch";
    if skill_id.is_none() {
        if let Some(existing_id) = patch.get("targetSkillId").and_then(JsonValue::as_i64) {
            if let Ok((builtin, old_name, old_instructions)) = connection.query_row(
                "SELECT builtin,name,instructions FROM strix_skills WHERE id=?1",
                [existing_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            ) {
                if builtin == 0 {
                    skill_id = Some(existing_id);
                    name = old_name;
                    base_instructions = old_instructions;
                }
            }
        }
    }
    if let Some(object) = candidate.as_object_mut() {
        object.insert("skillPatch".into(), patch.clone());
        object.insert(
            "applyRefinement".into(),
            JsonValue::String(refinement_status.into()),
        );
    }
    let instructions = apply_skill_patch(&base_instructions, &patch);
    if instructions.is_empty() {
        return Err("候选没有可沉淀的 Skill 补丁，请重新生成或补充候选".into());
    }
    let normalized = skill_compare_text(&instructions);
    if skill_id.is_none() {
        let duplicate = connection
            .prepare("SELECT id,instructions FROM strix_skills")
            .ok()
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })
                    .ok()
                    .and_then(|rows| {
                        rows.flatten()
                            .find(|(_, existing)| skill_compare_text(existing) == normalized)
                    })
            });
        if let Some((existing_id, _)) = duplicate {
            skill_id = Some(existing_id);
        }
    }
    if name.is_empty() {
        name = format!(
            "扫描学习 · {}",
            candidate
                .get("title")
                .and_then(JsonValue::as_str)
                .unwrap_or(&scan_id)
        );
    }
    let name = name.chars().take(80).collect::<String>();
    if skill_id.is_none() {
        connection.execute("INSERT INTO strix_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,1)", params![name,description,instructions]).map_err(|error| error.to_string())?;
        skill_id = Some(connection.last_insert_rowid());
    } else {
        connection.execute("UPDATE strix_skills SET instructions=?1,updated_at=datetime('now','localtime') WHERE id=?2 AND builtin=0", params![instructions,skill_id]).map_err(|error| error.to_string())?;
    }
    let skill_id = skill_id.ok_or("无法创建 Skill")?;
    connection.execute("UPDATE strix_learning_candidates SET status='applied',target_skill_id=?1,candidate_json=?2,reviewed_at=COALESCE(NULLIF(reviewed_at,''),datetime('now','localtime')),updated_at=datetime('now','localtime') WHERE id=?3", params![skill_id,candidate.to_string(),candidate_id]).map_err(|error| error.to_string())?;
    Ok(skill_id)
}

fn trace_quality_score(trace: &StrixTraceSummary, finding_count: usize) -> i64 {
    let mut score = 0;
    if trace.run_count > 0 {
        score += 25;
    }
    if trace.agent_count > 0 && trace.message_count >= 3 {
        score += 20;
    }
    if trace.tool_call_count >= 2 && trace.tool_result_count * 2 >= trace.tool_call_count.max(1) {
        score += 25;
    }
    if trace.reasoning_count > 0 {
        score += 10;
    }
    if finding_count > 0 {
        score += 20;
    }
    score
}

#[tauri::command]
pub fn delete_strix_knowledge(state: State<AppState>, knowledge_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let deleted = connection
        .execute(
            "DELETE FROM strix_knowledge_entries WHERE id=?1",
            [knowledge_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("知识条目不存在".into());
    }
    Ok(())
}

#[tauri::command]
pub fn analyze_strix_trace(
    state: State<AppState>,
    scan_id: String,
) -> Result<StrixKnowledgeEntry, String> {
    let connection = db::open(&state.db_path)?;
    let (trace, _) = collect_strix_trace(&connection, &scan_id, false, true)?;
    let project_id = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .unwrap_or(None);
    let mut statement = connection
        .prepare("SELECT DISTINCT title,severity FROM sentinel_findings WHERE scan_id=?1 AND (kind LIKE '%vulnerab%' OR kind='risk') AND trim(title)<>'' ORDER BY severity,title LIMIT 20")
        .map_err(|error| error.to_string())?;
    let findings = statement
        .query_map([&scan_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .flatten()
        .collect::<Vec<_>>();
    drop(statement);
    let top_tools = trace
        .tools
        .iter()
        .take(12)
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    let finding_names = findings
        .iter()
        .map(|item| item.0.clone())
        .collect::<Vec<_>>();
    let mut canonical_hasher = Sha256::new();
    canonical_hasher.update(b"task-knowledge-v2");
    canonical_hasher.update(trace.scan_type.as_bytes());
    for tool in &top_tools {
        canonical_hasher.update(tool.to_ascii_lowercase().as_bytes());
    }
    for finding in &finding_names {
        canonical_hasher.update(canonical_learning_text(finding).as_bytes());
    }
    let canonical_key = format!("{:x}", canonical_hasher.finalize());
    let quality_score = trace_quality_score(&trace, findings.len());
    if quality_score < 60 {
        return Err(format!(
            "该任务轨迹质量仅 {quality_score}/100：至少需要完整运行产物、Agent 消息和工具调用/结果闭环；未写入知识库"
        ));
    }
    let title = if trace.task_name.trim().is_empty() {
        format!("{} · {} 轨迹知识", trace.project_name, trace.scan_type)
    } else {
        format!("{} · 轨迹知识", trace.task_name)
    };
    let summary = format!(
        "候选知识质量 {}/100：本地分析 {} 个 Strix 运行、{} 个 Agent、{} 条消息和 {} 次工具调用；识别 {} 类已入库安全问题。该知识不包含目标凭据或原始工具参数。",
        quality_score, trace.run_count, trace.agent_count, trace.message_count, trace.tool_call_count, findings.len()
    );
    let patterns = serde_json::json!({
        "schemaVersion": 2,
        "normalizerVersion": "learning-canonical-v2",
        "knowledgeKind": "task_candidate",
        "canonicalKey": canonical_key,
        "qualityScore": quality_score,
        "scanType": trace.scan_type,
        "model": trace.model,
        "producer": {"model":trace.model,"instructionHash":trace.instruction_hash},
        "support": {"distinctScans":1,"distinctModels":1,"eligibleForAutomaticMerge":false},
        "facts": {"tools":top_tools,"findingClasses":finding_names},
        "tools": top_tools,
        "findingClasses": finding_names,
        "messageCount": trace.message_count,
        "reasoningCount": trace.reasoning_count,
        "toolCalls": trace.tool_call_count,
        "toolResults": trace.tool_result_count,
        "llmRequests": trace.llm_requests,
        "hookedRequests": trace.hooked_request_count,
        "requestUsageEntries": trace.usage_entry_count,
        "usageAgents": trace.usage_agent_count,
        "tokenUsageEstimated": trace.token_usage_estimated,
        "instructionHash": trace.instruction_hash,
        "learningPolicy": {"factsAreDeterministic":true,"modelTextIsProposal":true,"sameScanDifferentModelAddsSupport":false},
    });
    let finding_lines = if findings.is_empty() {
        "- 从资产证据和数据流开始，仅保留可复现且有安全影响的问题。".to_string()
    } else {
        findings
            .iter()
            .take(12)
            .map(|(name, severity)| {
                format!("- 验证 {name}（历史严重度：{severity}），不要仅凭指纹或路径推断。")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tool_lines = if trace.tools.is_empty() {
        "- 先读取确定性证据，再选择最小必要验证工具。".to_string()
    } else {
        trace
            .tools
            .iter()
            .take(10)
            .map(|tool| {
                format!(
                    "- `{}`：历史调用 {} 次；仅在有新证据时继续。",
                    tool.name, tool.calls
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let skill_instructions = format!(
        "## Objective\n复用已验证的分析路径，提高同类任务的证据质量和停止判断。\n\n## Proven tool workflow\n{tool_lines}\n\n## Vulnerability focus\n{finding_lines}\n\n## Guardrails\n- 不复制历史目标、Cookie、Token、请求头或原始敏感参数。\n- 每个结论必须绑定新任务中的 URL、代码位置或请求响应证据。\n- 两次验证没有新增证据时切换候选；不要把侦察信息升级为漏洞。\n- 保留 Strix 原生 CVSS、CWE、PoC 和修复建议结构。"
    );
    let mut source_hasher = Sha256::new();
    source_hasher.update(b"task-knowledge-source-v2");
    source_hasher.update(canonical_key.as_bytes());
    source_hasher.update(trace.instruction_hash.as_bytes());
    let source_hash = format!("{:x}", source_hasher.finalize());
    connection.execute(
        "INSERT INTO strix_knowledge_entries(scan_id,project_id,title,summary,patterns_json,skill_instructions,source_hash) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(scan_id) DO UPDATE SET project_id=excluded.project_id,title=excluded.title,summary=excluded.summary,patterns_json=excluded.patterns_json,skill_instructions=excluded.skill_instructions,source_hash=excluded.source_hash,updated_at=datetime('now','localtime')",
        params![scan_id,project_id,title,summary,patterns.to_string(),skill_instructions,source_hash],
    ).map_err(|error| error.to_string())?;
    connection
        .query_row(
            &format!("SELECT {KNOWLEDGE_COLUMNS} FROM strix_knowledge_entries WHERE scan_id=?1"),
            [&scan_id],
            knowledge_row,
        )
        .map_err(|error| error.to_string())
}

fn recurring_workflow_signals(events: &[StrixTraceEvent]) -> HashSet<String> {
    const NOISE_TOOLS: &[&str] = &[
        "create_todo",
        "update_todo",
        "create_note",
        "write_stdin",
        "finish_scan",
        "wait",
        "view_image",
        "create_agent",
        "stop_agent",
        "wait_for_message",
        "view_agent_graph",
        "web_search",
        "mark_todo_done",
        "delete_todo",
    ];
    const COMMAND_TOOLS: &[&str] = &[
        "curl",
        "httpx",
        "nuclei",
        "semgrep",
        "codeql",
        "ffuf",
        "sqlmap",
        "nikto",
        "nmap",
        "subfinder",
        "katana",
        "playwright",
        "python",
        "node",
    ];
    let mut signals = HashSet::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == "function_call")
    {
        if event.name == "exec_command" {
            let lower = event.detail.to_ascii_lowercase();
            for tool in COMMAND_TOOLS {
                if lower.contains(tool) {
                    signals.insert(format!("exec:{tool}"));
                }
            }
        } else if !event.name.is_empty() && !NOISE_TOOLS.contains(&event.name.as_str()) {
            signals.insert(event.name.clone());
        }
    }
    signals
}

#[tauri::command]
pub fn aggregate_strix_knowledge(
    state: State<AppState>,
    scan_type: String,
) -> Result<StrixKnowledgeEntry, String> {
    let scan_type = scan_type.trim().to_ascii_lowercase();
    if !["web", "code", "greybox", "cicd"].contains(&scan_type.as_str()) {
        return Err("不支持的任务类型".into());
    }
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare("SELECT id FROM sentinel_scans WHERE scan_type=?1 AND task_path<>'' ORDER BY created_at DESC LIMIT 80")
        .map_err(|error| error.to_string())?;
    let scan_ids = statement
        .query_map([&scan_type], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .flatten()
        .collect::<Vec<_>>();
    drop(statement);

    let mut qualified_ids = Vec::new();
    let mut excluded = 0i64;
    let mut workflow_support: HashMap<String, i64> = HashMap::new();
    let mut finding_support: HashMap<String, (String, String, i64)> = HashMap::new();
    let mut source_models = HashSet::new();
    let mut source_hasher = Sha256::new();
    for scan_id in scan_ids {
        let Ok((trace, events)) = collect_strix_trace(&connection, &scan_id, true, true) else {
            excluded += 1;
            continue;
        };
        let mut finding_statement = connection
            .prepare("SELECT DISTINCT title,severity FROM sentinel_findings WHERE scan_id=?1 AND (kind LIKE '%vulnerab%' OR kind='risk') AND trim(title)<>'' ORDER BY severity,title LIMIT 50")
            .map_err(|error| error.to_string())?;
        let findings = finding_statement
            .query_map([&scan_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .flatten()
            .collect::<Vec<_>>();
        drop(finding_statement);
        if trace_quality_score(&trace, findings.len()) < 60 {
            excluded += 1;
            continue;
        }
        qualified_ids.push(scan_id.clone());
        if !trace.model.trim().is_empty() {
            source_models.insert(trace.model.clone());
        }
        source_hasher.update(scan_id.as_bytes());
        source_hasher.update(trace.instruction_hash.as_bytes());
        for signal in recurring_workflow_signals(&events) {
            *workflow_support.entry(signal).or_default() += 1;
        }
        let mut scan_findings = HashSet::new();
        for (title, severity) in findings {
            let normalized = title.trim().to_ascii_lowercase();
            if normalized.is_empty() || !scan_findings.insert(normalized.clone()) {
                continue;
            }
            let entry = finding_support
                .entry(normalized)
                .or_insert((title, severity, 0));
            entry.2 += 1;
        }
    }
    let qualified = qualified_ids.len() as i64;
    if qualified < 2 {
        return Err(format!(
            "同类型轨迹中只有 {qualified} 个通过质量门槛；至少需要 2 个包含 Agent、工具调用和结果闭环的任务"
        ));
    }
    let mut workflows = workflow_support
        .into_iter()
        .filter(|(_, support)| *support >= 2 && *support * 2 >= qualified)
        .collect::<Vec<_>>();
    workflows.sort_by_key(|(_, support)| std::cmp::Reverse(*support));
    let mut findings = finding_support
        .into_values()
        .filter(|(_, _, support)| *support >= 2 && *support * 2 >= qualified)
        .collect::<Vec<_>>();
    findings.sort_by_key(|(_, _, support)| std::cmp::Reverse(*support));
    if workflows.is_empty() && findings.is_empty() {
        return Err(
            "轨迹数量足够，但没有形成跨任务重复出现的工具路径或漏洞类别；未生成噪音知识".into(),
        );
    }

    let confidence = (45
        + qualified.min(5) * 5
        + workflows.len().min(6) as i64 * 2
        + findings.len().min(4) as i64 * 4)
        .min(92);
    let scan_label = match scan_type.as_str() {
        "web" => "Web URL",
        "code" => "代码审计",
        "greybox" => "灰盒联测",
        "cicd" => "CI/CD",
        _ => &scan_type,
    };
    let workflow_json = workflows
        .iter()
        .map(|(name, support)| serde_json::json!({"name":name,"support":support,"total":qualified}))
        .collect::<Vec<_>>();
    let finding_json = findings
        .iter()
        .map(|(title, severity, support)| serde_json::json!({"title":title,"severity":severity,"support":support,"total":qualified}))
        .collect::<Vec<_>>();
    let tool_names = workflows
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let finding_names = findings
        .iter()
        .map(|(title, _, _)| title.clone())
        .collect::<Vec<_>>();
    let mut source_models = source_models.into_iter().collect::<Vec<_>>();
    source_models.sort();
    let mut canonical_hasher = Sha256::new();
    canonical_hasher.update(b"aggregate-knowledge-v2");
    canonical_hasher.update(scan_type.as_bytes());
    for name in &tool_names {
        canonical_hasher.update(name.to_ascii_lowercase().as_bytes());
    }
    for name in &finding_names {
        canonical_hasher.update(canonical_learning_text(name).as_bytes());
    }
    let canonical_key = format!("{:x}", canonical_hasher.finalize());
    let patterns = serde_json::json!({
        "schemaVersion": 2,
        "normalizerVersion": "learning-canonical-v2",
        "knowledgeKind": "aggregate",
        "canonicalKey": canonical_key,
        "scanType": scan_type,
        "qualityScore": confidence,
        "sourceScans": qualified,
        "excludedScans": excluded,
        "sourceScanIds": qualified_ids,
        "sourceModels": source_models,
        "support": {"distinctScans":qualified,"distinctModels":source_models.len(),"eligibleForAutomaticMerge":true},
        "tools": tool_names,
        "findingClasses": finding_names,
        "recurringWorkflow": workflow_json,
        "recurringFindings": finding_json,
        "qualityGate": "run + agent messages + >=2 tool calls + >=50% tool result closure; pattern support >=2 and >=50%",
        "learningPolicy": {"sameScanDifferentModelAddsSupport":false,"factsAreDeterministic":true,"modelWordingExcludedFromCanonicalKey":true},
    });
    source_hasher.update(patterns.to_string().as_bytes());
    let source_hash = format!("{:x}", source_hasher.finalize());
    let title = format!("{scan_label} · 多任务聚合知识");
    let summary = format!(
        "聚合 {qualified} 个高质量同类任务，排除 {excluded} 个不完整或低信号任务；保留 {} 条重复工作流和 {} 类重复漏洞，可信度 {confidence}/100。单次目标、凭据、原始命令参数和偶发工具噪音均未进入知识。",
        workflows.len(), findings.len()
    );
    let workflow_lines = if workflows.is_empty() {
        "- 没有达到跨任务支持门槛的工具路径；从新任务的确定性证据开始。".to_string()
    } else {
        workflows
            .iter()
            .map(|(name, support)| format!("- `{name}`：在 {support}/{qualified} 个高质量任务中出现；仅在同类证据成立时复用。"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let finding_lines = if findings.is_empty() {
        "- 不预设漏洞类型；只接受新任务中可复现的安全影响。".to_string()
    } else {
        findings
            .iter()
            .map(|(title, severity, support)| format!("- {title}（历史 {severity}，支持 {support}/{qualified}）：必须重新验证输入、影响与边界。"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let skill_instructions = format!(
        "## Aggregated scope\n来自 {qualified} 个通过质量门槛的 {scan_label} 任务；可信度 {confidence}/100。只复用跨任务重复模式，不复用目标数据。\n\n## Recurring workflow\n{workflow_lines}\n\n## Recurring vulnerability focus\n{finding_lines}\n\n## Guardrails\n- 新任务必须重新建立 URL、参数、代码位置、请求响应或数据流证据。\n- 不携带历史域名、IP、Cookie、Token、请求头、凭据或原始命令。\n- 单次偶发现象、纯侦察输出、todo/备注/等待操作不得升级为漏洞。\n- 两次验证没有新增证据时停止当前分支，并记录停止原因。"
    );
    let aggregate_id = format!("aggregate:{scan_type}");
    connection.execute(
        "INSERT INTO strix_knowledge_entries(scan_id,title,summary,patterns_json,skill_instructions,source_hash) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(scan_id) DO UPDATE SET title=excluded.title,summary=excluded.summary,patterns_json=excluded.patterns_json,skill_instructions=excluded.skill_instructions,source_hash=excluded.source_hash,updated_at=datetime('now','localtime')",
        params![aggregate_id,title,summary,patterns.to_string(),skill_instructions,source_hash],
    ).map_err(|error| error.to_string())?;
    connection
        .query_row(
            &format!("SELECT {KNOWLEDGE_COLUMNS} FROM strix_knowledge_entries WHERE scan_id=?1"),
            [&aggregate_id],
            knowledge_row,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn convert_strix_knowledge_to_skill(
    state: State<AppState>,
    knowledge_id: i64,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let (title, summary, instructions, patterns_json, linked_skill): (String, String, String, String, Option<i64>) = connection
        .query_row(
            "SELECT title,summary,skill_instructions,patterns_json,skill_id FROM strix_knowledge_entries WHERE id=?1",
            [knowledge_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|_| "知识条目不存在".to_string())?;
    if let Some(skill_id) = linked_skill {
        return Ok(skill_id);
    }
    let patterns = json(patterns_json);
    let quality_score = patterns
        .get("qualityScore")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let knowledge_kind = patterns
        .get("knowledgeKind")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    if knowledge_kind.is_empty() {
        return Err(
            "这是旧版未评分知识，请先从轨迹重新生成候选或执行同类聚合，再转换为 Skill".into(),
        );
    }
    if quality_score < 70 {
        return Err(format!(
            "知识质量 {quality_score}/100，低于转换 Skill 所需的 70 分；请补充证据或使用多任务聚合"
        ));
    }

    // A scored knowledge entry is already a deterministic, target-neutral
    // method card. Conversion must not vary with the model provider currently
    // configured, nor spend another refinement request.
    let refined = serde_json::json!({
        "addSections": [{"title": "沉淀知识", "content": instructions}],
        "decision": "create_new",
        "reasoning": "deterministic knowledge conversion"
    });
    let mut target_skill_id = refined.get("targetSkillId").and_then(JsonValue::as_i64);
    let mut base_instructions = String::new();
    let mut name = format!("知识 · {}", title)
        .chars()
        .take(72)
        .collect::<String>();
    if let Some(id) = target_skill_id {
        if let Ok((builtin, old_name, old_instructions)) = connection.query_row(
            "SELECT builtin,name,instructions FROM strix_skills WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        ) {
            if builtin == 0 {
                name = old_name;
                base_instructions = old_instructions;
            } else {
                target_skill_id = None;
                name = format!("{} · 增强版", old_name);
                base_instructions = old_instructions;
            }
        } else {
            target_skill_id = None;
        }
    }
    let merged = apply_skill_patch(&base_instructions, &refined);
    if merged.trim().is_empty() {
        return Err("全局去重后没有可沉淀的 Skill 内容".into());
    }
    let normalized = skill_compare_text(&merged);
    if target_skill_id.is_none() {
        let duplicate = connection
            .prepare("SELECT id,instructions FROM strix_skills")
            .ok()
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })
                    .ok()
                    .and_then(|rows| {
                        rows.flatten()
                            .find(|(_, existing)| skill_compare_text(existing) == normalized)
                    })
            });
        target_skill_id = duplicate.map(|(id, _)| id);
    }
    let skill_id = if let Some(id) = target_skill_id {
        connection.execute("UPDATE strix_skills SET instructions=?1,description=CASE WHEN trim(description)='' THEN ?2 ELSE description END,updated_at=datetime('now','localtime') WHERE id=?3 AND builtin=0", params![merged,summary,id]).map_err(|error| error.to_string())?;
        id
    } else {
        let mut candidate_name = name.clone();
        if connection
            .query_row(
                "SELECT COUNT(*) FROM strix_skills WHERE name=?1",
                [&candidate_name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0
        {
            candidate_name = format!(
                "{} · {}",
                name.chars().take(60).collect::<String>(),
                knowledge_id
            );
        }
        connection.execute("INSERT INTO strix_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,1)", params![candidate_name,summary,merged]).map_err(|error| error.to_string())?;
        connection.last_insert_rowid()
    };
    connection.execute("UPDATE strix_knowledge_entries SET skill_id=?1,updated_at=datetime('now','localtime') WHERE id=?2", params![skill_id,knowledge_id]).map_err(|error| error.to_string())?;
    Ok(skill_id)
}

#[tauri::command]
pub fn refine_strix_skill_with_knowledge(
    state: State<AppState>,
    skill_id: i64,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let (builtin, name, description, instructions): (i64, String, String, String) = connection
        .query_row(
            "SELECT builtin,name,description,instructions FROM strix_skills WHERE id=?1",
            [skill_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "目标 Skill 不存在".to_string())?;
    let candidate = serde_json::json!({
        "title": format!("{} 最新知识精炼", name),
        "summary": description,
        "qualityGate": {"disposition": "reusable_candidate"},
        "skillPatch": {"addSections": []}
    });
    let catalog = strix_learning_catalog(&connection, Some(skill_id))?;
    let settings = sentinel_settings(&connection);
    let home = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .to_path_buf();
    let environment = strix_runtime_env(&settings, &home)?;
    let patch = refine_learning_patch_for_apply(&environment, &candidate, &instructions, &catalog)?;
    let refined = apply_skill_patch(&instructions, &patch);
    if refined.trim().is_empty() {
        return Err("最新知识精炼没有返回可保存的内容".into());
    }
    if builtin != 0 {
        let clone_name = format!("{} · 最新知识增强", name)
            .chars()
            .take(80)
            .collect::<String>();
        connection.execute("INSERT INTO strix_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,1)", params![clone_name,description,refined]).map_err(|error| error.to_string())?;
        return Ok(connection.last_insert_rowid());
    }
    connection.execute("UPDATE strix_skills SET instructions=?1,updated_at=datetime('now','localtime') WHERE id=?2", params![refined,skill_id]).map_err(|error| error.to_string())?;
    Ok(skill_id)
}

fn portable_export_path(root: &Path, prefix: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    Ok(root.join(format!(
        "{prefix}-{}.json",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    )))
}

#[tauri::command]
pub fn export_strix_skills(state: State<AppState>) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare("SELECT name,description,instructions,enabled FROM strix_skills WHERE builtin=0 ORDER BY name").map_err(|error|error.to_string())?;
    let skills = statement.query_map([], |row| Ok(serde_json::json!({"name":row.get::<_,String>(0)?,"description":row.get::<_,String>(1)?,"instructions":row.get::<_,String>(2)?,"enabled":row.get::<_,i64>(3)?!=0}))).map_err(|error|error.to_string())?.flatten().collect::<Vec<_>>();
    let path = portable_export_path(&state.export_dir, "strix-skills")?;
    let payload = serde_json::json!({"schemaVersion":1,"kind":"oviraptor-strix-skills","exportedAt":chrono::Utc::now().to_rfc3339(),"skills":skills});
    fs::write(
        &path,
        serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn import_strix_skills(state: State<AppState>, path: String) -> Result<i64, String> {
    let payload: JsonValue =
        serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if !matches!(
        payload.get("kind").and_then(JsonValue::as_str),
        Some("oviraptor-strix-skills" | "asset-atlas-strix-skills")
    ) {
        return Err("不是 Oviraptor Strix Skill 导出文件".into());
    }
    let connection = db::open(&state.db_path)?;
    let mut imported = 0;
    for skill in payload
        .get("skills")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let name = skill
            .get("name")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        let instructions = skill
            .get("instructions")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        if name.is_empty()
            || name.chars().count() > 80
            || instructions.is_empty()
            || instructions.chars().count() > 30_000
        {
            continue;
        }
        let builtin = connection
            .query_row(
                "SELECT builtin FROM strix_skills WHERE name=?1",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .unwrap_or(None);
        if builtin == Some(1) {
            continue;
        }
        connection.execute("INSERT INTO strix_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,?4) ON CONFLICT(name) DO UPDATE SET description=excluded.description,instructions=excluded.instructions,enabled=excluded.enabled,updated_at=datetime('now','localtime') WHERE strix_skills.builtin=0",params![name,skill.get("description").and_then(JsonValue::as_str).unwrap_or(""),instructions,skill.get("enabled").and_then(JsonValue::as_bool).unwrap_or(true) as i64]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    Ok(imported)
}

fn sec_skill_line_is_unsafe(line: &str) -> bool {
    let normalized = line.to_ascii_lowercase().replace('`', "");
    [
        "reverse shell",
        "反弹 shell",
        "authorized_keys",
        ".ssh/id_rsa",
        "private_key",
        "base64 -d",
        "bash -i",
        "curl | bash",
        "wget | sh",
        "webshell",
        "免杀",
        "metadata.google.internal",
        "169.254.169.254",
        "docker.sock",
        "kubectl exec",
        "外传",
        "exfil",
        "绕过安全",
        "disable security",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn collect_full_sec_skill_sections(root: &Path) -> Result<(String, usize), String> {
    let mut files = Vec::new();
    fn walk(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in fs::read_dir(path)
            .map_err(|error| error.to_string())?
            .flatten()
        {
            let child = entry.path();
            if child.is_dir() {
                let name = child
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("");
                if !name.starts_with('.') {
                    walk(&child, files)?;
                }
            } else if child.is_file() {
                let name = child
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("");
                if !name.starts_with('.') {
                    files.push(child);
                }
            }
        }
        Ok(())
    }
    walk(root, &mut files)?;
    files.sort();
    let mut output = String::from(
        "# sec_skills · 内部完整方法包\n来源：本地 sec_skills；以下文本按文件原样导入，仅作为公司内部授权资产自查的 Skill 内容，不在导入阶段执行任何命令。\n\n",
    );
    for file in &files {
        let text = fs::read_to_string(file).map_err(|error| error.to_string())?;
        let relative = file.strip_prefix(root).unwrap_or(file);
        output.push_str(&format!("## {}\n\n{}\n\n", relative.display(), text.trim()));
    }
    Ok((output.trim().to_string(), files.len()))
}

#[tauri::command]
pub fn import_sec_skill_knowledge(
    state: State<AppState>,
    path: String,
) -> Result<JsonValue, String> {
    let root = PathBuf::from(path.trim());
    if !root.is_dir() {
        return Err("sec_skills 路径不存在或不是目录".into());
    }
    let (instructions, files_scanned) = collect_full_sec_skill_sections(&root)?;
    if files_scanned == 0 || instructions.len() < 200 {
        return Err("目录中没有可导入的文本 Skill 文件；未创建 Skill".into());
    }
    let mut hasher = Sha256::new();
    hasher.update(instructions.as_bytes());
    let source_hash = format!("{:x}", hasher.finalize());
    let connection = db::open(&state.db_path)?;
    let name = "sec_skills · 内部完整方法包";
    let description = format!(
        "从本地 sec_skills 按文件完整导入 {files_scanned} 个文本文件，供公司内部授权资产自查使用。source:{source_hash}"
    );
    connection
        .execute(
            "INSERT INTO strix_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,1) ON CONFLICT(name) DO UPDATE SET description=excluded.description,instructions=excluded.instructions,enabled=1,updated_at=datetime('now','localtime') WHERE strix_skills.builtin=0",
            params![name, description, instructions],
        )
        .map_err(|error| error.to_string())?;
    let skill_id = connection
        .query_row("SELECT id FROM strix_skills WHERE name=?1", [name], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "skillId": skill_id,
        "name": name,
        "filesScanned": files_scanned,
        "sectionsKept": files_scanned,
        "droppedLines": 0,
        "enabled": true,
        "sourceHash": source_hash,
    }))
}

fn source_cache_key(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.trim().as_bytes());
    format!("strix_source_cache:{:x}", hasher.finalize())
}

fn source_type(source: &str) -> &'static str {
    let lower = source.to_ascii_lowercase();
    if lower.contains("hackerone.com") {
        "hackerone"
    } else if lower.contains("medium.com") {
        "medium"
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        "web"
    } else {
        "manual"
    }
}

fn html_to_readable_text(input: &str) -> String {
    let mut code_blocks = Vec::new();
    let mut image_alts = Vec::new();
    let lower_input = input.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(start_rel) = lower_input[cursor..].find("<pre") {
        let start = cursor + start_rel;
        let Some(open_end_rel) = lower_input[start..].find('>') else {
            break;
        };
        let body_start = start + open_end_rel + 1;
        let Some(end_rel) = lower_input[body_start..].find("</pre>") else {
            break;
        };
        let body_end = body_start + end_rel;
        let mut block = input[body_start..body_end]
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        block = block
            .replace("<br>", "\n")
            .replace("<br/>", "\n")
            .replace("<br />", "\n");
        let mut stripped = String::with_capacity(block.len());
        let mut in_tag = false;
        for ch in block.chars() {
            if ch == '<' {
                in_tag = true;
                continue;
            }
            if in_tag {
                if ch == '>' {
                    in_tag = false;
                }
                continue;
            }
            stripped.push(ch);
        }
        if !stripped.trim().is_empty() {
            code_blocks.push(stripped.trim().to_string());
        }
        cursor = body_end + 6;
    }
    let mut image_cursor = 0usize;
    while let Some(start_rel) = lower_input[image_cursor..].find("<img") {
        let start = image_cursor + start_rel;
        let Some(end_rel) = lower_input[start..].find('>') else {
            break;
        };
        let tag = &input[start..start + end_rel + 1];
        let lower_tag = tag.to_ascii_lowercase();
        if let Some(alt_start) = lower_tag.find("alt=") {
            let rest = &tag[alt_start + 4..];
            let value = rest
                .trim_start_matches([' ', '\t', '\n', '\r'])
                .trim_start_matches(['"', '\'']);
            let end = value.find(['"', '\'', ' ', '>']).unwrap_or(value.len());
            let alt = value[..end].trim();
            if !alt.is_empty() {
                image_alts.push(alt.to_string());
            }
        }
        image_cursor = start + end_rel + 1;
    }
    let mut text = input.to_string();
    for tag in ["script", "style", "noscript", "template", "svg"] {
        let lower = text.to_ascii_lowercase();
        let open = format!("<{}", tag);
        let close = format!("</{}", tag);
        let mut cursor = 0usize;
        let mut ranges = Vec::new();
        while let Some(start_rel) = lower[cursor..].find(&open) {
            let start = cursor + start_rel;
            let Some(end_rel) = lower[start..].find(&close) else {
                ranges.push(start..text.len());
                break;
            };
            let close_start = start + end_rel;
            let end = lower[close_start..]
                .find('>')
                .map(|offset| close_start + offset + 1)
                .unwrap_or(text.len());
            ranges.push(start..end);
            cursor = end;
            if cursor >= text.len() {
                break;
            }
        }
        for range in ranges.into_iter().rev() {
            text.replace_range(range, " ");
        }
    }
    let mut output = String::with_capacity(text.len().min(256_000));
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in text.chars() {
        if ch == '<' {
            in_tag = true;
            tag.clear();
            continue;
        }
        if in_tag {
            if ch == '>' {
                in_tag = false;
                let name = tag
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "p" | "br"
                        | "div"
                        | "section"
                        | "article"
                        | "li"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "pre"
                ) {
                    output.push('\n');
                }
            } else if tag.len() < 32 {
                tag.push(ch);
            }
            continue;
        }
        output.push(ch);
    }
    let mut readable = output
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !code_blocks.is_empty() {
        readable.push_str("\n\n[代码块]\n");
        for block in code_blocks.into_iter().take(12) {
            readable.push_str("```text\n");
            readable.push_str(&block.chars().take(12_000).collect::<String>());
            readable.push_str("\n```\n");
        }
    }
    if !image_alts.is_empty() {
        readable.push_str("\n[图片说明]\n");
        for alt in image_alts.into_iter().take(24) {
            readable.push_str("- ");
            readable.push_str(&alt);
            readable.push('\n');
        }
    }
    readable.trim().to_string()
}

fn normalize_external_content(source: &str, content: String) -> String {
    let lower = content.trim_start().to_ascii_lowercase();
    let is_html = lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || lower.contains("<article")
        || source.to_ascii_lowercase().ends_with(".html");
    if is_html {
        let readable = html_to_readable_text(&content);
        if readable.len() >= 200 {
            return readable;
        }
    }
    content
}

fn fetch_external_source(source: &str) -> Result<(String, String), String> {
    const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
    let trimmed = source.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("Oviraptor/1.0 security-knowledge-import")
            .build()
            .map_err(|error| error.to_string())?;
        let response = client
            .get(trimmed)
            .send()
            .map_err(|error| format!("读取公开文章失败：{error}"))?;
        if !response.status().is_success() {
            return Err(format!("公开文章返回 HTTP {}", response.status().as_u16()));
        }
        let text = response
            .text()
            .map_err(|error| format!("读取公开文章正文失败：{error}"))?;
        if text.len() as u64 > MAX_SOURCE_BYTES {
            return Err("公开文章超过 16MB，拒绝读取；请保存正文或拆分后再分析".into());
        }
        Ok((
            trimmed.to_string(),
            normalize_external_content(trimmed, text),
        ))
    } else {
        let path = PathBuf::from(trimmed);
        let metadata =
            fs::metadata(&path).map_err(|error| format!("读取本地知识文件失败：{error}"))?;
        if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES {
            return Err("本地知识文件不存在、不是普通文件或超过 16MB".into());
        }
        let path_string = path.to_string_lossy().to_string();
        let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        Ok((
            path_string.clone(),
            normalize_external_content(&path_string, text),
        ))
    }
}

fn render_external_method_cards(cards: &[JsonValue]) -> String {
    let mut output = vec![
        "## 来源方法卡片".to_string(),
        "仅把公开文章抽象成可复用方法；文章中的一次性目标、凭据、攻击 payload 和危险命令不会进入 Skill。".to_string(),
    ];
    for (index, card) in cards.iter().enumerate() {
        let method = card
            .get("method")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        if method.is_empty() {
            continue;
        }
        let list = |key: &str| {
            card.get(key)
                .and_then(JsonValue::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(JsonValue::as_str)
                        .map(|value| format!("- {}", value.trim()))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|| "- 未声明；新任务必须补齐".into())
        };
        output.push(format!("### {}. {}", index + 1, method));
        output.push(format!("**前置条件**\n{}", list("preconditions")));
        output.push(format!("**安全验证**\n{}", list("safeVerification")));
        output.push(format!("**所需证据**\n{}", list("evidenceRequired")));
        output.push(format!("**负面信号**\n{}", list("negativeSignals")));
        output.push(format!("**停止条件**\n{}", list("stopConditions")));
        output.push(format!(
            "**严重度依据**\n{}",
            card.get("severityGuidance")
                .and_then(JsonValue::as_str)
                .unwrap_or("只有明确影响和复现证据时才评估严重度")
        ));
        if let Some(citation) = card.get("sourceCitation").and_then(JsonValue::as_str) {
            output.push(format!("**来源引用**\n{}", citation.trim()));
        }
    }
    output.push("## 统一停止条件\n连续两次验证没有新增证据时停止当前分支；纯版本、Banner、路径或 CVE 匹配只保留为 needs_verification。".into());
    output.join("\n\n")
}

#[tauri::command]
pub fn ingest_strix_knowledge_source(
    state: State<AppState>,
    source: String,
    force_refresh: Option<bool>,
) -> Result<StrixKnowledgeEntry, String> {
    let source = source.trim().to_string();
    if source.is_empty() {
        return Err(
            "请提供任意公开安全文章 URL，或 Safari/Chrome 保存的 HTML、Markdown 文件路径".into(),
        );
    }
    let connection = db::open(&state.db_path)?;
    let cache_key = source_cache_key(&source);
    if !force_refresh.unwrap_or(false) {
        if let Ok(cache_json) = connection.query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [&cache_key],
            |row| row.get::<_, String>(0),
        ) {
            let cache = json(cache_json);
            if let Some(knowledge_id) = cache.get("knowledgeId").and_then(JsonValue::as_i64) {
                if let Ok(entry) = connection.query_row(
                    &format!("SELECT {KNOWLEDGE_COLUMNS} FROM strix_knowledge_entries WHERE id=?1"),
                    [knowledge_id],
                    knowledge_row,
                ) {
                    return Ok(entry);
                }
            }
        }
    }
    let (canonical_source, content) = fetch_external_source(&source)?;
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let content_hash = format!("{:x}", hasher.finalize());
    if let Ok(entry) = connection.query_row(
        &format!("SELECT {KNOWLEDGE_COLUMNS} FROM strix_knowledge_entries WHERE source_hash=?1"),
        [&content_hash],
        knowledge_row,
    ) {
        let cache = serde_json::json!({"knowledgeId":entry.id,"contentHash":content_hash,"source":canonical_source,"fetchedAt":chrono::Utc::now().to_rfc3339()});
        connection.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![cache_key, cache.to_string()]).map_err(|error| error.to_string())?;
        return Ok(entry);
    }
    let settings = sentinel_settings(&connection);
    let home = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .to_path_buf();
    let environment = strix_runtime_env(&settings, &home)?;
    let prompt = format!(
        "你是防守型 AppSec 知识工程师。请把以下公开安全文章转换成可审核的方法卡片，不要复制文章原文，不要输出真实目标、凭据、Cookie、Token、一次性 URL、反弹 shell、外传、绕过安全边界或可直接造成破坏的命令。只输出 JSON：{{\"title\":\"\",\"summary\":\"\",\"methodCards\":[{{\"method\":\"\",\"preconditions\":[],\"safeVerification\":[],\"evidenceRequired\":[],\"negativeSignals\":[],\"stopConditions\":[],\"severityGuidance\":\"\",\"sourceCitation\":\"\",\"confidence\":0.0}}],\"qualityScore\":0}}。每个方法必须能跨目标复用，并明确证据和停止条件；文章中的纯故事、版本匹配和未验证猜测放入 negativeSignals。来源类型：{}；来源：{}；正文：{}",
        source_type(&canonical_source),
        canonical_source,
        content.chars().take(80_000).collect::<String>()
    );
    let analyzed = call_learning_llm(&environment, &prompt)
        .map_err(|error| format!("公开文章分析失败：{error}"))?;
    let cards = analyzed
        .get("methodCards")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let safe_cards = cards
        .into_iter()
        .filter(|card| {
            let text = card.to_string();
            !sec_skill_line_is_unsafe(&text)
                && card
                    .get("method")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
        })
        .take(24)
        .collect::<Vec<_>>();
    if safe_cards.is_empty() {
        return Err("文章没有提炼出包含前置条件、证据和停止条件的安全方法卡片".into());
    }
    let quality_score = analyzed
        .get("qualityScore")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        .clamp(0, 100);
    let title = analyzed
        .get("title")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("公开文章方法卡片")
        .chars()
        .take(120)
        .collect::<String>();
    let summary = analyzed
        .get("summary")
        .and_then(JsonValue::as_str)
        .unwrap_or("已缓存并抽象为方法卡片；扫描时只检索本地卡片，不重复抓取原文")
        .chars()
        .take(2000)
        .collect::<String>();
    let patterns = serde_json::json!({
        "knowledgeKind":"external_source",
        "sourceType":source_type(&canonical_source),
        "sourceUrl":canonical_source,
        "contentHash":content_hash,
        "methodCards":safe_cards,
        "qualityScore":quality_score,
        "cachedAt":chrono::Utc::now().to_rfc3339(),
    });
    let method_cards = patterns
        .get("methodCards")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let instructions = render_external_method_cards(&method_cards);
    let scan_id = format!("source:{content_hash}");
    connection.execute("INSERT INTO strix_knowledge_entries(scan_id,title,summary,patterns_json,skill_instructions,source_hash) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(scan_id) DO UPDATE SET title=excluded.title,summary=excluded.summary,patterns_json=excluded.patterns_json,skill_instructions=excluded.skill_instructions,source_hash=excluded.source_hash,updated_at=datetime('now','localtime')", params![scan_id,title,summary,patterns.to_string(),instructions,content_hash]).map_err(|error| error.to_string())?;
    let entry = connection
        .query_row(
            &format!("SELECT {KNOWLEDGE_COLUMNS} FROM strix_knowledge_entries WHERE scan_id=?1"),
            [&scan_id],
            knowledge_row,
        )
        .map_err(|error| error.to_string())?;
    let cache = serde_json::json!({"knowledgeId":entry.id,"contentHash":content_hash,"source":canonical_source,"fetchedAt":chrono::Utc::now().to_rfc3339()});
    connection.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![cache_key, cache.to_string()]).map_err(|error| error.to_string())?;
    Ok(entry)
}

#[tauri::command]
pub fn export_strix_knowledge(state: State<AppState>) -> Result<String, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare("SELECT title,summary,patterns_json,skill_instructions,source_hash FROM strix_knowledge_entries ORDER BY id").map_err(|error|error.to_string())?;
    let entries = statement.query_map([], |row| Ok(serde_json::json!({"title":row.get::<_,String>(0)?,"summary":row.get::<_,String>(1)?,"patterns":json(row.get::<_,String>(2)?),"skillInstructions":row.get::<_,String>(3)?,"sourceHash":row.get::<_,String>(4)?}))).map_err(|error|error.to_string())?.flatten().collect::<Vec<_>>();
    let path = portable_export_path(&state.export_dir, "strix-knowledge")?;
    let payload = serde_json::json!({"schemaVersion":1,"kind":"oviraptor-strix-knowledge","exportedAt":chrono::Utc::now().to_rfc3339(),"entries":entries});
    fs::write(
        &path,
        serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn import_strix_knowledge(state: State<AppState>, path: String) -> Result<i64, String> {
    let payload: JsonValue =
        serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if !matches!(
        payload.get("kind").and_then(JsonValue::as_str),
        Some("oviraptor-strix-knowledge" | "asset-atlas-strix-knowledge")
    ) {
        return Err("不是 Oviraptor Strix 知识库导出文件".into());
    }
    let connection = db::open(&state.db_path)?;
    let mut imported = 0;
    for entry in payload
        .get("entries")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let title = entry
            .get("title")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        let instructions = entry
            .get("skillInstructions")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        let source_hash = entry
            .get("sourceHash")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        if title.is_empty() || instructions.is_empty() || source_hash.is_empty() {
            continue;
        }
        let scan_id = format!(
            "imported-{}",
            source_hash.chars().take(32).collect::<String>()
        );
        connection.execute("INSERT INTO strix_knowledge_entries(scan_id,title,summary,patterns_json,skill_instructions,source_hash) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(scan_id) DO UPDATE SET title=excluded.title,summary=excluded.summary,patterns_json=excluded.patterns_json,skill_instructions=excluded.skill_instructions,source_hash=excluded.source_hash,updated_at=datetime('now','localtime')",params![scan_id,title,entry.get("summary").and_then(JsonValue::as_str).unwrap_or(""),entry.get("patterns").cloned().unwrap_or_default().to_string(),instructions,source_hash]).map_err(|error|error.to_string())?;
        imported += 1;
    }
    Ok(imported)
}
