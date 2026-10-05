fn start_workbench_scan_impl(
    app: &AppHandle,
    state: &AppState,
    request: WorkbenchStartRequest,
) -> Result<SentinelScan, String> {
    let scan_id = match &request {
        WorkbenchStartRequest::New(_) => Uuid::new_v4().to_string(),
        WorkbenchStartRequest::Retry(id) => id.clone(),
    };
    let connection = db::open(&state.db_path)?;
    let _lifecycle = claim_scan_control(&state.db_path, &scan_id)?;
    let (input, retry_basis) = match request {
        WorkbenchStartRequest::New(input) => (*input, None),
        WorkbenchStartRequest::Retry(_) => {
            let (input, basis) = load_workbench_retry_input(&connection, &scan_id)?;
            (input, Some(basis))
        }
    };
    let reusing_scan = retry_basis.is_some();
    let scan_type = input.scan_type.trim().to_lowercase();
    if !["code", "greybox", "cicd"].contains(&scan_type.as_str()) {
        return Err("不支持的 Native Agent 扫描类型".into());
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
    let max_budget_usd = workbench_budget(input.max_budget_usd)?;
    source_model_cost_admission(max_budget_usd.is_some())?;
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
    let project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id=?1 AND status='active'",
            [input.project_id],
            |row| row.get(0),
        )
        .map_err(|_| "项目不存在或已归档；恢复工作空间后才能启动 Native Agent".to_string())?;
    // Only new tasks inherit enabled skills; retry preserves even an empty selection.
    let effective_skill_ids =
        workbench_selected_skill_ids(&connection, &input.skill_ids, !reusing_scan)?;
    let (skill_names, skill_instructions) =
        agent_skill_instructions(&connection, &effective_skill_ids)?;
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
    let model_environment = model_runtime_env(&settings)?;
    // Resolve only the dependencies selected by the frozen backend matrix. A Native-only
    // task must not probe a retired backend or silently add another dependency set.
    // “火力全开” controls throughput, never an explicit mode or cost ceiling.
    let runtime_path = sentinel_runtime_path(&home);
    // A terminal label is not proof that an old source/Web caller has exited.
    // Keep the previous invocation locks through publication and launch.
    let _previous_workers = if reusing_scan {
        let tx = rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate)
            .map_err(|e|e.to_string())?;
        let status:String = tx.query_row("SELECT status FROM sentinel_scans WHERE id=?1
            AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",[&scan_id],|r|r.get(0))
            .map_err(|_| "workbench_retry_scope_unavailable")?;
        if matches!(status.as_str(),"scanning"|"pausing"|"draft") {
            return Err("workbench_retry_state_ineligible".into());
        }
        let owners=claim_scan_quiescence_in(&tx,&state.db_path,&scan_id)?;
        tx.commit().map_err(|e|e.to_string())?;
        owners
    } else { Vec::new() };
    let scan_work_root = web_start_root(&state.app_data_dir, &scan_id)?;
    // New attempts keep credentials with their own evidence. Read the latest
    // published attempt on retry; only historical tasks use the old root file.
    let persisted_auth_path = if reusing_scan {
        let previous: Option<(String,String)> = connection.query_row(
            "SELECT a.work_dir,s.task_path FROM sentinel_scan_attempts a JOIN sentinel_scans s ON s.id=a.scan_id WHERE a.scan_id=?1 ORDER BY a.attempt_number DESC LIMIT 1",
            [&scan_id], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional().map_err(|e| e.to_string())?;
        match previous {
            Some((path,task)) if Path::new(&task) == Path::new(&path).join("task.json") => PathBuf::from(path).join("auth-session.json"),
            _ => scan_work_root.join("auth-session.json"),
        }
    } else { scan_work_root.join("auth-session.json") };
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
        Some(workbench_current_browser_auth(
            &connection,
            &auth_session_ids,
            input.project_id,
            reusing_scan.then_some(scan_id.as_str()),
        )?)
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
        if let Some((ids,document)) = restore_workbench_browser_auth(
            &connection,&persisted_auth,input.project_id,&scan_id,
        )? {
            auth_type = "browser_session".into();
            auth_session_ids = ids;
            auth_session_id = auth_session_ids.first().cloned().unwrap_or_default();
            auth_profile_name = if auth_session_ids.len() > 1 {
                format!("{} 个浏览器身份矩阵",auth_session_ids.len())
            } else { value_first(&document,&["name"]) };
            auth_header_name.clear();
            auth_value.clear();
            browser_auth_document = Some(document);
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
    let previous_attempt = if reusing_scan {
        connection
            .query_row(
                "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
                [&scan_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?
    } else {
        0
    };
    let attempt_number = web_start_attempt_number(&connection,&scan_id,&scan_work_root,previous_attempt)?;
    let mut startup_files = WebStartFiles::allocate(&scan_work_root,&scan_id,attempt_number)?;
    let work_dir = startup_files.path.clone();
    if !agent_native_eligible(&scan_type, &source_path, &urls) {
        return Err("unsupported_capability: 当前工作台任务没有 Native 执行入口".into());
    }
    // This is a fresh attempt. Freeze its Native matrix only together with the
    // task/attempt, not through another connection during preparation.
    let mut backend_matrix = ScanBackendPlan {
        scan_id: scan_id.clone(), attempt_number: attempt_number.into(),
        targets: urls.iter().map(|url| ScanTargetBackend {
            url: url.clone(), backend: AgentBackendKind::Native,
            selection_reason: format!("attempt {attempt_number} 使用唯一可执行的 native 后端"),
        }).collect(), requires_node: false, requires_browser: false,
    };
    backend_matrix.refresh_requirements(true);
    prepare_scan_dependencies(&backend_matrix)?;
    let authenticated =
        browser_auth_document.is_some() || (auth_type != "none" && !auth_value.trim().is_empty());
    let auth_session_path = if authenticated {
        let path = work_dir.join("auth-session.json");
        let auth_document = browser_auth_document.unwrap_or_else(|| {
            serde_json::json!({"type":auth_type,"profile":auth_profile_name,"headerName":auth_header_name,"value":auth_value})
        });
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
    let base_instruction = format!("The supplied URL and local source targets are explicitly authorized for defensive security testing. Preserve the Native Agent vulnerability-verification, CVSS/CWE, remediation, evidence, and PoC workflow. Do not fabricate findings or classify reconnaissance-only observations as vulnerabilities. For web surfaces, each verifier must read the exact mounted frontend-evidence.json path before any request. Oviraptor has already explored rendered frontend states, captured runtime requests and parameters, parsed business JavaScript, built an investigation graph, and ranked hypotheses; do not repeat that inventory. Execute model-eligible investigation contracts in descending score order, up to {web_contract_limit} stable deduplicated contracts, using at most {web_verifier_limit} non-overlapping verifier agents. Obey each contract.requiredEvidence, contract.maxAttempts, contract.mutationPolicy, and contract.stopRules exactly. Oviraptor grants automatic bounded authorization for each contract's exact endpoint, method and maxAttempts: perform read-only and non-destructive control/test requests directly, clean up benign marker uploads, and never perform irreversible deletion, financial transactions, external messaging or persistent account/permission changes. Automatically close ordinary no-difference, exhausted, and routine 401/403 results without requesting human input, then continue the remaining queue. When no risk hypothesis is ready, use browser-observed API contracts for bounded coverage investigation. For framework applications, stop each branch at its attempt limit without a distinct response or security effect, then continue within the task cap. Identity differences are authorization candidates, not vulnerabilities, until the contract obtains a same-request control and cross-identity proof. Broad route crawling, framework inventory, bundle enumeration, and whole-site rediscovery are forbidden. Static frontends finish without code-slice exploration. Up to {web_discovery_passes} targeted discovery passes may be derived from distinct observed business words; a pass with no new verified endpoint ends fallback discovery. Treat isolated 401/403 responses as useful boundary evidence and continue other in-scope functions. Stop active discovery on confirmed WAF/bot challenge/CAPTCHA, sustained 429 or repeated homogeneous blocking responses. Never run recursive or repeated brute-force scans. If runtimeHookRecommended is true, use at most one narrowly scoped browser hook. Do not repeat JavaScript/vendor/framework inventory already completed by Oviraptor.{auth_instruction}");
    let instruction = AgentInstruction::new(format!(
        "{base_instruction}\n\n{skill_instructions}\n\n{}",
        input.instruction.trim()
    ));
    instruction.write_to(&work_dir)?;
    write_model_prompt_audit(&work_dir, instruction.as_str(), &model_environment)?;
    let task_path = work_dir.join("task.json");
    let mut policy = build_workbench_investigation_policy(&scan_mode,max_budget_usd,auth_session_ids.clone(),&effective_skill_ids,input.instruction.trim())?;
    policy["maxCritical"] = input.max_critical.into();
    policy["maxHigh"] = input.max_high.into();
    policy["blockRelease"] = input.block_release.into();
    let payload = serde_json::json!({"scanId":scan_id,"projectId":input.project_id,"projectName":project_name,"taskName":task_name,"scanType":scan_type,"attempt":attempt_number,"urls":urls,"sourcePath":source_path,"skills":skill_names,"scanMode":scan_mode,"scopeMode":scope_mode,"diffBase":input.diff_base,"maxBudgetUsd":max_budget_usd,"llmPolicy":{"model":model_environment.llm,"deployment":model_environment.deployment,"fullPower":model_environment.full_power,"promptAuditMode":model_environment.prompt_audit_mode},"runtimePolicy":{"backend":"native-agent"},"environment":environment,"authProfileName":auth_profile_name,"authType":auth_type,"authSessionId":auth_session_id,"authSessionIds":auth_session_ids,"authenticated":authenticated,"ciProvider":input.ci_provider.trim(),"repositoryUrl":input.repository_url.trim(),"branch":input.branch.trim(),"commitSha":input.commit_sha.trim(),"buildId":input.build_id.trim(),"policy":policy,"createdAt":chrono::Utc::now().to_rfc3339()});
    let mut payload = payload;
    payload["retryBasis"] = json!(retry_basis);
    fs::write(
        &task_path,
        serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let record: WorkbenchStartRecord = serde_json::from_value(payload).map_err(|e| e.to_string())?;
    let web_worker = if urls.is_empty() { PathBuf::new() } else { resolve_frontend_recon_worker(app)? };
    let publication = publish_workbench_start(
        &connection, &record, reusing_scan, &input.auth_session_scope_id,
        &work_dir, &backend_matrix, web_worker,
        &model_environment,
    );
    // A commit I/O error can have an unknown outcome. Preserve its files for
    // reconciliation; never delete evidence that may belong to a durable task.
    if publication.is_ok() || publication.as_ref().is_err_and(|e| e.starts_with("workbench_start_commit_unconfirmed:")) {
        startup_files.preserve = true;
    }
    let (_, agent_runtime) = publication?;
    // Both task-creating entries decide from the same frozen matrix, and a fully
    // native task runs the same per-target pipeline as an asset task.
    if backend_matrix
        .targets
        .iter()
        .all(|target| target.backend == AgentBackendKind::Native)
    {
        let agent_targets = urls
            .iter()
            .map(|url| (project_name.clone(), url.clone()))
            .collect::<Vec<_>>();
        append_runner_log(
            &work_dir.join("oviraptor-runner.log"),
            &format!(
                "Agent 协作台 URL 任务使用 Native Agent 后端：{} 个目标，与资产任务共用同一编排与状态逻辑",
                agent_targets.len()
            ),
        );
        let source_branch = if source_path.trim().is_empty() {
            None
        } else {
            Some((source_path.clone(), scan_type.clone(), input.diff_base.trim().to_string()))
        };
        let url_branch = if agent_targets.is_empty() {
            None
        } else {
            Some(agent_targets)
        };
        if let Some((path, kind, base)) = source_branch {
            let spawned = launch_native_source_pipeline(
                state.db_path.clone(),
                state.app_data_dir.clone(),
                scan_id.clone(),
                attempt_number as i64,
                work_dir.clone(),
                path,
                kind,
                base,
            );
            record_workbench_spawn_result(&state.db_path,&scan_id,attempt_number.into(),"source",spawned)?;
        }
        if let Some(targets) = url_branch {
            let spawned = launch_sentinel_url_pipeline(
                state.db_path.clone(),
                scan_id.clone(),
                attempt_number as i64,
                agent_runtime.worker,
                work_dir.clone(),
                targets,
                agent_runtime.proxies,
                agent_runtime.no_proxy,
                model_environment,
                runtime_path,
                agent_runtime.adaptive,
                agent_runtime.packet_budget,
                auth_session_path,
                None,
            );
            record_workbench_spawn_result(&state.db_path,&scan_id,attempt_number.into(),"web",spawned)?;
        }
        return sentinel_scan_by_id(&connection,&scan_id);
    }
    Err("backend_retired: 历史执行计划不可继续；请创建新的 Native 尝试".into())
}
