fn export_value(asset: &Asset, field: &str) -> String {
    match field {
        "projectName" => asset.project_name.clone(),
        "assetKey" => asset.asset_key.clone(),
        "company" => asset.company.clone(),
        "host" => asset.host.clone(),
        "link" => asset.link.clone(),
        "ip" => asset.ip.clone(),
        "port" => asset.port.clone(),
        "protocol" => asset.protocol.clone(),
        "domain" => asset.domain.clone(),
        "title" => asset.title.clone(),
        "statusCode" => asset.status_code.clone(),
        "probeOutcome" => asset.probe_outcome.clone(),
        "probeEntryState" => asset.probe_entry_state.clone(),
        "reviewTier" => asset.review_tier.clone(),
        "contentCategory" => asset.content_category.clone(),
        "score" => asset.score.clone(),
        "decision" => asset.decision.clone(),
        "note" => asset.note.clone(),
        "firstSeen" => asset.first_seen.clone(),
        "lastSeen" => asset.last_seen.clone(),
        "lastAlive" => asset.last_alive.clone().unwrap_or_default(),
        "projectFirstSeen" => asset.project_first_seen.clone(),
        "projectLastSeen" => asset.project_last_seen.clone(),
        "lastRunId" => asset
            .last_run_id
            .map(|value| value.to_string())
            .unwrap_or_default(),
        "isDeleted" => if asset.is_deleted {
            "回收站"
        } else {
            "正常"
        }
        .to_string(),
        "deletedAt" => asset.deleted_at.clone().unwrap_or_default(),
        "sentinelStatus" => asset.sentinel_status.clone(),
        "sentinelScanCount" => asset.sentinel_scan_count.to_string(),
        "sentinelSentAt" => asset.sentinel_sent_at.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

#[tauri::command]
pub fn export_assets(
    state: State<AppState>,
    request: ExportRequest,
) -> Result<ExportResult, String> {
    const EXPORT_FIELDS: &[&str] = &[
        "projectName",
        "assetKey",
        "company",
        "host",
        "link",
        "ip",
        "port",
        "protocol",
        "domain",
        "title",
        "statusCode",
        "probeOutcome",
        "probeEntryState",
        "reviewTier",
        "contentCategory",
        "score",
        "decision",
        "note",
        "firstSeen",
        "lastSeen",
        "lastAlive",
        "projectFirstSeen",
        "projectLastSeen",
        "lastRunId",
        "isDeleted",
        "deletedAt",
        "sentinelStatus",
        "sentinelScanCount",
        "sentinelSentAt",
    ];
    if request.fields.is_empty() {
        return Err("请至少选择一个导出字段".into());
    }
    if let Some(field) = request
        .fields
        .iter()
        .find(|field| !EXPORT_FIELDS.contains(&field.as_str()))
    {
        return Err(format!("不支持的导出字段：{field}"));
    }
    let connection = db::open(&state.db_path)?;
    let (filter, values) = asset_filter(&request.query, true);
    let sql = format!("{ASSET_SELECT} WHERE {filter} ORDER BY a.last_seen DESC,a.id DESC");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let assets = statement
        .query_map(params_from_iter(values.iter()), asset_from_row)
        .map_err(|error| error.to_string())?;
    let exports_dir = state.export_dir.clone();
    fs::create_dir_all(&exports_dir).map_err(|error| error.to_string())?;
    let path: PathBuf = exports_dir.join(format!("assets-{}.csv", Uuid::new_v4()));
    let mut file = File::create(&path).map_err(|error| error.to_string())?;
    file.write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|error| error.to_string())?;
    let mut writer = csv::Writer::from_writer(file);
    let labels: HashMap<&str, &str> = HashMap::from([
        ("projectName", "所属项目"),
        ("assetKey", "资产键"),
        ("company", "公司名称"),
        ("host", "主机"),
        ("link", "链接"),
        ("ip", "IP"),
        ("port", "端口"),
        ("protocol", "协议"),
        ("domain", "域名"),
        ("title", "标题"),
        ("statusCode", "状态码"),
        ("probeOutcome", "探测结果"),
        ("probeEntryState", "入口状态"),
        ("reviewTier", "优先级"),
        ("contentCategory", "内容分类"),
        ("score", "评分"),
        ("decision", "人工结论"),
        ("note", "备注"),
        ("firstSeen", "首次发现"),
        ("lastSeen", "最后发现"),
        ("lastAlive", "最后存活"),
        ("projectFirstSeen", "项目首次发现"),
        ("projectLastSeen", "项目最后发现"),
        ("lastRunId", "最近任务 ID"),
        ("isDeleted", "回收状态"),
        ("deletedAt", "移入回收站时间"),
        ("sentinelStatus", "Strix 状态"),
        ("sentinelScanCount", "Strix 扫描次数"),
        ("sentinelSentAt", "最近送扫时间"),
    ]);
    let headers = request
        .fields
        .iter()
        .map(|field| {
            if request.chinese_headers {
                labels
                    .get(field.as_str())
                    .copied()
                    .unwrap_or(field)
                    .to_string()
            } else {
                field.clone()
            }
        })
        .collect::<Vec<_>>();
    writer
        .write_record(headers)
        .map_err(|error| error.to_string())?;
    let mut rows = 0;
    for asset in assets {
        let asset = asset.map_err(|error| error.to_string())?;
        writer
            .write_record(
                request
                    .fields
                    .iter()
                    .map(|field| export_value(&asset, field)),
            )
            .map_err(|error| error.to_string())?;
        rows += 1;
    }
    writer.flush().map_err(|error| error.to_string())?;
    Ok(ExportResult {
        path: path.to_string_lossy().to_string(),
        rows,
    })
}
