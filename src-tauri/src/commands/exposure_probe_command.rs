// #4 — operator-triggered deterministic exposure probe against the scan's
// authorized target. Makes bounded read-only GETs for the common exposed
// endpoints and the homepage, classifies them with the shared rules in
// exposure_endpoints.rs, and records findings. This is an explicit operator recon
// action (like running nikto by hand) — NOT an autonomous attack — so it carries
// its own small hard cap and stays off the agent budget ledger (that is why it is
// a command, not a change to the in-scan recon). SSRF floor: an internal/loopback
// literal target is refused. In its own source unit so it does not touch the
// exposure_endpoints.rs file another session is editing.

/// Returns the number of exposed-endpoint findings written (homepage version/CORS
/// findings are upserted best-effort on top).
#[tauri::command]
pub fn probe_exposed_endpoints(
    state: State<AppState>,
    scan_id: String,
    base_url: String,
) -> Result<usize, String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    let parsed = reqwest::Url::parse(&base_url).map_err(|_| "base_url 无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("只支持 http/https 目标".into());
    }
    let host = parsed.host_str().ok_or("base_url 缺少主机名")?;
    if host_is_internal_literal(host) {
        return Err("内网/环回/链路本地地址默认拒绝（SSRF 硬守卫）；内网探测须经签名授权".into());
    }
    let connection = db::open(&state.db_path)?;
    let exists: bool = connection
        .query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)", [&scan_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("任务不存在".into());
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| "HTTP 客户端不可用".to_string())?;
    let fetch = |url: &str| -> Option<(u16, Vec<(String, String)>, String)> {
        let resp = client
            .get(url)
            .header(reqwest::header::USER_AGENT, "oviraptor-exposure/1.0")
            .header(reqwest::header::CACHE_CONTROL, "no-store")
            .send()
            .ok()?;
        let status = resp.status().as_u16();
        let headers = agent_security_relevant_headers(resp.headers());
        let mut buf = Vec::new();
        let _ = std::io::Read::take(resp, 262_144).read_to_end(&mut buf);
        Some((status, headers, String::from_utf8_lossy(&buf).into_owned()))
    };
    // Homepage: version/middleware + CORS (best-effort, upserted).
    if let Some((status, headers, body)) = fetch(parsed.as_str()) {
        emit_homepage_exposure_findings(&connection, &scan_id, &base_url, status, &headers, &body);
    }
    // Catalog endpoints.
    let base = base_url.trim_end_matches('/');
    let mut written = 0usize;
    for probe in EXPOSURE_PROBES {
        let url = format!("{base}{}", probe.path);
        let Some((status, _headers, body)) = fetch(&url) else { continue };
        if let Some((title, severity, cwe)) = classify_exposure_endpoint(probe.path, status, &body) {
            let _ = connection.execute(
                "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
                 VALUES(?1,?2,'exposure','exposed_endpoint',?3,?4,?5,?6) \
                 ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
                 title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
                params![scan_id, base_url, probe.path, title, severity, serde_json::json!({"url": url, "status": status, "cwe": cwe}).to_string()],
            );
            written += 1;
        }
    }
    Ok(written)
}
