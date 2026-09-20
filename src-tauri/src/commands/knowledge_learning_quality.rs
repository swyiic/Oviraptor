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
