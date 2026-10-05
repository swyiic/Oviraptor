#[tauri::command]
pub fn list_agent_knowledge(
    state: State<AppState>,
    scan_id: Option<String>,
) -> Result<Vec<AgentKnowledgeEntry>, String> {
    let connection = db::open(&state.db_path)?;
    query_agent_knowledge(&connection, scan_id.as_deref())
}

fn query_agent_knowledge(
    connection: &rusqlite::Connection,
    scan_id: Option<&str>,
) -> Result<Vec<AgentKnowledgeEntry>, String> {
    let mut statement = connection
        .prepare(&format!("SELECT {KNOWLEDGE_COLUMNS} FROM agent_knowledge_entries WHERE (?1 IS NULL OR scan_id=?1) ORDER BY updated_at DESC,id DESC"))
        .map_err(|error| error.to_string())?;
    let entries = statement
        .query_map([scan_id], knowledge_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(entries)
}

fn generate_learning_candidate_with_environment(
    db_path: &Path,
    scan_id: &str,
    environment: &ModelRuntimeEnv,
) -> Result<LearningGenerationOutcome, String> {
    let connection = db::open(db_path)?;
    let (trace, events) = collect_agent_trace(&connection, scan_id, true, true)?;
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
            &format!("SELECT {LEARNING_CANDIDATE_COLUMNS} FROM agent_learning_candidates WHERE scan_id=?1 AND source_hash=?2"),
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
            "sourceAuthority":trace.source_authority,
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
            "INSERT INTO agent_learning_candidates(scan_id,project_id,scan_type,title,summary,candidate_json,status,source_hash) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7) ON CONFLICT(scan_id,source_hash) DO UPDATE SET title=excluded.title,summary=excluded.summary,candidate_json=excluded.candidate_json,status='pending',reviewed_at='',updated_at=datetime('now','localtime') WHERE agent_learning_candidates.status IN ('pending','rejected')",
        params![scan_id, project_id, trace.scan_type, title, summary, candidate_json.to_string(), source_hash],
        )
        .map_err(|error| error.to_string())?;
    let candidate = connection
        .query_row(
            &format!("SELECT {LEARNING_CANDIDATE_COLUMNS} FROM agent_learning_candidates WHERE scan_id=?1 AND source_hash=?2"),
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

fn schedule_learning_candidate(db_path: PathBuf, scan_id: String, environment: ModelRuntimeEnv) {
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
    } else {
        let object = value.as_object()?;
        let title = object
            .get("title")
            .or(object.get("name"))
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

fn agent_learning_catalog(
    connection: &rusqlite::Connection,
    exclude_skill_id: Option<i64>,
) -> Result<String, String> {
    let mut sections = Vec::new();
    let mut skills = connection
        .prepare("SELECT id,name,description,instructions,builtin FROM agent_skills WHERE enabled=1 AND (?1 IS NULL OR id<>?1) ORDER BY builtin DESC,updated_at DESC,id DESC LIMIT 24")
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
        // This is a global refinement catalog, not a task-scoped retrieval.
        // Per-project task cards must not leak into another project's prompt.
        .prepare("SELECT id,title,summary,patterns_json,skill_instructions FROM agent_knowledge_entries WHERE project_id IS NULL ORDER BY updated_at DESC,id DESC LIMIT 96")
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
    let mut knowledge_added = 0;
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
        if !matches!(kind, "aggregate" | "external_source") || distinct_scans < 2 && kind == "aggregate" {
            // A manually reviewed one-off can still be converted explicitly,
            // but task-derived cards never enter this cross-project catalog.
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
        knowledge_added += 1;
        if knowledge_added >= 16 {
            break;
        }
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

#[cfg(test)]
#[test]
fn global_learning_catalog_excludes_task_data_and_bounds_context() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch(
        "CREATE TABLE agent_skills(id INTEGER PRIMARY KEY,name TEXT,description TEXT,instructions TEXT,builtin INTEGER,enabled INTEGER,updated_at TEXT);
         CREATE TABLE agent_knowledge_entries(id INTEGER PRIMARY KEY,project_id INTEGER,title TEXT,summary TEXT,patterns_json TEXT,skill_instructions TEXT,updated_at TEXT);",
    ).unwrap();
    connection.execute("INSERT INTO agent_skills VALUES(1,'active','','safe',1,1,'2026-01-01')", []).unwrap();
    connection.execute("INSERT INTO agent_skills VALUES(2,'disabled','','secret',0,0,'2026-01-02')", []).unwrap();
    let insert = |id: i64, project: Option<i64>, kind: &str, support: i64| {
        let patterns = serde_json::json!({
            "knowledgeKind":kind,"qualityScore":90,
            "support":{"distinctScans":support}
        });
        let updated = if id == 4 || id == 5 { "2026-01-02" } else { "2026-01-01" };
        connection.execute(
            "INSERT INTO agent_knowledge_entries VALUES(?1,?2,?3,'summary',?4,'method',?5)",
            rusqlite::params![id,project,format!("entry-{id}"),patterns.to_string(),updated],
        ).unwrap();
    };
    insert(1, Some(7), "aggregate", 4);
    insert(2, None, "task_candidate", 2);
    insert(3, None, "aggregate", 1);
    insert(4, None, "aggregate", 2);
    insert(5, None, "external_source", 1);
    for id in 6..30 {
        insert(id, None, "aggregate", 2);
    }
    let catalog = agent_learning_catalog(&connection, None).unwrap();
    assert!(catalog.contains("active"));
    assert!(!catalog.contains("disabled"));
    assert!(!catalog.contains("entry-1（"));
    assert!(!catalog.contains("entry-2（"));
    assert!(!catalog.contains("entry-3（"));
    assert!(catalog.contains("entry-4（"));
    assert!(catalog.contains("entry-5（"));
    assert_eq!(catalog.matches("知识 #").count(), 16);
    assert!(catalog.chars().count() <= 42_000);
}

fn refine_learning_patch_for_apply(
    environment: &ModelRuntimeEnv,
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
pub fn list_agent_learning_candidates(
    state: State<AppState>,
    status: Option<String>,
    scan_id: Option<String>,
) -> Result<Vec<AgentLearningCandidate>, String> {
    let connection = db::open(&state.db_path)?;
    query_agent_learning_candidates(&connection, status.as_deref(), scan_id.as_deref())
}

fn query_agent_learning_candidates(
    connection: &rusqlite::Connection,
    status: Option<&str>,
    scan_id: Option<&str>,
) -> Result<Vec<AgentLearningCandidate>, String> {
    let mut statement = connection
        .prepare(&format!("SELECT {LEARNING_CANDIDATE_COLUMNS} FROM agent_learning_candidates WHERE (?1 IS NULL OR status=?1) AND (?2 IS NULL OR scan_id=?2) ORDER BY updated_at DESC,id DESC"))
        .map_err(|error| error.to_string())?;
    let result = statement
        .query_map(params![status, scan_id], learning_candidate_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string());
    result
}

#[cfg(test)]
#[test]
fn task_learning_readers_filter_by_scan_without_changing_the_global_catalog() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch(
        "CREATE TABLE agent_knowledge_entries(id INTEGER,scan_id TEXT,project_id INTEGER,title TEXT,summary TEXT,patterns_json TEXT,source_hash TEXT,skill_id INTEGER,created_at TEXT,updated_at TEXT); \
         CREATE TABLE agent_learning_candidates(id INTEGER,scan_id TEXT,project_id INTEGER,scan_type TEXT,title TEXT,summary TEXT,candidate_json TEXT,status TEXT,target_skill_id INTEGER,source_hash TEXT,created_at TEXT,reviewed_at TEXT,updated_at TEXT); \
         INSERT INTO agent_knowledge_entries VALUES(1,'scan-a',1,'A','summary','{}','hash-a',NULL,'t','t'),(2,'scan-b',2,'B','summary','{}','hash-b',NULL,'t','t'); \
         INSERT INTO agent_learning_candidates VALUES(1,'scan-a',1,'web','A','summary','{}','pending',NULL,'hash-a','t','','t'),(2,'scan-b',2,'web','B','summary','{}','accepted',NULL,'hash-b','t','','t');",
    ).unwrap();
    assert_eq!(query_agent_knowledge(&connection, None).unwrap().len(), 2);
    let knowledge = query_agent_knowledge(&connection, Some("scan-a")).unwrap();
    assert_eq!(knowledge.len(), 1);
    assert_eq!(knowledge[0].scan_id, "scan-a");
    assert!(query_agent_knowledge(&connection, Some("unknown")).unwrap().is_empty());
    assert_eq!(query_agent_learning_candidates(&connection, None, None).unwrap().len(), 2);
    let candidates = query_agent_learning_candidates(&connection, Some("pending"), Some("scan-a")).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].scan_id, "scan-a");
    assert!(query_agent_learning_candidates(&connection, Some("accepted"), Some("scan-a")).unwrap().is_empty());
}

#[tauri::command]
pub fn generate_agent_learning_candidate(
    state: State<AppState>,
    scan_id: String,
) -> Result<AgentLearningCandidate, String> {
    let settings = sentinel_settings(&db::open(&state.db_path)?);
    let environment = model_runtime_env(&settings)?;
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
pub fn review_agent_learning_candidate(
    state: State<AppState>,
    candidate_id: i64,
    decision: String,
    target_skill_id: Option<i64>,
) -> Result<AgentLearningCandidate, String> {
    let decision = decision.trim().to_ascii_lowercase();
    if !["accepted", "rejected", "pending"].contains(&decision.as_str()) {
        return Err("候选审核状态必须是 accepted、rejected 或 pending".into());
    }
    let connection = db::open(&state.db_path)?;
    let changed = connection
        .execute("UPDATE agent_learning_candidates SET status=?1,target_skill_id=COALESCE(?2,target_skill_id),reviewed_at=CASE WHEN ?1='pending' THEN '' ELSE datetime('now','localtime') END,updated_at=datetime('now','localtime') WHERE id=?3", params![decision,target_skill_id,candidate_id])
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("学习候选不存在".into());
    }
    connection
        .query_row(
            &format!(
                "SELECT {LEARNING_CANDIDATE_COLUMNS} FROM agent_learning_candidates WHERE id=?1"
            ),
            [candidate_id],
            learning_candidate_row,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_agent_learning_candidate(
    state: State<AppState>,
    candidate_id: i64,
) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let deleted = connection
        .execute(
            "DELETE FROM agent_learning_candidates WHERE id=?1",
            [candidate_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("学习候选不存在".into());
    }
    Ok(())
}

#[tauri::command]
pub fn apply_agent_learning_candidate(
    state: State<AppState>,
    candidate_id: i64,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    let (status, target_skill_id, candidate_json, scan_id): (String, Option<i64>, String, String) = connection
        .query_row("SELECT status,target_skill_id,candidate_json,scan_id FROM agent_learning_candidates WHERE id=?1", [candidate_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))
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
                "SELECT builtin,name,instructions FROM agent_skills WHERE id=?1",
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
                "SELECT builtin,name,instructions FROM agent_skills WHERE id=?1",
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
            .prepare("SELECT id,instructions FROM agent_skills")
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
        connection.execute("INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,1)", params![name,description,instructions]).map_err(|error| error.to_string())?;
        skill_id = Some(connection.last_insert_rowid());
    } else {
        connection.execute("UPDATE agent_skills SET instructions=?1,updated_at=datetime('now','localtime') WHERE id=?2 AND builtin=0", params![instructions,skill_id]).map_err(|error| error.to_string())?;
    }
    let skill_id = skill_id.ok_or("无法创建 Skill")?;
    connection.execute("UPDATE agent_learning_candidates SET status='applied',target_skill_id=?1,candidate_json=?2,reviewed_at=COALESCE(NULLIF(reviewed_at,''),datetime('now','localtime')),updated_at=datetime('now','localtime') WHERE id=?3", params![skill_id,candidate.to_string(),candidate_id]).map_err(|error| error.to_string())?;
    Ok(skill_id)
}

fn trace_quality_score(trace: &AgentTraceSummary, finding_count: usize) -> i64 {
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
pub fn delete_agent_knowledge(state: State<AppState>, knowledge_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let deleted = connection
        .execute(
            "DELETE FROM agent_knowledge_entries WHERE id=?1",
            [knowledge_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("知识条目不存在".into());
    }
    Ok(())
}
