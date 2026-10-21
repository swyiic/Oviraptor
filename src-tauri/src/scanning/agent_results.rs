//! Bounded, read-only views of the desktop owner's saved results. No raw HTTP
//! material, credentials, local paths or hidden finding payload enters a model.
use super::*;

#[path = "agent_result_evidence.rs"]
mod evidence;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Results {
    task_id: String,
    section: Option<String>,
    after_id: Option<i64>,
    limit: Option<i64>,
    kind: Option<String>,
    record_id: Option<i64>,
}

impl Results {
    pub(super) fn parse(args: &Value) -> Result<Self, String> {
        let value: Self =
            serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
        if value.task_id.is_empty()
            || value.task_id.len() > 128
            || value.task_id.chars().any(char::is_control)
            || value
                .after_id
                .is_some_and(|n| !(0..=9_007_199_254_740_991).contains(&n))
            || value.limit.is_some_and(|n| !(1..=50).contains(&n))
            || value
                .section
                .as_deref()
                .is_some_and(|s| !matches!(s, "targets" | "findings" | "evidence"))
            || value.kind.as_deref().is_some_and(|s| {
                s.is_empty()
                    || s.len() > 80
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
            || value
                .record_id
                .is_some_and(|n| !(1..=9_007_199_254_740_991).contains(&n))
            || match value.section.as_deref().unwrap_or("targets") {
                "evidence" => {
                    value.record_id.is_none()
                        || value.after_id.is_some()
                        || value.limit.is_some()
                        || value.kind.is_some()
                }
                "findings" => value.record_id.is_some(),
                _ => value.record_id.is_some() || value.kind.is_some(),
            }
        {
            return Err("scan_arguments_invalid".into());
        }
        Ok(value)
    }
}

fn text(value: &str, limit: usize) -> (String, bool) {
    // Redact the complete input before clipping, including credentials whose
    // key/value or URL userinfo delimiter crosses the display boundary.
    static URLS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let urls = URLS.get_or_init(|| regex::Regex::new(r#"(?i)https?://[^\s\"'<>]+"#).unwrap());
    let value = urls.replace_all(value, |matched: &regex::Captures<'_>| {
        let mut parsed = match reqwest::Url::parse(&matched[0]) {
            Ok(value) => value,
            Err(_) => return "[url]".into(),
        };
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
        parsed.set_query(None);
        parsed.set_fragment(None);
        parsed.to_string()
    });
    let value = crate::agent_runtime::secrets::redact_text_with(&value, None);
    static PATHS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let paths = PATHS.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)(?:/(?:Users|home|private|var|tmp|Applications|Volumes)/|[a-z]:\\)[^\s\"'<>]*"#,
        )
        .unwrap()
    });
    let value = paths.replace_all(&value, "[local-path]");
    (
        value.chars().take(limit).collect(),
        value.chars().count() > limit,
    )
}

fn url(value: &str) -> Option<String> {
    let mut value = reqwest::Url::parse(value).ok()?;
    if !matches!(value.scheme(), "http" | "https") || value.host_str().is_none() {
        return None;
    }
    value.set_username("").ok()?;
    value.set_password(None).ok()?;
    value.set_query(None);
    value.set_fragment(None);
    Some(text(value.as_str(), 2048).0)
}

fn counts(c: &rusqlite::Connection, task: &str, proof: bool) -> Result<Value, String> {
    let mut values = serde_json::Map::new();
    let (mut total, mut other, mut compacted) = (0i64, 0i64, false);
    let mut statement = c.prepare("SELECT kind,COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND kind<>'vulnerability' GROUP BY kind").map_err(|_| "scan_results_unavailable")?;
    let rows = statement
        .query_map([task], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|_| "scan_results_unavailable")?;
    for row in rows {
        let (kind, count) = row.map_err(|_| "scan_results_unavailable")?;
        total += count;
        let (label, truncated) = text(&kind, 80);
        compacted |= truncated || label != kind;
        if values.len() < 32 || values.contains_key(&label) {
            let previous = values.get(&label).and_then(Value::as_i64).unwrap_or(0);
            values.insert(label, json!(previous + count));
        } else {
            other += count;
            compacted = true;
        }
    }
    let vulnerabilities = if proof {
        let mut statement = c.prepare("SELECT record_json FROM sentinel_findings WHERE scan_id=?1 AND kind='vulnerability'").map_err(|_| "scan_results_unavailable")?;
        let rows = statement
            .query_map([task], |r| r.get::<_, String>(0))
            .map_err(|_| "scan_results_unavailable")?;
        let mut visible = 0i64;
        for row in rows {
            if proof_finding_is_visible(&row.map_err(|_| "scan_results_unavailable")?) {
                visible += 1;
            }
        }
        visible
    } else {
        c.query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND kind='vulnerability'",
            [task],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|_| "scan_results_unavailable")?
    };
    values.insert("vulnerability".into(), json!(vulnerabilities));
    Ok(
        json!({"visibleRecordsByKind":values,"visibleVulnerabilityRecords":vulnerabilities,
        "totalVisibleRecords":total+vulnerabilities,"otherVisibleRecords":other,"recordKindLabelsCompacted":compacted,
        "confirmedVulnerabilities":null,"visibilityPolicy":if proof {"proof"} else {"breadth_or_legacy"},
        "interpretation":"记录种类与可见性统计，不是已验证漏洞总数；任务结束或侦察记录不代表目标安全。"}),
    )
}

pub(super) fn read(
    c: &rusqlite::Connection,
    project: i64,
    input: Results,
) -> Result<Value, String> {
    let transaction = c
        .unchecked_transaction()
        .map_err(|_| "scan_results_unavailable")?;
    let c = &transaction;
    owned_scope(c, project, &input.task_id)?;
    let proof = findings_read_policy_in(c, &input.task_id)?.ok_or("scan_task_unavailable")?;
    let scan = sentinel_scan_by_id(c, &input.task_id)?;
    let section = input.section.as_deref().unwrap_or("targets");
    if section == "evidence" {
        return evidence::read(
            c,
            project,
            &scan,
            input.record_id.ok_or("scan_arguments_invalid")?,
            proof,
        );
    }
    let mut summary = counts(c, &scan.id, proof)?;
    if let Some(kind) = input.kind.as_deref() {
        summary["filteredVisibleRecords"] = if kind == "vulnerability" {
            summary["visibleVulnerabilityRecords"].clone()
        } else {
            json!(c
                .query_row(
                    "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND kind=?2",
                    params![scan.id, kind],
                    |r| r.get::<_, i64>(0)
                )
                .map_err(|_| "scan_results_unavailable")?)
        };
    }
    let after = input.after_id.unwrap_or(0);
    let limit = input.limit.unwrap_or(20) as usize;
    let mut rows = Vec::new();
    let mut last = after;
    if section == "targets" {
        let mut query = c.prepare("SELECT id,asset_id,url,status,scan_mode,routing_reason,last_attempt_number,updated_at FROM sentinel_targets WHERE scan_id=?1 AND project_id=?2 AND id>?3 ORDER BY id LIMIT ?4").map_err(|_| "scan_results_unavailable")?;
        let items = query
            .query_map(params![scan.id, project, after, limit as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(|_| "scan_results_unavailable")?;
        for item in items {
            let (id, asset, target, status, mode, reason, attempt, updated) =
                item.map_err(|_| "scan_results_unavailable")?;
            last = id;
            let (reason, truncated) = text(&reason, 1500);
            rows.push(json!({"id":id,"assetId":asset,"assetIdNamespace":"desktop","url":url(&target),
                "status":text(&status,80).0,"scanMode":text(&mode,80).0,"routingReason":reason,
                "routingReasonTruncated":truncated,"attempt":attempt,"updatedAt":text(&updated,80).0}));
        }
    } else {
        // A raw window bounds work even when many records are hidden by proof
        // policy. An empty visible page may still have a progressing cursor.
        let sql = if input.kind.is_some() {
            "SELECT id,target_url,stage,kind,title,severity,record_json,updated_at FROM sentinel_findings WHERE scan_id=?1 AND kind=?3 AND id>?2 ORDER BY id LIMIT 200"
        } else {
            "SELECT id,target_url,stage,kind,title,severity,record_json,updated_at FROM sentinel_findings WHERE scan_id=?1 AND id>?2 ORDER BY id LIMIT 200"
        };
        let mut query = c.prepare(sql).map_err(|_| "scan_results_unavailable")?;
        let values = if let Some(kind) = input.kind.as_ref() {
            vec![
                rusqlite::types::Value::Text(scan.id.clone()),
                rusqlite::types::Value::Integer(after),
                rusqlite::types::Value::Text(kind.clone()),
            ]
        } else {
            vec![
                rusqlite::types::Value::Text(scan.id.clone()),
                rusqlite::types::Value::Integer(after),
            ]
        };
        let items = query
            .query_map(rusqlite::params_from_iter(values), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(|_| "scan_results_unavailable")?;
        for item in items {
            let (id, target, stage, kind, title, severity, record, updated) =
                item.map_err(|_| "scan_results_unavailable")?;
            last = id;
            let pair = kind == "vulnerability" && proof_finding_is_visible(&record);
            if proof && kind == "vulnerability" && !pair {
                continue;
            }
            let (title, truncated) = text(&title, 500);
            rows.push(json!({"id":id,"url":url(&target),"stage":text(&stage,80).0,"kind":text(&kind,80).0,
                "title":title,"titleTruncated":truncated,"severity":text(&severity,40).0,"updatedAt":text(&updated,80).0,
                "proofPairFieldsRecorded":pair,"independentVerificationByThisRead":false}));
            if rows.len() >= limit {
                break;
            }
        }
    }
    let more = if section == "targets" {
        c.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1 AND project_id=?2 AND id>?3)",params![scan.id,project,last], |r| r.get::<_,bool>(0))
    } else if let Some(kind) = input.kind.as_deref() {
        c.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_findings WHERE scan_id=?1 AND kind=?3 AND id>?2)",params![scan.id,last,kind], |r| r.get::<_,bool>(0))
    } else {
        c.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_findings WHERE scan_id=?1 AND id>?2)",params![scan.id,last], |r| r.get::<_,bool>(0))
    }.map_err(|_| "scan_results_unavailable")?;
    let mut task = projection(c, &scan)?;
    task["title"] = json!(text(&scan.task_name, 240).0);
    let mut output = json!({"projectId":project,"task":task,"summary":summary,"section":section,"filters":{"kind":input.kind},"rows":rows,
        "afterId":after,"nextAfterId":if more {Some(last)}else{None},"pageComplete":!more,
        "snapshotScope":"single_request","source":"desktop_scanning_owner","readOnly":true,
        "rawEvidenceIncluded":false,"outputSizeLimited":false,
        "pagination":"使用相同 taskId、section 和 kind，按 nextAfterId 继续；有游标的空页不代表结果为空。summary是任务整体统计，filteredVisibleRecords才是所选种类数量。每次调用读取当前快照，运行中结果仍可能增长。"});
    // The Host returns both JSON and a text representation. Keep their combined
    // payload below its worker result ceiling without losing the next row.
    while serde_json::to_vec(&output)
        .map_err(|_| "scan_results_unavailable")?
        .len()
        > 24_000
    {
        let rows = output["rows"]
            .as_array_mut()
            .ok_or("scan_results_unavailable")?;
        if rows.pop().is_none() {
            return Err("scan_results_summary_too_large".into());
        }
        if rows.is_empty() {
            return Err("scan_results_page_too_large".into());
        }
        let cursor = rows.last().and_then(|r| r["id"].as_i64()).unwrap_or(after);
        output["nextAfterId"] = json!(cursor);
        output["pageComplete"] = json!(false);
        output["outputSizeLimited"] = json!(true);
    }
    Ok(output)
}
