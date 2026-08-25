// Strix integration compatibility layer.
//
// Keep upstream-version assumptions in this file. A routine Strix upgrade
// should normally require changes here plus its focused tests only. Business
// pipelines consume detected capabilities and artifact helpers instead of
// branching on Strix versions throughout the application.

const STRIX_INTEGRATION_TARGET_VERSION: &str = "1.5.3";
const STRIX_SUPPORTED_MAJOR: u64 = 1;
const STRIX_MINIMUM_SUPPORTED_VERSION: (u64, u64, u64) = (1, 5, 3);
const DEFAULT_STRIX_SANDBOX_IMAGE: &str = "ghcr.io/usestrix/strix-sandbox:1.3.0";

const STRIX_RUN_ARTIFACT: &str = "run.json";
const STRIX_VULNERABILITIES_ARTIFACT: &str = "vulnerabilities.json";
const STRIX_SARIF_ARTIFACT: &str = "findings.sarif";
const STRIX_CSV_ARTIFACT: &str = "vulnerabilities.csv";
const STRIX_AGENT_STATE_ARTIFACT: &str = ".state/agents.db";
const STRIX_AGENT_MESSAGES_QUERY: &str = "SELECT message_data FROM agent_messages ORDER BY id";
const STRIX_AGENT_TRACE_QUERY: &str =
    "SELECT id,session_id,message_data,created_at FROM agent_messages ORDER BY id";
const STRIX_AGENT_SESSION_COUNT_QUERY: &str = "SELECT COUNT(*) FROM agent_sessions";

#[derive(Debug, Clone, PartialEq, Eq)]
struct StrixCliCapabilities {
    version: String,
    mount_flag: bool,
    target_flag: bool,
    target_list_flag: bool,
    instruction_file_flag: bool,
    non_interactive_flag: bool,
    scan_mode_flag: bool,
    max_turns_flag: bool,
    max_budget_flag: Option<String>,
    config_flag: bool,
    scope_mode_flag: bool,
    diff_base_flag: bool,
}

fn strix_help_has_option(help: &str, option: &str) -> bool {
    help.split_whitespace().any(|token| {
        token.trim_matches(|value: char| {
            matches!(value, '[' | ']' | ',' | '(' | ')' | ':' | ';')
        }) == option
    })
}

fn validate_strix_version(version: &str) -> Result<(u64, u64, u64), String> {
    let detected = version
        .split_whitespace()
        .map(|part| {
            part.trim_start_matches('v')
                .trim_matches(|character: char| {
                    !character.is_ascii_alphanumeric() && character != '.' && character != '-'
                })
        })
        .find(|part| part.matches('.').count() >= 2)
        .map(version_tuple)
        .unwrap_or((0, 0, 0));
    if detected.0 == 0 {
        return Err(format!(
            "无法识别 Strix 版本“{}”；Oviraptor 当前适配目标为 {}",
            version.trim(),
            STRIX_INTEGRATION_TARGET_VERSION
        ));
    }
    if detected.0 != STRIX_SUPPORTED_MAJOR {
        return Err(format!(
            "Strix {} 属于尚未审核的大版本；Oviraptor 当前只支持 {}.x（适配目标 {}）。请先更新 strix_compatibility.rs 的 CLI、产物和数据库映射",
            version.trim(),
            STRIX_SUPPORTED_MAJOR,
            STRIX_INTEGRATION_TARGET_VERSION
        ));
    }
    if detected < STRIX_MINIMUM_SUPPORTED_VERSION {
        return Err(format!(
            "Strix {} 已低于最低支持版本 {}；请在环境中心升级后重试",
            version.trim(),
            STRIX_INTEGRATION_TARGET_VERSION
        ));
    }
    Ok(detected)
}

fn parse_strix_cli_capabilities(help: &str, version: &str) -> Result<StrixCliCapabilities, String> {
    validate_strix_version(version)?;
    let capabilities = StrixCliCapabilities {
        version: version.trim().to_string(),
        mount_flag: strix_help_has_option(help, "--mount"),
        target_flag: strix_help_has_option(help, "--target"),
        target_list_flag: strix_help_has_option(help, "--target-list"),
        instruction_file_flag: strix_help_has_option(help, "--instruction-file"),
        non_interactive_flag: strix_help_has_option(help, "--non-interactive"),
        scan_mode_flag: strix_help_has_option(help, "--scan-mode"),
        max_turns_flag: strix_help_has_option(help, "--max-turns"),
        max_budget_flag: if strix_help_has_option(help, "--max-budget-usd") {
            Some("--max-budget-usd".into())
        } else if strix_help_has_option(help, "--max-budget") {
            Some("--max-budget".into())
        } else {
            None
        },
        config_flag: strix_help_has_option(help, "--config"),
        scope_mode_flag: strix_help_has_option(help, "--scope-mode"),
        diff_base_flag: strix_help_has_option(help, "--diff-base"),
    };
    let mut missing = Vec::new();
    if !capabilities.target_flag && !capabilities.target_list_flag {
        missing.push("--target/--target-list");
    }
    if !capabilities.instruction_file_flag {
        missing.push("--instruction-file");
    }
    if !capabilities.non_interactive_flag {
        missing.push("--non-interactive");
    }
    if !capabilities.scan_mode_flag {
        missing.push("--scan-mode");
    }
    if !capabilities.config_flag {
        missing.push("--config");
    }
    if !missing.is_empty() {
        return Err(format!(
            "当前 Strix CLI 不兼容 Oviraptor，缺少必要能力：{}；检测版本：{}；适配目标：{}",
            missing.join("、"),
            capabilities.version,
            STRIX_INTEGRATION_TARGET_VERSION
        ));
    }
    Ok(capabilities)
}

static STRIX_CAPABILITY_CACHE: OnceLock<
    Mutex<HashMap<String, Result<StrixCliCapabilities, String>>>,
> = OnceLock::new();

fn strix_executable_cache_key(executable: &str) -> String {
    let configured = PathBuf::from(executable);
    let discovered = if configured.components().count() == 1 {
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .map(|directory| directory.join(executable))
            .find(|candidate| candidate.is_file())
            .unwrap_or(configured)
    } else {
        configured
    };
    let path = fs::canonicalize(&discovered).unwrap_or(discovered);
    let metadata = fs::metadata(&path).ok();
    let length = metadata.as_ref().map(|value| value.len()).unwrap_or(0);
    let modified = metadata
        .and_then(|value| value.modified().ok())
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("{}|{length}|{modified}", path.to_string_lossy())
}

fn strix_cli_capabilities(executable: &str) -> Result<StrixCliCapabilities, String> {
    let cache_key = strix_executable_cache_key(executable);
    let cache = STRIX_CAPABILITY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(&cache_key) {
            return cached.clone();
        }
    }
    let inspect = || -> Result<StrixCliCapabilities, String> {
        let help = Command::new(executable)
            .arg("--help")
            .output()
            .map_err(|error| format!("无法检查 Strix CLI 能力：{error}"))?;
        let help_text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&help.stdout),
            String::from_utf8_lossy(&help.stderr)
        );
        if help_text.trim().is_empty() {
            return Err("Strix --help 没有返回可解析的命令能力".into());
        }
        let version = Command::new(executable)
            .arg("--version")
            .output()
            .ok()
            .map(|output| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            })
            .unwrap_or_else(|| "unknown".into());
        parse_strix_cli_capabilities(&help_text, &version)
    }();
    if let Ok(mut guard) = cache.lock() {
        guard.insert(cache_key, inspect.clone());
    }
    inspect
}

fn append_strix_local_directory(
    command: &mut Command,
    capabilities: &StrixCliCapabilities,
    directory: &Path,
) -> Result<&'static str, String> {
    if capabilities.mount_flag {
        command.arg("--mount").arg(directory);
        Ok("--mount")
    } else if capabilities.target_flag {
        command.arg("--target").arg(directory);
        Ok("--target")
    } else {
        Err(format!(
            "Strix {} 既不支持 --mount，也不支持使用 --target 传入本地证据目录",
            capabilities.version
        ))
    }
}

fn append_strix_budget(
    command: &mut Command,
    capabilities: &StrixCliCapabilities,
    budget: Option<f64>,
) {
    if let (Some(flag), Some(value)) = (capabilities.max_budget_flag.as_deref(), budget) {
        command.arg(flag).arg(format!("{value:.2}"));
    }
}

fn strix_runtime_policy(capabilities: &StrixCliCapabilities, image: &str) -> JsonValue {
    serde_json::json!({
        "integrationTarget": STRIX_INTEGRATION_TARGET_VERSION,
        "supportedMajor": STRIX_SUPPORTED_MAJOR,
        "cliVersion": capabilities.version,
        "image": image,
        "defaultImage": DEFAULT_STRIX_SANDBOX_IMAGE,
        "artifactContract": {
            "run": STRIX_RUN_ARTIFACT,
            "vulnerabilities": STRIX_VULNERABILITIES_ARTIFACT,
            "sarif": STRIX_SARIF_ARTIFACT,
            "csv": STRIX_CSV_ARTIFACT,
            "agentState": STRIX_AGENT_STATE_ARTIFACT
        }
    })
}

fn strix_agent_state_path(run_dir: &Path) -> PathBuf {
    run_dir.join(STRIX_AGENT_STATE_ARTIFACT)
}

fn strix_run_completed(dir: &Path) -> bool {
    let run = fs::read(dir.join(STRIX_RUN_ARTIFACT))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok())
        .unwrap_or_default();
    let status = value_first(&run, &["status"]).to_ascii_lowercase();
    matches!(
        status.as_str(),
        "completed" | "complete" | "finished" | "succeeded" | "success" | "done"
    )
}

fn strix_completed_artifact(root: &Path) -> bool {
    strix_run_dirs(root)
        .map(|dirs| dirs.iter().any(|dir| strix_run_completed(dir)))
        .unwrap_or(false)
}
