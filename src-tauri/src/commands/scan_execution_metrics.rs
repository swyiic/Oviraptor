#[derive(Default)]
struct LiveStrixMetrics {
    requests: i64,
    maintenance_requests: i64,
    failed_requests: i64,
    context_errors: i64,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    total_tokens: i64,
    meaningful_tools: usize,
    unique_tool_results: usize,
    verification_tool_results: usize,
    max_tool_repeats: usize,
    directory_discovery_calls: usize,
    directory_block_signals: usize,
    // A coordinator can legitimately make several model calls while a
    // delegated verifier is still running. Those calls do not themselves
    // produce tool results, so they must not trip the no-progress fuse.
    active_child_agents: usize,
    waiting_on_agents: bool,
    latest_event: String,
    last_model_error: String,
    model_requests_in_flight: i64,
    model_in_flight_input_tokens: i64,
}

fn uncached_strix_tokens(metrics: &LiveStrixMetrics) -> i64 {
    metrics
        .input_tokens
        .saturating_sub(metrics.cached_tokens)
        .saturating_add(metrics.output_tokens)
}

fn usage_cached_tokens(usage: &JsonValue) -> i64 {
    let direct = [
        "input_tokens_details",
        "prompt_tokens_details",
        "inputTokensDetails",
        "promptTokensDetails",
    ]
    .into_iter()
    .find_map(|key| usage.get(key))
    .and_then(|details| {
        details
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        usage_number(
                            item,
                            &[
                                "cached_tokens",
                                "cachedTokens",
                                "cache_read_input_tokens",
                                "cacheReadInputTokens",
                            ],
                        )
                    })
                    .sum()
            })
            .or_else(|| {
                Some(usage_number(
                    details,
                    &[
                        "cached_tokens",
                        "cachedTokens",
                        "cache_read_input_tokens",
                        "cacheReadInputTokens",
                    ],
                ))
            })
    })
    .or_else(|| {
        Some(usage_number(
            usage,
            &[
                "cached_tokens",
                "cachedTokens",
                "cache_read_input_tokens",
                "cacheReadInputTokens",
            ],
        ))
    })
    .unwrap_or(0);
    if direct > 0 {
        return direct;
    }
    usage
        .get("request_usage_entries")
        .and_then(JsonValue::as_array)
        .map(|items| items.iter().map(usage_cached_tokens).sum())
        .unwrap_or(0)
}

fn usage_number(usage: &JsonValue, keys: &[&str]) -> i64 {
    for key in keys {
        let Some(value) = usage.get(*key) else {
            continue;
        };
        let value = match value {
            JsonValue::Number(number) => number.as_i64().unwrap_or(0),
            JsonValue::String(text) => text.trim().parse::<i64>().unwrap_or(0),
            _ => 0,
        };
        if value > 0 {
            return value;
        }
    }
    0
}

fn usage_request_count(usage: &JsonValue) -> i64 {
    let direct = usage_number(usage, &["requests", "request_count", "requestCount"]);
    if direct > 0 {
        return direct;
    }
    usage
        .get("request_usage_entries")
        .and_then(JsonValue::as_array)
        .map(|items| items.len() as i64)
        .unwrap_or(0)
}

fn usage_input_tokens(usage: &JsonValue) -> i64 {
    let direct = usage_number(
        usage,
        &[
            "input_tokens",
            "prompt_tokens",
            "inputTokens",
            "promptTokens",
        ],
    );
    if direct > 0 {
        return direct;
    }
    usage
        .get("request_usage_entries")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    usage_number(
                        item,
                        &[
                            "input_tokens",
                            "prompt_tokens",
                            "inputTokens",
                            "promptTokens",
                        ],
                    )
                })
                .sum()
        })
        .unwrap_or(0)
}

fn usage_output_tokens(usage: &JsonValue) -> i64 {
    let direct = usage_number(
        usage,
        &[
            "output_tokens",
            "completion_tokens",
            "outputTokens",
            "completionTokens",
        ],
    );
    if direct > 0 {
        return direct;
    }
    usage
        .get("request_usage_entries")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    usage_number(
                        item,
                        &[
                            "output_tokens",
                            "completion_tokens",
                            "outputTokens",
                            "completionTokens",
                        ],
                    )
                })
                .sum()
        })
        .unwrap_or(0)
}

fn usage_total_tokens(usage: &JsonValue) -> i64 {
    let direct = usage_number(usage, &["total_tokens", "totalTokens"]);
    if direct > 0 {
        direct
    } else {
        usage_input_tokens(usage) + usage_output_tokens(usage)
    }
}

fn is_meaningful_strix_tool(tool: &str) -> bool {
    !tool.trim().is_empty()
        && !matches!(
            tool,
            "think"
                | "agent_finish"
                | "wait_for_message"
                | "view_agent_graph"
                | "stop_agent"
                | "load_skill"
                | "list_notes"
                | "list_requests"
                | "scope_rules"
                | "create_todo"
                | "update_todo"
                | "create_note"
                | "create_agent"
                | "create_dependency_report"
                | "finish_scan"
        )
}

fn strix_tool_invocation_key(name: &str, arguments: &str) -> String {
    let normalized_arguments = serde_json::from_str::<JsonValue>(arguments)
        .ok()
        .and_then(|value| serde_json::to_string(&value).ok())
        .unwrap_or_else(|| arguments.split_whitespace().collect::<Vec<_>>().join(" "));
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(b"\0");
    hasher.update(normalized_arguments.as_bytes());
    format!("{name}:{:x}", hasher.finalize())
}

fn is_target_verification_tool(name: &str, arguments: &str) -> bool {
    let tool = name.trim().to_ascii_lowercase();
    if [
        "browser_request",
        "http_request",
        "send_request",
        "replay_request",
        "repeat_request",
        "caido_request",
        "raw_http",
        "race_request",
    ]
    .iter()
    .any(|candidate| tool == *candidate || tool.contains(candidate))
    {
        return true;
    }
    if !matches!(tool.as_str(), "exec_command" | "shell" | "terminal" | "python") {
        return false;
    }
    let command = arguments.to_ascii_lowercase();
    [
        "curl ",
        "agent-browser ",
        "httpie ",
        "nuclei ",
        "ffuf ",
        "gobuster ",
        "feroxbuster ",
        "dirsearch ",
        "/adapter/",
        "rust-native-race-scheduler",
    ]
    .iter()
    .any(|marker| command.contains(marker))
}

fn target_verification_output_is_usable(name: &str, output: &str) -> bool {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if [
        "could not resolve host",
        "connection refused",
        "failed to connect",
        "operation timed out",
        "request timed out",
        "no such file or directory",
        "command not found",
        "request not found",
        "\"success\": false",
        "\"success\":false",
        "traceback (most recent call last)",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return false;
    }
    if let Some((_, tail)) = lower.rsplit_once("process exited with code ") {
        let code = tail
            .split(|character: char| !character.is_ascii_digit())
            .next()
            .unwrap_or("");
        if !code.is_empty() && code != "0" {
            return false;
        }
    }
    let tool = name.trim().to_ascii_lowercase();
    if tool.contains("repeat_request") {
        return serde_json::from_str::<JsonValue>(trimmed)
            .ok()
            .and_then(|value| value.get("success").and_then(JsonValue::as_bool))
            .unwrap_or_else(|| lower.contains("\"response\"") && !lower.contains("error"));
    }
    // Successful shell wrappers include process metadata plus a response body;
    // a bare exit-code line proves command execution, not a target response.
    if matches!(tool.as_str(), "exec_command" | "shell" | "terminal" | "python") {
        let content_lines = trimmed
            .lines()
            .filter(|line| {
                let line = line.trim().to_ascii_lowercase();
                !line.is_empty()
                    && !line.starts_with("chunk id:")
                    && !line.starts_with("wall time:")
                    && !line.starts_with("process exited with code")
                    && line != "final output:"
            })
            .count();
        return content_lines > 0;
    }
    true
}

/// Directory discovery is a bounded exception for ordinary server-rendered
/// Web targets. Recognize both first-class tools and shell wrappers so the
/// runtime fuse can stop repeated wordlist scans.
fn is_directory_discovery_tool(name: &str, detail: &str) -> bool {
    let haystack = format!("{} {}", name, detail).to_ascii_lowercase();
    [
        "ffuf",
        "dirsearch",
        "gobuster",
        "feroxbuster",
        "ferox",
        "wfuzz",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn is_directory_block_signal(output: &str) -> bool {
    let value = output.to_ascii_lowercase();
    [
        "429 too many requests",
        "status: 429",
        "status_code\":429",
        "rate limit",
        "captcha",
        "cloudflare challenge",
        "cloudflare ray id",
        "cf-chl-",
        "aws waf",
        "akamai reference",
        "incapsula incident",
        "verify you are human",
        "waf blocked",
        "web application firewall",
        "js challenge",
        "验证码",
        "人机验证",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn hard_fuse_reason(reason: &str) -> bool {
    is_directory_block_signal(reason)
}

fn live_strix_metrics(work_dir: &Path) -> LiveStrixMetrics {
    let mut metrics = LiveStrixMetrics::default();
    let hook_usage = llm_hook::usage_from_file(&work_dir.join("llm-hook.jsonl"));
    metrics.model_requests_in_flight = hook_usage.in_flight_requests;
    metrics.model_in_flight_input_tokens = hook_usage.in_flight_input_tokens;
    let use_hook_usage = hook_usage.requests > 0 || hook_usage.failed_requests > 0;
    if use_hook_usage {
        metrics.requests = hook_usage
            .requests
            .saturating_sub(hook_usage.maintenance_requests);
        metrics.maintenance_requests = hook_usage.maintenance_requests;
        metrics.failed_requests = hook_usage.failed_requests;
        metrics.context_errors = hook_usage.context_errors;
        metrics.input_tokens = hook_usage.input_tokens;
        metrics.output_tokens = hook_usage.output_tokens;
        metrics.cached_tokens = hook_usage.cached_tokens;
        metrics.total_tokens = hook_usage.total_tokens;
        metrics.last_model_error = hook_usage.last_error;
    }
    let Ok(run_dirs) = strix_run_dirs(work_dir) else {
        return metrics;
    };
    let mut result_fingerprints = HashSet::new();
    let mut verification_result_fingerprints = HashSet::new();
    let mut tool_invocations: HashMap<String, usize> = HashMap::new();
    for dir in run_dirs {
        if let Ok(bytes) = fs::read(dir.join(STRIX_RUN_ARTIFACT)) {
            if let Ok(run) = serde_json::from_slice::<JsonValue>(&bytes) {
                let usage = run.get("llm_usage").unwrap_or(&JsonValue::Null);
                if !use_hook_usage {
                    metrics.requests += usage_request_count(usage);
                    metrics.input_tokens += usage_input_tokens(usage);
                    metrics.output_tokens += usage_output_tokens(usage);
                    metrics.cached_tokens += usage_cached_tokens(usage);
                    metrics.total_tokens += usage_total_tokens(usage);
                }
            }
        }
        // Read orchestration state from the run directory, not the target
        // directory. A root coordinator may wait for a live child verifier
        // while no new tool result has landed; that is progress and must not
        // trip the no-progress fuse.
        if let Ok(bytes) = fs::read(strix_agent_state_path(&dir)) {
            if let Ok(state) = serde_json::from_slice::<JsonValue>(&bytes) {
                let statuses = state.get("statuses").and_then(JsonValue::as_object);
                let parents = state.get("parent_of").and_then(JsonValue::as_object);
                if let (Some(statuses), Some(parents)) = (statuses, parents) {
                    for (agent_id, status) in statuses {
                        let Some(parent) = parents.get(agent_id) else { continue };
                        if parent.is_null() { continue; }
                        let status = status.as_str().unwrap_or_default().to_ascii_lowercase();
                        if matches!(status.as_str(), "running" | "starting" | "waiting") {
                            metrics.active_child_agents += 1;
                        }
                    }
                }
                metrics.waiting_on_agents = metrics.waiting_on_agents
                    || state
                        .get("wait_kinds")
                        .and_then(JsonValue::as_object)
                        .map(|items| items.values().any(|value| value.as_str() == Some("agents")))
                        .unwrap_or(false);
            }
        }
        let agents_path = strix_agent_state_path(&dir);
        let mut structured_tools = false;
        if let Ok(agent_db) = rusqlite::Connection::open(&agents_path) {
            if let Ok(mut statement) =
                agent_db.prepare(STRIX_AGENT_MESSAGES_QUERY)
            {
                let rows = statement.query_map([], |row| row.get::<_, String>(0));
                if let Ok(rows) = rows {
                    let mut call_details: HashMap<String, (String, bool)> = HashMap::new();
                    for raw in rows.flatten() {
                        let message = json(raw);
                        let event_type = message
                            .get("type")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("message");
                        let call_id = message
                            .get("call_id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("");
                        let mut name = message
                            .get("name")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("")
                            .to_string();
                        if event_type == "function_call" {
                            let arguments = message
                                .get("arguments")
                                .and_then(JsonValue::as_str)
                                .unwrap_or("");
                            if !call_id.is_empty() && !name.is_empty() {
                                call_details.insert(
                                    call_id.to_string(),
                                    (name.clone(), is_target_verification_tool(&name, arguments)),
                                );
                            }
                            if is_meaningful_strix_tool(&name) {
                                structured_tools = true;
                                metrics.meaningful_tools += 1;
                                *tool_invocations
                                    .entry(strix_tool_invocation_key(&name, arguments))
                                    .or_default() += 1;
                                if is_directory_discovery_tool(
                                    &name,
                                    arguments,
                                ) {
                                    metrics.directory_discovery_calls += 1;
                                }
                                metrics.latest_event = format!("正在调用工具 {name}");
                            }
                        } else if event_type == "function_call_output" {
                            let verification_call = call_details
                                .get(call_id)
                                .map(|(_, verification)| *verification)
                                .unwrap_or(false);
                            if name.is_empty() {
                                name = call_details
                                    .get(call_id)
                                    .map(|(name, _)| name.clone())
                                    .unwrap_or_default();
                            }
                            if is_meaningful_strix_tool(&name) {
                                structured_tools = true;
                                let output = message
                                    .get("output")
                                    .and_then(JsonValue::as_str)
                                    .unwrap_or("");
                                if is_directory_discovery_tool(&name, "")
                                    && is_directory_block_signal(output)
                                {
                                    metrics.directory_block_signals += 1;
                                }
                                let mut hasher = Sha256::new();
                                hasher.update(name.as_bytes());
                                hasher.update(b"\0");
                                hasher.update(output.as_bytes());
                                let fingerprint = format!("{:x}", hasher.finalize());
                                result_fingerprints.insert(fingerprint.clone());
                                if verification_call
                                    && target_verification_output_is_usable(&name, output)
                                {
                                    verification_result_fingerprints.insert(fingerprint);
                                }
                                metrics.latest_event = format!("工具 {name} 已返回，正在判断证据");
                            }
                        } else if event_type == "reasoning" {
                            metrics.latest_event = "正在分析现有响应并选择下一步".into();
                        } else if message.get("role").and_then(JsonValue::as_str)
                            == Some("assistant")
                        {
                            metrics.latest_event = "正在整理当前阶段结论".into();
                        }
                    }
                }
            }
        }
        if !structured_tools {
            if let Ok(log) = fs::read_to_string(dir.join("strix.log")) {
                for line in log
                    .lines()
                    .filter(|line| line.contains("Tool ") && line.contains(" completed"))
                {
                    let tool = line
                        .split("Tool ")
                        .nth(1)
                        .unwrap_or("")
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_end_matches('.');
                    if is_meaningful_strix_tool(tool) {
                        metrics.meaningful_tools += 1;
                        *tool_invocations.entry(tool.to_string()).or_default() += 1;
                        if is_directory_discovery_tool(tool, line) {
                            metrics.directory_discovery_calls += 1;
                        }
                        metrics.latest_event = format!("工具 {tool} 已完成");
                    }
                }
            }
        }
    }
    metrics.unique_tool_results = result_fingerprints.len();
    metrics.verification_tool_results = verification_result_fingerprints.len();
    metrics.max_tool_repeats = tool_invocations.values().copied().max().unwrap_or(0);
    if metrics.latest_event.is_empty() {
        metrics.latest_event = if metrics.context_errors > 0 {
            "模型拒绝请求：上下文窗口不足".into()
        } else if metrics.failed_requests > 0 {
            "模型接口返回错误，正在停止当前 URL".into()
        } else if metrics.requests > 0 {
            "等待 Strix 写入结构化工具事件".into()
        } else {
            "正在启动 Strix Agent".into()
        };
    }
    metrics
}

fn aggregate_hook_usage(root: &Path) -> llm_hook::UsageTotals {
    fn walk(path: &Path, depth: usize, totals: &mut llm_hook::UsageTotals) {
        if !path.is_dir() || depth == 0 {
            return;
        }
        let usage = llm_hook::usage_from_file(&path.join("llm-hook.jsonl"));
        totals.requests += usage.requests;
        totals.maintenance_requests += usage.maintenance_requests;
        totals.failed_requests += usage.failed_requests;
        totals.maintenance_failed_requests += usage.maintenance_failed_requests;
        totals.context_errors += usage.context_errors;
        totals.input_tokens += usage.input_tokens;
        totals.output_tokens += usage.output_tokens;
        totals.cached_tokens += usage.cached_tokens;
        totals.total_tokens += usage.total_tokens;
        totals.in_flight_requests += usage.in_flight_requests;
        totals.in_flight_input_tokens += usage.in_flight_input_tokens;
        if !usage.last_error.is_empty() {
            totals.last_error = usage.last_error;
        }
        if let Ok(entries) = fs::read_dir(path) {
            for child in entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
            {
                walk(&child, depth - 1, totals);
            }
        }
    }
    let mut totals = llm_hook::UsageTotals::default();
    walk(root, 8, &mut totals);
    totals
}

fn persist_hook_usage(db_path: &Path, scan_id: &str, root: &Path) {
    let usage = aggregate_hook_usage(root);
    if usage.requests <= 0 {
        return;
    }
    if let Ok(connection) = db::open(db_path) {
        let _ = connection.execute(
            "UPDATE sentinel_scans SET llm_requests=MAX(llm_requests,?1),input_tokens=MAX(input_tokens,?2),output_tokens=MAX(output_tokens,?3),cached_tokens=MAX(cached_tokens,?4),total_tokens=MAX(total_tokens,?5),updated_at=datetime('now','localtime') WHERE id=?6",
            params![usage.requests, usage.input_tokens, usage.output_tokens, usage.cached_tokens, usage.total_tokens, scan_id],
        );
        sync_sentinel_attempt(&connection, scan_id);
    }
}
