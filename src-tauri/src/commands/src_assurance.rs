const SRC_ASSURANCE_ADAPTER_NAME: &str = "src-capabilities.json";

struct BuiltInOastReceiver {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    base_url: String,
    poll_url: String,
    raw_url: String,
    race_url: String,
    network_reachable: bool,
}

impl Drop for BuiltInOastReceiver {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn target_host_port(target_url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = target_url.split_once("://")?;
    let authority = rest.split('/').next()?.rsplit('@').next()?;
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let host = authority.get(1..end)?.to_string();
        let port = authority
            .get(end + 1..)
            .and_then(|value| value.strip_prefix(':'))
            .and_then(|value| value.parse().ok())
            .unwrap_or(if scheme == "https" { 443 } else { 80 });
        return Some((host, port));
    }
    let mut parts = authority.rsplitn(2, ':');
    let last = parts.next()?;
    let previous = parts.next();
    if let (Some(host), Ok(port)) = (previous, last.parse::<u16>()) {
        Some((host.to_string(), port))
    } else {
        Some((
            authority.to_string(),
            if scheme == "https" { 443 } else { 80 },
        ))
    }
}

fn local_address_for_target(target_url: &str) -> Option<std::net::IpAddr> {
    use std::net::ToSocketAddrs;
    let (host, port) = target_host_port(target_url)?;
    let destination = (host.as_str(), port).to_socket_addrs().ok()?.next()?;
    let bind = if destination.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let socket = std::net::UdpSocket::bind(bind).ok()?;
    socket.connect(destination).ok()?;
    socket.local_addr().ok().map(|value| value.ip())
}

fn oast_event_value(source: std::net::SocketAddr, request: &[u8], token: &str) -> JsonValue {
    let first_line = request
        .split(|byte| *byte == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).trim().to_string())
        .unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let mut hasher = Sha256::new();
    hasher.update(request);
    serde_json::json!({
        "receivedAt": chrono::Utc::now().to_rfc3339(),
        "sourceIp": source.ip().to_string(),
        "method": method.chars().take(16).collect::<String>(),
        "path": path.chars().take(1000).collect::<String>(),
        "tokenMatched": path.contains(token),
        "requestBytes": request.len(),
        "requestSha256": format!("{:x}", hasher.finalize())
    })
}

fn adapter_http_response(status: &str, value: &JsonValue) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{\"ok\":false}".to_vec());
    let mut response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
    response.extend(body);
    response
}

fn start_builtin_oast_receiver(
    target_url: &str,
    target_dir: &Path,
) -> Result<BuiltInOastReceiver, String> {
    let listener = std::net::TcpListener::bind("0.0.0.0:0")
        .map_err(|error| format!("内置 OAST 监听失败：{error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let local_ip = local_address_for_target(target_url)
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    let network_reachable = !local_ip.is_loopback() && !local_ip.is_unspecified();
    let display_ip = if local_ip.is_ipv6() {
        format!("[{local_ip}]")
    } else {
        local_ip.to_string()
    };
    let token = Uuid::new_v4().simple().to_string();
    let base_url = format!("http://{display_ip}:{port}/callback/{token}");
    let poll_url = format!("http://{display_ip}:{port}/events/{token}");
    let raw_url = format!("http://{display_ip}:{port}/adapter/{token}/raw");
    let race_url = format!("http://{display_ip}:{port}/adapter/{token}/race");
    let events_path = target_dir.join("oast-events.jsonl");
    let _ = fs::write(&events_path, "");
    let thread_events = events_path.clone();
    let thread_token = token.clone();
    let thread_target_url = target_url.to_string();
    let thread_allowed_host = reqwest::Url::parse(target_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread = std::thread::spawn(move || {
        let mut adapter_jobs = Vec::<std::thread::JoinHandle<()>>::new();
        while !thread_stop.load(std::sync::atomic::Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, source)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 4096];
                    while request.len() < 64 * 1024 {
                        match stream.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(size) => {
                                request.extend_from_slice(&buffer[..size]);
                                if let Some(header_end) = request
                                    .windows(4)
                                    .position(|value| value == b"\r\n\r\n")
                                    .map(|index| index + 4)
                                {
                                    let headers = String::from_utf8_lossy(&request[..header_end]);
                                    let content_length = headers
                                        .lines()
                                        .find_map(|line| {
                                            line.split_once(':')
                                                .filter(|(name, _)| {
                                                    name.eq_ignore_ascii_case("content-length")
                                                })
                                                .and_then(|(_, value)| {
                                                    value.trim().parse::<usize>().ok()
                                                })
                                        })
                                        .unwrap_or(0)
                                        .min(64 * 1024);
                                    if request.len() >= header_end + content_length {
                                        break;
                                    }
                                }
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) =>
                            {
                                break
                            }
                            Err(_) => break,
                        }
                    }
                    let event = oast_event_value(source, &request, &thread_token);
                    let event_path = event.get("path").and_then(JsonValue::as_str).unwrap_or("");
                    let raw_request = event_path == format!("/adapter/{thread_token}/raw");
                    let race_request = event_path == format!("/adapter/{thread_token}/race");
                    if raw_request || race_request {
                        adapter_jobs.retain(|job| !job.is_finished());
                        if adapter_jobs.len() >= 2 {
                            let _ = stream.write_all(&adapter_http_response(
                                "429 Too Many Requests",
                                &serde_json::json!({"ok":false,"error":"adapter_busy"}),
                            ));
                            continue;
                        }
                        let body_offset = request
                            .windows(4)
                            .position(|value| value == b"\r\n\r\n")
                            .map(|index| index + 4)
                            .unwrap_or(request.len());
                        let body = request[body_offset..].to_vec();
                        let target = thread_target_url.clone();
                        let host = thread_allowed_host.clone();
                        let cancelled = thread_stop.clone();
                        adapter_jobs.push(std::thread::spawn(move || {
                            let value = if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                                serde_json::json!({"ok":false,"error":"adapter_cancelled"})
                            } else if raw_request {
                                native_raw_http(&target, &body)
                            } else {
                                match serde_json::from_slice::<JsonValue>(&body) {
                                    Ok(contract) => native_race_schedule(&contract, &host, &cancelled),
                                    Err(error) => serde_json::json!({"ok":false,"error":format!("invalid contract: {error}")}),
                                }
                            };
                            let _ = stream.write_all(&adapter_http_response("200 OK", &value));
                        }));
                        continue;
                    }
                    if event_path == format!("/events/{thread_token}") {
                        let body = fs::read_to_string(&thread_events).unwrap_or_default();
                        let body = format!("[{}]", body.lines().collect::<Vec<_>>().join(","));
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(), body
                        );
                        let _ = stream.write_all(response.as_bytes());
                    } else {
                        if event
                            .get("tokenMatched")
                            .and_then(JsonValue::as_bool)
                            .unwrap_or(false)
                        {
                            if let Ok(mut file) = OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&thread_events)
                            {
                                let _ = writeln!(file, "{event}");
                            }
                        }
                        let _ = stream.write_all(
                            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }
        for job in adapter_jobs {
            let _ = job.join();
        }
    });
    Ok(BuiltInOastReceiver {
        stop,
        thread: Some(thread),
        base_url,
        poll_url,
        raw_url,
        race_url,
        network_reachable,
    })
}

fn stage_builtin_src_assurance(
    target_url: &str,
    target_dir: &Path,
) -> Result<BuiltInOastReceiver, String> {
    let oast = start_builtin_oast_receiver(target_url, target_dir)?;
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "adapter": {
            "path": format!("/workspace/{}/{}", STRIX_WEB_EVIDENCE_DIRECTORY, SRC_ASSURANCE_ADAPTER_NAME),
            "runtime": "rust-native-host-http",
            "commands": {
                "rawHttp": format!("curl -fsS -X POST --data-binary @<bounded-request-file> '{}'", oast.raw_url),
                "race": format!("curl -fsS -X POST -H 'Content-Type: application/json' --data-binary @<request-contract.json> '{}'", oast.race_url)
            },
            "rawHttp": {"available":true,"maxRequestBytes":65536,"maxResponseBytes":262144,"singleConnectionPerInvocation":true},
            "raceScheduler": {"available":true,"maxConcurrency":64,"maxAttempts":128,"writeContractsRequireCleanup":true},
            "controlledWrite": {"available":"contract_gated","deniedMethods":["DELETE","CONNECT","TRACE"]}
        },
        "oast": {
            "available": oast.network_reachable,
            "mode": "builtin-local-http",
            "callbackUrl": oast.base_url,
            "pollUrl": oast.poll_url,
            "reason": if oast.network_reachable { "内置 HTTP 回连监听已按目标路由自动启动" } else { "目标路由只能确定回环地址；外部目标无法回连本机" }
        }
    });
    fs::write(
        target_dir.join("src-capabilities.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(oast)
}
