// RCE command gate (Track C, code_exec capability). Proof-oriented: the agent
// probes command execution through a vulnerability it already found, to PROVE
// the finding — not to operate on the host. This module decides, deterministically
// and never by the model, whether a command may run at all, run automatically
// (a harmless identity/read-only safelist), or needs the operator's single-use
// per-action approval. A target page can inject text that drives the agent, so
// the classification is a fixed rule here, not the agent's judgement.
//
// It executes nothing. The probe tool calls `rce_command_gate_on` and only on
// `Allow` sends the payload — through the same brokered HTTP transport that keeps
// the scope / SSRF / budget / identity gates. Three composed floors:
//   1. code_exec must be authorized for this scan attempt (the mandatory switch);
//   2. the command class (harmless / needs-approval / denied);
//   3. for needs-approval, the single-use human approval (high_risk_action_gate_on).
//
// Both `scan_capability_class_authorized` and `high_risk_action_gate_on` live in
// sibling source units include!'d into the same `scanning` module.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RceCommandClass {
    /// Identity / read-only network safelist (id, whoami, ifconfig, …). Proves
    /// code execution with no side effect; auto-allowed once code_exec is authorized.
    Harmless,
    /// Anything else that is not catastrophic: the operator approves one run.
    NeedsApproval,
    /// Catastrophic / irreversible (filesystem wipe, fork bomb, shutdown, …).
    /// Not auto-run. The operator must approve this exact command once so the
    /// affected party can see the proof; a refusal still blocks it.
    Denied,
}

// Base programs whose identity/read-only output proves execution without touching
// state. Kept deliberately small; everything outside it goes to human approval.
const RCE_HARMLESS_PROGRAMS: &[&str] =
    &["id", "whoami", "ifconfig", "uname", "hostname", "pwd", "ip", "arch"];

// Catastrophic / irreversible fragments. Case-insensitive substring match: this
// is a floor against obvious destruction, not a complete sandbox — the real
// boundary is that only `Harmless` auto-runs and everything else needs approval.
const RCE_DESTRUCTIVE_FRAGMENTS: &[&str] = &[
    "rm -r", "rm -f", "rm -rf", "rmdir ", "unlink ", "mkfs", "dd if=", "dd of=",
    "of=/dev/", "> /dev/sd", ">/dev/sd", ":(){", "fork()", "shutdown", "reboot",
    "halt", "poweroff", "init 0", "init 6", "wipefs", "fdisk", "parted", "mkswap",
    "> /etc/", ">/etc/", "truncate -s 0", "chmod -r 000", "chown -r ", "killall -9",
];
// Note: off-host fetch (curl/wget), pipe-to-shell and the like are NOT hard-denied
// — they are dual-use pentest actions, so they fall to NeedsApproval and the
// operator decides. Catastrophic / irreversible host destruction is the same
// path: one explicit approval for that exact command, never an automatic run.

/// Classify a command for the RCE proof gate. Checks the catastrophic floor first
/// (so `id && rm -rf /` is Denied, not Harmless), then the harmless safelist,
/// which additionally requires no shell metacharacters (chaining / redirection)
/// and only simple flag/sub-command arguments. Everything else is NeedsApproval.
fn classify_rce_command(command: &str) -> RceCommandClass {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return RceCommandClass::NeedsApproval;
    }
    let lower = trimmed.to_ascii_lowercase();
    if RCE_DESTRUCTIVE_FRAGMENTS
        .iter()
        .any(|fragment| lower.contains(fragment))
    {
        return RceCommandClass::Denied;
    }
    // Shell chaining / redirection / substitution can hide a second command behind
    // a harmless-looking first token, so their presence bars the harmless tier.
    let has_shell_metachars = trimmed.chars().any(|c| matches!(c, ';' | '|' | '&' | '`' | '>' | '<' | '\n' | '\r' | '('))
        || trimmed.contains("$(")
        || trimmed.contains("${");
    if !has_shell_metachars {
        let mut parts = trimmed.split_whitespace();
        if let Some(program) = parts.next() {
            let program = program.to_ascii_lowercase();
            let args_simple = parts.all(|arg| {
                arg.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            });
            if RCE_HARMLESS_PROGRAMS.contains(&program.as_str()) && args_simple {
                return RceCommandClass::Harmless;
            }
        }
    }
    RceCommandClass::NeedsApproval
}

/// Stable fingerprint for the per-action approval row: the exact (trimmed) command,
/// so approving one command never approves a different one.
fn rce_action_fingerprint(command: &str) -> String {
    let digest = Sha256::digest(format!("code_exec:{}", command.trim()).as_bytes());
    format!("{digest:x}")
}

/// The gate the probe tool calls before executing a command. Returns an
/// `Allow` only when code_exec is authorized AND the command is harmless, or a
/// single-use approval exists; otherwise Pending (awaiting the operator) or Denied.
// Live: called by agent_tool_probe_command_execution, which is dispatched in
// agent_tools_dispatch.rs and offered only when code_exec is authorized.
fn rce_command_gate_on(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    fencing_token: &str,
    command: &str,
    target: &str,
) -> Result<HighRiskGate, String> {
    if !scan_capability_class_authorized(connection, scan_id, attempt_number, "code_exec")? {
        return Ok(HighRiskGate::Denied(
            "code_exec 能力未被该任务授权；无法执行命令".into(),
        ));
    }
    match classify_rce_command(command) {
        RceCommandClass::Harmless => Ok(HighRiskGate::Allow),
        RceCommandClass::NeedsApproval => high_risk_action_gate_on(
            connection,
            scan_id,
            attempt_number,
            fencing_token,
            "code_exec",
            &rce_action_fingerprint(command),
            &format!("执行命令以证明 RCE：{}", command.trim()),
            target,
            false,
            Some("probe_command_execution"),
        ),
        RceCommandClass::Denied => high_risk_action_gate_on(
            connection,
            scan_id,
            attempt_number,
            fencing_token,
            "code_exec",
            &rce_action_fingerprint(command),
            &format!("破坏性命令，需操作者单次同意后才会发到目标：{}", command.trim()),
            target,
            false,
            Some("probe_command_execution"),
        ),
    }
}

/// The live coordinator fencing token for this run's scan attempt, so a consumed
/// approval cannot carry across a lease epoch. Best-effort: a missing lease table
/// or row (single-tool / test contexts) yields an empty token rather than failing
/// — the approval binding then degrades to (scan, attempt, "") which is still
/// per-attempt and single-use.
fn current_coordinator_fencing_token(connection: &rusqlite::Connection, context: &AgentRunContext) -> String {
    connection
        .query_row(
            "SELECT fencing_token FROM agent_coordinator_leases WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3",
            params![context.scan_id, context.attempt_number, context.target_url],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .unwrap_or_default()
}

/// Prove a command-injection finding on the AUTHORIZED target by delivering one
/// gated command through the vulnerable request and capturing the output. This is
/// the only sanctioned path for command execution: the raw `command` is gated by
/// `rce_command_gate_on` (code_exec authorized? harmless? else single-use operator
/// approval; irreversible destruction uses that same single-use approval, with
/// the command text shown to the operator) and only then substituted
/// for the `{{CMD}}` placeholder and sent through the SAME brokered transport as
/// replay (scope / SSRF / budget / identity gates all apply). v1 is read-method
/// only, so it cannot be used to smuggle a write past the mutation-contract gate.
fn agent_tool_probe_command_execution(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let command = value_first(arguments, &["command"]);
    if command.trim().is_empty() {
        return agent_tool_error("缺少 command", "invalid_arguments");
    }
    let method = {
        let raw = value_first(arguments, &["method"]);
        if raw.is_empty() { "GET".to_string() } else { raw.to_ascii_uppercase() }
    };
    let url_template = value_first(arguments, &["url"]);
    if url_template.is_empty() {
        return agent_tool_error("缺少 url", "invalid_arguments");
    }
    if !AGENT_READ_METHODS.contains(&method.as_str()) {
        if !matches!(method.as_str(), "POST" | "PUT" | "PATCH") {
            return agent_tool_error("命令投递只允许只读方法，或经写门禁的 POST/PUT/PATCH", "method_not_supported");
        }
        if let Some(gate) = agent_write_gate(context, runtime, arguments, &method, &url_template) {
            return gate;
        }
    }
    // Deterministic command gate (never the model's judgement).
    let connection = match db::open(&context.db_path) {
        Ok(connection) => connection,
        Err(error) => return agent_tool_error(&error, "tool_authorization_unavailable"),
    };
    let fencing = current_coordinator_fencing_token(&connection, context);
    let gate = match rce_command_gate_on(
        &connection,
        &context.scan_id,
        context.attempt_number,
        &fencing,
        &command,
        &context.target_url,
    ) {
        Ok(gate) => gate,
        Err(error) => return agent_tool_error(&error, "high_risk_gate_unavailable"),
    };
    match gate {
        HighRiskGate::Denied(reason) => {
            return serde_json::json!({
                "accepted": false,
                "code": "high_risk_action_denied",
                "error": reason,
                "command": command,
            });
        }
        HighRiskGate::Pending(request_id) => {
            return serde_json::json!({
                "accepted": false,
                "code": "high_risk_action_pending_approval",
                "requestId": request_id,
                "command": command,
                "hint": "该命令需操作者单次批准；operator 在任务里批准后，用完全相同的参数重试本工具",
            });
        }
        HighRiskGate::Allow => {}
    }
    // Substitute the GATED command for the placeholder, so what is sent is exactly
    // what was authorized — the agent cannot gate `id` and smuggle something else.
    let placeholder = {
        let raw = value_first(arguments, &["placeholder"]);
        if raw.is_empty() { "{{CMD}}".to_string() } else { raw }
    };
    let url = url_template.replace(&placeholder, command.trim());
    let mut extra_headers = Vec::new();
    if let Some(map) = arguments.get("headers").and_then(JsonValue::as_object) {
        for (name, value) in map {
            if let Some(text) = value.as_str() {
                extra_headers.push((name.to_ascii_lowercase(), text.replace(&placeholder, command.trim())));
            }
        }
    }
    let family = {
        let raw = value_first(arguments, &["family"]);
        if raw.is_empty() { "command_injection".to_string() } else { raw }
    };
    let response = match agent_http_request(
        context,
        runtime,
        &value_first(arguments, &["identity"]),
        &method,
        &url,
        extra_headers,
        None,
        None,
        "",
        &family,
        ScopeSource::HttpReplay,
        "probe_command_execution",
    ) {
        Ok(value) => value,
        Err(error) => return error,
    };
    if let Err(error) = persist_agent_evidence(context, &response, "probe_command_execution") {
        return serde_json::json!({"error": error, "code": "evidence_write_failed"});
    }
    let mut out = response;
    if let Some(object) = out.as_object_mut() {
        object.insert("rceCommand".into(), serde_json::json!(command.trim()));
        object.insert("rceDelivered".into(), serde_json::json!(true));
    }
    out
}

#[cfg(test)]
mod rce_command_gate_tests {
    use super::{
        classify_rce_command, decide_high_risk_action_on, rce_command_gate_on,
        set_scan_capability_authorization_on, HighRiskGate, RceCommandClass,
    };
    use rusqlite::Connection;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY);
             INSERT INTO sentinel_scans(id) VALUES('scan-a');",
        )
        .unwrap();
        c
    }

    #[test]
    fn harmless_safelist_is_classified_harmless() {
        for cmd in ["id", "whoami", "ifconfig", "uname -a", "ip addr", "id -u"] {
            assert_eq!(classify_rce_command(cmd), RceCommandClass::Harmless, "{cmd}");
        }
    }

    #[test]
    fn destructive_commands_are_denied_even_chained_after_harmless() {
        for cmd in ["rm -rf /", "shutdown now", "id && rm -rf /var", "dd if=/dev/zero of=/dev/sda", ":(){ :|:& };:"] {
            assert_eq!(classify_rce_command(cmd), RceCommandClass::Denied, "{cmd}");
        }
    }

    #[test]
    fn other_commands_and_chaining_need_approval() {
        for cmd in ["cat /etc/passwd", "id; whoami", "ls -la /root", "curl http://x", "uname > /tmp/x"] {
            assert_eq!(classify_rce_command(cmd), RceCommandClass::NeedsApproval, "{cmd}");
        }
    }

    #[test]
    fn gate_denies_when_code_exec_not_authorized() {
        let c = db();
        // Even a harmless command is denied if the scan never authorized code_exec.
        assert!(matches!(
            rce_command_gate_on(&c, "scan-a", 1, "fence-1", "id", "http://t").unwrap(),
            HighRiskGate::Denied(_)
        ));
    }

    #[test]
    fn gate_allows_harmless_once_authorized() {
        let c = db();
        set_scan_capability_authorization_on(&c, "scan-a", 1, true, &["code_exec".into()], "op").unwrap();
        assert_eq!(
            rce_command_gate_on(&c, "scan-a", 1, "fence-1", "whoami", "http://t").unwrap(),
            HighRiskGate::Allow
        );
        // Destructive is not auto-run; the operator must approve that exact command.
        assert!(matches!(
            rce_command_gate_on(&c, "scan-a", 1, "fence-1", "rm -rf /", "http://t").unwrap(),
            HighRiskGate::Pending(_)
        ));
    }

    #[test]
    fn gate_requires_single_use_approval_for_other_commands() {
        let c = db();
        set_scan_capability_authorization_on(&c, "scan-a", 1, true, &["code_exec".into()], "op").unwrap();
        // First ask records a pending request.
        let pending = rce_command_gate_on(&c, "scan-a", 1, "fence-1", "cat /etc/passwd", "http://t").unwrap();
        let request_id = match pending {
            HighRiskGate::Pending(id) => id,
            other => panic!("expected Pending, got {other:?}"),
        };
        // Operator approves; the next gate consumes it exactly once.
        decide_high_risk_action_on(&c, &request_id, true, "op").unwrap();
        assert_eq!(
            rce_command_gate_on(&c, "scan-a", 1, "fence-1", "cat /etc/passwd", "http://t").unwrap(),
            HighRiskGate::Allow
        );
        // Single-use: the same action under the same fencing token cannot re-run
        // on the spent approval — it is Denied ("已被使用，需重新批准"), not silently
        // re-approved and not a fresh Pending. A new run needs a new fencing epoch.
        assert!(matches!(
            rce_command_gate_on(&c, "scan-a", 1, "fence-1", "cat /etc/passwd", "http://t").unwrap(),
            HighRiskGate::Denied(_)
        ));
    }
}
