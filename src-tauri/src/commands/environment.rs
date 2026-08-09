#[tauri::command]
pub fn check_environment(
    state: State<AppState>,
    profile_id: Option<i64>,
) -> Result<EnvironmentReport, String> {
    let connection = db::open(&state.db_path)?;
    let profile_json: String = if let Some(id) = profile_id {
        connection
            .query_row(
                "SELECT settings_json FROM config_profiles WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "{}".into())
    } else {
        connection
            .query_row(
                "SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "{}".into())
    };
    let profile = json(profile_json);
    let configured_python = profile
        .get("pythonExecutable")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("python3")
        .to_string();
    let runtime_path = std::env::var_os("PATH").unwrap_or_default();
    let command_check = |command: &str, args: &[&str]| -> (bool, String) {
        match Command::new(command)
            .args(args)
            .env("PATH", &runtime_path)
            .output()
        {
            Ok(output) => {
                let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if text.is_empty() {
                    text = String::from_utf8_lossy(&output.stderr).trim().to_string();
                }
                (
                    output.status.success(),
                    text.lines().next().unwrap_or("").to_string(),
                )
            }
            Err(error) => (false, error.to_string()),
        }
    };
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let generic_python = matches!(configured_python.as_str(), "python" | "python3" | "py");
    let mut python_candidates: Vec<String> = Vec::new();
    if !generic_python {
        python_candidates.push(configured_python.clone());
    }
    if !home.is_empty() {
        python_candidates.extend([
            format!("{home}/oviraptor/runtime/python/bin/python3"),
            format!("{home}/.pyenv/shims/python"),
            format!("{home}/.pyenv/shims/python3"),
            format!("{home}/.pyenv/versions/3.12.10/bin/python"),
            format!("{home}/.local/bin/python3"),
        ]);
    }
    if cfg!(target_os = "windows") {
        let runtime = profile
            .get("windowsRuntimeDirectory")
            .and_then(JsonValue::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("C:\\oviraptor\\runtime")
            .trim_end_matches(&['\\', '/'][..]);
        python_candidates.extend([
            format!("{runtime}\\python\\Scripts\\python.exe"),
            format!("{runtime}\\python\\python.exe"),
            format!("{runtime}\\python.exe"),
            format!("{home}\\.pyenv\\pyenv-win\\shims\\python.exe"),
        ]);
    }
    python_candidates.extend([configured_python.clone(), "python3".into(), "python".into()]);
    let mut python = "python3".to_string();
    let mut python_ok = false;
    let mut python_version = String::new();
    for candidate in python_candidates {
        let (ok, version) = command_check(&candidate, ["--version"].as_ref());
        if ok {
            python = candidate;
            python_ok = true;
            python_version = version;
            break;
        }
    }
    let mut node = "node".to_string();
    let mut node_ok = false;
    let mut node_version = String::new();
    for candidate in [
        "node",
        "/opt/homebrew/bin/node",
        "/usr/local/bin/node",
        "/usr/bin/node",
    ] {
        let (ok, version) = command_check(candidate, ["--version"].as_ref());
        if ok {
            node = candidate.to_string();
            node_ok = true;
            node_version = version;
            break;
        }
    }
    let configured_redis = profile
        .get("redisCliExecutable")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let windows_runtime = profile
        .get("windowsRuntimeDirectory")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("C:\\oviraptor\\runtime");
    let mut redis_candidates: Vec<String> = if cfg!(target_os = "macos") {
        vec![
            "redis-cli".into(),
            "/opt/homebrew/bin/redis-cli".into(),
            "/usr/local/bin/redis-cli".into(),
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            "redis-cli".into(),
            "memurai-cli.exe".into(),
            format!(
                "{}\\redis-cli.exe",
                windows_runtime.trim_end_matches(&['\\', '/'][..])
            ),
            "C:\\Program Files\\Redis\\redis-cli.exe".into(),
            "C:\\Program Files\\Memurai\\memurai-cli.exe".into(),
        ]
    } else {
        vec!["redis-cli".into(), "/usr/bin/redis-cli".into()]
    };
    if let Some(configured) = configured_redis {
        redis_candidates.insert(0, configured);
    }
    let mut redis_ok = false;
    let mut redis_version = String::new();
    for candidate in redis_candidates {
        let (ok, version) = command_check(&candidate, ["--version"].as_ref());
        if ok {
            redis_ok = true;
            redis_version = format!("{} · {}", candidate, version);
            break;
        }
    }
    let configured_strix = profile
        .get("strixExecutable")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let mut strix_candidates: Vec<String> = if cfg!(target_os = "windows") {
        vec![
            "strix".into(),
            format!(
                "{}\\python\\Scripts\\strix.exe",
                windows_runtime.trim_end_matches(&['\\', '/'][..])
            ),
            format!(
                "{}\\strix.exe",
                windows_runtime.trim_end_matches(&['\\', '/'][..])
            ),
            format!("{home}\\.strix\\bin\\strix.exe"),
        ]
    } else {
        vec![
            "strix".into(),
            format!("{home}/.strix/bin/strix"),
            "/opt/homebrew/bin/strix".into(),
            "/usr/local/bin/strix".into(),
        ]
    };
    if let Some(configured) = configured_strix {
        strix_candidates.insert(0, configured);
    }
    let mut strix_ok = false;
    let mut strix_version = String::new();
    for candidate in strix_candidates {
        let (ok, version) = command_check(&candidate, ["--version"].as_ref());
        if ok {
            strix_ok = true;
            strix_version = format!("{} · {}", candidate, version);
            break;
        }
    }
    let mut docker_candidates: Vec<String> = if cfg!(target_os = "windows") {
        vec![
            "docker.exe".into(),
            "C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe".into(),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            "docker".into(),
            "/Applications/Docker.app/Contents/Resources/bin/docker".into(),
            "/opt/homebrew/bin/docker".into(),
            "/usr/local/bin/docker".into(),
        ]
    } else {
        vec!["docker".into(), "/usr/bin/docker".into()]
    };
    docker_candidates.dedup();
    let mut docker_command = String::new();
    let mut docker_ok = false;
    let mut docker_version = String::new();
    for candidate in docker_candidates {
        let (ok, version) = command_check(&candidate, ["--version"].as_ref());
        if ok {
            docker_command = candidate;
            docker_ok = true;
            docker_version = version;
            break;
        }
    }
    let (docker_daemon_ok, docker_daemon_detail) = if docker_ok {
        command_check(&docker_command, &["info", "--format", "{{.ServerVersion}}"])
    } else {
        (false, "Docker CLI 不可用".into())
    };
    let modules = ["pandas", "requests", "openpyxl", "tldextract"];
    let module_code = format!("import {}", modules.join(","));
    let (modules_ok, module_detail) = command_check(&python, &["-c", &module_code]);
    let mut dependencies = Vec::new();
    for module in modules {
        let (available, detail) = command_check(
            &python,
            &[
                "-c",
                &format!("import {module}; print(getattr({module}, '__version__', 'ok'))"),
            ],
        );
        dependencies.push(EnvironmentDependency {
            name: module.to_string(),
            command: python.clone(),
            version: detail.clone(),
            available,
            detail: if available {
                "import ok".into()
            } else {
                detail
            },
        });
    }
    if !modules_ok {
        dependencies
            .iter_mut()
            .filter(|item| !item.available)
            .for_each(|item| item.detail = module_detail.clone());
    }
    Ok(EnvironmentReport {
        os: if cfg!(target_os = "macos") {
            "macOS"
        } else if cfg!(target_os = "windows") {
            "Windows"
        } else {
            "Linux"
        }
        .into(),
        arch: std::env::consts::ARCH.into(),
        python: if python_ok {
            format!("{} · {}", python, python_version)
        } else {
            format!("{} · {}", python, python_version)
        },
        node: if node_ok {
            format!("{} · {}", node, node_version)
        } else {
            format!("不可用 · {}", node_version)
        },
        redis_cli: if redis_ok {
            redis_version
        } else {
            format!("不可用 · {}", redis_version)
        },
        strix_cli: if strix_ok {
            if docker_daemon_ok {
                format!("{} · Docker 就绪", strix_version)
            } else {
                format!("{} · Docker daemon 未就绪", strix_version)
            }
        } else {
            format!("不可用 · {}", strix_version)
        },
        docker_cli: if docker_ok {
            format!("{} · {}", docker_command, docker_version)
        } else {
            format!("不可用 · {}", docker_version)
        },
        docker_daemon: if docker_daemon_ok {
            format!("可用 · Server {}", docker_daemon_detail)
        } else {
            format!("不可用 · {}", docker_daemon_detail)
        },
        dependencies,
        checked_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn profile_settings_for_path(db_path: &Path, profile_id: Option<i64>) -> Result<JsonValue, String> {
    let connection = db::open(db_path)?;
    let text: String = if let Some(id) = profile_id {
        connection
            .query_row(
                "SELECT settings_json FROM config_profiles WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "{}".into())
    } else {
        connection
            .query_row(
                "SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "{}".into())
    };
    Ok(json(text))
}

fn strix_version(executable: &str) -> Result<String, String> {
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .map_err(|error| format!("无法执行 {executable} --version：{error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = if stdout.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };
    if !output.status.success() {
        return Err(format!("{executable} --version 失败：{detail}"));
    }
    detail
        .split_whitespace()
        .map(|part| {
            part.trim_start_matches('v')
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.' && ch != '-')
        })
        .find(|part| {
            part.split('.').take(3).all(|piece| {
                !piece.is_empty() && piece.chars().next().is_some_and(|ch| ch.is_ascii_digit())
            }) && part.matches('.').count() >= 2
        })
        .map(str::to_string)
        .ok_or_else(|| format!("无法从版本输出中解析 Strix 版本：{detail}"))
}

fn version_tuple(version: &str) -> (u64, u64, u64) {
    let mut parts = version
        .trim_start_matches('v')
        .split(|ch: char| ch == '.' || ch == '-')
        .take(3)
        .map(|part| part.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn latest_strix_release() -> Result<(String, String), String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| format!("无法创建 Strix 更新检查客户端：{error}"))?;
    let response = client
        .get("https://api.github.com/repos/usestrix/strix/releases/latest")
        .header(
            reqwest::header::USER_AGENT,
            concat!("Oviraptor/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .map_err(|error| format!("连接 GitHub 检查 Strix 更新失败：{error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("读取 Strix 更新响应失败：{error}"))?;
    if !status.is_success() {
        return Err(format!(
            "GitHub 更新接口返回 HTTP {status}：{}",
            body.chars().take(500).collect::<String>()
        ));
    }
    let payload: JsonValue =
        serde_json::from_str(&body).map_err(|error| format!("解析 Strix 更新响应失败：{error}"))?;
    let version = payload
        .get("tag_name")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim_start_matches('v')
        .to_string();
    if version.is_empty() {
        return Err("GitHub 更新响应没有 tag_name".into());
    }
    let release_url = payload
        .get("html_url")
        .and_then(JsonValue::as_str)
        .unwrap_or("https://github.com/usestrix/strix/releases/latest")
        .to_string();
    Ok((version, release_url))
}

fn cached_strix_release(connection: &rusqlite::Connection) -> Option<(String, String, String)> {
    let version: String = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='strix_latest_version'",
            [],
            |row| row.get(0),
        )
        .ok()?;
    let checked_at: String = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='strix_update_checked_at'",
            [],
            |row| row.get(0),
        )
        .ok()?;
    let release_url: String = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key='strix_release_url'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "https://github.com/usestrix/strix/releases/latest".into());
    let checked = chrono::DateTime::parse_from_rfc3339(&checked_at).ok()?;
    if chrono::Utc::now()
        .signed_duration_since(checked.with_timezone(&chrono::Utc))
        .num_hours()
        >= 12
    {
        return None;
    }
    Some((version, release_url, checked_at))
}

fn save_strix_release_cache(
    connection: &rusqlite::Connection,
    version: &str,
    release_url: &str,
    checked_at: &str,
) -> Result<(), String> {
    for (key, value) in [
        ("strix_latest_version", version),
        ("strix_release_url", release_url),
        ("strix_update_checked_at", checked_at),
    ] {
        connection
            .execute(
                "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn check_strix_update_inner(
    db_path: &Path,
    profile_id: Option<i64>,
    force: bool,
) -> Result<StrixUpdateStatus, String> {
    let settings = profile_settings_for_path(db_path, profile_id)?;
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let executable = resolve_strix_executable(&settings, Path::new(&home)).unwrap_or_default();
    let installed = !executable.is_empty();
    let current_version = if installed {
        strix_version(&executable).unwrap_or_default()
    } else {
        String::new()
    };
    let connection = db::open(db_path)?;
    let cached = if force {
        None
    } else {
        cached_strix_release(&connection)
    };
    let (latest_version, release_url, checked_at, check_error) = match cached {
        Some((version, url, checked_at)) => (version, url, checked_at, String::new()),
        None => {
            let checked_at = chrono::Utc::now().to_rfc3339();
            match latest_strix_release() {
                Ok((version, url)) => {
                    save_strix_release_cache(&connection, &version, &url, &checked_at)?;
                    (version, url, checked_at, String::new())
                }
                Err(error) => (
                    String::new(),
                    "https://github.com/usestrix/strix/releases/latest".into(),
                    checked_at,
                    error,
                ),
            }
        }
    };
    let update_available = installed
        && !current_version.is_empty()
        && !latest_version.is_empty()
        && version_tuple(&latest_version) > version_tuple(&current_version);
    Ok(StrixUpdateStatus {
        installed,
        executable,
        current_version,
        latest_version,
        update_available,
        checked_at,
        release_url,
        check_error,
    })
}

#[tauri::command]
pub async fn check_strix_update(
    state: State<'_, AppState>,
    profile_id: Option<i64>,
    force: Option<bool>,
) -> Result<StrixUpdateStatus, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        check_strix_update_inner(&db_path, profile_id, force.unwrap_or(false))
    })
    .await
    .map_err(|error| format!("Strix 更新检查线程失败：{error}"))?
}

fn emit_environment_install_log(app: &AppHandle, stage: &str, stream: &str, message: &str) {
    let _ = app.emit(
        "environment-install-log",
        serde_json::json!({
            "stage": stage,
            "stream": stream,
            "message": message,
        }),
    );
}

fn run_environment_install_step(
    app: &AppHandle,
    stage: &str,
    description: &str,
    mut command: Command,
) -> Result<(), String> {
    emit_environment_install_log(app, stage, "status", &format!("开始：{description}"));
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("{description}无法启动：{error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{description}无法读取 stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{description}无法读取 stderr"))?;
    let (sender, receiver) = mpsc::channel::<(&'static str, String)>();
    let stdout_sender = sender.clone();
    let stdout_thread = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = stdout_sender.send(("stdout", line));
        }
    });
    let stderr_thread = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = sender.send(("stderr", line));
        }
    });
    let mut tail = VecDeque::with_capacity(24);
    for (stream, line) in receiver {
        let line = line.trim_end().to_string();
        if line.is_empty() {
            continue;
        }
        emit_environment_install_log(app, stage, stream, &line);
        if tail.len() == 24 {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    let status = child
        .wait()
        .map_err(|error| format!("{description}等待失败：{error}"))?;
    if !status.success() {
        let detail = tail.into_iter().collect::<Vec<_>>().join("\n");
        let message = if detail.is_empty() {
            format!("{description}失败，退出状态：{status}")
        } else {
            format!("{description}失败：\n{detail}")
        };
        emit_environment_install_log(app, stage, "error", &message);
        return Err(message);
    }
    emit_environment_install_log(app, stage, "success", &format!("完成：{description}"));
    Ok(())
}

fn update_strix_inner(
    app: &AppHandle,
    db_path: &Path,
    profile_id: Option<i64>,
) -> Result<StrixUpdateStatus, String> {
    if !cfg!(target_os = "macos") {
        return Err(
            "当前的一键 Strix 升级仅在 macOS 主控端开放；Windows Worker 请在运行环境页按提示安装"
                .into(),
        );
    }
    let connection = db::open(db_path)?;
    let running: i64 = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM runs WHERE status IN ('queued','running','cancel_requested')) + (SELECT COUNT(*) FROM sentinel_scans WHERE status IN ('queued','scanning','pausing'))",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if running > 0 {
        return Err(format!("当前有 {running} 个采集或 Strix 扫描任务正在运行。为避免升级中断任务，请任务结束或取消后再升级"));
    }
    drop(connection);

    emit_environment_install_log(
        app,
        "strix-update-check",
        "status",
        "正在检查本机版本和 GitHub 最新正式版",
    );
    let before = check_strix_update_inner(db_path, profile_id, true)?;
    if !before.check_error.is_empty() {
        emit_environment_install_log(app, "strix-update-check", "error", &before.check_error);
        return Err(before.check_error);
    }
    if before.installed && !before.update_available {
        let message = format!("Strix {} 已是最新版本", before.current_version);
        emit_environment_install_log(app, "complete", "success", &message);
        return Ok(before);
    }
    emit_environment_install_log(
        app,
        "strix-update-check",
        "success",
        &format!(
            "本机 {} → 最新 {}",
            if before.current_version.is_empty() {
                "未安装"
            } else {
                &before.current_version
            },
            before.latest_version
        ),
    );

    let can_self_update = before.installed && version_tuple(&before.current_version) >= (1, 4, 0);
    if can_self_update {
        emit_environment_install_log(
            app,
            "strix-update",
            "status",
            &format!("执行：{} --update", before.executable),
        );
        let mut command = Command::new(&before.executable);
        command.arg("--update");
        configure_strix_console(&mut command);
        run_environment_install_step(app, "strix-update", "执行 Strix 内置升级", command)?;
    } else {
        let script = format!(
            "set -o pipefail; curl --fail --show-error --location --connect-timeout 10 --max-time 180 --retry 2 https://strix.ai/install | VERSION={} /bin/bash",
            before.latest_version
        );
        emit_environment_install_log(
            app,
            "strix-update",
            "status",
            &format!(
                "旧版不支持 --update，执行官方安装器并固定版本 {}：\n{script}",
                before.latest_version
            ),
        );
        let mut command = Command::new("/bin/bash");
        command.args(["-lc", &script]);
        configure_strix_console(&mut command);
        run_environment_install_step(app, "strix-update", "下载并安装 Strix 官方版本", command)?;
    }

    let settings = profile_settings_for_path(db_path, profile_id)?;
    let home = std::env::var("HOME").unwrap_or_default();
    let executable = resolve_strix_executable(&settings, Path::new(&home))?;
    let current_version = strix_version(&executable)?;
    if version_tuple(&current_version) < version_tuple(&before.latest_version) {
        let message = format!(
            "升级命令已结束，但版本校验失败：当前 {current_version}，预期至少 {}。可执行文件：{executable}",
            before.latest_version
        );
        emit_environment_install_log(app, "strix-update-verify", "error", &message);
        return Err(message);
    }
    let checked_at = chrono::Utc::now().to_rfc3339();
    let connection = db::open(db_path)?;
    save_strix_release_cache(
        &connection,
        &before.latest_version,
        &before.release_url,
        &checked_at,
    )?;
    let result = StrixUpdateStatus {
        installed: true,
        executable,
        current_version: current_version.clone(),
        latest_version: before.latest_version,
        update_available: false,
        checked_at,
        release_url: before.release_url,
        check_error: String::new(),
    };
    emit_environment_install_log(
        app,
        "strix-update-verify",
        "success",
        &format!("版本校验通过：Strix {current_version}"),
    );
    emit_environment_install_log(
        app,
        "complete",
        "success",
        "Strix 升级完成；新的扫描任务将使用新版本",
    );
    Ok(result)
}

#[tauri::command]
pub async fn update_strix(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: Option<i64>,
) -> Result<StrixUpdateStatus, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || update_strix_inner(&app, &db_path, profile_id))
        .await
        .map_err(|error| format!("Strix 升级线程失败：{error}"))?
}

fn install_windows_environment(
    app: &AppHandle,
    state: &AppState,
    profile_id: Option<i64>,
) -> Result<String, String> {
    emit_environment_install_log(
        app,
        "prepare",
        "status",
        "开始安装 Windows Worker 基础环境；系统安装器如需授权会显示确认窗口",
    );
    let winget_available = Command::new("winget")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !winget_available {
        return Err(
            "未找到 winget。请从 Microsoft Store 安装“应用安装程序”，或按页面手动步骤安装依赖"
                .into(),
        );
    }
    for (stage, package, description) in [
        ("python", "Python.Python.3.12", "Python 3.12"),
        ("node", "OpenJS.NodeJS.LTS", "Node.js LTS"),
        ("docker", "Docker.DockerDesktop", "Docker Desktop"),
        ("tailscale", "Tailscale.Tailscale", "Tailscale"),
    ] {
        let mut command = Command::new("winget");
        command.args([
            "install",
            "--id",
            package,
            "--exact",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]);
        run_environment_install_step(
            app,
            stage,
            &format!("通过 winget 安装 {description}"),
            command,
        )?;
    }

    let profile = {
        let connection = db::open(&state.db_path)?;
        let text: String = if let Some(id) = profile_id {
            connection
                .query_row(
                    "SELECT settings_json FROM config_profiles WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "{}".into())
        } else {
            connection
                .query_row(
                    "SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "{}".into())
        };
        json(text)
    };
    let runtime_root = profile
        .get("windowsRuntimeDirectory")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(r"C:\oviraptor\runtime");
    let runtime_python_dir = PathBuf::from(runtime_root).join("python");
    let runtime_python = runtime_python_dir.join("Scripts/python.exe");
    if !runtime_python.is_file() {
        let mut venv = Command::new("py");
        venv.args(["-3.12", "-m", "venv"]).arg(&runtime_python_dir);
        run_environment_install_step(app, "python-venv", "创建 Oviraptor Python 环境", venv)?;
    }
    let mut pip = Command::new(&runtime_python);
    pip.args([
        "-m",
        "pip",
        "install",
        "--disable-pip-version-check",
        "pandas",
        "requests",
        "openpyxl",
        "tldextract",
    ]);
    run_environment_install_step(app, "python-modules", "安装 Python 模块", pip)?;

    let runtime_strix = runtime_python_dir.join("Scripts/strix.exe");
    let mut strix_install = Command::new(&runtime_python);
    strix_install.args([
        "-m",
        "pip",
        "install",
        "--disable-pip-version-check",
        "strix-agent",
    ]);
    let strix_result =
        run_environment_install_step(app, "strix", "安装 Strix CLI（strix-agent）", strix_install);

    if let Some(id) = profile_id {
        let connection = db::open(&state.db_path)?;
        let existing: String = connection
            .query_row(
                "SELECT settings_json FROM config_profiles WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "{}".into());
        let mut settings = json(existing);
        if let Some(object) = settings.as_object_mut() {
            object.insert(
                "pythonExecutable".into(),
                JsonValue::String(runtime_python.to_string_lossy().to_string()),
            );
            if runtime_strix.is_file() {
                object.insert(
                    "strixExecutable".into(),
                    JsonValue::String(runtime_strix.to_string_lossy().to_string()),
                );
            }
            connection
                .execute(
                    "UPDATE config_profiles SET settings_json=?1,updated_at=datetime('now','localtime') WHERE id=?2",
                    params![settings.to_string(), id],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    if let Err(error) = strix_result {
        emit_environment_install_log(
            app,
            "manual",
            "stderr",
            &format!("Strix 自动安装未完成：{error}\n请按页面的 Windows/WSL 手动步骤处理"),
        );
    }
    emit_environment_install_log(
        app,
        "manual",
        "stderr",
        "Windows 没有官方 redis-cli 安装包；如扫描流程需要，请安装 Memurai CLI 并在运行方案中填写 C:\\Program Files\\Memurai\\memurai-cli.exe。Docker Desktop 与 Tailscale 首次使用需要打开并登录。",
    );
    let result =
        "Windows 基础环境已安装；请启动 Docker Desktop、登录 Tailscale，并重新执行环境检测";
    emit_environment_install_log(app, "complete", "success", result);
    Ok(result.into())
}

#[tauri::command]
pub fn install_environment_dependencies(
    app: AppHandle,
    state: State<AppState>,
    profile_id: Option<i64>,
) -> Result<String, String> {
    if cfg!(target_os = "windows") {
        return install_windows_environment(&app, &state, profile_id);
    }
    if !cfg!(target_os = "macos") {
        return Err("Linux 请按环境检测结果手动安装依赖，并配置 PATH".into());
    }
    emit_environment_install_log(
        &app,
        "prepare",
        "status",
        "开始检查并安装 Oviraptor 运行环境",
    );
    let connection = db::open(&state.db_path)?;
    let settings_text: String = if let Some(id) = profile_id {
        connection
            .query_row(
                "SELECT settings_json FROM config_profiles WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "{}".into())
    } else {
        connection
            .query_row(
                "SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "{}".into())
    };
    let profile = json(settings_text);
    let configured_python = profile
        .get("pythonExecutable")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("python3");
    let home = std::env::var("HOME").unwrap_or_default();
    let brew = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew", "brew"]
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .map(str::to_string);
    let mut messages = Vec::new();
    let brew = match brew {
        Some(path) => {
            emit_environment_install_log(
                &app,
                "homebrew",
                "success",
                &format!("已找到 Homebrew：{path}"),
            );
            path
        }
        None => {
            let mut command = Command::new("/bin/bash");
            command
                .arg("-c")
                .arg("NONINTERACTIVE=1 /bin/bash -c \"$(curl --fail --show-error --location --connect-timeout 15 --max-time 300 --retry 2 --retry-delay 2 https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"");
            run_environment_install_step(&app, "homebrew", "下载并安装 Homebrew", command)?;
            ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
                .into_iter()
                .find(|candidate| {
                    Command::new(candidate)
                        .arg("--version")
                        .output()
                        .is_ok_and(|output| output.status.success())
                })
                .ok_or_else(|| "Homebrew 安装完成但找不到 brew".to_string())?
                .to_string()
        }
    };
    let mut brew_install = Command::new(&brew);
    brew_install
        .args(["install", "python", "node", "redis"])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ENV_HINTS", "1");
    run_environment_install_step(
        &app,
        "packages",
        "安装 Python、Node.js 和 redis-cli",
        brew_install,
    )?;
    messages.push("Python、Node.js、redis-cli 安装完成".to_string());

    let runtime_dir = PathBuf::from(&home).join("oviraptor/runtime/python");
    let runtime_python = runtime_dir.join("bin/python3");
    let python_candidates = [
        runtime_python.to_string_lossy().to_string(),
        configured_python.to_string(),
        "/opt/homebrew/bin/python3".to_string(),
        "/usr/local/bin/python3".to_string(),
        "python3".to_string(),
    ];
    let python = python_candidates
        .iter()
        .find(|candidate| {
            Command::new(candidate)
                .args(["--version"])
                .output()
                .is_ok_and(|result| result.status.success())
        })
        .ok_or_else(|| "Python 安装完成但找不到 python3".to_string())?;
    if !runtime_python.exists() {
        fs::create_dir_all(
            runtime_dir
                .parent()
                .ok_or_else(|| "无法确定 Python runtime 目录".to_string())?,
        )
        .map_err(|error| format!("创建 Python runtime 目录失败：{error}"))?;
        let mut venv = Command::new(python);
        venv.args(["-m", "venv"]).arg(&runtime_dir);
        run_environment_install_step(&app, "python-venv", "创建 Oviraptor Python 虚拟环境", venv)?;
    } else {
        emit_environment_install_log(
            &app,
            "python-venv",
            "success",
            &format!("Python 虚拟环境已存在：{}", runtime_dir.display()),
        );
    }
    let mut pip = Command::new(&runtime_python);
    pip.args([
        "-m",
        "pip",
        "install",
        "--disable-pip-version-check",
        "pandas",
        "requests",
        "openpyxl",
        "tldextract",
    ]);
    run_environment_install_step(&app, "python-modules", "安装 Python 模块", pip)?;
    let runtime_python = runtime_python.to_string_lossy().to_string();
    if let Some(id) = profile_id.or_else(|| {
        connection
            .query_row(
                "SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .ok()
    }) {
        let existing: String = connection
            .query_row(
                "SELECT settings_json FROM config_profiles WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "{}".into());
        let mut settings = json(existing);
        if let Some(object) = settings.as_object_mut() {
            object.insert("pythonExecutable".into(), JsonValue::String(runtime_python));
            connection
                .execute(
                    "UPDATE config_profiles SET settings_json=?1,updated_at=datetime('now','localtime') WHERE id=?2",
                    params![settings.to_string(), id],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    messages.push("Python 模块安装完成".to_string());

    let docker_app = Path::new("/Applications/Docker.app");
    if !docker_app.exists() {
        let mut docker = Command::new(&brew);
        docker
            .args(["install", "--cask", "docker-desktop"])
            .env("HOMEBREW_NO_AUTO_UPDATE", "1")
            .env("HOMEBREW_NO_ENV_HINTS", "1");
        run_environment_install_step(&app, "docker", "安装 Docker Desktop", docker)?;
        messages.push("Docker Desktop 下载完成".to_string());
    } else {
        emit_environment_install_log(
            &app,
            "docker",
            "success",
            "Docker Desktop 已安装；如果 daemon 未就绪，请启动 Docker.app",
        );
        messages.push("Docker Desktop 已存在".to_string());
    }

    let tailscale_app = Path::new("/Applications/Tailscale.app");
    if !tailscale_app.exists() {
        let mut tailscale = Command::new(&brew);
        tailscale
            .args(["install", "--cask", "tailscale-app"])
            .env("HOMEBREW_NO_AUTO_UPDATE", "1")
            .env("HOMEBREW_NO_ENV_HINTS", "1");
        run_environment_install_step(&app, "tailscale", "安装 Tailscale", tailscale)?;
        messages.push("Tailscale 安装完成（请打开并登录）".to_string());
    } else {
        emit_environment_install_log(
            &app,
            "tailscale",
            "success",
            "Tailscale 已安装；Worker 启用前请确认已经登录 Tailnet",
        );
        messages.push("Tailscale 已存在".to_string());
    }

    let strix = Path::new(&home).join(".strix/bin/strix");
    if !strix.exists() {
        let mut install = Command::new("/bin/bash");
        install.arg("-c").arg(
            "set -o pipefail; curl --fail --show-error --location --connect-timeout 15 --max-time 300 --retry 2 --retry-delay 2 https://strix.ai/install | /bin/bash",
        );
        run_environment_install_step(&app, "strix", "下载并安装 Strix CLI", install)?;
        messages.push("Strix CLI 安装完成".to_string());
    } else {
        emit_environment_install_log(
            &app,
            "strix",
            "success",
            &format!("Strix CLI 已安装：{}", strix.display()),
        );
        messages.push("Strix CLI 已存在".to_string());
    }
    let result = messages.join("；");
    emit_environment_install_log(&app, "complete", "success", &result);
    Ok(result)
}
