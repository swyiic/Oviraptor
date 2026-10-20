//! Selected saved evidence for one owner-visible record. No generic JSON dump.
use super::*;

#[derive(Default)]
struct View {
    limited: bool,
}
impl View {
    fn pick(
        &mut self,
        source: &Value,
        strings: &[&str],
        numbers: &[&str],
        booleans: &[&str],
    ) -> Value {
        let mut out = serde_json::Map::new();
        for key in strings {
            if let Some(value) = source.get(*key).and_then(Value::as_str) {
                let (value, cut) = text(value, 200);
                self.limited |= cut;
                out.insert((*key).into(), json!(value));
            }
        }
        for key in numbers {
            if let Some(value) = source
                .get(*key)
                .and_then(Value::as_u64)
                .filter(|n| *n <= 9_007_199_254_740_991)
            {
                out.insert((*key).into(), json!(value));
            }
        }
        for key in booleans {
            if let Some(value) = source.get(*key).and_then(Value::as_bool) {
                out.insert((*key).into(), json!(value));
            }
        }
        Value::Object(out)
    }
    fn urls(&mut self, source: Option<&Value>) -> Value {
        let Some(items) = source.and_then(Value::as_array) else {
            return Value::Null;
        };
        self.limited |= items.len() > 5;
        json!({"savedCount":items.len(),"items":items.iter().take(5).filter_map(|v|v.as_str().and_then(url)).collect::<Vec<_>>()})
    }
    fn detail(&mut self, kind: &str, source: &Value) -> Value {
        match kind {
            "investigation_decision" => {
                let mut out = self.pick(
                    source,
                    &[
                        "stopReason",
                        "automationTier",
                        "requestedModeCeiling",
                        "apiEvidenceSource",
                    ],
                    &[
                        "schemaVersion",
                        "informationGain",
                        "identityCount",
                        "observedRequestCount",
                        "readyHypotheses",
                        "verifiedRuntimeApiCount",
                        "sourceMappedReadOnlyApiCount",
                    ],
                    &[
                        "eligibleForModel",
                        "baselineInvestigationAllowed",
                        "sourceGuidedInvestigationAllowed",
                        "standardInvestigationAllowed",
                        "runtimeProbeAvailable",
                        "authSessionCaptureAvailable",
                    ],
                );
                out["coverage"] = self.pick(
                    &source["coverage"],
                    &[],
                    &["actionCount", "identityCount", "requestCount", "stateCount"],
                    &[],
                );
                out["baseline"] = self.pick(
                    &source["baseline"],
                    &[],
                    &[
                        "addedApis",
                        "addedParameters",
                        "removedApis",
                        "removedParameters",
                    ],
                    &["available"],
                );
                if let Some(items) = source.get("manualDeepDive").and_then(Value::as_array) {
                    self.limited |= items.len() > 2;
                    let entries = items
                        .iter()
                        .take(2)
                        .map(|v| {
                            self.pick(
                                v,
                                &[
                                    "title",
                                    "category",
                                    "classification",
                                    "priority",
                                    "reason",
                                    "missingEvidence",
                                    "stopCondition",
                                ],
                                &["rank"],
                                &[],
                            )
                        })
                        .collect::<Vec<_>>();
                    out["recordedManualCandidates"] =
                        json!({"savedCount":items.len(),"items":entries});
                }
                out
            }
            "runtime_diagnostics" => {
                let mut out = json!({});
                out["analysisSummary"] = self.pick(
                    &source["analysisSummary"],
                    &["engine", "runtimeCaptureStatus"],
                    &[
                        "apiCandidates",
                        "identityCount",
                        "runtimeRequests",
                        "verifiedApis",
                    ],
                    &["runtimeBrowserAvailable", "runtimeProbeAvailable"],
                );
                out["collectionOutcome"] = self.pick(
                    &source["collectionOutcome"],
                    &[],
                    &["failedIdentities"],
                    &["partialEvidence", "runtimeCaptureComplete"],
                );
                if let Some(items) = source.get("runtimeDiagnostics").and_then(Value::as_array) {
                    self.limited |= items.len() > 3;
                    let entries = items
                        .iter()
                        .take(3)
                        .map(|v| {
                            let mut item = self.pick(
                                v,
                                &[
                                    "captureStatus",
                                    "failedStage",
                                    "runtimeStopReason",
                                    "stopReason",
                                    "cdpTransport",
                                ],
                                &[],
                                &["available"],
                            );
                            item["captureErrorRecorded"] = json!(v
                                .get("captureError")
                                .and_then(Value::as_str)
                                .is_some_and(|s| !s.is_empty()));
                            item["browserErrorOutputRecorded"] = json!(v
                                .get("browserStderr")
                                .and_then(Value::as_str)
                                .is_some_and(|s| !s.is_empty()));
                            item
                        })
                        .collect::<Vec<_>>();
                    out["diagnostics"] = json!({"savedCount":items.len(),"items":entries});
                }
                out
            }
            "js_file" => {
                let mut out = self.pick(
                    source,
                    &["analysisStatus"],
                    &["size", "statusCode"],
                    &["external"],
                );
                out["url"] = source
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(url)
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                let mut analysis = self.pick(
                    &source["analysis"],
                    &["extractionEngine"],
                    &["apiCandidates"],
                    &[],
                );
                for (key, field) in [("routes", "routeCount"), ("parseErrors", "parseErrorCount")] {
                    if let Some(items) = source["analysis"].get(key).and_then(Value::as_array) {
                        analysis[field] = json!(items.len());
                    }
                }
                let mut counts = json!({});
                for key in [
                    "apiPrefixes",
                    "baseUrls",
                    "businessPaths",
                    "storageReferences",
                ] {
                    if let Some(items) = source["analysis"]["stringEvidence"]
                        .get(key)
                        .and_then(Value::as_array)
                    {
                        counts[key] = json!(items.len());
                    }
                }
                analysis["stringEvidenceCounts"] = counts;
                out["analysis"] = analysis;
                out
            }
            "endpoint" => {
                let mut out = self.pick(
                    source,
                    &["method", "note", "source"],
                    &["statusCode", "responseTime"],
                    &[],
                );
                out["url"] = source
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(url)
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                out
            }
            "fingerprint" => {
                let mut out = json!({});
                for key in ["backend", "cdn", "frontend", "server", "waf"] {
                    out[key] =
                        self.pick(&source[key], &["name", "framework", "confidence"], &[], &[]);
                }
                out
            }
            "tech_stack" => {
                let mut out = self.pick(source, &["framework", "poweredBy", "server"], &[], &[]);
                out["baseUrls"] = self.urls(source.get("baseUrls"));
                out
            }
            "links" => {
                let mut out = self.pick(source, &[], &["count"], &[]);
                out["links"] = self.urls(source.get("items"));
                out
            }
            "runtime_feature" => {
                let mut out = self.pick(
                    source,
                    &["title"],
                    &["depth", "formCount", "interactiveCount"],
                    &[],
                );
                out["url"] = source
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(url)
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                if let Some(items) = source.get("fieldNames").and_then(Value::as_array) {
                    out["fieldCount"] = json!(items.len());
                }
                out
            }
            "runtime_action" => {
                let mut out = self.pick(
                    source,
                    &["label", "outcome", "role"],
                    &["blockedRequestCount", "durationMs", "requestCount", "score"],
                    &["captureOnly", "stateChanged"],
                );
                for key in ["beforeUrl", "afterUrl"] {
                    out[key] = source
                        .get(key)
                        .and_then(Value::as_str)
                        .and_then(url)
                        .map(Value::String)
                        .unwrap_or(Value::Null);
                }
                out["errorRecorded"] = json!(source
                    .get("error")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty()));
                out
            }
            "vulnerability" => {
                let mut out = self.pick(
                    source,
                    &["impact", "category", "classification", "verificationStatus"],
                    &[],
                    &[],
                );
                for (old, recorded) in [
                    ("impact", "recordedImpact"),
                    ("classification", "recordedClassification"),
                    ("verificationStatus", "recordedVerificationStatus"),
                ] {
                    if let Some(value) = out.as_object_mut().and_then(|o| o.remove(old)) {
                        out[recorded] = value;
                    }
                }
                out["pairedRequestFieldsRecorded"] =
                    json!(proof_finding_is_visible(&source.to_string()));
                out
            }
            _ => json!({"available":false,"reason":"unsupported_record_kind"}),
        }
    }
}

pub(super) fn read(
    c: &rusqlite::Connection,
    project: i64,
    scan: &SentinelScan,
    record_id: i64,
    proof: bool,
) -> Result<Value, String> {
    let row = c.query_row("SELECT target_url,stage,kind,title,severity,CASE WHEN length(CAST(record_json AS BLOB))<=262144 THEN record_json ELSE NULL END,updated_at FROM sentinel_findings WHERE id=?1 AND scan_id=?2",
        params![record_id,scan.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?)))
        .optional().map_err(|_|"scan_results_unavailable")?.ok_or("scan_record_unavailable")?;
    let (target, stage, kind, title, severity, raw, updated) = row;
    let pair = kind == "vulnerability" && raw.as_deref().is_some_and(proof_finding_is_visible);
    if proof && kind == "vulnerability" && !pair {
        return Err("scan_record_unavailable".into());
    }
    let mut view = View::default();
    let detail = match raw {
        None => json!({"available":false,"reason":"record_size_limit"}),
        Some(ref raw) => match serde_json::from_str::<Value>(raw) {
            Ok(source) if source.is_object() => view.detail(&kind, &source),
            _ => json!({"available":false,"reason":"invalid_record_json"}),
        },
    };
    let mut task = projection(c, scan)?;
    task["title"] = json!(text(&scan.task_name, 240).0);
    let output = json!({"projectId":project,"task":task,"section":"evidence","readOnly":true,
        "record":{"id":record_id,"url":url(&target),"stage":text(&stage,80).0,"kind":text(&kind,80).0,"title":text(&title,500).0,"severity":text(&severity,40).0,"updatedAt":text(&updated,80).0,
        "proofPairFieldsRecorded":pair,"independentVerificationByThisRead":false,"evidence":detail,"evidenceTruncated":view.limited},
        "evidenceProjection":"selected_saved_fields_redacted","rawEvidenceIncluded":false,"snapshotScope":"single_request","source":"desktop_scanning_owner",
        "interpretation":"仅返回此已保存记录的选定分析字段；未返回原始HTTP、页面/脚本正文、Cookie、身份凭据或本地路径。采集记录和已记录影响不等于本次独立验证；候选及后续建议不授予执行权限。"});
    if serde_json::to_vec(&output)
        .map_err(|_| "scan_results_unavailable")?
        .len()
        > 24_000
    {
        return Err("scan_results_page_too_large".into());
    }
    Ok(output)
}
