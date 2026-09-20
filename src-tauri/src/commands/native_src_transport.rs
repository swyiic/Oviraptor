fn adapter_request_once(contract: &JsonValue, allowed_host: &str) -> JsonValue {
    if let Err(error) = native_race_contract(contract, allowed_host) {
        return serde_json::json!({"error":error});
    }
    let result = (|| -> Result<JsonValue, String> {
        let url =
            reqwest::Url::parse(&value_first(contract, &["url"])).map_err(|_| "invalid_url")?;
        let method = value_first(contract, &["method"]).to_ascii_uppercase();
        let method = reqwest::Method::from_bytes(if method.is_empty() {
            b"GET"
        } else {
            method.as_bytes()
        })
        .map_err(|_| "invalid_method")?;
        let timeout = contract
            .get("timeout")
            .and_then(JsonValue::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(5.0)
            .clamp(0.5, 15.0);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs_f64(timeout))
            .http1_only()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        let mut request = client.request(method, url);
        if let Some(headers) = contract.get("headers").and_then(JsonValue::as_object) {
            for (name, value) in headers {
                if let Some(value) = value.as_str() {
                    request = request.header(name, value);
                }
            }
        }
        if let Some(body) = contract.get("body") {
            request = request.body(
                body.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| body.to_string()),
            );
        }
        let started = Instant::now();
        let mut response = request.send().map_err(|e| e.without_url().to_string())?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .chars()
            .take(160)
            .collect::<String>();
        let mut bytes = Vec::new();
        Read::by_ref(&mut response)
            .take(262144)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(
            serde_json::json!({"status":status,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes)),
            "elapsedMs":started.elapsed().as_millis(),"contentType":content_type,"truncated":bytes.len()>=262144}),
        )
    })();
    result.unwrap_or_else(
        |error| serde_json::json!({"error":error.chars().take(300).collect::<String>()}),
    )
}

fn native_raw_request_method(target: &reqwest::Url, request: &[u8]) -> Result<String, String> {
    if request.is_empty() || request.len() > 65536 {
        return Err("raw request must be 1..65536 bytes".into());
    }
    if !matches!(target.scheme(), "http" | "https") || target.host_str().is_none() {
        return Err("url must use http:// or https:// with an explicit host".into());
    }
    let headers_end = request
        .windows(4)
        .position(|v| v == b"\r\n\r\n")
        .unwrap_or(request.len());
    let headers = String::from_utf8_lossy(&request[..headers_end]);
    let mut lines = headers.lines();
    let parts = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>();
    if parts.len() < 3 {
        return Err("raw request is missing a valid request line".into());
    }
    let method = parts[0].to_ascii_uppercase();
    if matches!(method.as_str(), "DELETE" | "CONNECT" | "TRACE") {
        return Err(format!(
            "method {method} is disabled by the built-in adapter"
        ));
    }
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("host") {
                let authority =
                    reqwest::Url::parse(&format!("{}://{}/", target.scheme(), value.trim()))
                        .map_err(|_| "invalid raw request Host")?;
                if authority.host_str() != target.host_str()
                    || authority.port_or_known_default() != target.port_or_known_default()
                {
                    return Err("raw request Host must match the explicit target authority".into());
                }
            }
        }
    }
    Ok(method)
}

fn native_raw_http(target_url: &str, request: &[u8]) -> JsonValue {
    fn exchange(target_url: &str, request: &[u8]) -> Result<JsonValue, String> {
        use rustls_platform_verifier::BuilderVerifierExt;
        use std::net::ToSocketAddrs;
        let target = reqwest::Url::parse(target_url).map_err(|_| "invalid explicit target URL")?;
        let method = native_raw_request_method(&target, request)?;
        let host = target.host_str().ok_or("missing target host")?;
        let port = target
            .port_or_known_default()
            .ok_or("missing target port")?;
        let timeout = Duration::from_secs(5);
        let started = Instant::now();
        let mut socket = None;
        let mut error = "no resolved address".to_string();
        for address in (host.trim_matches(['[', ']']), port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
        {
            match std::net::TcpStream::connect_timeout(&address, timeout) {
                Ok(stream) => {
                    socket = Some(stream);
                    break;
                }
                Err(e) => error = e.to_string(),
            }
        }
        let socket = socket.ok_or(error)?;
        socket
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        trait RawStream: Read + Write {}
        impl<T: Read + Write> RawStream for T {}
        let mut stream: Box<dyn RawStream> = if target.scheme() == "https" {
            // Preserve certificate verification; never fall back to cleartext
            // or silently accept an untrusted certificate.
            let config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_platform_verifier()
            .map_err(|e| e.to_string())?
            .with_no_client_auth();
            let server_name =
                rustls::pki_types::ServerName::try_from(host.trim_matches(['[', ']']).to_string())
                    .map_err(|e| e.to_string())?;
            let connection =
                rustls::ClientConnection::new(std::sync::Arc::new(config), server_name)
                    .map_err(|e| e.to_string())?;
            Box::new(rustls::StreamOwned::new(connection, socket))
        } else {
            Box::new(socket)
        };
        stream.write_all(request).map_err(|e| e.to_string())?;
        stream.flush().map_err(|e| e.to_string())?;
        let mut response = Vec::new();
        let mut buffer = [0; 16384];
        while response.len() < 262144 {
            let remaining = buffer.len().min(262144 - response.len());
            match stream.read(&mut buffer[..remaining]) {
                Ok(0) => break,
                Ok(count) => response.extend_from_slice(&buffer[..count]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    break
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        let status_line = response
            .split(|v| *v == b'\n')
            .next()
            .unwrap_or_default()
            .iter()
            .map(|v| char::from(*v))
            .collect::<String>();
        Ok(serde_json::json!({
            "ok":true,"adapter":"bounded-raw-http","transport":"rust-native-tcp-tls",
            "target":format!("{}://{}:{}",target.scheme(),host,port),"requestMethod":method,
            "requestBytes":request.len(),"responseBytes":response.len(),
            "responseSha256":format!("{:x}",Sha256::digest(&response)),
            "statusLine":status_line.trim_end_matches('\r').chars().take(240).collect::<String>(),
            "elapsedMs":started.elapsed().as_millis(),"truncated":response.len()>=262144
        }))
    }
    exchange(target_url, request)
        .unwrap_or_else(|error| serde_json::json!({"ok":false,"error":error}))
}

fn native_race_contract(contract: &JsonValue, allowed_host: &str) -> Result<(), String> {
    if !contract.is_object() {
        return Err("request contract must be a JSON object".into());
    }
    let url = reqwest::Url::parse(&value_first(contract, &["url"])).map_err(|_| "invalid_url")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case(allowed_host))
    {
        return Err("target_host_out_of_scope".into());
    }
    let method = value_first(contract, &["method"]).to_ascii_uppercase();
    if matches!(method.as_str(), "DELETE" | "CONNECT" | "TRACE") {
        return Err("method_disabled".into());
    }
    let body = contract
        .get("body")
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_default();
    if body.len() > 65536 {
        return Err("request body exceeds adapter limit".into());
    }
    Ok(())
}

fn native_race_schedule(
    contract: &JsonValue,
    allowed_host: &str,
    cancelled: &std::sync::atomic::AtomicBool,
) -> JsonValue {
    let validate = || -> Result<(), String> {
        native_race_contract(contract, allowed_host)?;
        let method = value_first(contract, &["method"]).to_ascii_uppercase();
        if !matches!(method.as_str(), "" | "GET" | "HEAD" | "OPTIONS")
            && (contract
                .get("cleanup")
                .and_then(JsonValue::as_object)
                .is_none()
                || value_first(contract, &["invariant"]).trim().is_empty())
        {
            return Err("write race contracts require cleanup.url and a business invariant".into());
        }
        if let Some(cleanup) = contract.get("cleanup") {
            native_race_contract(cleanup, allowed_host)?;
        }
        Ok(())
    };
    if let Err(error) = validate() {
        return serde_json::json!({"ok":false,"error":error});
    }
    let concurrency = contract
        .get("concurrency")
        .and_then(JsonValue::as_u64)
        .unwrap_or(8)
        .clamp(2, 64) as usize;
    let attempts = (contract
        .get("attempts")
        .and_then(JsonValue::as_u64)
        .unwrap_or(16)
        .min(128) as usize)
        .max(concurrency);
    let started = Instant::now();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::<JsonValue>::new());
    let workers_ok = std::thread::scope(|scope| {
        let handles = (0..concurrency)
            .map(|_| {
                scope.spawn(|| {
                    while !cancelled.load(std::sync::atomic::Ordering::Relaxed)
                        && next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < attempts
                    {
                        let result = adapter_request_once(contract, allowed_host);
                        results
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(result);
                    }
                })
            })
            .collect::<Vec<_>>();
        handles.into_iter().all(|handle| handle.join().is_ok())
    });
    let results = results.into_inner().unwrap_or_else(|e| e.into_inner());
    let cleanup = contract
        .get("cleanup")
        .filter(|_| !results.is_empty())
        .map(|value| {
            let mut cleanup = value.clone();
            if cleanup.get("method").is_none() {
                cleanup["method"] = JsonValue::String("POST".into());
            }
            adapter_request_once(&cleanup, allowed_host)
        });
    let mut statuses = std::collections::BTreeMap::<String, usize>::new();
    let mut hashes = HashSet::new();
    let mut errors = attempts.saturating_sub(results.len());
    for result in &results {
        if result.get("error").is_some() {
            errors += 1;
        } else {
            *statuses
                .entry(
                    result
                        .get("status")
                        .map(JsonValue::to_string)
                        .unwrap_or_default(),
                )
                .or_default() += 1;
            hashes.insert(value_first(result, &["sha256"]));
        }
    }
    serde_json::json!({"ok":workers_ok && !cancelled.load(std::sync::atomic::Ordering::Relaxed),"cancelled":cancelled.load(std::sync::atomic::Ordering::Relaxed),"adapter":"bounded-race-scheduler","attempts":attempts,"concurrency":concurrency,
        "statuses":statuses,"distinctResponseHashes":hashes.len(),"errors":errors,"cleanup":cleanup,
        "invariant":value_first(contract,&["invariant"]).chars().take(500).collect::<String>(),"elapsedMs":started.elapsed().as_millis(),
        "note":"response differences are evidence candidates; the business invariant still decides whether a race exists"})
}

#[cfg(test)]
mod native_src_transport_tests {
    use super::*;
    #[test]
    fn native_race_respects_configured_limits_and_preserves_summary_contract() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..4 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = [0; 4096];
                let _ = socket.read(&mut request);
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .unwrap();
            }
        });
        let result = native_race_schedule(
            &serde_json::json!({"url":format!("http://{address}/fixture"),"method":"GET","attempts":4,"concurrency":2}),
            "127.0.0.1",
            &std::sync::atomic::AtomicBool::new(false),
        );
        server.join().unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["attempts"], 4);
        assert_eq!(result["concurrency"], 2);
        assert_eq!(result["statuses"]["200"], 4);
        assert_eq!(result["errors"], 0);
        assert_eq!(result["distinctResponseHashes"], 1);
    }
    #[test]
    fn native_raw_preserves_binary_body_duplicate_headers_and_response_bytes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut request = format!("POST /fixture HTTP/1.1\r\nHost: {address}\r\nX-Fixture: one\r\nX-Fixture: two\r\nContent-Length: 3\r\nConnection: close\r\n\r\n").into_bytes();
        request.extend_from_slice(&[0, 255, 1]);
        let expected = request.clone();
        let response =
            b"HTTP/1.1 200 OK\r\nX-Fixture: one\r\nX-Fixture: two\r\nContent-Length: 2\r\n\r\nok";
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = vec![0; expected.len()];
            socket.read_exact(&mut bytes).unwrap();
            assert_eq!(bytes, expected);
            socket.write_all(response).unwrap();
        });
        let result = native_raw_http(&format!("http://{address}"), &request);
        server.join().unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["responseBytes"], response.len());
        assert_eq!(
            result["responseSha256"],
            format!("{:x}", Sha256::digest(response))
        );
        assert_eq!(result["statusLine"], "HTTP/1.1 200 OK");
    }

    #[test]
    fn native_adapter_rejects_invalid_contracts_before_sending() {
        let target = reqwest::Url::parse("http://example.test:8080").unwrap();
        assert!(
            native_raw_request_method(&target, b"GET / HTTP/1.1\r\nHost: other.test\r\n\r\n")
                .is_err()
        );
        assert!(native_raw_request_method(&target, b"DELETE / HTTP/1.1\r\n\r\n").is_err());
        let contract = serde_json::json!({"url":"http://example.test/fixture","method":"POST"});
        assert_eq!(
            native_race_schedule(
                &contract,
                "example.test",
                &std::sync::atomic::AtomicBool::new(false)
            )["ok"],
            false
        );
        let oversized =
            serde_json::json!({"url":"http://example.test/fixture","body":"x".repeat(65537)});
        assert!(native_race_contract(&oversized, "example.test").is_err());
        let cancelled = native_race_schedule(
            &serde_json::json!({"url":"http://example.test/fixture","method":"GET"}),
            "example.test",
            &std::sync::atomic::AtomicBool::new(true),
        );
        assert_eq!(cancelled["ok"], false);
        assert_eq!(cancelled["cancelled"], true);
        assert_eq!(cancelled["errors"], 16);
    }

    #[test]
    fn native_adapter_does_not_block_callbacks_while_raw_request_is_running() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (started, ready) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buffer = [0; 2048];
            if socket.read(&mut buffer).unwrap_or(0) == 0 {
                return;
            }
            started.send(()).unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(3));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        let root =
            std::env::temp_dir().join(format!("oviraptor-adapter-callback-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let receiver = stage_builtin_src_assurance(&format!("http://{address}"), &root).unwrap();
        let raw_url = receiver.raw_url.clone();
        let raw = thread::spawn(move || {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap()
                .post(raw_url)
                .body(format!(
                    "GET /fixture HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
                ))
                .send()
                .unwrap()
                .text()
                .unwrap()
        });
        let request_started = ready.recv_timeout(Duration::from_secs(3));
        let callback = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap()
            .get(&receiver.base_url)
            .send();
        let _ = release.send(());
        let raw_result = raw.join().unwrap();
        server.join().unwrap();
        assert!(request_started.is_ok());
        assert_eq!(callback.unwrap().status().as_u16(), 204);
        assert_eq!(
            serde_json::from_str::<JsonValue>(&raw_result).unwrap()["ok"],
            true
        );
        drop(receiver);
        let _ = fs::remove_dir_all(root);
    }
}
