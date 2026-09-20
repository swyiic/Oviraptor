// On-disk artifacts of a frontend recon run: the target descriptor, source slices and
// the evidence bundle the result importer reads back. Included from frontend_recon.rs.

fn json_array_len(value: &JsonValue, key: &str) -> usize {
    value
        .get(key)
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn frontend_recon_target(recon_path: &Path, url: &str) -> Option<JsonValue> {
    let recon = fs::read(recon_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok())?;
    recon
        .get("targets")
        .and_then(JsonValue::as_array)
        .and_then(|targets| {
            targets.iter().find(|target| {
                let candidate = value_first(target, &["url", "finalUrl"]);
                asset_match_keys(&candidate)
                    .iter()
                    .any(|key| asset_match_keys(url).contains(key))
            })
        })
        .cloned()
}

fn bounded_utf8_bytes(value: &str, limit: usize) -> &str {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn frontend_slice_manifest(
    index: &[JsonValue],
    max_total_bytes: usize,
    available_slice_bytes: usize,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "policy": {
            "sliceArtifactMaxBytes": max_total_bytes,
            "maxReads": 3,
            "maxBytesPerRead": 14_000,
            "maxCumulativeReadBytes": max_total_bytes,
            "availableSliceBytes": available_slice_bytes,
            "stopAfterUnproductiveReads": 2,
            "readOnly": true,
            "completeBundleAvailable": false,
            "preferredKinds": ["dependency-definition", "http-client", "network-call", "business-flow", "marker-window"]
        },
        "slices": index,
    }))
    .unwrap_or_default()
}

fn write_frontend_code_slices(
    target: &JsonValue,
    target_dir: &Path,
    max_total_bytes: usize,
) -> usize {
    let Some(slices) = target
        .pointer("/aiFallback/codeSlices")
        .and_then(JsonValue::as_array)
    else {
        return 0;
    };
    let slices_dir = target_dir.join("frontend-code-slices");
    let mut index = Vec::new();
    let mut files = Vec::<(String, Vec<u8>)>::new();
    let mut seen_ids = HashSet::new();
    let mut total_slice_bytes = 0usize;
    for slice in slices.iter().take(8) {
        let id = bounded_text(slice, &["id"], 32)
            .chars()
            .filter(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
            .collect::<String>();
        let context = value_first(slice, &["context"]);
        if id.is_empty()
            || !seen_ids.insert(id.clone())
            || context.trim().is_empty()
            || total_slice_bytes >= max_total_bytes
        {
            continue;
        }
        let file_name = format!("{id}.js");
        let header = format!(
            "/* Oviraptor bounded code slice; source: {}; range: {}-{} */\n",
            bounded_text(slice, &["source"], 700),
            slice.get("start").and_then(JsonValue::as_i64).unwrap_or(0),
            slice.get("end").and_then(JsonValue::as_i64).unwrap_or(0),
        );
        let mut body = bounded_utf8_bytes(&context, context.len().min(14_000)).to_string();
        let mut accepted = None;
        while body.len() >= 200 {
            let content = format!("{header}{body}\n").into_bytes();
            let entry = serde_json::json!({
                "id": id,
                "file": format!("frontend-code-slices/{file_name}"),
                "source": bounded_text(slice, &["source"], 700),
                "kind": bounded_text(slice, &["kind"], 80),
                "marker": bounded_text(slice, &["marker"], 120),
                "start": slice.get("start").and_then(JsonValue::as_i64).unwrap_or(0),
                "end": slice.get("end").and_then(JsonValue::as_i64).unwrap_or(0),
                "bytes": body.len(),
            });
            let mut trial_index = index.clone();
            trial_index.push(entry.clone());
            let manifest = frontend_slice_manifest(
                &trial_index,
                max_total_bytes,
                total_slice_bytes + content.len(),
            );
            let projected = total_slice_bytes + content.len() + manifest.len();
            if projected <= max_total_bytes {
                accepted = Some((content, entry));
                break;
            }
            let overflow = projected.saturating_sub(max_total_bytes).max(1);
            if body.len() <= 200 + overflow {
                break;
            }
            let next_len = body.len() - overflow;
            body = bounded_utf8_bytes(&body, next_len).to_string();
        }
        if let Some((content, entry)) = accepted {
            total_slice_bytes += content.len();
            files.push((file_name, content));
            index.push(entry);
        }
    }
    if index.is_empty() || fs::create_dir_all(&slices_dir).is_err() {
        return 0;
    }
    let manifest = frontend_slice_manifest(&index, max_total_bytes, total_slice_bytes);
    let mut written = 0usize;
    for (file_name, content) in files {
        if fs::write(slices_dir.join(file_name), &content).is_ok() {
            written += content.len();
        }
    }
    if fs::write(target_dir.join("frontend-code-index.json"), &manifest).is_ok() {
        written += manifest.len();
    }
    written
}

fn write_frontend_evidence(
    recon_path: &Path,
    url: &str,
    target_dir: &Path,
    route: &FrontendRoute,
    packet_budget: usize,
    db_path: Option<&Path>,
    scan_id: &str,
) {
    let target = frontend_recon_target(recon_path, url).unwrap_or_else(
        || serde_json::json!({"url": url, "errors": ["frontend recon result unavailable"]}),
    );
    let local_knowledge_matches = db_path
        .and_then(|path| db::open(path).ok())
        .map(|connection| {
            let project_id = connection
                .query_row(
                    "SELECT project_id FROM sentinel_scans WHERE id=?1",
                    [scan_id],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()
                .ok()
                .flatten()
                .flatten();
            let mut matches = Vec::new();
            let mut seen = HashSet::new();
            for opportunity in target
                .get("opportunities")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
            {
                if let Ok(items) =
                    opportunity_knowledge_matches(&connection, project_id, opportunity)
                {
                    for item in items {
                        let id = item.get("id").and_then(JsonValue::as_i64).unwrap_or(0);
                        if id > 0 && seen.insert(id) {
                            matches.push(item);
                        }
                        if matches.len() >= 6 {
                            break;
                        }
                    }
                }
                if matches.len() >= 6 {
                    break;
                }
            }
            matches
        })
        .unwrap_or_default();
    let evidence_budget = packet_budget.saturating_mul(2) / 3;
    let slice_budget = packet_budget.saturating_sub(evidence_budget);
    let mut evidence = compact_frontend_evidence(&target, url, route, evidence_budget);
    let contract_limit = web_mode_contract_limit(&route.mode) as usize;
    let fallback_api_limit = match route.mode.as_str() {
        "quick" => 6,
        "deep" => 48,
        _ => 20,
    };
    if let Some(path) = db_path {
        if let Ok(connection) = db::open(path) {
            let metrics = read_investigation_metrics(&connection, scan_id, url)
                .ok()
                .flatten();
            let minimum_hypothesis_score = match route.mode.as_str() {
                "deep" => 35,
                "standard" => 50,
                _ => 65,
            };
            let hypotheses = read_investigation_hypotheses(&connection, scan_id, url, "")
                .unwrap_or_default()
                .into_iter()
                // The verifier receives only concrete, model-eligible contracts.
                // Candidate/template rows remain in the local graph until the
                // deterministic collector can obtain a real request contract.
                .filter(|item| {
                    item.score >= minimum_hypothesis_score
                        && matches!(item.status.as_str(), "ready" | "in_progress")
                        && item
                            .decision
                            .get("eligibleForModel")
                            .and_then(JsonValue::as_bool)
                            .unwrap_or(false)
                })
                .take(contract_limit)
                .map(|item| serde_json::json!({
                    "hypothesisKey":item.hypothesis_key,
                    "category":item.category,
                    "title":item.title,
                    "score":item.score,
                    "confidence":item.confidence,
                    "status":item.status,
                    "contract":item.contract,
                    "mutationApproval":item.mutation_approval,
                    "evidence":item.evidence,
                    "decision":item.decision,
                }))
                .collect::<Vec<_>>();
            let api_models = read_investigation_apis(&connection, scan_id, url)
                .unwrap_or_default()
                .into_iter()
                .filter(|item| item.baseline_status != "unchanged" || item.source.contains("runtime"))
                .take(fallback_api_limit)
                .map(|item| serde_json::json!({
                    "apiKey":item.api_key,"method":item.method,"path":item.normalized_path,
                    "source":item.source,"confidence":item.confidence,"authScope":item.auth_scope,
                    "parameters":item.parameters,"requestSchema":item.request_schema,
                    "responseSchema":item.response_schema,"stateKeys":item.state_keys,
                    "actionKeys":item.action_keys,"identityKeys":item.identity_keys,
                    "baselineStatus":item.baseline_status,
                }))
                .collect::<Vec<_>>();
            let actions = read_investigation_actions(&connection, scan_id, url)
                .unwrap_or_default()
                .into_iter()
                .take(if route.mode == "deep" { 32 } else { 16 })
                .map(|item| serde_json::json!({
                    "actionKey":item.action_key,"stateKey":item.state_key,"type":item.action_type,
                    "label":item.label,"outcome":item.outcome,"valueScore":item.value_score,
                    "protocol":item.protocol,
                }))
                .collect::<Vec<_>>();
            let identity_differences = read_investigation_identity_diffs(&connection, scan_id, url)
                .unwrap_or_default()
                .into_iter()
                .take(if route.mode == "deep" { 32 } else { 16 })
                .map(|item| serde_json::json!({
                    "apiKey":item.api_key,"leftIdentity":item.left_identity_key,
                    "rightIdentity":item.right_identity_key,"differenceType":item.difference_type,
                    "riskScore":item.risk_score,"matrix":item.matrix,
                    "classification":"authorization_candidate_not_vulnerability",
                }))
                .collect::<Vec<_>>();
            if let Some(object) = evidence.as_object_mut() {
                let standard_allowed = metrics
                    .as_ref()
                    .and_then(|item| item.decision.get("standardInvestigationAllowed"))
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                let baseline_allowed = metrics
                    .as_ref()
                    .and_then(|item| item.decision.get("baselineInvestigationAllowed"))
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                let automation_tier = metrics
                    .as_ref()
                    .and_then(|item| item.decision.get("automationTier"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!("recon_only"));
                let source_guided = metrics
                    .as_ref()
                    .and_then(|item| item.decision.get("sourceGuidedInvestigationAllowed"))
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                let incremental_decision = compact_incremental_decision(
                    metrics.as_ref().map(|item| &item.decision),
                );
                let manual_deep_dive = compact_manual_deep_dive(
                    metrics.as_ref().map(|item| &item.decision),
                );
                let agent_rule = if metrics.as_ref().map(|item| item.token_worthy).unwrap_or(false) {
                    "Execute every listed eligible hypothesis sequentially without waiting for an operator. Oviraptor grants automatic bounded authorization for the contract's exact endpoint, method and maxAttempts. Perform read-only and non-destructive control/test requests directly; benign marker uploads must be cleaned up. Never perform irreversible deletion, financial transactions, external messaging or persistent account/permission changes. Mark ordinary/no-impact outcomes exhausted and continue to the next contract. Finish the target after the bounded queue is exhausted, even when no finding is confirmed."
                } else if source_guided {
                    "No risk hypothesis is asserted. The anonymous page did not naturally issue a business XHR/fetch, but Oviraptor recovered exact high-confidence GET/HEAD calls from source-map call sites. Validate only those listed source-guided apiModels with bounded read-only requests, preserve control responses, and stop at the task limit. Never execute inferred writes, placeholder URLs, or arbitrary string combinations."
                } else if standard_allowed {
                    "No risk hypothesis is asserted. Perform a progressive coverage investigation beginning with the listed browser-observed apiModels and current authorized session. Prefer meaningful non-telemetry APIs, obtain read-only control responses, compare status/schema/identity scope when available, and use remaining discovery passes only for business-route-derived hidden interfaces. Finish the target after the coverage plan even if all results are normal."
                } else if baseline_allowed {
                    "The browser reached an interactive application but did not naturally emit a usable business API. Run the required low-cost baseline families, then use only the configured targeted discovery passes derived from same-origin links, forms, script call sites and business words. Promote an endpoint only after a real response; stop the branch when it has no new endpoint or response difference."
                } else {
                    "The local evidence gate is closed. Preserve the deterministic reconnaissance result and finish without model-side discovery."
                };
                object.insert("investigation".into(), serde_json::json!({
                    "modelGate":metrics.as_ref().map(|item| item.token_worthy).unwrap_or(false),
                    "standardInvestigationAllowed":standard_allowed,
                    "baselineInvestigationAllowed":baseline_allowed,
                    "sourceGuidedInvestigationAllowed":source_guided,
                    "automationTier":automation_tier,
                    "informationGain":metrics.as_ref().map(|item| item.information_gain).unwrap_or(0),
                    "stopReason":metrics.as_ref().map(|item| item.stop_reason.clone()).unwrap_or_default(),
                    "incrementalDecision":incremental_decision,
                    "manualDeepDive":manual_deep_dive,
                    "hypotheses":hypotheses,"apiModels":api_models,"actions":actions,
                    "identityDifferences":identity_differences,
                    "automationPolicy":{
                        "mode":"ai_auto_sequential",
                        "maxContracts":contract_limit,
                        "fallbackApiLimit":fallback_api_limit,
                        "requiredBaselineFamilies":["information_disclosure","error_handling","authentication_session","authorization","input_reflection_xss","hidden_interface_discovery","business_flow"],
                        "budgetSemantics":"expand_while_new_evidence_then_close_on_no_progress",
                        "authorizationMode":"automatic_bounded",
                        "humanReviewStage":"optional_final_review",
                        "suspiciousOnlyEscalation":true,
                    },
                    "manualReviewRule":"manualDeepDive contains deterministic coverage gaps, not findings. After the automatic queue, report the highest-priority untested leads with their missing evidence and stop condition. Never claim they are vulnerabilities and never spend model turns rediscovering them.",
                    "agentRule":agent_rule
                }));
            }
        }
    }
    if !local_knowledge_matches.is_empty() {
        if let Some(object) = evidence.as_object_mut() {
            object.insert(
                "localKnowledgeMatches".into(),
                JsonValue::Array(local_knowledge_matches),
            );
            if let Some(plan) = object
                .get_mut("verificationPlan")
                .and_then(JsonValue::as_object_mut)
            {
                plan.insert("localKnowledgeMatched".into(), JsonValue::Bool(true));
            }
        }
        trim_evidence_to_budget(&mut evidence, evidence_budget.clamp(1024, 64 * 1024));
    }
    if route.surface != "static_frontend" {
        write_frontend_code_slices(&target, target_dir, slice_budget);
    }
    if let Ok(bytes) = serde_json::to_vec(&evidence) {
        let _ = fs::write(target_dir.join("frontend-evidence.json"), bytes);
    }
}

fn approved_strix_proxies(settings: &JsonValue) -> Vec<(String, String)> {
    if !settings
        .get("strixProxyEnabled")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return Vec::new();
    }
    settings
        .get("authorizedProxyPool")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let raw = item.as_str()?.trim();
            let (tag, url) = raw.split_once('|').unwrap_or(("ALL", raw));
            let url = url.trim();
            if !(url.starts_with("http://")
                || url.starts_with("https://")
                || url.starts_with("socks5://")
                || url.starts_with("socks5h://"))
            {
                return None;
            }
            Some((tag.trim().to_ascii_uppercase(), url.to_string()))
        })
        .collect()
}
