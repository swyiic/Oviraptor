fn evidence_types(value: &str) -> String {
    let mapping = [
        ("证书组织:", "cert_org"),
        ("证书序列号:", "cert_serial"),
        ("标题全称:", "name_title"),
        ("正文全称:", "name_body"),
        ("独有标题:", "unique_title"),
        ("独有正文:", "unique_body"),
        ("响应头哈希:", "header_hash"),
        ("弱JARM:", "jarm"),
        ("Banner结构指纹:", "banner_fid"),
        ("图标哈希:", "icon_hash"),
        ("FOFA站点特征:", "fid"),
        ("高可信候选C段:", "cidr24"),
        ("确认资产C段:", "cidr24"),
        ("种子C段:", "cidr24"),
        ("精确主机:", "seed_domain"),
        ("注册域:", "seed_domain"),
        ("证书域:", "seed_domain"),
        ("备案号:", "icp"),
        ("明确IP:", "seed_ip"),
    ];
    let mut found = BTreeSet::new();
    for piece in value.split(" | ") {
        found.insert(
            mapping
                .iter()
                .find(|(prefix, _)| piece.starts_with(prefix))
                .map(|(_, name)| *name)
                .unwrap_or("other"),
        );
    }
    found.into_iter().collect::<Vec<_>>().join(" | ")
}

fn classify(row: &Record, recent_since: &str) -> (String, String, String, String) {
    let company = text(row, &["company"]);
    let cert_org = text(row, &["cert.subject.org"]);
    let evidence = text(row, &["evidence"]);
    let protocol = text(row, &["protocol"]).to_ascii_lowercase();
    let updated = text(row, &["lastupdatetime"]);
    let updated = updated.get(..updated.len().min(10)).unwrap_or_default();
    let status = text(row, &["status_code"]);
    let cert_exact = !company.is_empty() && cert_org == company;
    let name_match = ["标题全称:", "正文全称:", "独有标题:", "独有正文:"]
        .iter()
        .any(|marker| evidence.contains(marker));
    let seed_domain_match = ["精确主机:", "注册域:", "证书域:"]
        .iter()
        .any(|marker| evidence.contains(marker));
    let strong = cert_exact || seed_domain_match;
    let is_web = protocol.contains("http");
    let recent = updated >= recent_since;
    let active = ACTIVE_STATUSES.contains(&status.as_str());
    let reason = if seed_domain_match {
        "给定域名范围命中"
    } else {
        "证书组织与企业全称一致"
    };
    if strong && is_web && recent && active {
        return (
            "P1_强归属且近期活跃".into(),
            reason.into(),
            "近期可响应".into(),
            "priority".into(),
        );
    }
    if strong && is_web && recent {
        return (
            "P2_强归属但入口待验证".into(),
            reason.into(),
            format!(
                "HTTP状态:{}",
                if status.is_empty() { "未知" } else { &status }
            ),
            "priority".into(),
        );
    }
    if !strong && name_match && is_web && recent && active {
        return (
            "P3_名称命中待确认".into(),
            "页面标题/正文命中企业名称".into(),
            "近期可响应".into(),
            "priority".into(),
        );
    }
    if !strong && !name_match {
        return (
            String::new(),
            "仅弱指纹/网络邻接，不能证明归属".into(),
            "隔离".into(),
            "Q1_弱证据隔离".into(),
        );
    }
    if strong {
        return (
            String::new(),
            reason.into(),
            "过旧或非Web".into(),
            "Q2_强归属历史档案".into(),
        );
    }
    (
        String::new(),
        "页面名称命中".into(),
        "过旧、非Web或无有效响应".into(),
        "Q3_名称历史档案".into(),
    )
}

#[derive(Default)]
struct Cluster {
    assets: HashSet<String>,
    hosts: Vec<String>,
    domains: BTreeSet<String>,
    ips: BTreeSet<String>,
    subject_cn: BTreeSet<String>,
    statuses: BTreeMap<String, usize>,
    tiers: BTreeMap<String, usize>,
    latest: String,
}

fn update_cluster(cluster: &mut Cluster, row: &Record, tier: &str) {
    let asset = text(row, &["asset_key"]);
    if !asset.is_empty() {
        cluster.assets.insert(asset);
    }
    let host = text(row, &["link", "host"]);
    if !host.is_empty() && cluster.hosts.len() < 5 && !cluster.hosts.contains(&host) {
        cluster.hosts.push(host);
    }
    for (field, target) in [
        ("domain", &mut cluster.domains),
        ("ip", &mut cluster.ips),
        ("cert.subject.cn", &mut cluster.subject_cn),
    ] {
        let value = text(row, &[field]);
        if !value.is_empty() && target.len() < 20 {
            target.insert(value);
        }
    }
    *cluster
        .statuses
        .entry(
            text(row, &["status_code"])
                .chars()
                .next()
                .map(|_| text(row, &["status_code"]))
                .unwrap_or_else(|| "unknown".into()),
        )
        .or_default() += 1;
    *cluster.tiers.entry(tier.to_string()).or_default() += 1;
    let updated = text(row, &["lastupdatetime"]);
    if updated > cluster.latest {
        cluster.latest = updated;
    }
}

fn write_clusters(
    path: &Path,
    clusters: HashMap<(String, String), Cluster>,
    key_name: &str,
) -> Result<(), String> {
    let fields = [
        "company",
        key_name,
        "asset_count",
        "p1_count",
        "p2_count",
        "p3_count",
        "latest_update",
        "status_summary",
        "subject_cn_examples",
        "domain_examples",
        "ip_examples",
        "representative_urls",
        "recommendation",
    ];
    let mut entries = clusters.into_iter().collect::<Vec<_>>();
    entries.sort_by_key(|(_, cluster)| std::cmp::Reverse(cluster.assets.len()));
    let mut writer = WriterBuilder::new()
        .from_path(path)
        .map_err(|error| error.to_string())?;
    writer
        .write_record(fields)
        .map_err(|error| error.to_string())?;
    for ((company, key), cluster) in entries {
        let p1 = cluster
            .tiers
            .get("P1_强归属且近期活跃")
            .copied()
            .unwrap_or(0);
        let p2 = cluster
            .tiers
            .get("P2_强归属但入口待验证")
            .copied()
            .unwrap_or(0);
        let p3 = cluster.tiers.get("P3_名称命中待确认").copied().unwrap_or(0);
        let mut statuses = cluster.statuses.into_iter().collect::<Vec<_>>();
        statuses.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        writer
            .write_record([
                company,
                key,
                cluster.assets.len().to_string(),
                p1.to_string(),
                p2.to_string(),
                p3.to_string(),
                cluster.latest,
                statuses
                    .into_iter()
                    .map(|(key, count)| format!("{key}:{count}"))
                    .collect::<Vec<_>>()
                    .join(" | "),
                cluster
                    .subject_cn
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(" | "),
                cluster.domains.into_iter().collect::<Vec<_>>().join(" | "),
                cluster
                    .ips
                    .into_iter()
                    .take(10)
                    .collect::<Vec<_>>()
                    .join(" | "),
                cluster.hosts.join(" | "),
                if p1 > 0 {
                    "优先确认整个证书簇"
                } else {
                    "抽查代表入口后决定"
                }
                .into(),
            ])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

pub fn refine_candidates(
    input: &Path,
    query_log: &Path,
    output: &Path,
    recent_since: &str,
) -> Result<serde_json::Value, String> {
    fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let (source_fields, rows) = read_csv(input)?;
    let mut asset_companies: HashMap<String, HashSet<String>> = HashMap::new();
    let mut bucket_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut tier_counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rows {
        let (tier, _, _, bucket) = classify(row, recent_since);
        *bucket_counts.entry(bucket).or_default() += 1;
        if !tier.is_empty() {
            *tier_counts.entry(tier).or_default() += 1;
            asset_companies
                .entry(text(row, &["asset_key"]))
                .or_default()
                .insert(text(row, &["company"]));
        }
    }
    let mut fields = vec![
        "review_tier",
        "attribution_reason",
        "activity_state",
        "evidence_types",
        "company_match_count",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    fields.extend(source_fields.clone());
    let tiers = [
        ("P1_强归属且近期活跃", "P1_active_strong.csv"),
        ("P2_强归属但入口待验证", "P2_strong_needs_validation.csv"),
        ("P3_名称命中待确认", "P3_name_candidates.csv"),
    ];
    let mut tier_rows: HashMap<&str, Vec<Record>> =
        tiers.iter().map(|(tier, _)| (*tier, Vec::new())).collect();
    let mut priority = Vec::new();
    let mut cert_clusters: HashMap<(String, String), Cluster> = HashMap::new();
    let mut name_clusters: HashMap<(String, String), Cluster> = HashMap::new();
    for row in &rows {
        let (tier, reason, activity, _) = classify(row, recent_since);
        if tier.is_empty() {
            continue;
        }
        let mut out = row.clone();
        out.insert("review_tier".into(), tier.clone());
        out.insert("attribution_reason".into(), reason);
        out.insert("activity_state".into(), activity);
        out.insert(
            "evidence_types".into(),
            evidence_types(&text(row, &["evidence"])),
        );
        out.insert(
            "company_match_count".into(),
            asset_companies
                .get(&text(row, &["asset_key"]))
                .map(HashSet::len)
                .unwrap_or(0)
                .to_string(),
        );
        tier_rows
            .get_mut(tier.as_str())
            .expect("known tier")
            .push(out.clone());
        priority.push(out);
        let company = text(row, &["company"]);
        if tier.starts_with("P1_") || tier.starts_with("P2_") {
            let key = text(row, &["cert.sn", "cert.subject.cn", "domain", "ip"]);
            update_cluster(cert_clusters.entry((company, key)).or_default(), row, &tier);
        } else {
            let key = text(row, &["domain", "ip", "host"]);
            update_cluster(name_clusters.entry((company, key)).or_default(), row, &tier);
        }
    }
    for (tier, file) in tiers {
        write_csv(&output.join(file), &fields, tier_rows.get(tier).unwrap())?;
    }
    write_csv(&output.join("priority_review.csv"), &fields, &priority)?;
    write_clusters(
        &output.join("certificate_clusters.csv"),
        cert_clusters,
        "certificate_key",
    )?;
    write_clusters(
        &output.join("name_domain_clusters.csv"),
        name_clusters,
        "domain_or_ip",
    )?;
    let retry_actions = build_retry_plan(query_log, &output.join("retry_plan.csv"))?;
    let summary = json!({
        "source": input.to_string_lossy(), "source_rows": rows.len(), "recent_since": recent_since,
        "tier_counts": tier_counts, "bucket_counts": bucket_counts, "priority_rows": priority.len(),
        "certificate_clusters": csv_row_count(&output.join("certificate_clusters.csv"))?,
        "name_domain_clusters": csv_row_count(&output.join("name_domain_clusters.csv"))?, "retry_actions": retry_actions,
        "notes": ["原 output 未修改；弱指纹明细继续保留在原 candidates.csv。", "P1/P2 表示归属证据强，不等同于入口一定可以在浏览器直接打开。", "retry_plan 只是离线计划，不会自动请求 FOFA。"]
    });
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&summary).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let mut summary_writer = WriterBuilder::new()
        .from_path(output.join("summary.csv"))
        .map_err(|error| error.to_string())?;
    summary_writer
        .write_record(["metric", "value"])
        .map_err(|error| error.to_string())?;
    summary_writer
        .write_record(["source_rows", &rows.len().to_string()])
        .map_err(|error| error.to_string())?;
    summary_writer
        .write_record(["priority_rows", &priority.len().to_string()])
        .map_err(|error| error.to_string())?;
    summary_writer.flush().map_err(|error| error.to_string())?;
    Ok(summary)
}

fn csv_row_count(path: &Path) -> Result<usize, String> {
    let mut reader = ReaderBuilder::new()
        .from_path(path)
        .map_err(|error| error.to_string())?;
    Ok(reader.records().filter_map(Result::ok).count())
}

fn build_retry_plan(query_log: &Path, output: &Path) -> Result<BTreeMap<String, usize>, String> {
    if !query_log.exists() {
        return Ok(BTreeMap::new());
    }
    let (mut fields, rows) = read_csv(query_log)?;
    fields.extend(
        [
            "retry_priority",
            "retry_action",
            "recommended_query",
            "recommended_full",
            "recommended_interval",
        ]
        .into_iter()
        .map(str::to_string),
    );
    let mut output_rows = Vec::new();
    let mut stats = BTreeMap::new();
    for mut row in rows {
        if text(&row, &["status"]) == "ok" {
            continue;
        }
        let category = text(&row, &["category"]);
        let query = text(&row, &["query"]);
        let total = number(&row, &["total_matches"]);
        let (priority, action, recommended) = match category.as_str() {
            "jarm" | "header_hash" | "banner_fid" => {
                (9, "不重试：弱指纹全网重复度过高", String::new())
            }
            "cidr24" => (
                3,
                "受控重试：只查Web协议；仍过宽时拆分/28",
                format!("({query}) && (protocol=\"http\" || protocol=\"https\")"),
            ),
            "cert_org" | "name_page" | "seed_domain" | "icp" | "seed_ip" => {
                (1, "优先重试种子查询", query)
            }
            "cert_serial" if total <= 10_000 => (2, "重试证书查询，但只取近期数据", query),
            "cert_serial" => (4, "证书结果过宽，先复核证书簇再决定", String::new()),
            _ => (5, "人工判断", query),
        };
        *stats.entry(action.to_string()).or_default() += 1;
        row.insert("retry_priority".into(), priority.to_string());
        row.insert("retry_action".into(), action.into());
        row.insert("recommended_query".into(), recommended);
        row.insert("recommended_full".into(), "false".into());
        row.insert("recommended_interval".into(), "8.0".into());
        output_rows.push(row);
    }
    write_csv(output, &fields, &output_rows)?;
    Ok(stats)
}
