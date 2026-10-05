const FOFA_BASE_FIELDS: &[&str] = &[
    "host",
    "link",
    "ip",
    "port",
    "protocol",
    "domain",
    "title",
    "icp",
    "cert.subject.org",
    "cert.subject.cn",
    "cert.domain",
    "cert.sn",
    "org",
    "server",
    "status_code",
    "jarm",
];
const FOFA_PRO_FIELDS: &[&str] = &[
    "header_hash",
    "banner_hash",
    "banner_fid",
    "cname",
    "lastupdatetime",
    "product",
    "product_category",
];
const CANDIDATE_FIELDS: &[&str] = &[
    "company",
    "decision",
    "reviewer",
    "review_note",
    "score",
    "confidence",
    "evidence",
    "phases",
    "asset_key",
    "host",
    "link",
    "ip",
    "port",
    "protocol",
    "domain",
    "title",
    "icp",
    "cert.subject.org",
    "cert.subject.cn",
    "cert.domain",
    "cert.sn",
    "org",
    "server",
    "status_code",
    "jarm",
    "header_hash",
    "banner_hash",
    "banner_fid",
    "cname",
    "lastupdatetime",
    "product",
    "product_category",
    "product.version",
    "icon_hash",
    "cname_domain",
    "cert.is_valid",
    "cert.is_match",
    "cert.is_equal",
    "fid",
    "matched_queries",
];

#[derive(Clone)]
struct EvidenceQuery {
    company: String,
    phase: String,
    category: String,
    label: String,
    score: i64,
    query: String,
}

fn split_seed(value: &str) -> Vec<String> {
    value
        .split(['|', ';', '；', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}
fn fofa_quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{}\"", value.replace('"', "\\\"")))
}
fn normalize_host(value: &str) -> String {
    let raw = value.trim().trim_start_matches("*.");
    reqwest::Url::parse(raw)
        .or_else(|_| reqwest::Url::parse(&format!("http://{raw}")))
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| {
                host.trim_start_matches("*.")
                    .trim_end_matches('.')
                    .to_ascii_lowercase()
            })
        })
        .unwrap_or_default()
}

fn build_fofa_queries(seeds: &Path, enable_cidr24: bool) -> Result<Vec<EvidenceQuery>, String> {
    let (_, rows) = read_csv(seeds)?;
    let mut queries = Vec::new();
    for row in rows {
        let names = split_seed(&text(&row, &["names", "full_names", "企业全称", "名称"]));
        let domains = split_seed(&text(
            &row,
            &["domains", "domain", "域名", "urls", "资产域名"],
        ));
        let icps = split_seed(&text(&row, &["icps", "icp", "备案号"]));
        let ips = split_seed(&text(&row, &["ip_ranges", "ips", "ip", "IP段", "IP"]));
        let orgs = split_seed(&text(&row, &["asn_orgs", "asn_org", "ASN组织", "网络组织"]));
        let keywords = split_seed(&text(
            &row,
            &["keywords", "unique_keywords", "独有关键词", "系统关键词"],
        ));
        let company = {
            let value = text(&row, &["company", "企业", "企业名称", "name"]);
            if value.is_empty() {
                names
                    .first()
                    .or(domains.first())
                    .cloned()
                    .unwrap_or_else(|| "未命名种子".into())
            } else {
                value
            }
        };
        let mut push = |category: &str, label: String, score: i64, query: String| {
            queries.push(EvidenceQuery {
                company: company.clone(),
                phase: "seed".into(),
                category: category.into(),
                label,
                score,
                query,
            })
        };
        for name in names {
            let quoted = fofa_quote(&name);
            push(
                "name_page",
                format!("标题全称:{name}"),
                45,
                format!("title={quoted}"),
            );
            push(
                "name_page",
                format!("正文全称:{name}"),
                30,
                format!("body={quoted}"),
            );
            push(
                "cert_org",
                format!("证书组织:{name}"),
                70,
                format!("cert.subject.org={quoted}"),
            );
        }
        for domain in domains {
            let host = normalize_host(&domain);
            if host.is_empty() {
                continue;
            }
            let quoted = fofa_quote(&host);
            push(
                "seed_domain",
                format!("精确主机:{host}"),
                100,
                format!("host={quoted}"),
            );
            push(
                "seed_domain",
                format!("注册域:{host}"),
                95,
                format!("domain={quoted}"),
            );
            push(
                "seed_domain",
                format!("证书域:{host}"),
                90,
                format!("cert.domain={quoted}"),
            );
        }
        for icp in icps {
            push(
                "icp",
                format!("备案号:{icp}"),
                95,
                format!("icp={}", fofa_quote(&icp)),
            );
        }
        for ip in ips {
            push(
                "seed_ip",
                format!("明确IP:{ip}"),
                100,
                format!("ip={}", fofa_quote(&ip)),
            );
            if enable_cidr24 {
                if let Ok(address) = ip.parse::<std::net::Ipv4Addr>() {
                    let octets = address.octets();
                    let cidr = format!("{}.{}.{}.0/24", octets[0], octets[1], octets[2]);
                    push(
                        "cidr24",
                        format!("种子C段:{cidr}"),
                        12,
                        format!("ip={}", fofa_quote(&cidr)),
                    );
                }
            }
        }
        for org in orgs {
            push(
                "asn_org",
                format!("ASN组织:{org}"),
                25,
                format!("org={}", fofa_quote(&org)),
            );
        }
        for keyword in keywords {
            let quoted = fofa_quote(&keyword);
            push(
                "unique_keyword",
                format!("独有标题:{keyword}"),
                35,
                format!("title={quoted}"),
            );
            push(
                "unique_keyword",
                format!("独有正文:{keyword}"),
                25,
                format!("body={quoted}"),
            );
        }
    }
    let mut unique = HashMap::new();
    for query in queries {
        let key = (
            query.company.clone(),
            query.category.clone(),
            query.query.clone(),
        );
        if unique
            .get(&key)
            .map(|old: &EvidenceQuery| old.score < query.score)
            .unwrap_or(true)
        {
            unique.insert(key, query);
        }
    }
    Ok(unique.into_values().collect())
}

#[derive(Default)]
struct Candidate {
    row: Record,
    scores: HashMap<String, i64>,
    evidence: BTreeSet<String>,
    phases: BTreeSet<String>,
    queries: BTreeSet<String>,
}

fn flatten_fofa(values: &serde_json::Value, fields: &[String]) -> Vec<Record> {
    values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            if let Some(object) = item.as_object() {
                Some(
                    fields
                        .iter()
                        .map(|field| {
                            (
                                field.clone(),
                                object
                                    .get(field)
                                    .map(|value| {
                                        value
                                            .as_str()
                                            .map(str::to_string)
                                            .unwrap_or_else(|| value.to_string())
                                    })
                                    .unwrap_or_default(),
                            )
                        })
                        .collect(),
                )
            } else {
                item.as_array().map(|array| {
                    fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| {
                            (
                                field.clone(),
                                array
                                    .get(index)
                                    .map(|value| {
                                        value
                                            .as_str()
                                            .map(str::to_string)
                                            .unwrap_or_else(|| value.to_string())
                                    })
                                    .unwrap_or_default(),
                            )
                        })
                        .collect()
                })
            }
        })
        .collect()
}

pub struct CollectOptions {
    pub key: String,
    pub proxy: String,
    pub page_size: usize,
    pub max_pages: usize,
    pub timeout: Duration,
    pub interval: Duration,
    pub full: bool,
    pub enable_cidr24: bool,
    pub profile: String,
    pub cache: bool,
}

fn fofa_fields(profile: &str) -> Vec<String> {
    let mut fields = FOFA_BASE_FIELDS
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    if profile != "basic" {
        fields.extend(
            FOFA_PRO_FIELDS
                .iter()
                .take(if profile == "personal" {
                    3
                } else {
                    FOFA_PRO_FIELDS.len()
                })
                .map(|v| v.to_string()),
        );
    }
    if matches!(profile, "business" | "corporate" | "auto") {
        fields.extend(
            [
                "product.version",
                "icon_hash",
                "cname_domain",
                "cert.is_valid",
                "cert.is_match",
                "cert.is_equal",
            ]
            .iter()
            .map(|v| v.to_string()),
        );
    }
    if profile == "corporate" {
        fields.push("fid".into());
    }
    fields
}

fn host_matches_scope(candidate: &str, expected: &str) -> bool {
    let candidate = candidate.trim().trim_end_matches('.').to_ascii_lowercase();
    let expected = expected.trim().trim_end_matches('.').to_ascii_lowercase();
    !expected.is_empty() && (candidate == expected || candidate.ends_with(&format!(".{expected}")))
}

pub fn collect_fofa(
    seeds: &Path,
    output: &Path,
    options: CollectOptions,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<serde_json::Value, String> {
    if options.key.trim().is_empty() {
        return Err("未配置 FOFA Key".into());
    }
    fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let queries = build_fofa_queries(seeds, options.enable_cidr24)?;
    if queries.is_empty() {
        return Err("没有生成任何 FOFA 查询".into());
    }
    let mut fields = fofa_fields(&options.profile);
    let cache_directory = output.join("native-cache");
    if options.cache {
        fs::create_dir_all(&cache_directory).map_err(|error| error.to_string())?;
    }
    let mut next_request = Instant::now();
    let mut builder = reqwest::blocking::Client::builder().timeout(options.timeout);
    if !options.proxy.trim().is_empty() {
        builder = builder.proxy(
            reqwest::Proxy::all(options.proxy.trim())
                .map_err(|error| format!("代理配置无效：{error}"))?,
        );
    }
    let client = builder.build().map_err(|error| error.to_string())?;
    let mut candidates: HashMap<(String, String), Candidate> = HashMap::new();
    let mut logs = Vec::new();
    for (index, query) in queries.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("任务已取消".into());
        }
        progress(index, queries.len(), &query.label);
        let effective_size = if query.query.contains("body=") {
            options.page_size.min(500)
        } else {
            options.page_size.max(1)
        };
        let mut page = 1usize;
        let mut fetched = 0usize;
        let mut total_matches = 0usize;
        let mut status = "ok".to_string();
        let mut error = String::new();
        loop {
            let encoded = base64::engine::general_purpose::STANDARD.encode(query.query.as_bytes());
            let mut endpoint = reqwest::Url::parse("https://fofa.info/api/v1/search/all")
                .expect("static FOFA URL");
            endpoint
                .query_pairs_mut()
                .append_pair("key", &options.key)
                .append_pair("qbase64", &encoded)
                .append_pair("fields", &fields.join(","))
                .append_pair("page", &page.to_string())
                .append_pair("size", &effective_size.to_string())
                .append_pair("full", &options.full.to_string())
                .append_pair("r_type", "json");
            let cache_key = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
            let cache_file = cache_directory.join(format!("{cache_key}.json"));
            let cached_payload = if options.cache {
                fs::read(&cache_file)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .filter(|value| {
                        value.get("results").and_then(|v| v.as_array()).is_some()
                            && value.get("error").and_then(|v| v.as_bool()) != Some(true)
                    })
            } else {
                None
            };
            let fetched_payload = (|| -> Result<serde_json::Value, String> {
                if let Some(payload) = cached_payload {
                    return Ok(payload);
                }
                for attempt in 0..3 {
                    while Instant::now() < next_request {
                        if cancel.load(Ordering::Relaxed) {
                            return Err("__CANCELLED__".into());
                        }
                        std::thread::sleep(
                            next_request
                                .saturating_duration_since(Instant::now())
                                .min(Duration::from_millis(100)),
                        );
                    }
                    if cancel.load(Ordering::Relaxed) {
                        return Err("__CANCELLED__".into());
                    }
                    next_request = Instant::now() + options.interval;
                    match client
                        .get(endpoint.clone())
                        .header("User-Agent", "Authorized-Asset-Inventory/3.0")
                        .send()
                    {
                        Ok(response) => {
                            let status_code = response.status();
                            if attempt < 2
                                && (status_code.is_server_error() || status_code.as_u16() == 429)
                            {
                                next_request = next_request
                                    .max(Instant::now() + Duration::from_secs(1 << attempt));
                                continue;
                            }
                            let response = response
                                .error_for_status()
                                .map_err(|error| error.without_url().to_string())?;
                            let body = response
                                .text()
                                .map_err(|error| error.without_url().to_string())?;
                            let payload: serde_json::Value = serde_json::from_str(&body)
                                .map_err(|error| format!("FOFA 响应 JSON 解析失败：{error}"))?;
                            if options.cache
                                && payload
                                    .get("results")
                                    .and_then(|value| value.as_array())
                                    .is_some()
                                && payload.get("error").and_then(|value| value.as_bool())
                                    != Some(true)
                            {
                                let staged = cache_directory
                                    .join(format!("{cache_key}-{}.tmp", uuid::Uuid::new_v4()));
                                fs::write(
                                    &staged,
                                    serde_json::to_vec(&payload)
                                        .map_err(|error| error.to_string())?,
                                )
                                .map_err(|error| error.to_string())?;
                                fs::rename(staged, &cache_file)
                                    .map_err(|error| error.to_string())?;
                            }
                            return Ok(payload);
                        }
                        Err(error) if attempt < 2 && (error.is_timeout() || error.is_connect()) => {
                            next_request = next_request
                                .max(Instant::now() + Duration::from_secs(1 << attempt));
                        }
                        Err(error) => return Err(error.without_url().to_string()),
                    }
                }
                Err("FOFA 网络重试耗尽".into())
            })();
            let payload = match fetched_payload {
                Ok(payload) => payload,
                Err(detail) => {
                    status = "error".into();
                    error = detail.replace(&options.key, "[REDACTED]");
                    break;
                }
            };
            if payload
                .get("error")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                status = "error".into();
                error = payload
                    .get("errmsg")
                    .and_then(|value| value.as_str())
                    .unwrap_or("FOFA 返回未知错误")
                    .into();
                error = error.replace(&options.key, "[REDACTED]");
                let lower = error.to_lowercase();
                let field_restriction = ["field", "字段", "会员", "权限"]
                    .iter()
                    .any(|value| lower.contains(value));
                let safe = [
                    "host",
                    "link",
                    "ip",
                    "port",
                    "protocol",
                    "domain",
                    "title",
                    "icp",
                    "org",
                    "server",
                    "status_code",
                ]
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>();
                if field_restriction && fields != safe {
                    fields = safe;
                    status = "ok".into();
                    error.clear();
                    progress(
                        index,
                        queries.len(),
                        "当前字段权限不足，回退基础资产字段后重试",
                    );
                    continue;
                }
                break;
            }
            let records = flatten_fofa(
                payload.get("results").unwrap_or(&serde_json::Value::Null),
                &fields,
            );
            total_matches = payload
                .get("size")
                .and_then(|value| value.as_u64())
                .unwrap_or(records.len() as u64) as usize;
            for record in &records {
                if query.category == "seed_domain" {
                    let expected =
                        normalize_host(query.label.rsplit(':').next().unwrap_or_default());
                    let candidate = normalize_host(&text(record, &["host", "link"]));
                    if !expected.is_empty()
                        && !candidate.is_empty()
                        && !host_matches_scope(&candidate, &expected)
                    {
                        continue;
                    }
                }
                let host = text(record, &["host", "link"]);
                let asset_key = if !host.is_empty() {
                    format!(
                        "{}|{}|{}",
                        host.to_ascii_lowercase(),
                        text(record, &["ip"]),
                        text(record, &["port"])
                    )
                } else {
                    format!(
                        "{}|{}|{}",
                        text(record, &["protocol"]),
                        text(record, &["ip"]),
                        text(record, &["port"])
                    )
                };
                let item = candidates
                    .entry((query.company.clone(), asset_key.clone()))
                    .or_default();
                item.row
                    .entry("company".into())
                    .or_insert_with(|| query.company.clone());
                item.row.entry("asset_key".into()).or_insert(asset_key);
                for (field, value) in record {
                    if !value.is_empty()
                        && item
                            .row
                            .get(field)
                            .map(|old| old.is_empty())
                            .unwrap_or(true)
                    {
                        item.row.insert(field.clone(), value.clone());
                    }
                }
                item.scores
                    .entry(query.category.clone())
                    .and_modify(|score| *score = (*score).max(query.score))
                    .or_insert(query.score);
                item.evidence.insert(query.label.clone());
                item.phases.insert(query.phase.clone());
                item.queries.insert(query.query.clone());
            }
            fetched += records.len();
            if records.len() < effective_size
                || fetched >= total_matches
                || (options.max_pages > 0 && page >= options.max_pages)
            {
                break;
            }
            page += 1;
        }
        let mut log = Record::new();
        for (key, value) in [
            ("company", query.company.clone()),
            ("phase", query.phase.clone()),
            ("category", query.category.clone()),
            ("label", query.label.clone()),
            ("score", query.score.to_string()),
            ("query", query.query.clone()),
            ("total_matches", total_matches.to_string()),
            ("fetched", fetched.to_string()),
            ("status", status),
            ("error", error),
        ] {
            log.insert(key.into(), value);
        }
        logs.push(log);
    }
    let plan_fields = ["company", "phase", "category", "label", "score", "query"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let plans = queries
        .iter()
        .map(|query| {
            [
                ("company", query.company.clone()),
                ("phase", query.phase.clone()),
                ("category", query.category.clone()),
                ("label", query.label.clone()),
                ("score", query.score.to_string()),
                ("query", query.query.clone()),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect()
        })
        .collect::<Vec<Record>>();
    write_csv(&output.join("query_plan.csv"), &plan_fields, &plans)?;
    write_csv(
        &output.join("query_log.csv"),
        &[
            "company",
            "phase",
            "category",
            "label",
            "score",
            "query",
            "total_matches",
            "fetched",
            "status",
            "error",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>(),
        &logs,
    )?;
    let mut rows = Vec::new();
    for (_, mut candidate) in candidates {
        let score = candidate.scores.values().sum::<i64>().min(100);
        candidate.row.insert("score".into(), score.to_string());
        candidate.row.insert(
            "confidence".into(),
            if score >= 85 {
                "high"
            } else if score >= 55 {
                "medium"
            } else {
                "low"
            }
            .into(),
        );
        candidate.row.insert(
            "evidence".into(),
            candidate
                .evidence
                .into_iter()
                .collect::<Vec<_>>()
                .join(" | "),
        );
        candidate.row.insert(
            "phases".into(),
            candidate.phases.into_iter().collect::<Vec<_>>().join(" | "),
        );
        candidate.row.insert(
            "matched_queries".into(),
            candidate
                .queries
                .into_iter()
                .collect::<Vec<_>>()
                .join(" | "),
        );
        rows.push(candidate.row);
    }
    rows.sort_by(|left, right| {
        text(left, &["company"])
            .cmp(&text(right, &["company"]))
            .then(number(right, &["score"]).cmp(&number(left, &["score"])))
    });
    let candidate_fields = CANDIDATE_FIELDS
        .iter()
        .map(|field| (*field).to_string())
        .collect::<Vec<_>>();
    write_csv(&output.join("candidates.csv"), &candidate_fields, &rows)?;
    write_csv(
        &output.join("review_candidates.csv"),
        &candidate_fields,
        &rows
            .iter()
            .filter(|row| number(row, &["score"]) >= 30)
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    let failed_queries = logs
        .iter()
        .filter(|row| text(row, &["status"]) == "error")
        .count();
    if failed_queries > 0 {
        return Err(format!("FOFA {failed_queries}/{} 条查询未完成；已保留 {} 条候选和 query_log.csv，未将不完整采集标为完成", queries.len(), rows.len()));
    }
    progress(queries.len(), queries.len(), "采集完成");
    Ok(json!({"queries": queries.len(), "candidates": rows.len(), "native": true}))
}
