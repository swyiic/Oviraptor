// #5 — cloud object-storage (OSS / S3 / COS / …) exposure probe. Operator command,
// like the exposure probe (#4) and the artifact analyzer (#1): an explicit human
// action, off the agent budget, SSRF-guarded. Two tiers:
//
//   READ  (always): GET the bucket root and classify an anonymous public LISTING
//          (the bucket enumerates its objects without credentials). Harmless.
//   CANARY (gated): prove anonymous WRITE with a reversible marker — PUT a random
//          innocuous object, GET it back to confirm, then DELETE it. This is a
//          target-mutating action, so it is refused unless the scan has authorized
//          the `target_write` high-risk class with a legal attestation (the
//          mandatory switch); the operator invoking the command is the per-action
//          human approval. The canary cleans up after itself; if the DELETE fails
//          the result says so loudly so the operator can remove it by hand.
//
// It writes nothing but a single self-deleting marker, never lists/reads other
// objects' contents, and never touches an internal/loopback literal (SSRF floor).

/// Provider label from the bucket host, for the finding only (best-effort).
fn bucket_provider(host: &str) -> &'static str {
    let h = host.to_ascii_lowercase();
    if h.contains(".aliyuncs.com") {
        "aliyun_oss"
    } else if h.contains(".amazonaws.com") || h.ends_with(".r2.cloudflarestorage.com") {
        "aws_s3"
    } else if h.contains(".myqcloud.com") {
        "tencent_cos"
    } else if h.contains(".huaweicloud.com") || h.contains(".obs.") {
        "huawei_obs"
    } else if h.contains(".qiniucs.com") || h.contains(".qbox.me") {
        "qiniu"
    } else if h.contains(".blob.core.windows.net") {
        "azure_blob"
    } else if h.contains("storage.googleapis.com") {
        "gcs"
    } else {
        "unknown"
    }
}

/// True when a 2xx bucket-root body is an anonymous object LISTING — the common
/// S3/OSS XML listing shapes. A plain 200 HTML page or an access-denied XML is NOT
/// a listing, so this does not over-claim on a bucket that merely answers.
fn bucket_lists_publicly(status: u16, body: &str) -> bool {
    if !(200..300).contains(&status) {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    (lower.contains("<listbucketresult") || lower.contains("<listallmybucketsresult"))
        && !lower.contains("<error")
        && !lower.contains("accessdenied")
}

/// A write is accepted on these success codes. Object PUT typically returns 200 or
/// 201 (sometimes 204 with no body).
fn bucket_write_accepted(status: u16) -> bool {
    matches!(status, 200 | 201 | 204)
}

/// Random, clearly-labelled canary object key that cannot collide with real data.
fn bucket_canary_key() -> String {
    format!("oviraptor-authorized-pentest-canary-{}.txt", Uuid::new_v4())
}

/// The canary object body: an innocuous, self-describing marker that is safe to
/// delete and carries a unique token we read back to confirm WE wrote it.
fn bucket_canary_body(token: &str) -> String {
    format!(
        "oviraptor authorized-pentest write-exposure canary. safe to delete. token={token}"
    )
}

#[tauri::command]
pub fn probe_cloud_bucket(
    state: State<AppState>,
    scan_id: String,
    bucket_url: String,
    canary: Option<bool>,
) -> Result<JsonValue, String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    let parsed = reqwest::Url::parse(&bucket_url).map_err(|_| "bucket_url 无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("只支持 http/https".into());
    }
    let host = parsed.host_str().ok_or("bucket_url 缺少主机名")?.to_string();
    if host_is_internal_literal(&host) {
        return Err("内网/环回/链路本地地址默认拒绝(SSRF 硬守卫)".into());
    }
    let connection = db::open(&state.db_path)?;
    let exists: bool = connection
        .query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)", [&scan_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("任务不存在".into());
    }
    let provider = bucket_provider(&host);
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "HTTP 客户端不可用".to_string())?;
    let base = bucket_url.trim_end_matches('/').to_string();
    let mut findings: Vec<JsonValue> = Vec::new();

    // READ tier: anonymous public listing.
    let listing = client
        .get(parsed.as_str())
        .header(reqwest::header::USER_AGENT, "oviraptor-bucket/1.0")
        .send();
    let mut public_listing = false;
    if let Ok(resp) = listing {
        let status = resp.status().as_u16();
        let mut buf = Vec::new();
        let _ = std::io::Read::take(resp, 65_536).read_to_end(&mut buf);
        let body = String::from_utf8_lossy(&buf);
        public_listing = bucket_lists_publicly(status, &body);
        if public_listing {
            let record = serde_json::json!({
                "url": base, "provider": provider, "status": status, "cwe": "CWE-284",
                "detail": "匿名可列举对象(public bucket listing)"
            });
            write_cloud_bucket_finding(&connection, &scan_id, &base, "public_listing", "high",
                &format!("对象存储可匿名列举：{host}"), &record);
            findings.push(record);
        }
    }

    // CANARY tier: gated reversible write proof.
    let mut canary_result = serde_json::Value::Null;
    if canary.unwrap_or(false) {
        if !scan_capability_class_authorized(&connection, &scan_id, scan_attempt_number(&connection, &scan_id), "target_write")
            .unwrap_or(false)
        {
            return Err("写入 canary 需先为该任务授权 target_write 能力并完成合法授权声明(强制开关)".into());
        }
        let token = Uuid::new_v4().to_string();
        let key = bucket_canary_key();
        let object_url = format!("{base}/{key}");
        let body = bucket_canary_body(&token);
        let put = client
            .put(&object_url)
            .header(reqwest::header::USER_AGENT, "oviraptor-bucket/1.0")
            .header(reqwest::header::CONTENT_TYPE, "text/plain")
            .body(body.clone())
            .send();
        match put {
            Ok(resp) => {
                let put_status = resp.status().as_u16();
                if bucket_write_accepted(put_status) {
                    // Confirm WE wrote it (read back the unique token).
                    let readback = client.get(&object_url).send();
                    let confirmed = readback
                        .map(|r| {
                            let ok = r.status().is_success();
                            let mut buf = Vec::new();
                            let _ = std::io::Read::take(r, 4_096).read_to_end(&mut buf);
                            ok && String::from_utf8_lossy(&buf).contains(&token)
                        })
                        .unwrap_or(false);
                    // Always attempt cleanup.
                    let delete_status = client
                        .delete(&object_url)
                        .send()
                        .map(|r| r.status().as_u16())
                        .unwrap_or(0);
                    let cleaned = matches!(delete_status, 200 | 202 | 204);
                    if confirmed {
                        let record = serde_json::json!({
                            "url": object_url, "provider": provider, "putStatus": put_status,
                            "readbackConfirmed": true, "deleteStatus": delete_status,
                            "cleanedUp": cleaned, "cwe": "CWE-732",
                            "detail": "匿名可写入对象(public bucket write);已回读确认并尝试清理 canary"
                        });
                        write_cloud_bucket_finding(&connection, &scan_id, &object_url, "public_write", "critical",
                            &format!("对象存储可匿名写入：{host}"), &record);
                        findings.push(record.clone());
                        canary_result = record;
                    } else {
                        canary_result = serde_json::json!({
                            "writeAccepted": true, "readbackConfirmed": false,
                            "deleteStatus": delete_status, "cleanedUp": cleaned,
                            "note": "PUT 返回成功码但回读未确认;未登记写入漏洞以免误报"
                        });
                    }
                    if !cleaned {
                        canary_result["cleanupWarning"] = serde_json::json!(
                            format!("canary 对象可能未删除,请手动清理:{object_url}")
                        );
                    }
                } else {
                    canary_result = serde_json::json!({"writeAccepted": false, "putStatus": put_status});
                }
            }
            Err(_) => {
                canary_result = serde_json::json!({"writeAccepted": false, "error": "PUT 请求失败"});
            }
        }
    }

    Ok(serde_json::json!({
        "host": host,
        "provider": provider,
        "publicListing": public_listing,
        "canary": canary_result,
        "findings": findings.len(),
    }))
}

fn scan_attempt_number(connection: &rusqlite::Connection, scan_id: &str) -> i64 {
    connection
        .query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [scan_id], |r| r.get::<_, i64>(0))
        .unwrap_or(1)
        .max(1)
}

fn write_cloud_bucket_finding(
    connection: &rusqlite::Connection,
    scan_id: &str,
    target_url: &str,
    record_key: &str,
    severity: &str,
    title: &str,
    record: &JsonValue,
) {
    let _ = connection.execute(
        "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
         VALUES(?1,?2,'exposure','cloud_bucket',?3,?4,?5,?6) \
         ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
         title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
        params![scan_id, target_url, record_key, title, severity, record.to_string()],
    );
}

#[cfg(test)]
mod cloud_bucket_tests {
    use super::*;

    #[test]
    fn provider_is_classified_from_host() {
        assert_eq!(bucket_provider("mybucket.oss-cn-hangzhou.aliyuncs.com"), "aliyun_oss");
        assert_eq!(bucket_provider("x.s3.amazonaws.com"), "aws_s3");
        assert_eq!(bucket_provider("x-1251.cos.ap-guangzhou.myqcloud.com"), "tencent_cos");
        assert_eq!(bucket_provider("example.com"), "unknown");
    }

    #[test]
    fn public_listing_needs_a_real_listing_body() {
        assert!(bucket_lists_publicly(200, "<?xml version=\"1.0\"?><ListBucketResult><Contents><Key>a</Key></Contents></ListBucketResult>"));
        // Access-denied XML is not a public listing.
        assert!(!bucket_lists_publicly(200, "<Error><Code>AccessDenied</Code></Error>"));
        // A listing shape but a 403 is not public.
        assert!(!bucket_lists_publicly(403, "<ListBucketResult></ListBucketResult>"));
        // Plain HTML 200 is not a listing.
        assert!(!bucket_lists_publicly(200, "<html><body>hello</body></html>"));
    }

    #[test]
    fn write_accept_codes() {
        for s in [200u16, 201, 204] {
            assert!(bucket_write_accepted(s), "{s}");
        }
        for s in [403u16, 401, 405, 500] {
            assert!(!bucket_write_accepted(s), "{s}");
        }
    }

    #[test]
    fn canary_key_is_unique_and_labelled() {
        let a = bucket_canary_key();
        let b = bucket_canary_key();
        assert_ne!(a, b);
        assert!(a.contains("oviraptor-authorized-pentest-canary-") && a.ends_with(".txt"));
    }

    #[test]
    fn canary_body_carries_the_token() {
        let body = bucket_canary_body("tok123");
        assert!(body.contains("tok123") && body.contains("safe to delete"));
    }
}
