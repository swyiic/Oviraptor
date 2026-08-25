fn source_language(extension: &str) -> Option<&'static str> {
    match extension {
        "rs" => Some("Rust"),
        "ts" | "tsx" => Some("TypeScript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("JavaScript"),
        "vue" => Some("Vue SFC"),
        "java" => Some("Java"),
        "kt" | "kts" => Some("Kotlin"),
        "py" => Some("Python"),
        "php" => Some("PHP"),
        "go" => Some("Go"),
        "cs" => Some("C#"),
        "c" | "h" => Some("C"),
        "cc" | "cpp" | "cxx" | "hpp" => Some("C++"),
        "swift" => Some("Swift"),
        "rb" => Some("Ruby"),
        "scala" => Some("Scala"),
        "dart" => Some("Dart"),
        "sol" => Some("Solidity"),
        "html" | "htm" => Some("HTML"),
        "css" | "scss" | "sass" | "less" => Some("CSS"),
        "sql" => Some("SQL"),
        "sh" | "bash" | "zsh" => Some("Shell"),
        _ => None,
    }
}

fn source_line_counts(path: &Path, language: &str) -> Option<(u64, u64, u64, u64)> {
    let text = fs::read_to_string(path).ok()?;
    if text.as_bytes().contains(&0) {
        return None;
    }
    let mut physical = 0;
    let mut code = 0;
    let mut comments = 0;
    let mut blank = 0;
    let mut in_block_comment = false;
    for line in text.lines() {
        physical += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank += 1;
            continue;
        }
        if in_block_comment {
            comments += 1;
            if trimmed.contains("*/") || trimmed.contains("-->") {
                in_block_comment = false;
            }
            continue;
        }
        let line_comment = match language {
            "Python" | "Ruby" | "Shell" => trimmed.starts_with('#'),
            "SQL" => trimmed.starts_with("--"),
            "HTML" | "Vue SFC" => trimmed.starts_with("<!--"),
            _ => trimmed.starts_with("//"),
        };
        let block_comment =
            trimmed.starts_with("/*") || trimmed.starts_with('*') || trimmed.starts_with("<!--");
        if line_comment || block_comment {
            comments += 1;
            if (trimmed.starts_with("/*") && !trimmed.contains("*/"))
                || (trimmed.starts_with("<!--") && !trimmed.contains("-->"))
            {
                in_block_comment = true;
            }
        } else {
            code += 1;
            if let Some(start) = trimmed.find("/*") {
                if !trimmed[start + 2..].contains("*/") {
                    in_block_comment = true;
                }
            }
        }
    }
    Some((physical, code, comments, blank))
}

fn inspect_source_tree(root: &Path) -> JsonValue {
    fn visit(
        path: &Path,
        root: &Path,
        depth: usize,
        counts: &mut HashMap<String, [u64; 6]>,
        manifests: &mut Vec<String>,
        samples: &mut Vec<PathBuf>,
        total: &mut u64,
        skipped_large: &mut u64,
    ) {
        if depth > 24 || *total >= 100_000 {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if child.is_dir() {
                if [
                    ".git",
                    "node_modules",
                    "target",
                    "dist",
                    "build",
                    "vendor",
                    ".venv",
                    "venv",
                    "coverage",
                    ".next",
                ]
                .contains(&name.as_str())
                {
                    continue;
                }
                visit(
                    &child,
                    root,
                    depth + 1,
                    counts,
                    manifests,
                    samples,
                    total,
                    skipped_large,
                );
                continue;
            }
            *total += 1;
            let relative = child
                .strip_prefix(root)
                .unwrap_or(&child)
                .to_string_lossy()
                .replace('\\', "/");
            if [
                "package.json",
                "Cargo.toml",
                "tauri.conf.json",
                "pom.xml",
                "build.gradle",
                "build.gradle.kts",
                "requirements.txt",
                "pyproject.toml",
                "composer.json",
                "go.mod",
                "Gemfile",
            ]
            .contains(&name.as_str())
            {
                manifests.push(relative);
                samples.push(child.clone());
            }
            let extension = child
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if let Some(language) = source_language(&extension) {
                let size = entry.metadata().map(|v| v.len()).unwrap_or(0);
                let value = counts.entry(language.into()).or_default();
                value[0] += 1;
                value[1] += size;
                if size > 5 * 1024 * 1024 {
                    *skipped_large += 1;
                } else if let Some((lines, code, comments, blank)) =
                    source_line_counts(&child, language)
                {
                    value[2] += lines;
                    value[3] += code;
                    value[4] += comments;
                    value[5] += blank;
                }
            }
        }
    }
    let mut counts = HashMap::new();
    let mut manifests = Vec::new();
    let mut samples = Vec::new();
    let mut total = 0;
    let mut skipped_large = 0;
    visit(
        root,
        root,
        0,
        &mut counts,
        &mut manifests,
        &mut samples,
        &mut total,
        &mut skipped_large,
    );
    let code_files = counts.values().map(|value| value[0]).sum::<u64>();
    let total_lines = counts.values().map(|value| value[2]).sum::<u64>();
    let code_lines = counts.values().map(|value| value[3]).sum::<u64>();
    let comment_lines = counts.values().map(|value| value[4]).sum::<u64>();
    let blank_lines = counts.values().map(|value| value[5]).sum::<u64>();
    let mut languages=counts.into_iter().map(|(name,value)|serde_json::json!({"name":name,"files":value[0],"bytes":value[1],"lines":value[2],"codeLines":value[3],"commentLines":value[4],"blankLines":value[5],"percent":if code_files==0{0.0}else{value[0] as f64*100.0/code_files as f64}})).collect::<Vec<_>>();
    languages.sort_by(|a, b| {
        b.get("files")
            .and_then(JsonValue::as_u64)
            .cmp(&a.get("files").and_then(JsonValue::as_u64))
    });
    let mut manifest_documents = Vec::new();
    for (manifest, path) in manifests.iter().zip(samples.iter()).take(30) {
        if let Ok(text) = fs::read_to_string(path) {
            if text.len() < 2_000_000 {
                manifest_documents.push((manifest.clone(), text.to_ascii_lowercase()));
            }
        }
    }
    let mut frameworks = Vec::new();
    let mut add = |name: &str, layer: &str, needle: &str| {
        if let Some((manifest, _)) = manifest_documents
            .iter()
            .find(|(_, content)| content.contains(needle))
        {
            frameworks.push(serde_json::json!({"name":name,"layer":layer,"evidence":manifest}))
        }
    };
    add("Tauri", "Desktop runtime", "tauri");
    add("Vue", "Frontend", "\"vue\"");
    add("React", "Frontend", "\"react\"");
    add("Angular", "Frontend", "@angular/core");
    add("Next.js", "Frontend", "\"next\"");
    add("Vite", "Build", "\"vite\"");
    add("Spring", "Backend", "spring-boot");
    add("Django", "Backend", "django");
    add("Flask", "Backend", "flask");
    add("FastAPI", "Backend", "fastapi");
    add("Laravel", "Backend", "laravel/framework");
    add("Symfony", "Backend", "symfony/framework");
    add("Gin", "Backend", "github.com/gin-gonic/gin");
    add("Fiber", "Backend", "github.com/gofiber/fiber");
    add("Axum", "Backend", "axum");
    add("Actix Web", "Backend", "actix-web");
    let architecture = if frameworks
        .iter()
        .any(|v| v.get("name").and_then(JsonValue::as_str) == Some("Tauri"))
    {
        "Tauri desktop application"
    } else if frameworks
        .iter()
        .any(|v| v.get("layer").and_then(JsonValue::as_str) == Some("Frontend"))
    {
        "Web application"
    } else {
        "Source repository"
    };
    serde_json::json!({"architecture":architecture,"root":root,"totalFiles":total,"codeFiles":code_files,"lineStats":{"physical":total_lines,"code":code_lines,"comments":comment_lines,"blank":blank_lines,"skippedLargeFiles":skipped_large,"maxFileBytes":5*1024*1024},"languages":languages,"frameworks":frameworks,"manifests":manifests,"detectedAt":chrono::Utc::now().to_rfc3339()})
}

fn insert_source_inventory(
    connection: &rusqlite::Connection,
    scan_id: &str,
    source_path: &str,
) -> Result<(), String> {
    if source_path.is_empty() {
        return Ok(());
    }
    let inventory = inspect_source_tree(Path::new(source_path));
    insert_finding(
        connection,
        scan_id,
        source_path,
        "local-inventory",
        "source_inventory",
        "repository",
        "源码架构与语言清单",
        "info",
        &inventory,
    )
}

fn start_strix_workbench_scan_impl(
    state: &AppState,
    input: StrixWorkbenchInput,
    reuse_scan_id: Option<String>,
) -> Result<SentinelScan, String> {
    let scan_type = input.scan_type.trim().to_lowercase();
    if !["code", "greybox", "cicd"].contains(&scan_type.as_str()) {
        return Err("不支持的 Strix 扫描类型".into());
    }
    let source_path = input.source_path.trim().to_string();
    if ["code", "cicd"].contains(&scan_type.as_str()) && source_path.is_empty() {
        return Err("代码审计与 CI/CD 任务必须选择源码目录".into());
    }
    if !source_path.is_empty() && !Path::new(&source_path).is_dir() {
        return Err("源码目录不存在或不可读取".into());
    }
    let mut urls = input
        .urls
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    if scan_type == "greybox" && urls.is_empty() {
        return Err("灰盒任务至少需要一个 URL".into());
    }
    if urls.len() > 200 {
        return Err("单个工作台任务最多 200 个 URL".into());
    }
    if urls
        .iter()
        .any(|url| !(url.starts_with("http://") || url.starts_with("https://")))
    {
        return Err("URL 必须以 http:// 或 https:// 开头".into());
    }
    let scan_mode = match input.scan_mode.as_str() {
        "quick" | "standard" | "deep" => input.scan_mode,
        _ if scan_type == "cicd" => "quick".into(),
        _ => "deep".into(),
    };
    let scope_mode = match input.scope_mode.as_str() {
        "auto" | "diff" | "full" => input.scope_mode,
        _ if scan_type == "cicd" => "auto".into(),
        _ => "full".into(),
    };
    if input
        .max_budget_usd
        .is_some_and(|value| value <= 0.0 || value > 10_000.0)
    {
        return Err("Token 预算对应的美元上限必须大于 0 且不超过 10000".into());
    }
    let environment = input
        .environment
        .trim()
        .chars()
        .take(80)
        .collect::<String>();
    let mut auth_type = match input.auth_type.trim().to_ascii_lowercase().as_str() {
        "cookie" | "bearer" | "header" if scan_type == "greybox" => {
            input.auth_type.trim().to_ascii_lowercase()
        }
        _ => "none".into(),
    };
    if input.auth_value.len() > 32_768 {
        return Err("单次认证会话内容不能超过 32KB".into());
    }
    if auth_type == "header"
        && input.auth_header_name.trim().is_empty()
        && !input.auth_value.trim().is_empty()
    {
        return Err("自定义 Header 认证必须填写 Header 名称".into());
    }
    if input.max_critical < 0
        || input.max_high < 0
        || input.max_critical > 10_000
        || input.max_high > 10_000
    {
        return Err("CI/CD 门禁阈值必须在 0 到 10000 之间".into());
    }
    let connection = db::open(&state.db_path)?;
    let project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id=?1 AND status='active'",
            [input.project_id],
            |row| row.get(0),
        )
        .map_err(|_| "项目不存在或已归档；恢复工作空间后才能启动 Strix".to_string())?;
    // Asset/Workbench tasks with no explicit selection inherit every enabled
    // configured Skill. The final skill_names string records exactly what was
    // injected, so a later audit can reproduce the prompt composition.
    let effective_skill_ids = if input.skill_ids.is_empty() {
        let mut statement = connection
            .prepare("SELECT id FROM strix_skills WHERE enabled=1 ORDER BY builtin DESC,id")
            .map_err(|error| error.to_string())?;
        let ids = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|error| error.to_string())?
            .flatten()
            .collect::<Vec<_>>();
        ids
    } else {
        input.skill_ids.clone()
    };
    let (skill_names, skill_instructions) =
        strix_skill_instructions(&connection, &effective_skill_ids)?;
    let task_name = if input.task_name.trim().is_empty() {
        format!("{} · {}", project_name, scan_type)
    } else {
        input.task_name.trim().chars().take(120).collect()
    };
    let settings = sentinel_settings(&connection);
    let home = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .to_path_buf();
    let strix = resolve_strix_executable(&settings, &home)?;
    let strix_cli = strix_cli_capabilities(&strix)?;
    let strix_environment = strix_runtime_env(&settings, &home)?;
    // “火力全开” controls local throughput and budget only. It must not
    // silently turn an explicitly selected Quick/Standard task into Deep.
    let max_budget_usd = if strix_environment.full_power {
        None
    } else {
        input.max_budget_usd
    };
    let runtime_path = sentinel_runtime_path(&home);
    let docker = ensure_docker_ready(&home, &runtime_path)?;
    let reusing_scan = reuse_scan_id.is_some();
    let scan_id = reuse_scan_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let scan_work_root = state.app_data_dir.join("strix-jobs").join(&scan_id);
    fs::create_dir_all(&scan_work_root).map_err(|error| error.to_string())?;
    fs::write(scan_work_root.join(".oviraptor-scan-id"), &scan_id)
        .map_err(|error| error.to_string())?;
    let persisted_auth_path = scan_work_root.join("auth-session.json");
    let expected_authenticated = if reusing_scan {
        connection
            .query_row(
                "SELECT authenticated FROM sentinel_scan_contexts WHERE scan_id=?1",
                [&scan_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .unwrap_or(0)
            != 0
    } else {
        false
    };
    let mut auth_profile_name = input.auth_profile_name.trim().to_string();
    let mut auth_header_name = input.auth_header_name.trim().to_string();
    let mut auth_value = input.auth_value.clone();
    let mut auth_session_ids = input
        .auth_session_ids
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let mut auth_session_id = input.auth_session_id.trim().to_string();
    if !auth_session_id.is_empty() {
        auth_session_ids.push(auth_session_id.clone());
    }
    auth_session_ids.sort();
    auth_session_ids.dedup();
    if auth_session_ids.len() > 5 {
        return Err("单个灰盒任务最多比较 5 个登录身份".into());
    }
    if !reusing_scan && !auth_session_ids.is_empty() {
        crate::auth_session::validate_draft_sessions_for_task(
            &connection,
            &auth_session_ids,
            input.project_id,
            &input.auth_session_scope_id,
        )?;
    }
    auth_session_id = auth_session_ids.first().cloned().unwrap_or_default();
    let mut browser_auth_document = if scan_type == "greybox" && !auth_session_ids.is_empty() {
        let mut documents = crate::auth_session::distinct_session_documents_for_scan(
            &connection,
            &auth_session_ids,
            input.project_id,
        )?;
        Some(if documents.len() == 1 {
            documents.remove(0)
        } else {
            serde_json::json!({
                "schemaVersion":2,
                "kind":"identity-matrix",
                "sessions":documents,
                "comparisonPolicy":"same-target-same-action-plan",
                "identityIsolation":"dedicated-webview-and-distinct-auth-material"
            })
        })
    } else {
        None
    };
    if browser_auth_document.is_some() {
        auth_type = "browser_session".into();
        auth_profile_name = if auth_session_ids.len() > 1 {
            format!("{} 个浏览器身份矩阵", auth_session_ids.len())
        } else {
            connection
                .query_row(
                    "SELECT name FROM browser_auth_sessions WHERE id=?1",
                    [&auth_session_id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "浏览器登录会话".into())
        };
        auth_header_name.clear();
        auth_value.clear();
    } else if expected_authenticated && auth_value.trim().is_empty() {
        let persisted_auth = fs::read_to_string(&persisted_auth_path)
            .ok()
            .and_then(|text| serde_json::from_str::<JsonValue>(&text).ok())
            .ok_or_else(|| {
                "该灰盒任务的本地认证会话已不存在；为避免误以未登录状态重扫，请新建任务并重新填写认证信息"
                    .to_string()
            })?;
        if persisted_auth.get("sessions").and_then(JsonValue::as_array).is_some() {
            auth_type = "browser_session_matrix".into();
            auth_session_ids = persisted_auth
                .get("sessions")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .map(|value| value_first(value, &["id"]))
                .filter(|value| !value.is_empty())
                .collect();
            auth_session_id = auth_session_ids.first().cloned().unwrap_or_default();
            auth_profile_name = format!("{} 个浏览器身份矩阵", auth_session_ids.len());
            browser_auth_document = Some(persisted_auth);
        } else if persisted_auth.get("schemaVersion").and_then(JsonValue::as_i64) == Some(1) {
            auth_type = "browser_session".into();
            auth_session_id = value_first(&persisted_auth, &["id"]);
            auth_profile_name = "浏览器登录会话".into();
            browser_auth_document = Some(persisted_auth);
        } else {
            auth_type = value_first(&persisted_auth, &["type"]);
            auth_profile_name = value_first(&persisted_auth, &["profile"]);
            auth_header_name = value_first(&persisted_auth, &["headerName"]);
            auth_value = value_first(&persisted_auth, &["value"]);
            if !["cookie", "bearer", "header"].contains(&auth_type.as_str())
                || auth_value.trim().is_empty()
            {
                return Err("该灰盒任务保存的认证会话无效；请重新登录或新建任务".into());
            }
        }
    }
    let minimum_attempt = if reusing_scan {
        connection
            .query_row(
                "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
                [&scan_id],
                |row| row.get::<_, u32>(0),
            )
            .unwrap_or(0)
            .saturating_add(1)
    } else {
        1
    };
    let work_dir = next_scan_attempt_work_dir(&scan_work_root, minimum_attempt)?;
    fs::write(work_dir.join(".oviraptor-scan-id"), &scan_id).map_err(|error| error.to_string())?;
    let attempt_number = scan_attempt_number(&work_dir);
    let authenticated =
        browser_auth_document.is_some() || (auth_type != "none" && !auth_value.trim().is_empty());
    let auth_session_path = if authenticated {
        let path = work_dir.join("auth-session.json");
        let auth_document = browser_auth_document.unwrap_or_else(|| {
            serde_json::json!({"type":auth_type,"profile":auth_profile_name,"headerName":auth_header_name,"value":auth_value})
        });
        crate::auth_session::write_session_document(&persisted_auth_path, &auth_document)?;
        crate::auth_session::write_session_document(&path, &auth_document)?;
        Some(path)
    } else {
        None
    };
    let auth_instruction = if authenticated {
        " For this authorized grey-box task, read auth-session.json and apply cookies, storage and reusable authentication headers only to its scopeHosts. Browser-managed headers must be regenerated. Never print or copy credential values into reports. A single 401/403 is an authorization boundary, not a global stop condition; stop only on repeated confirmed WAF, bot challenge, CAPTCHA or rate-limit evidence."
    } else {
        ""
    };
    let web_contract_limit = web_mode_contract_limit(&scan_mode);
    let web_verifier_limit = web_mode_verifier_limit(&scan_mode);
    let web_discovery_passes = web_mode_discovery_passes(&scan_mode);
    let base_instruction = format!("The supplied URL and local source targets are explicitly authorized for defensive security testing. Preserve Strix native vulnerability verification, CVSS/CWE, remediation, evidence, and PoC workflow. Do not fabricate findings or classify reconnaissance-only observations as vulnerabilities. For web surfaces, each verifier must read the exact mounted frontend-evidence.json path before any request. Oviraptor has already explored rendered frontend states, captured runtime requests and parameters, parsed business JavaScript, built an investigation graph, and ranked hypotheses; do not repeat that inventory. Execute model-eligible investigation contracts in descending score order, up to {web_contract_limit} stable deduplicated contracts, using at most {web_verifier_limit} non-overlapping verifier agents. Obey each contract.requiredEvidence, contract.maxAttempts, contract.mutationPolicy, and contract.stopRules exactly. Oviraptor grants automatic bounded authorization for each contract's exact endpoint, method and maxAttempts: perform read-only and non-destructive control/test requests directly, clean up benign marker uploads, and never perform irreversible deletion, financial transactions, external messaging or persistent account/permission changes. Automatically close ordinary no-difference, exhausted, and routine 401/403 results without requesting human input, then continue the remaining queue. When no risk hypothesis is ready, use browser-observed API contracts for bounded coverage investigation. For framework applications, stop each branch at its attempt limit without a distinct response or security effect, then continue within the task cap. Identity differences are authorization candidates, not vulnerabilities, until the contract obtains a same-request control and cross-identity proof. Broad route crawling, framework inventory, bundle enumeration, and whole-site rediscovery are forbidden. Static frontends finish without code-slice exploration. Up to {web_discovery_passes} targeted discovery passes may be derived from distinct observed business words; a pass with no new verified endpoint ends fallback discovery. Treat isolated 401/403 responses as useful boundary evidence and continue other in-scope functions. Stop active discovery on confirmed WAF/bot challenge/CAPTCHA, sustained 429 or repeated homogeneous blocking responses. Never run recursive or repeated brute-force scans. If runtimeHookRecommended is true, use at most one narrowly scoped browser hook. Do not repeat JavaScript/vendor/framework inventory already completed by Oviraptor.{auth_instruction}");
    let instruction = format!(
        "{base_instruction}\n\n{skill_instructions}\n\n{}",
        input.instruction.trim()
    );
    let instruction_path = work_dir.join("strix-instruction.md");
    fs::write(&instruction_path, &instruction).map_err(|error| error.to_string())?;
    write_strix_prompt_audit(&work_dir, &instruction, &strix_environment)?;
    let task_path = state
        .app_data_dir
        .join("sentinel-tasks")
        .join(format!("{scan_id}.json"));
    if let Some(parent) = task_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let policy = serde_json::json!({"maxCritical":input.max_critical,"maxHigh":input.max_high,"blockRelease":input.block_release,"authSessionId":auth_session_id,"authSessionIds":auth_session_ids,"identityComparison":auth_session_ids.len()>1});
    let payload = serde_json::json!({"scanId":scan_id,"projectId":input.project_id,"projectName":project_name,"taskName":task_name,"scanType":scan_type,"attempt":attempt_number,"urls":urls,"sourcePath":source_path,"skills":skill_names,"scanMode":scan_mode,"scopeMode":scope_mode,"diffBase":input.diff_base,"maxBudgetUsd":max_budget_usd,"llmPolicy":{"model":strix_environment.llm,"deployment":strix_environment.deployment,"fullPower":strix_environment.full_power,"promptAuditMode":strix_environment.prompt_audit_mode},"runtimePolicy":strix_runtime_policy(&strix_cli,&strix_environment.image),"environment":environment,"authProfileName":auth_profile_name,"authType":auth_type,"authSessionId":auth_session_id,"authSessionIds":auth_session_ids,"authenticated":authenticated,"ciProvider":input.ci_provider.trim(),"repositoryUrl":input.repository_url.trim(),"branch":input.branch.trim(),"commitSha":input.commit_sha.trim(),"buildId":input.build_id.trim(),"policy":policy,"createdAt":chrono::Utc::now().to_rfc3339()});
    fs::write(
        &task_path,
        serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if reusing_scan {
        connection.execute(
            "UPDATE sentinel_scans SET project_id=?1,project_name=?2,status='scanning',current_checkpoint=?3,task_path=?4,previous_scan_id='',scan_type=?5,task_name=?6,source_path=?7,skill_names=?8,attempt_count=?9,updated_at=datetime('now','localtime') WHERE id=?10",
            params![input.project_id,project_name,format!("Strix 工作台正在当前任务中执行第 {attempt_number} 次尝试"),task_path.to_string_lossy(),scan_type,task_name,source_path,skill_names,attempt_number,scan_id],
        ).map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM sentinel_processes WHERE scan_id=?1",
                [&scan_id],
            )
            .map_err(|error| error.to_string())?;
        connection.execute("UPDATE sentinel_targets SET status='scanning',updated_at=datetime('now','localtime') WHERE scan_id=?1", [&scan_id]).map_err(|error| error.to_string())?;
    } else {
        connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name,source_path,skill_names,attempt_count) VALUES(?1,?2,?3,'scanning',?4,?5,?6,?7,?8,?9,?10)", params![scan_id,input.project_id,project_name,format!("Strix 工作台任务正在执行第 {attempt_number} 次尝试"),task_path.to_string_lossy(),scan_type,task_name,source_path,skill_names,attempt_number]).map_err(|error| error.to_string())?;
        crate::auth_session::bind_draft_sessions_to_scan(
            &connection,
            &auth_session_ids,
            input.project_id,
            &input.auth_session_scope_id,
            &scan_id,
        )?;
    }
    record_sentinel_attempt_start(&connection, &scan_id, attempt_number as i64, &work_dir)?;
    connection.execute("INSERT INTO sentinel_scan_contexts(scan_id,environment,auth_profile_name,auth_type,authenticated,ci_provider,repository_url,branch,commit_sha,build_id,policy_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(scan_id) DO UPDATE SET environment=excluded.environment,auth_profile_name=excluded.auth_profile_name,auth_type=excluded.auth_type,authenticated=excluded.authenticated,ci_provider=excluded.ci_provider,repository_url=excluded.repository_url,branch=excluded.branch,commit_sha=excluded.commit_sha,build_id=excluded.build_id,policy_json=excluded.policy_json,gate_status='',gate_reason='',updated_at=datetime('now','localtime')",params![scan_id,environment,auth_profile_name,auth_type,authenticated as i64,input.ci_provider.trim(),input.repository_url.trim(),input.branch.trim(),input.commit_sha.trim(),input.build_id.trim(),policy.to_string()]).map_err(|error|error.to_string())?;
    for target in urls
        .iter()
        .chain((!source_path.is_empty()).then_some(&source_path))
    {
        connection.execute("INSERT OR IGNORE INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,'scanning')", params![input.project_id,scan_id,project_name,target]).map_err(|error| error.to_string())?;
    }
    insert_source_inventory(&connection, &scan_id, &source_path)?;
    let result = sentinel_scan_by_id(&connection, &scan_id)?;
    launch_strix_workbench_pipeline(
        state.db_path.clone(),
        scan_id,
        strix,
        docker,
        work_dir,
        urls,
        source_path,
        instruction_path,
        scan_mode,
        scope_mode,
        input.diff_base.trim().to_string(),
        max_budget_usd,
        strix_environment,
        runtime_path,
        auth_session_path,
    );
    Ok(result)
}
