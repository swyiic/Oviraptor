// Code-exec A: sandboxed LOCAL analysis script (no network). The agent writes a
// short Python script that runs under the macOS Seatbelt sandbox — network denied,
// writes confined to a throwaway scratch dir, CPU/memory/time/output bounded — to
// COMPUTE over data it already captured: decode a token, derive a ciphertext from
// a leaked public key, brute a local hash, parse a .map. It cannot reach the
// target or touch the host. Gated like every high-risk capability: code_exec must
// be authorized for the scan, and each distinct script needs the operator's
// single-use approval (arbitrary code has no harmless tier). Hitting the target
// from a script is a SEPARATE, network-scoped sandbox (code-exec B), not this.
fn agent_tool_run_analysis_script(
    context: &AgentRunContext,
    _runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let language = value_first(arguments, &["language"]);
    if language != "python" {
        return agent_tool_error(
            "run_analysis_script v1 仅支持 language=python（沙箱内 /usr/bin/python3，无网络）",
            "language_not_supported",
        );
    }
    let script = value_first(arguments, &["script"]);
    if script.trim().is_empty() {
        return agent_tool_error("缺少 script", "invalid_arguments");
    }
    let connection = match db::open(&context.db_path) {
        Ok(connection) => connection,
        Err(error) => return agent_tool_error(&error, "tool_authorization_unavailable"),
    };
    // Gate 1: the scan must have authorized code_exec (the mandatory switch).
    if !scan_capability_class_authorized(&connection, &context.scan_id, context.attempt_number, "code_exec")
        .unwrap_or(false)
    {
        return serde_json::json!({
            "accepted": false,
            "code": "high_risk_action_denied",
            "error": "code_exec 能力未被该任务授权；无法运行脚本",
        });
    }
    // Gate 2: single-use operator approval, bound to this exact script.
    let fencing = current_coordinator_fencing_token(&connection, context);
    let fingerprint = format!("{:x}", Sha256::digest(format!("analysis_script:{}", script.trim()).as_bytes()));
    let purpose = value_first(arguments, &["purpose"]);
    let summary = if purpose.trim().is_empty() {
        "运行本地分析脚本（Seatbelt 沙箱、禁网）".to_string()
    } else {
        format!("运行本地分析脚本（沙箱、禁网）：{}", purpose.trim())
    };
    match high_risk_action_gate_on(
        &connection, &context.scan_id, context.attempt_number, &fencing,
        "code_exec", &fingerprint, &summary, "local-sandbox", true,
    ) {
        Ok(HighRiskGate::Allow) => {}
        Ok(HighRiskGate::Pending(request_id)) => {
            return serde_json::json!({
                "accepted": false,
                "code": "high_risk_action_pending_approval",
                "requestId": request_id,
                "hint": "该脚本需操作者单次批准；operator 批准后用完全相同的参数重试本工具",
            });
        }
        Ok(HighRiskGate::Denied(reason)) => {
            return serde_json::json!({"accepted": false, "code": "high_risk_action_denied", "error": reason});
        }
        Err(error) => return agent_tool_error(&error, "high_risk_gate_unavailable"),
    }
    // Throwaway sandbox dir: the script and its optional input live here, and the
    // sandbox may write only here. Removed after the run regardless of outcome.
    let dir = std::env::temp_dir().join(format!("oviraptor-poc-{}", uuid::Uuid::new_v4()));
    if std::fs::create_dir_all(&dir).is_err() {
        return agent_tool_error("无法创建沙箱临时目录", "sandbox_scratch_unavailable");
    }
    if std::fs::write(dir.join("script.py"), script.as_bytes()).is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        return agent_tool_error("无法写入脚本", "sandbox_scratch_unavailable");
    }
    if let Some(input) = arguments.get("input").and_then(JsonValue::as_str) {
        let _ = std::fs::write(dir.join("input.dat"), input.as_bytes());
    }
    let cancel = agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number);
    let result = crate::tool_supply::macos_static_parser::run_sandboxed(
        std::path::Path::new("/usr/bin/python3"),
        &["script.py".to_string()],
        &dir,
        &dir,
        &crate::tool_supply::macos_static_parser::SandboxLimits::default(),
        &|| cancel.is_cancelled(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(run) => serde_json::json!({
            "accepted": true,
            "exitCode": run.exit_code,
            "timedOut": run.timed_out,
            "networkDisabled": run.network_disabled,
            "stdout": String::from_utf8_lossy(&run.stdout).chars().take(12_000).collect::<String>(),
            "stderr": String::from_utf8_lossy(&run.stderr).chars().take(4_000).collect::<String>(),
            "truncated": run.truncated,
        }),
        Err(error) => agent_tool_error(&error.describe(), "sandbox_run_failed"),
    }
}
