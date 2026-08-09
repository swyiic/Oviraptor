#[derive(Clone)]
struct StrixRuntimeEnv {
    llm: String,
    api_key: String,
    api_base: String,
    deployment: String,
    full_power: bool,
    prompt_audit_mode: String,
}

fn shell_assignment(path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines().find_map(|line| {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        let value = value
            .trim()
            .trim_matches(|character| character == '\'' || character == '"');
        (!value.is_empty() && !value.chars().any(char::is_control)).then(|| value.to_string())
    })
}

fn strix_cli_env(home: &Path) -> JsonValue {
    fs::read(home.join(".strix/cli-config.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok())
        .and_then(|value| value.get("env").cloned())
        .unwrap_or_default()
}

fn active_strix_llm_profile(settings: &JsonValue) -> Option<&JsonValue> {
    let profiles = settings.get("strixLlmProfiles")?.as_array()?;
    let active_id = settings
        .get("strixActiveLlmProfileId")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    profiles
        .iter()
        .find(|profile| {
            !active_id.is_empty()
                && profile.get("id").and_then(JsonValue::as_str) == Some(active_id)
        })
        .or_else(|| profiles.first())
}

fn strix_runtime_env(settings: &JsonValue, home: &Path) -> Result<StrixRuntimeEnv, String> {
    let cli = strix_cli_env(home);
    let active_profile = active_strix_llm_profile(settings);
    let deployment = active_profile
        .and_then(|profile| profile.get("deployment"))
        .and_then(JsonValue::as_str)
        .filter(|value| *value == "local")
        .unwrap_or("cloud")
        .to_string();
    let full_power = deployment == "local"
        && settings
            .get("strixLocalFullPower")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
    let prompt_audit_mode = match settings
        .get("strixPromptAuditMode")
        .and_then(JsonValue::as_str)
        .unwrap_or("off")
    {
        "metadata" => "metadata",
        "full" => "full",
        _ => "off",
    }
    .to_string();
    let setting = |key: &str| {
        settings
            .get(key)
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let profile_setting = |key: &str, legacy_key: &str| {
        active_profile
            .and_then(|profile| profile.get(key))
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| setting(legacy_key))
    };
    let cli_value = |key: &str| {
        cli.get(key)
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let llm = {
        let configured = profile_setting("llm", "strixLlm");
        if !configured.is_empty() {
            configured
        } else if let Ok(value) = std::env::var("STRIX_LLM") {
            value.trim().to_string()
        } else {
            shell_assignment(&home.join(".zshrc"), "STRIX_LLM").unwrap_or_default()
        }
    };
    if llm.is_empty() {
        return Err(
            "Strix 模型未配置：请在配置中心填写 STRIX_LLM（例如 openai/模型名），任务尚未启动"
                .into(),
        );
    }
    let api_key = if deployment == "local" {
        let configured = active_profile
            .and_then(|profile| profile.get("apiKey"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if configured.is_empty() {
            "local".to_string()
        } else {
            configured
        }
    } else {
        let configured = profile_setting("apiKey", "strixApiKey");
        if !configured.is_empty() {
            configured
        } else {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| cli_value("OPENAI_API_KEY"))
        }
    };
    let api_base = if deployment == "local" {
        active_profile
            .and_then(|profile| profile.get("apiBase"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    } else {
        let configured = profile_setting("apiBase", "strixApiBase");
        if !configured.is_empty() {
            configured
        } else {
            std::env::var("OPENAI_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| cli_value("OPENAI_BASE_URL"))
        }
    };
    if deployment == "local" && api_base.is_empty() {
        return Err(
            "本地 Strix 模型必须配置 OPENAI_BASE_URL，防止误用云端环境变量或 CLI 凭据".into(),
        );
    }
    if api_key.is_empty() {
        return Err(
            "Strix API 凭据未配置：请在配置中心填写 OPENAI_API_KEY（第三方 OpenAI 兼容服务同时填写 OPENAI_BASE_URL）".into(),
        );
    }
    Ok(StrixRuntimeEnv {
        llm,
        api_key,
        api_base,
        deployment,
        full_power,
        prompt_audit_mode,
    })
}

fn command_strix_env(command: &mut Command, environment: &StrixRuntimeEnv) {
    command.env("STRIX_LLM", &environment.llm);
    if !environment.api_key.is_empty() {
        command.env("OPENAI_API_KEY", &environment.api_key);
    }
    if !environment.api_base.is_empty() {
        command.env("OPENAI_BASE_URL", &environment.api_base);
    }
}

fn strix_hook_api_base(environment: &StrixRuntimeEnv) -> String {
    if !environment.api_base.trim().is_empty() {
        environment
            .api_base
            .trim()
            .trim_end_matches('/')
            .to_string()
    } else if environment.llm.starts_with("openai/") {
        "https://api.openai.com/v1".into()
    } else {
        String::new()
    }
}

fn write_strix_prompt_audit(
    work_dir: &Path,
    instruction: &str,
    environment: &StrixRuntimeEnv,
) -> Result<(), String> {
    if environment.prompt_audit_mode == "off" {
        return Ok(());
    }
    let mut hasher = Sha256::new();
    hasher.update(instruction.as_bytes());
    let audit = StrixPromptAudit {
        capture_mode: environment.prompt_audit_mode.clone(),
        source: "oviraptor_generated_instruction".into(),
        capture_level: "generated_instruction".into(),
        exact_model_request: false,
        model: environment.llm.clone(),
        deployment: environment.deployment.clone(),
        full_power: environment.full_power,
        recorded_at: chrono::Utc::now().to_rfc3339(),
        instruction_sha256: format!("{:x}", hasher.finalize()),
        instruction_chars: instruction.chars().count() as i64,
        instruction: (environment.prompt_audit_mode == "full")
            .then(|| retained_trace_text(instruction)),
        notice: "这是 Oviraptor 生成并交给 Strix 的 instruction 快照，不是 Strix 最终发送给模型的完整请求。system、developer、tool schema 与最终对话组装需要 Strix 请求层 Hook 才能精确捕获。".into(),
    };
    fs::write(
        work_dir.join("strix-prompt-audit.json"),
        serde_json::to_vec_pretty(&audit).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn test_strix_llm(
    state: State<AppState>,
    input: StrixLlmTestInput,
) -> Result<StrixLlmTestResult, String> {
    let deployment = match input.deployment.trim() {
        "local" => "local",
        "cloud" => "cloud",
        _ => return Err("模型部署类型必须是 cloud 或 local".into()),
    };
    let model = input.llm.trim().to_string();
    if model.is_empty() {
        return Err("请填写 STRIX_LLM".into());
    }
    let home = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .to_path_buf();
    let cli = strix_cli_env(&home);
    let cli_value = |key: &str| {
        cli.get(key)
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let base = input.api_base.trim().trim_end_matches('/').to_string();
    let base = if !base.is_empty() {
        base
    } else if deployment == "cloud" {
        std::env::var("OPENAI_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| cli_value("OPENAI_BASE_URL"))
            .trim()
            .trim_end_matches('/')
            .to_string()
    } else {
        String::new()
    };
    let base = if base.is_empty() && deployment == "cloud" {
        "https://api.openai.com/v1".to_string()
    } else {
        base
    };
    if base.is_empty()
        || !(base.starts_with("http://") || base.starts_with("https://"))
        || base.chars().any(char::is_control)
    {
        return Err("请填写有效的 OPENAI_BASE_URL".into());
    }
    let api_key = if input.api_key.trim().is_empty() {
        if deployment == "local" {
            "local".to_string()
        } else {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| cli_value("OPENAI_API_KEY"))
        }
    } else {
        input.api_key.trim().to_string()
    };
    if api_key.is_empty() {
        return Err("云端模型测试必须填写 OPENAI_API_KEY，或先配置 Strix CLI 凭据".into());
    }
    let chat_model = openai_chat_completion_model(&model);
    if chat_model.is_empty() {
        return Err("STRIX_LLM 中的模型名不能为空".into());
    }
    let endpoint = format!("{base}/chat/completions");
    let temp_id = Uuid::new_v4();
    let temp_dir = std::env::temp_dir();
    let header_path = temp_dir.join(format!("oviraptor-llm-test-{temp_id}.headers"));
    let body_path = temp_dir.join(format!("oviraptor-llm-test-{temp_id}.json"));
    let response_path = temp_dir.join(format!("oviraptor-llm-test-{temp_id}.response"));
    write_private_temp_file(
        &header_path,
        format!("Authorization: Bearer {api_key}\nContent-Type: application/json\n").as_bytes(),
    )
    .map_err(|error| format!("无法创建模型测试 Header：{error}"))?;
    let request_body = serde_json::to_vec(&serde_json::json!({
        "model": chat_model,
        "messages": [{"role": "user", "content": "Reply with OK."}],
        "temperature": 0,
        "max_tokens": 4,
        "stream": false
    }))
    .map_err(|error| error.to_string())?;
    if let Err(error) = write_private_temp_file(&body_path, &request_body) {
        let _ = fs::remove_file(&header_path);
        return Err(format!("无法创建模型测试请求：{error}"));
    }
    let output_result = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--connect-timeout")
        .arg("5")
        .arg("--max-time")
        .arg("30")
        .arg("--request")
        .arg("POST")
        .arg("--output")
        .arg(&response_path)
        .arg("--write-out")
        .arg("%{http_code}")
        .arg("-H")
        .arg(format!("@{}", header_path.to_string_lossy()))
        .arg("--data-binary")
        .arg(format!("@{}", body_path.to_string_lossy()))
        .arg(endpoint)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let response_body = fs::read_to_string(&response_path).unwrap_or_default();
    let _ = fs::remove_file(&header_path);
    let _ = fs::remove_file(&body_path);
    let _ = fs::remove_file(&response_path);
    let output = output_result.map_err(|error| format!("无法启动 curl：{error}"))?;
    let code_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let code = code_text.parse::<u16>().unwrap_or(0);
    if (200..300).contains(&code) {
        Ok(StrixLlmTestResult {
            ok: true,
            status: code.to_string(),
            message: format!("模型 Chat Completions 测试通过（HTTP {code}）"),
            model,
            deployment: deployment.into(),
        })
    } else {
        let body_error = llm_test_error_message(&response_body);
        let stderr_error = String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(220)
            .collect::<String>();
        let error_text = if !body_error.is_empty() {
            body_error
        } else if !stderr_error.is_empty() {
            stderr_error
        } else {
            "服务未返回成功响应".into()
        };
        Err(if code > 0 {
            format!("模型 Chat Completions 测试失败（HTTP {code}）：{error_text}")
        } else if error_text.is_empty() {
            "模型连通性测试失败，请检查地址、网络和服务状态".into()
        } else {
            format!("模型连通性测试失败：{error_text}")
        })
    }
}

fn fofa_transport_error(error: &reqwest::Error, proxy_url: &str) -> String {
    if error.is_timeout() {
        return if proxy_url.is_empty() {
            "FOFA 连接超时，请检查当前网络后重试".into()
        } else {
            format!("FOFA 连接超时；当前配置的代理 {proxy_url} 没有正常转发请求")
        };
    }
    if !proxy_url.is_empty() {
        return format!("无法通过代理 {proxy_url} 连接 FOFA；请确认代理程序已启动且端口正确");
    }
    "无法连接 FOFA，请检查 DNS、网络或防火墙后重试".into()
}

pub(crate) fn request_fofa_account(key: &str, proxy_url: &str) -> Result<JsonValue, String> {
    if !key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("FOFA Key 格式无效".into());
    }
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));
    if !proxy_url.is_empty() {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|_| "代理地址格式无效，应类似 http://127.0.0.1:7890".to_string())?;
        builder = builder.proxy(proxy);
    }
    let client = builder
        .build()
        .map_err(|_| "无法初始化 FOFA 网络客户端".to_string())?;
    let mut response = client
        .get(format!("https://fofa.info/api/v1/info/my?key={key}"))
        .send()
        .map_err(|error| fofa_transport_error(&error, proxy_url))?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(|_| "读取 FOFA 响应失败".to_string())?;
    let payload: JsonValue = serde_json::from_slice(&body)
        .map_err(|_| format!("FOFA 返回了无法解析的响应（HTTP {}）", status.as_u16()))?;
    if !status.is_success()
        || payload
            .get("error")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
    {
        let message = payload
            .get("errmsg")
            .or_else(|| payload.get("message"))
            .and_then(JsonValue::as_str)
            .unwrap_or("Key 无效或 FOFA 拒绝了请求");
        return Err(format!("FOFA 鉴权失败：{message}"));
    }
    Ok(payload)
}

#[tauri::command]
pub async fn test_fofa_api(input: FofaApiTestInput) -> Result<FofaApiTestResult, String> {
    let key = input.key.trim().to_string();
    let proxy_url = input.proxy_url.trim().to_string();
    if key.is_empty() {
        return Err("请先填写 FOFA key".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let payload = request_fofa_account(&key, &proxy_url)?;
        let account = payload
            .get("email")
            .or_else(|| payload.get("username"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string();
        let plan = payload
            .get("vip_level")
            .or_else(|| payload.get("vipLevel"))
            .or_else(|| payload.get("level"))
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_default();
        Ok(FofaApiTestResult {
            ok: true,
            status: "available".into(),
            message: if proxy_url.is_empty() {
                "FOFA API 可用，鉴权与直连网络均正常".into()
            } else {
                format!("FOFA API 可用，代理 {proxy_url} 工作正常")
            },
            account,
            plan,
        })
    })
    .await
    .map_err(|error| format!("FOFA 测试任务异常：{error}"))?
}

fn openai_chat_completion_model(llm: &str) -> String {
    let value = llm.trim();
    value
        .strip_prefix("openai/")
        .unwrap_or(value)
        .trim()
        .to_string()
}

fn write_private_temp_file(path: &Path, content: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.write_all(content).map_err(|error| error.to_string())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, content).map_err(|error| error.to_string())
    }
}

fn llm_test_error_message(response_body: &str) -> String {
    let trimmed = response_body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let parsed = serde_json::from_str::<JsonValue>(trimmed).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(JsonValue::as_str)
        })
        .unwrap_or(trimmed);
    message
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(260)
        .collect()
}

fn command_proxy(command: &mut Command, proxy: Option<&str>, no_proxy: &str) {
    if let Some(proxy) = proxy {
        command
            .env("HTTP_PROXY", proxy)
            .env("HTTPS_PROXY", proxy)
            .env("ALL_PROXY", proxy);
    }
    if !no_proxy.trim().is_empty() {
        command.env("NO_PROXY", no_proxy);
    }
}
