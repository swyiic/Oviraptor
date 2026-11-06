// #6 — Code-exec B: target-scoped network sandbox. Agent-written PoC code runs in
// the macOS Seatbelt sandbox with network egress RE-ALLOWED only to one localhost
// port, where a DETERMINISTIC forward-proxy (below) connects exclusively to the
// vetted authorized target. So the PoC can reach the target — and ONLY the target:
// Seatbelt denies every other host/port and all DNS (verified by the egress
// enforcement test), and the proxy hardcodes the one target address (never the
// code's choice), so there is no path to any other destination.
//
// Why a proxy instead of pinning the target IP in the profile: sandbox-exec's
// `remote ip` grammar accepts only `*` or `localhost` for the host (a specific IP
// literal is rejected — verified empirically), so per-IP egress cannot be expressed
// directly. Allowing one localhost port + a fixed-target proxy is the deterministic
// equivalent that Seatbelt CAN enforce.
//
// Gated like every high-risk capability: the scan must authorize BOTH code_exec
// (arbitrary code) AND egress (target network), and each distinct script needs the
// operator's single-use approval. The target is context.target_url, resolved and
// vetted here (reusing the same SSRF pin as the broker) — never a model-chosen host.

/// A deterministic loopback→target forward-proxy. It listens on 127.0.0.1:<ephemeral>
/// and forwards EVERY accepted connection to one fixed, pre-vetted target address —
/// the connecting code cannot choose or change the destination. Bounded: a max
/// connection count and per-connection byte cap, idle read timeouts, and a clean
/// shutdown on drop.
struct TargetEgressProxy {
    port: u16,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl TargetEgressProxy {
    fn start(target: std::net::SocketAddr, max_conns: usize, per_conn_bytes: usize) -> std::io::Result<Self> {
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sd = shutdown.clone();
        let handle = std::thread::spawn(move || {
            let mut conns = 0usize;
            loop {
                if sd.load(Ordering::Relaxed) {
                    break;
                }
                match listener.accept() {
                    Ok((client, _)) => {
                        if conns >= max_conns {
                            let _ = client.shutdown(std::net::Shutdown::Both);
                            continue;
                        }
                        conns += 1;
                        std::thread::spawn(move || proxy_forward(client, target, per_conn_bytes));
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self { port, shutdown, handle: Some(handle) })
    }
    fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for TargetEgressProxy {
    fn drop(&mut self) {
        self.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Copy up to `cap` bytes from `from` to `to`, then close both halves.
fn proxy_pump(mut from: std::net::TcpStream, mut to: std::net::TcpStream, cap: usize) {
    let mut buf = [0u8; 16_384];
    let mut total = 0usize;
    loop {
        match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
                total += n;
                if total > cap {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = to.shutdown(std::net::Shutdown::Both);
    let _ = from.shutdown(std::net::Shutdown::Both);
}

/// Forward one accepted client connection to the fixed target, both directions.
fn proxy_forward(client: std::net::TcpStream, target: std::net::SocketAddr, cap: usize) {
    // A socket accepted from a non-blocking listener can inherit non-blocking mode
    // (macOS), which would make the pump's read() return WouldBlock and forward
    // nothing. Force blocking (with a read timeout below) so the copy works.
    let _ = client.set_nonblocking(false);
    let Ok(server) = std::net::TcpStream::connect_timeout(&target, Duration::from_secs(10)) else {
        let _ = client.shutdown(std::net::Shutdown::Both);
        return;
    };
    let _ = client.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = server.set_read_timeout(Some(Duration::from_secs(30)));
    let (Ok(client2), Ok(server2)) = (client.try_clone(), server.try_clone()) else {
        let _ = client.shutdown(std::net::Shutdown::Both);
        let _ = server.shutdown(std::net::Shutdown::Both);
        return;
    };
    let up = std::thread::spawn(move || proxy_pump(client2, server2, cap));
    proxy_pump(server, client, cap);
    let _ = up.join();
}

/// Resolve and vet the scan's authorized target to a single socket address, reusing
/// the broker's SSRF rules: a literal internal/loopback IP (or a name resolving to
/// one) is refused unless internal_scope is authorized. This is the ONLY place the
/// proxy's destination comes from — never the model.
fn vetted_target_addr(context: &AgentRunContext) -> Result<(std::net::SocketAddr, String, u16), String> {
    use std::net::ToSocketAddrs;
    let parsed = reqwest::Url::parse(&context.target_url).map_err(|_| "目标 URL 无效".to_string())?;
    let host = parsed.host_str().ok_or("目标缺少主机名")?.to_string();
    let port = parsed
        .port_or_known_default()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let allow_internal = agent_internal_scope_authorized(context);
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = stripped.parse::<std::net::IpAddr>() {
        if ip_is_internal(ip) && !allow_internal {
            return Err("目标为内网/环回地址,未授权 internal_scope,拒绝".into());
        }
        return Ok((std::net::SocketAddr::new(ip, port), host, port));
    }
    let addrs: Vec<std::net::SocketAddr> = (stripped, port)
        .to_socket_addrs()
        .map_err(|_| "目标 DNS 解析失败".to_string())?
        .collect();
    if !allow_internal && addrs.iter().any(|a| ip_is_internal(a.ip())) {
        return Err("目标解析到内网地址(可能的 DNS 重绑定),拒绝".into());
    }
    // v1 pins the first IPv4 address (the proxy connects to exactly this one).
    let addr = addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
        .ok_or("目标无可用地址")?;
    Ok((addr, host, port))
}

/// Code-exec B: run a short Python PoC in the Seatbelt sandbox whose ONLY network
/// reach is the authorized target, via the deterministic loopback proxy. Reads
/// ./target.json for the proxy host/port and the real target host/port/scheme (use
/// the proxy address to connect, the target host for TLS SNI / Host header).
fn agent_tool_run_target_poc(
    context: &AgentRunContext,
    _runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let language = value_first(arguments, &["language"]);
    if language != "python" {
        return agent_tool_error(
            "run_target_poc v1 仅支持 language=python(沙箱内 /usr/bin/python3,出网只能到目标)",
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
    // Gate 1: BOTH code_exec (arbitrary code) and egress (target network) authorized.
    let code_exec = scan_capability_class_authorized(&connection, &context.scan_id, context.attempt_number, "code_exec").unwrap_or(false);
    let egress = scan_capability_class_authorized(&connection, &context.scan_id, context.attempt_number, "egress").unwrap_or(false);
    if !(code_exec && egress) {
        return serde_json::json!({
            "accepted": false,
            "code": "high_risk_action_denied",
            "error": "run_target_poc 需同时授权 code_exec 与 egress 两个高危能力类",
        });
    }
    // Resolve+vet the target BEFORE asking for approval, so the operator approves a
    // concrete destination.
    let (target_addr, target_host, target_port) = match vetted_target_addr(context) {
        Ok(value) => value,
        Err(error) => return agent_tool_error(&error, "target_scope_rejected"),
    };
    // Gate 5 (§4d(三) 执行前复扫): PoC code must not call out to anything but the
    // vetted target. Flags external callback URLs / IPs (host ∉ allowed_origins and
    // ≠ the target) + fetch-exec + dep-install; never blobs/keys (legit crypto). The
    // fixed-target egress proxy is still the hard boundary; this catches intent and
    // poisoned material early, before the operator is asked to approve.
    let poison = poison_scan_egress_code(&script, &context.execution_plan.allowed_origins, &target_host);
    if !poison.is_empty() {
        let found = poison.iter().map(|a| format!("{}（{}）", a.kind, a.preview)).collect::<Vec<_>>().join("；");
        return serde_json::json!({
            "accepted": false,
            "code": "poison_blocked",
            "error": format!("PoC 含对目标之外的回连 / 取-执行 / 依赖拉取，已拒绝（防供应链投毒）。只对授权目标发请求、用 stdlib 自己写：{found}"),
        });
    }
    // Gate 2: single-use operator approval, bound to this exact script, on egress.
    let fencing = current_coordinator_fencing_token(&connection, context);
    let fingerprint = format!("{:x}", Sha256::digest(format!("target_poc:{}", script.trim()).as_bytes()));
    let purpose = value_first(arguments, &["purpose"]);
    let summary = if purpose.trim().is_empty() {
        format!("沙箱内运行 PoC,出网仅限目标 {target_host}:{target_port}")
    } else {
        format!("沙箱内运行 PoC(出网仅限目标 {target_host}:{target_port}):{}", purpose.trim())
    };
    match high_risk_action_gate_on(
        &connection, &context.scan_id, context.attempt_number, &fencing,
        "egress", &fingerprint, &summary, &context.target_url, false,
        Some("run_target_poc"),
    ) {
        Ok(HighRiskGate::Allow) => {}
        Ok(HighRiskGate::Pending(request_id)) => {
            return serde_json::json!({
                "accepted": false,
                "code": "high_risk_action_pending_approval",
                "requestId": request_id,
                "hint": "该 PoC 需操作者单次批准;批准后用完全相同参数重试",
            });
        }
        Ok(HighRiskGate::Denied(reason)) => {
            return serde_json::json!({"accepted": false, "code": "high_risk_action_denied", "error": reason});
        }
        Err(error) => return agent_tool_error(&error, "high_risk_gate_unavailable"),
    }

    // Start the deterministic proxy to the one vetted target address.
    let proxy = match TargetEgressProxy::start(target_addr, 64, 16 * 1024 * 1024) {
        Ok(proxy) => proxy,
        Err(_) => return agent_tool_error("无法启动目标出网代理", "egress_proxy_unavailable"),
    };
    let proxy_port = proxy.port();

    let dir = std::env::temp_dir().join(format!("oviraptor-poc-net-{}", uuid::Uuid::new_v4()));
    if std::fs::create_dir_all(&dir).is_err() {
        return agent_tool_error("无法创建沙箱临时目录", "sandbox_scratch_unavailable");
    }
    let target_doc = serde_json::json!({
        "proxyHost": "127.0.0.1",
        "proxyPort": proxy_port,
        "targetHost": target_host,
        "targetPort": target_port,
        "note": "连接 proxyHost:proxyPort;TLS 用 targetHost 作 SNI、HTTP 用 targetHost 作 Host 头。只能到达该目标。"
    });
    if std::fs::write(dir.join("script.py"), script.as_bytes()).is_err()
        || std::fs::write(dir.join("target.json"), target_doc.to_string().as_bytes()).is_err()
    {
        let _ = std::fs::remove_dir_all(&dir);
        return agent_tool_error("无法写入脚本/目标信息", "sandbox_scratch_unavailable");
    }
    if let Some(input) = arguments.get("input").and_then(JsonValue::as_str) {
        let _ = std::fs::write(dir.join("input.dat"), input.as_bytes());
    }

    let cancel = agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number);
    let result = crate::tool_supply::macos_static_parser::run_sandboxed_egress(
        std::path::Path::new("/usr/bin/python3"),
        &["script.py".to_string()],
        &dir,
        &dir,
        &[proxy_port],
        &crate::tool_supply::macos_static_parser::SandboxLimits::default(),
        &|| cancel.is_cancelled(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    drop(proxy); // shut the proxy down before returning
    match result {
        Ok(run) => serde_json::json!({
            "accepted": true,
            "exitCode": run.exit_code,
            "timedOut": run.timed_out,
            "networkScopedToTarget": !run.network_disabled,
            "target": format!("{target_host}:{target_port}"),
            "stdout": String::from_utf8_lossy(&run.stdout).chars().take(12_000).collect::<String>(),
            "stderr": String::from_utf8_lossy(&run.stderr).chars().take(4_000).collect::<String>(),
            "truncated": run.truncated,
        }),
        Err(error) => agent_tool_error(&error.describe(), "sandbox_run_failed"),
    }
}

#[cfg(test)]
mod target_egress_proxy_tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn proxy_forwards_only_to_the_fixed_target() {
        // A stand-in "target" echo-ish server that returns a fixed banner.
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_addr = target.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in target.incoming() {
                if let Ok(mut s) = stream {
                    let _ = s.write_all(b"TARGET-BANNER");
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
            }
        });
        let proxy = TargetEgressProxy::start(target_addr, 8, 1_000_000).unwrap();
        // Connecting to the proxy port reaches the fixed target, whatever the client
        // "intended" — the destination is not the client's to choose.
        let mut c = TcpStream::connect(("127.0.0.1", proxy.port())).unwrap();
        let mut buf = Vec::new();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let _ = c.read_to_end(&mut buf);
        assert_eq!(&buf, b"TARGET-BANNER");
    }

    #[test]
    fn poc_reaches_target_only_through_proxy_not_directly() {
        // End-to-end #6: a real Python PoC in the egress sandbox reaches a real HTTP
        // target ONLY via the deterministic proxy; a direct connect to the target's
        // own port (not the allowed localhost proxy port) is refused by Seatbelt.
        if !Path::new("/usr/bin/sandbox-exec").exists() || !Path::new("/usr/bin/python3").exists() {
            eprintln!("sandbox-exec/python3 unavailable; skipping #6 end-to-end");
            return;
        }
        // A tiny HTTP target.
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_port = target.local_addr().unwrap().port();
        let target_addr = target.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in target.incoming() {
                if let Ok(mut s) = stream {
                    let _ = s.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 9\r\n\r\nTARGET-OK");
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
            }
        });
        let proxy = TargetEgressProxy::start(target_addr, 8, 1_000_000).unwrap();
        let proxy_port = proxy.port();

        let dir = std::env::temp_dir().join(format!("ovr-poc-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = format!(
            "import socket\n\
             def get(host,port):\n\
             \x20   try:\n\
             \x20       c=socket.create_connection((host,port),timeout=3)\n\
             \x20       c.sendall(b'GET / HTTP/1.0\\r\\nHost: t\\r\\n\\r\\n')\n\
             \x20       d=c.recv(256); c.close(); return d.decode('latin1','replace')\n\
             \x20   except Exception as e:\n\
             \x20       return 'ERR:'+type(e).__name__\n\
             print('PROXY='+('TARGET-OK' in get('127.0.0.1',{proxy_port}) and 'REACHED' or 'MISS'))\n\
             print('DIRECT='+get('127.0.0.1',{target_port}))\n"
        );
        std::fs::write(dir.join("script.py"), script).unwrap();

        let mut limits = crate::tool_supply::macos_static_parser::SandboxLimits::default();
        limits.wall_seconds = 20;
        let run = crate::tool_supply::macos_static_parser::run_sandboxed_egress(
            Path::new("/usr/bin/python3"),
            &["script.py".to_string()],
            &dir,
            &dir,
            &[proxy_port],
            &limits,
            &|| false,
        )
        .expect("egress sandbox run");
        let _ = std::fs::remove_dir_all(&dir);
        drop(proxy);
        let out = String::from_utf8_lossy(&run.stdout);
        assert!(out.contains("PROXY=REACHED"), "PoC must reach the target through the proxy; got: {out}\nstderr: {}", String::from_utf8_lossy(&run.stderr));
        assert!(out.contains("DIRECT=ERR:"), "direct connect to the target's own port must be refused; got: {out}");
    }

    #[test]
    fn proxy_shuts_down_on_drop() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_addr = target.local_addr().unwrap();
        let proxy = TargetEgressProxy::start(target_addr, 8, 1000).unwrap();
        let port = proxy.port();
        drop(proxy);
        // After shutdown the listener is gone, so a connect either refuses or the
        // stream closes immediately with no data.
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(mut c) = TcpStream::connect(("127.0.0.1", port)) {
            c.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            let mut buf = [0u8; 4];
            assert!(matches!(c.read(&mut buf), Ok(0) | Err(_)), "closed proxy must not serve");
        }
    }
}
