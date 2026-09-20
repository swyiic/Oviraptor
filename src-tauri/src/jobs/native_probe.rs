const PROBE_OUTCOMES: &[&str] = &[
    "web_alive",
    "web_restricted",
    "browser_render_required",
    "virtual_host_required",
    "web_abnormal",
    "tcp_alive_non_http",
    "blocked_content",
    "unreachable",
    "skipped",
];
const PROBE_FIELDS: &[&str] = &[
    "probe_checked_at",
    "probe_input_url",
    "probe_effective_url",
    "probe_outcome",
    "probe_entry_state",
    "probe_status_code",
    "probe_title",
    "probe_content_type",
    "probe_server",
    "probe_latency_ms",
    "probe_tls_verified",
    "probe_attempts",
    "probe_body_truncated",
    "probe_error",
    "content_category",
    "content_risk_score",
    "content_matches",
    "body_sha256",
    "probe_body_bytes",
    "probe_virtual_host",
    "probe_redirect_scope",
];

#[derive(Clone, Debug)]
pub struct ProbeOptions {
    pub workers: usize,
    pub timeout: Duration,
    pub retries: usize,
    pub max_body_bytes: usize,
    pub include_other: bool,
    pub include_weak: bool,
    pub allow_private: bool,
    pub strict_tls: bool,
    pub scheme_fallback: bool,
    pub content_threshold: i64,
    pub replace_default_content_rules: bool,
    pub gambling_keywords: Vec<String>,
    pub porn_keywords: Vec<String>,
    pub negative_keywords: Vec<String>,
    pub custom_keywords: Vec<String>,
    pub priority_rate: f64,
    pub other_rate: f64,
    pub per_host_interval: Duration,
}

fn build_probe_url(row: &Record) -> Option<String> {
    let mut raw = text(row, &["link", "host", "ip"]);
    if raw.is_empty() {
        return None;
    }
    let protocol = text(row, &["protocol"]).to_ascii_lowercase();
    let port = number(row, &["port"]);
    if !raw.contains("://") {
        let scheme = if protocol.contains("https")
            || [443, 4443, 6443, 7443, 8443, 9443, 10443].contains(&port)
        {
            "https"
        } else if protocol.contains("http") {
            "http"
        } else if port > 0 {
            "tcp+web"
        } else {
            return None;
        };
        let needs_port =
            port > 0 && !((scheme == "http" && port == 80) || (scheme == "https" && port == 443));
        raw = format!(
            "{scheme}://{raw}{}",
            if needs_port {
                format!(":{port}")
            } else {
                String::new()
            }
        );
    }
    Some(raw)
}

fn non_public_literal(url: &str) -> bool {
    let normalized = url
        .replace("tcp+web://", "http://")
        .replace("tcp+tls://", "https://")
        .replace("tcp://", "http://");
    let Ok(parsed) = reqwest::Url::parse(&normalized) else {
        return true;
    };
    let Some(host) = parsed.host_str() else {
        return true;
    };
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
    {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| match ip {
            IpAddr::V4(ip) => {
                ip.is_private()
                    || ip.is_loopback()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || ip.is_broadcast()
            }
            IpAddr::V6(ip) => {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
            }
        })
        .unwrap_or(false)
}

fn empty_probe(url: &str, outcome: &str, error: &str) -> Record {
    let mut result = PROBE_FIELDS
        .iter()
        .map(|field| ((*field).to_string(), String::new()))
        .collect::<Record>();
    result.insert(
        "probe_checked_at".into(),
        chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
    );
    result.insert("probe_input_url".into(), url.into());
    result.insert("probe_effective_url".into(), url.into());
    result.insert("probe_outcome".into(), outcome.into());
    result.insert("probe_error".into(), error.into());
    result.insert("content_category".into(), "not_checked".into());
    result.insert("content_risk_score".into(), "0".into());
    result.insert("probe_attempts".into(), "0".into());
    result
}

fn html_title(body: &str) -> String {
    let lower = body.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(open) = lower[start..].find('>').map(|value| start + value + 1) else {
        return String::new();
    };
    let Some(end) = lower[open..].find("</title").map(|value| open + value) else {
        return String::new();
    };
    body[open..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}

fn javascript_shell(body: &str, content_type: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    if !content_type.to_ascii_lowercase().contains("html") && !lower.contains("<html") {
        return false;
    }
    let scripts = lower.matches("<script").count();
    let root = [
        "id=\"app\"",
        "id='app'",
        "id=\"root\"",
        "id='root'",
        "id=\"__next\"",
        "id=\"__nuxt\"",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    let mut visible = String::new();
    let mut in_tag = false;
    for character in body.chars() {
        if character == '<' {
            in_tag = true;
        } else if character == '>' {
            in_tag = false;
        } else if !in_tag && !character.is_whitespace() {
            visible.push(character);
        }
    }
    visible.chars().count() < 40 && scripts > 0 && (root || scripts >= 2)
}

fn tcp_alive(url: &str, timeout: Duration) -> bool {
    let Ok(parsed) = reqwest::Url::parse(
        url.replace("tcp+web://", "http://")
            .replace("tcp+tls://", "https://")
            .replace("tcp://", "http://")
            .as_str(),
    ) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let Some(port) = parsed.port_or_known_default() else {
        return false;
    };
    (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| {
            addresses.find(|address| TcpStream::connect_timeout(address, timeout).is_ok())
        })
        .is_some()
}

fn probe_one(
    row: &Record,
    options: &ProbeOptions,
    client: &reqwest::blocking::Client,
    content_rules: &native_content::ContentRules,
    pacing: &native_probe_control::ProbePacing,
    rate: f64,
    cancel: &AtomicBool,
) -> Record {
    let Some(input_url) = build_probe_url(row) else {
        return empty_probe("", "skipped", "missing_or_non_web_url");
    };
    if !options.allow_private && non_public_literal(&input_url) {
        return empty_probe(&input_url, "skipped", "non_public_ip");
    }
    let tcp_mode = input_url.starts_with("tcp+") || input_url.starts_with("tcp://");
    let normalized = input_url
        .replace("tcp+web://", "http://")
        .replace("tcp+tls://", "https://");
    let mut candidates = vec![normalized.clone()];
    if tcp_mode || options.scheme_fallback {
        // One scheme alternation, used by both the tcp+ form and the fallback.
        candidates.push(if normalized.starts_with("https://") {
            normalized.replacen("https://", "http://", 1)
        } else {
            normalized.replacen("http://", "https://", 1)
        });
    }
    let mut attempts = 0usize;
    let mut last_error = String::new();
    let mut abnormal = None;
    for candidate in candidates {
        for _ in 0..=options.retries {
            let host = reqwest::Url::parse(&candidate)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string))
                .unwrap_or_default();
            if !pacing.wait(&host, rate, options.per_host_interval, cancel) {
                return empty_probe(&input_url, "skipped", "__CANCELLED__");
            }
            attempts += 1;
            let started = Instant::now();
            match client
                .get(&candidate)
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (compatible; AuthorizedAssetVerifier/2.0)",
                )
                .header("Accept-Encoding", "identity")
                .send()
            {
                Ok(mut response) => {
                    let status = response.status().as_u16();
                    let final_url = response.url().to_string();
                    let content_type = response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    let server = response
                        .headers()
                        .get(reqwest::header::SERVER)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    let mut bytes = Vec::new();
                    if let Err(error) = std::io::Read::by_ref(&mut response)
                        .take((options.max_body_bytes + 1) as u64)
                        .read_to_end(&mut bytes)
                    {
                        last_error = error.to_string();
                        continue;
                    }
                    let truncated = bytes.len() > options.max_body_bytes;
                    bytes.truncate(options.max_body_bytes);
                    let body = String::from_utf8_lossy(&bytes);
                    let title = html_title(&body);
                    let (content_category, content_score, content_matches) = content_rules
                        .classify(&final_url, &title, &body, options.content_threshold);
                    let state = if (200..400).contains(&status) || [401, 403].contains(&status) {
                        "usable_or_restricted"
                    } else if [404, 410].contains(&status) {
                        "reachable_but_path_missing"
                    } else if status >= 500 {
                        "reachable_server_error"
                    } else {
                        "reachable_client_error"
                    };
                    let outcome = if matches!(content_category.as_str(), "gambling" | "porn") {
                        "blocked_content"
                    } else if [401, 403, 407, 429].contains(&status) {
                        "web_restricted"
                    } else if (200..300).contains(&status) && bytes.is_empty() {
                        "web_abnormal"
                    } else if (200..300).contains(&status) && javascript_shell(&body, &content_type)
                    {
                        "browser_render_required"
                    } else if (200..400).contains(&status) {
                        "web_alive"
                    } else {
                        "web_abnormal"
                    };
                    let mut result = empty_probe(&input_url, outcome, "");
                    for (key, value) in [
                        ("probe_effective_url", final_url),
                        ("probe_entry_state", state.into()),
                        ("probe_status_code", status.to_string()),
                        ("probe_title", title),
                        ("probe_content_type", content_type),
                        ("probe_server", server),
                        (
                            "probe_latency_ms",
                            started.elapsed().as_millis().to_string(),
                        ),
                        (
                            "probe_tls_verified",
                            if candidate.starts_with("https://") {
                                options.strict_tls.to_string()
                            } else {
                                "not_applicable".into()
                            },
                        ),
                        ("probe_attempts", attempts.to_string()),
                        ("probe_body_truncated", truncated.to_string()),
                        ("content_category", content_category),
                        ("content_risk_score", content_score.to_string()),
                        ("content_matches", content_matches),
                        ("body_sha256", format!("{:x}", Sha256::digest(&bytes))),
                        ("probe_body_bytes", bytes.len().to_string()),
                    ] {
                        result.insert(key.into(), value);
                    }
                    if outcome != "web_abnormal" {
                        return result;
                    }
                    abnormal = Some(result);
                }
                Err(error) => last_error = error.to_string(),
            }
        }
    }
    if let Some(mut result) = abnormal {
        result.insert("probe_attempts".into(), attempts.to_string());
        return result;
    }
    if cancel.load(Ordering::Relaxed) {
        return empty_probe(&input_url, "skipped", "__CANCELLED__");
    }
    if tcp_alive(&input_url, options.timeout) {
        let mut result = empty_probe(
            &input_url,
            "tcp_alive_non_http",
            &format!("Web协议未识别，仅确认TCP存活：{last_error}"),
        );
        result.insert("probe_entry_state".into(), "tcp_alive_non_http".into());
        result.insert("probe_attempts".into(), (attempts + 1).to_string());
        result.insert("content_category".into(), "not_applicable".into());
        return result;
    }
    let mut result = empty_probe(&input_url, "unreachable", &last_error);
    result.insert("probe_attempts".into(), (attempts + 1).to_string());
    result
}

fn row_identity(row: &Record) -> String {
    format!(
        "{}\u{1f}{}",
        text(row, &["company"]),
        if !text(row, &["asset_key"]).is_empty() {
            text(row, &["asset_key"])
        } else {
            format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}",
                text(row, &["link"]),
                text(row, &["host"]),
                text(row, &["ip"]),
                text(row, &["port"])
            )
        }
    )
}
fn other_bucket(row: &Record) -> &'static str {
    if !text(row, &["company"]).is_empty()
        && text(row, &["company"]) == text(row, &["cert.subject.org"])
    {
        "Q2"
    } else if ["标题全称:", "正文全称:", "独有标题:", "独有正文:"]
        .iter()
        .any(|marker| text(row, &["evidence"]).contains(marker))
    {
        "Q3"
    } else {
        "Q1"
    }
}

pub fn probe_assets(
    refined_dir: &Path,
    other_input: &Path,
    output: &Path,
    options: ProbeOptions,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Result<serde_json::Value, String> {
    fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let sources = [
        ("P1", refined_dir.join("P1_active_strong.csv")),
        ("P2", refined_dir.join("P2_strong_needs_validation.csv")),
        ("P3", refined_dir.join("P3_name_candidates.csv")),
    ];
    let mut stages: Vec<(String, Vec<String>, Vec<Record>)> = Vec::new();
    let mut priority_ids = HashSet::new();
    for (stage, path) in sources {
        let (fields, rows) = read_csv(&path)?;
        priority_ids.extend(rows.iter().map(row_identity));
        stages.push((stage.into(), fields, rows));
    }
    if options.include_other || options.include_weak {
        let (fields, rows) = read_csv(other_input)?;
        for stage in ["Q2", "Q3", "Q1"] {
            if (stage == "Q1" && !options.include_weak) || (stage != "Q1" && !options.include_other)
            {
                continue;
            }
            stages.push((
                stage.into(),
                fields.clone(),
                rows.iter()
                    .filter(|row| {
                        !priority_ids.contains(&row_identity(row)) && other_bucket(row) == stage
                    })
                    .cloned()
                    .collect(),
            ));
        }
    }
    let total = stages.iter().map(|(_, _, rows)| rows.len()).sum::<usize>();
    // Cache identity includes the full selected input and all effective probe
    // options. Changing a rule, URL, retry policy or TLS policy starts a new
    // journal instead of reusing incompatible historical outcomes.
    let mut digest = Sha256::new();
    digest.update(format!("native-probe-v5|{options:?}"));
    for (stage, fields, rows) in &stages {
        digest.update(stage.as_bytes());
        digest.update(serde_json::to_vec(fields).map_err(|error| error.to_string())?);
        for row in rows {
            let canonical = row.iter().collect::<BTreeMap<_, _>>();
            digest.update(serde_json::to_vec(&canonical).map_err(|error| error.to_string())?);
        }
    }
    let fingerprint = format!("{:x}", digest.finalize());
    let journal_path = output.join(format!("probe-{fingerprint}.jsonl"));
    let mut cached = HashMap::<(String, String), Record>::new();
    if let Ok(file) = fs::File::open(&journal_path) {
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if let (Some(stage), Some(row)) = (
                    value.get("stage").and_then(|v| v.as_str()),
                    value.get("row"),
                ) {
                    if let Ok(row) = serde_json::from_value::<Record>(row.clone()) {
                        cached.insert((stage.to_string(), row_identity(&row)), row);
                    }
                }
            }
        }
    }
    let mut journal = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&journal_path)
        .map_err(|error| error.to_string())?;
    // Separate a potentially truncated last record after an interrupted write.
    journal
        .write_all(b"\n")
        .map_err(|error| error.to_string())?;
    let mut completed = 0usize;
    let mut reused = 0usize;
    let mut stage_summary = serde_json::Map::new();
    let content_rules = Arc::new(native_content::ContentRules::from_options(&options));
    let pacing = Arc::new(native_probe_control::ProbePacing::new());
    let client = Arc::new(
        reqwest::blocking::Client::builder()
            .timeout(options.timeout)
            .danger_accept_invalid_certs(!options.strict_tls)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|error| error.to_string())?,
    );
    for (stage, source_fields, rows) in stages {
        let mut output_fields = source_fields;
        for field in PROBE_FIELDS {
            if !output_fields.iter().any(|existing| existing == field) {
                output_fields.push((*field).into());
            }
        }
        let mut buckets: HashMap<String, Vec<Record>> = PROBE_OUTCOMES
            .iter()
            .map(|outcome| ((*outcome).into(), Vec::new()))
            .collect();
        let mut pending = Vec::new();
        for row in &rows {
            if let Some(previous) = cached.remove(&(stage.clone(), row_identity(row))) {
                let outcome = text(&previous, &["probe_outcome"]);
                if PROBE_OUTCOMES.contains(&outcome.as_str()) {
                    buckets.entry(outcome).or_default().push(previous);
                    completed += 1;
                    reused += 1;
                    continue;
                }
            }
            pending.push(row.clone());
        }
        progress(completed, total);
        let rate = if stage.starts_with('P') {
            options.priority_rate
        } else {
            options.other_rate
        };
        for batch in pending.chunks(options.workers.max(1)) {
            if cancel.load(Ordering::Relaxed) {
                return Err("__CANCELLED__".into());
            }
            std::thread::scope(|scope| -> Result<(), String> {
                let (sender, receiver) = std::sync::mpsc::channel();
                let mut handles = Vec::new();
                for row in batch {
                    let sender = sender.clone();
                    let client = &client;
                    let options = &options;
                    let content_rules = &content_rules;
                    let pacing = &pacing;
                    handles.push(scope.spawn(move || {
                        let result =
                            probe_one(row, options, client, content_rules, pacing, rate, cancel);
                        let _ = sender.send((row.clone(), result));
                    }));
                }
                drop(sender);
                // Receive by completion order, not launch order: a slow first
                // host must not hide all other completed requests from the UI.
                for (mut row, result) in receiver {
                    if text(&result, &["probe_error"]) == "__CANCELLED__" {
                        continue;
                    }
                    let outcome = text(&result, &["probe_outcome"]);
                    row.extend(result);
                    let mut line = serde_json::to_vec(&json!({"stage": stage, "row": row}))
                        .map_err(|error| error.to_string())?;
                    line.push(b'\n');
                    journal
                        .write_all(&line)
                        .and_then(|_| journal.flush())
                        .map_err(|error| error.to_string())?;
                    buckets
                        .entry(if PROBE_OUTCOMES.contains(&outcome.as_str()) {
                            outcome
                        } else {
                            "unreachable".into()
                        })
                        .or_default()
                        .push(row);
                    completed += 1;
                    progress(completed, total);
                }
                for handle in handles {
                    handle
                        .join()
                        .map_err(|_| "Rust 探测工作线程异常退出".to_string())?;
                }
                Ok(())
            })?;
            if cancel.load(Ordering::Relaxed) {
                return Err("__CANCELLED__".into());
            }
        }
        let mut counts = serde_json::Map::new();
        for outcome in PROBE_OUTCOMES {
            let bucket = buckets.remove(*outcome).unwrap_or_default();
            counts.insert((*outcome).into(), json!(bucket.len()));
            write_csv(
                &output.join(format!("{stage}_{outcome}.csv")),
                &output_fields,
                &bucket,
            )?;
        }
        stage_summary.insert(
            stage,
            json!({"complete": true, "selected_rows": rows.len(), "counts": counts}),
        );
    }
    let checkpoint = json!({"version": 2, "probe_version": 5, "native": true, "fingerprint":fingerprint,"reused_results":reused,"stage_order": stage_summary.keys().collect::<Vec<_>>(), "stages": stage_summary});
    fs::write(
        output.join("checkpoint.json"),
        serde_json::to_vec_pretty(&checkpoint).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&checkpoint).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(checkpoint)
}
