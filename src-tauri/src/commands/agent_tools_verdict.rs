fn agent_tool_record_hypothesis_result(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    let key = value_first(arguments, &["hypothesisKey"]);
    let mut status = value_first(arguments, &["status"]);
    let contract = value_first(arguments, &["contractKey"]);
    let mut downgraded_from = String::new();
    let mut missing_evidence = Vec::new();
    let mut bound: Option<ConfirmedPair> = None;
    if status == "confirmed" {
        let missing: Vec<&str> = [
            ("impact", "影响"),
            ("reproductionSteps", "可复现步骤"),
        ]
        .into_iter()
        .filter(|(field, _)| value_first(arguments, &[field]).is_empty())
        .map(|(_, label)| label)
        .collect();
        let refusal = if missing.is_empty() {
            None
        } else {
            Some(refuse_confirmation(
                "confirmed_requires_full_contract",
                &format!("确认结论缺少：{}", missing.join("、")),
            ))
        };
        let outcome = match refusal {
            Some(refusal) => Err(refusal),
            None => agent_confirmed_finding_gate(context, runtime, arguments),
        };
        match outcome {
            Ok(pair) => bound = Some(pair),
            Err(refusal) => {
                // §10.3: an unsupported confirmation becomes insufficient evidence.
                // It is never kept as a finding and never fails the run.
                missing_evidence.push(value_first(&refusal, &["code"]));
                downgraded_from = "confirmed".to_string();
                status = "insufficient_evidence".to_string();
            }
        }
    }
    // A verdict may only close work this run actually executed; otherwise a
    // rejected or timed-out contract would be silently marked finished.
    if !contract.is_empty() && runtime.contract_attempts.get(&contract).copied().unwrap_or(0) == 0
    {
        return agent_tool_error(
            "该契约还没有执行过任何请求，不能出具结论",
            "contract_not_executed",
        );
    }
    let graph_status = match status.as_str() {
        "confirmed" => "validated",
        "rejected" => "rejected",
        "exhausted" => "exhausted",
        _ => "ready",
    };
    if status == "confirmed" {
        let Some(pair) = bound.as_ref() else {
            return refuse_confirmation(
                "confirmed_requires_bound_pair",
                "确认结论未绑定控制/测试请求与响应差异记录",
            );
        };
        if let Err(error) = persist_agent_vulnerability(context, arguments, &key, pair) {
            return serde_json::json!({"error": error, "code": "finding_persist_failed"});
        }
        runtime.confirmed_findings += 1;
    }
    // A failed persistence must not leave a validated hypothesis or an in-memory
    // verdict behind. This is particularly important when a Reviewer later reads
    // the graph after the candidate store rejected the write.
    let graph_updated = update_agent_hypothesis_status(context, &key, graph_status);
    if runtime.verdict_keys.insert(format!("{key}:{status}")) {
        runtime.last_progress.new_verdicts += 1;
    }
    if !contract.is_empty() {
        runtime.contract_outcomes.insert(contract, status.clone());
    }
    let family = value_first(arguments, &["family"]);
    if runtime.credit_family(&family) {
        runtime.last_progress.new_families += 1;
    }
    let verdict_id = runtime.verdict_id_for(&format!("{key}:{status}"));
    for entry in runtime
        .coverage
        .iter_mut()
        .filter(|entry| entry.family == family && entry.verdict_id.is_empty())
    {
        entry.verdict_id = verdict_id.clone();
    }
    serde_json::json!({
        "hypothesisKey": key,
        "status": status,
        "graphStatus": graph_status,
        "graphUpdated": graph_updated,
        "countedAsVulnerability": status == "confirmed",
        "downgradedFrom": downgraded_from,
        "missingEvidence": missing_evidence,
        "boundRequests": bound.as_ref().map(|pair| serde_json::json!({
            "controlRequestId": pair.control_request_id,
            "testRequestId": pair.test_request_id,
            "responseDifferenceArtifactId": pair.difference_artifact_id,
        })),
        "note": if status == "insufficient_evidence" { "证据不足不能算漏洞，也不能算成功验证" } else { "" },
    })
}

/// Whether a response may credit coverage: a refused or scope-external redirect
/// means the target answer came from somewhere the plan never authorized (§5.4).
fn agent_redirect_credits_coverage(response: &JsonValue) -> bool {
    matches!(
        value_first(response, &["redirectCode"]).as_str(),
        "" | "redirect_within_scope"
    )
}

/// A confirmed finding has to point at requests this run really sent: at least
/// two executed exchanges, and the control and test request each name a path
/// The two executed requests a confirmed finding is bound to, plus the difference
/// record that links them (§10.1).
#[derive(Clone, Debug)]
struct ConfirmedPair {
    control_request_id: String,
    test_request_id: String,
    difference_artifact_id: String,
    control_summary: String,
    test_summary: String,
}

impl ConfirmedPair {
    fn summary_of(trace: &AgentRequestTrace) -> String {
        format!(
            "{} {} [{}] 身份 {} 状态 {}",
            trace.method,
            trace.path,
            trace.id,
            if trace.identity.is_empty() {
                "未标注".to_string()
            } else {
                trace.identity.clone()
            },
            trace.status
        )
    }
}

fn refuse_confirmation(code: &'static str, reason: &str) -> JsonValue {
    serde_json::json!({
        "error": reason,
        "code": code,
        "status": "insufficient_evidence",
    })
}

/// Read back one recorded HTTP exchange. The metadata file holds the model view, so
/// the structural facts a confirmation depends on survived redaction (§10.2).
fn agent_http_record(context: &AgentRunContext, artifact_id: &str) -> Option<JsonValue> {
    if artifact_id.is_empty() || !artifact_id.ends_with(".json") || artifact_id.contains('/') {
        return None;
    }
    let path = context
        .target_dir
        .join(AGENT_HTTP_DIRECTORY)
        .join(artifact_id);
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<JsonValue>(&text).ok()
}

/// §10.2: every hard condition a confirmed finding must satisfy, checked against
/// this execution chain's own records. Anything else is insufficient evidence.
fn agent_confirmed_finding_gate(
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    arguments: &JsonValue,
) -> Result<ConfirmedPair, JsonValue> {
    let control_id = value_first(arguments, &["controlRequestId"]);
    let test_id = value_first(arguments, &["testRequestId"]);
    let artifact_id = value_first(arguments, &["responseDifferenceArtifactId"]);
    if control_id.is_empty() || test_id.is_empty() || artifact_id.is_empty() {
        return Err(refuse_confirmation(
            "confirmed_requires_bound_pair",
            "确认结论必须引用 controlRequestId、testRequestId 与 responseDifferenceArtifactId",
        ));
    }
    if control_id == test_id {
        return Err(refuse_confirmation(
            "confirmed_requires_distinct_requests",
            "控制请求与测试请求必须是两条不同的已执行请求",
        ));
    }
    let (Some(control), Some(test)) = (runtime.request(&control_id), runtime.request(&test_id))
    else {
        return Err(refuse_confirmation(
            "confirmed_request_not_executed",
            "引用的请求不属于本执行链的账本",
        ));
    };
    let contract = value_first(arguments, &["contractKey"]);
    let family = value_first(arguments, &["family"]);
    let linked = |trace: &AgentRequestTrace| {
        if !contract.is_empty() {
            // A coverage family is not a substitute for the contract being
            // adjudicated. Otherwise a valid A/B pair for one business object
            // can be relabelled as proof for another contract in that family.
            trace.contract_key == contract
        } else {
            !family.is_empty() && trace.family == family
        }
    };
    if !linked(control) || !linked(test) {
        return Err(refuse_confirmation(
            "confirmed_request_unlinked",
            "引用的请求与该假设的契约或覆盖族没有关联",
        ));
    }
    // The claim's family is not an authority field: when a contract is given,
    // `linked` checks the key alone. Reject a missing or mismatched family so
    // that re-labelling an authorization pair cannot bypass its stronger gate.
    if family.is_empty() || control.family != family || test.family != family {
        return Err(refuse_confirmation(
            "confirmed_family_mismatch",
            "结论覆盖族必须与两条已执行请求一致",
        ));
    }
    if control.family == "authorization" {
        // A parameter mutation or two unrelated objects does not establish an
        // authorization boundary. Only the broker's same-request A/B identity
        // comparison can supply the pair for this family. The Reviewer still
        // decides whether its observed difference actually proves impact.
        if control.tool != "compare_identities"
            || test.tool != "compare_identities"
            || control.invocation_id <= 0
            || control.invocation_id != test.invocation_id
            || control.identity == test.identity
            || control.method != test.method
            || control.origin != test.origin
            || control.path != test.path
            || control.parameters != test.parameters
        {
            return Err(refuse_confirmation(
                "authorization_pair_not_comparable",
                "越权结论要求同一业务请求在两个任务身份下形成可比的 A/B 记录",
            ));
        }
    }
    // The contrast: a different identity handle or a different parameter set.
    if control.identity == test.identity && control.parameters == test.parameters {
        return Err(refuse_confirmation(
            "confirmed_requires_contrast",
            "两侧既没有更换身份也没有更换参数，不构成控制/测试对照",
        ));
    }
    if control.artifact_id.is_empty()
        || control.artifact_id == test.artifact_id
        || control.structure_hash.is_empty()
        || test.structure_hash.is_empty()
    {
        return Err(refuse_confirmation(
            "confirmed_requires_two_responses",
            "两侧必须各自留下响应 artifact 与结构指纹",
        ));
    }
    let record = match agent_read_diff_record(context, &artifact_id) {
        Some(record) => record,
        None => {
            return Err(refuse_confirmation(
                "difference_artifact_missing",
                "引用的响应差异 artifact 不存在",
            ))
        }
    };
    let side_request = |side: &str| -> String {
        record
            .get(side)
            .cloned()
            .map(|row| value_first(&row, &["requestId"]))
            .unwrap_or_default()
    };
    let forward =
        side_request("left") == control_id && side_request("right") == test_id;
    let reversed = side_request("left") == test_id && side_request("right") == control_id;
    if !(forward || reversed) {
        return Err(refuse_confirmation(
            "difference_artifact_mismatch",
            "响应差异 artifact 关联的不是这两条请求",
        ));
    }
    if control.family == "authorization"
        && (value_first(&record, &["contractKey"]) != contract
            || value_first(&record, &["method"]) != control.method)
    {
        return Err(refuse_confirmation(
            "authorization_difference_unlinked",
            "越权差异记录与本契约或请求方法不一致",
        ));
    }
    if control.family == "authorization" {
        // A same-request A/B comparison proves that two sessions behaved
        // differently; it does not establish which session owns the object.
        // In particular, /api/profile returning each account's own role is
        // expected personalization, not evidence of IDOR. Until an
        // independently validated owner/object/control-group contract is
        // bound to this exact pair, keep the candidate reviewable but never
        // publish it as a confirmed vulnerability.
        return Err(refuse_confirmation(
            "authorization_object_control_missing",
            "缺少由代码核验的对象归属及合法控制组；双身份响应差异不能单独确认越权",
        ));
    }
    if record.get("materialDifference").and_then(JsonValue::as_bool) != Some(true) {
        return Err(refuse_confirmation(
            "difference_not_material",
            "该差异记录本身未被判定为实质差异",
        ));
    }
    for trace in [control, test] {
        let Some(response) = agent_http_record(context, &trace.artifact_id)
            .and_then(|row| row.get("response").cloned())
        else {
            return Err(refuse_confirmation(
                "confirmed_response_record_missing",
                "引用的响应 artifact 无法读取",
            ));
        };
        if !value_first(&response, &["redirectCode"]).is_empty() {
            return Err(refuse_confirmation(
                "misled_by_redirect",
                "至少一侧被重定向，可能是登录跳转而非授权行为",
            ));
        }
        if value_first(&response, &["cacheState"]) == "hit" {
            return Err(refuse_confirmation(
                "misled_by_cache",
                "至少一侧的响应来自缓存，不能作为实时行为证据",
            ));
        }
        let challenge = format!(
            "http {} {} {}",
            response.get("status").and_then(JsonValue::as_i64).unwrap_or(0),
            value_first(&response, &["contentType"]),
            value_first(&response, &["preview"])
        );
        if is_directory_block_signal(&challenge) {
            return Err(refuse_confirmation(
                "misled_by_waf",
                "至少一侧命中 WAF、验证码或统一拦截页",
            ));
        }
    }
    Ok(ConfirmedPair {
        control_request_id: control_id,
        test_request_id: test_id,
        difference_artifact_id: artifact_id,
        control_summary: ConfirmedPair::summary_of(control),
        test_summary: ConfirmedPair::summary_of(test),
    })
}

fn update_agent_hypothesis_status(context: &AgentRunContext, key: &str, status: &str) -> bool {
    let Ok(connection) = db::open(&context.db_path) else {
        return false;
    };
    connection
        .execute(
            "UPDATE investigation_hypotheses SET status=?1,updated_at=datetime('now','localtime') WHERE scan_id=?2 AND hypothesis_key=?3",
            params![status, context.scan_id, key],
        )
        .map(|rows| rows > 0)
        .unwrap_or(false)
}

/// The close-out (§9.1): the ledger's own evidence decides what counts as
/// covered. A model may propose a family and give reasons, it cannot write the
/// final ledger — an unbacked `covered` claim is downgraded and reported.
fn agent_tool_finish_target(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    runtime.not_applicable.clear();
    let mut claimed: HashMap<String, (String, String, Vec<String>)> = HashMap::new();
    for row in arguments
        .get("coverage")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let family = value_first(&row, &["family"]);
        if family.is_empty() {
            continue;
        }
        let cited = row
            .get("evidenceIds")
            .and_then(JsonValue::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        claimed.insert(
            family,
            (
                value_first(&row, &["status"]),
                value_first(&row, &["reason"]),
                cited,
            ),
        );
    }
    // Every entry of the ledger for one family, and whether it proves the family.
    let entries_for = |runtime: &AgentToolRuntime, family: &str| -> Vec<JsonValue> {
        runtime
            .coverage
            .iter()
            .filter(|entry| entry.family == family)
            .map(|entry| {
                serde_json::json!({
                    "id": entry.id,
                    "kind": entry.evidence_kind,
                    "result": entry.result,
                    "reasonCode": entry.reason_code,
                    "requestRecordIds": entry.request_record_ids,
                    "toolInvocationIds": entry.tool_invocation_ids,
                    "verdictId": entry.verdict_id,
                    "contractKey": entry.contract_key,
                })
            })
            .collect()
    };
    let mut ledger_rows: Vec<JsonValue> = Vec::new();
    let mut covered: Vec<String> = Vec::new();
    let mut insufficient: Vec<JsonValue> = Vec::new();
    let mut not_applicable: Vec<JsonValue> = Vec::new();
    let mut not_covered: Vec<JsonValue> = Vec::new();
    let mut unsupported: Vec<JsonValue> = Vec::new();
    for family in agent_required_families().iter().map(|value| value.to_string()) {
        let evidence = entries_for(runtime, &family);
        let proven = runtime.has_coverage_evidence(&family);
        let claim = claimed.get(&family);
        // A citation the ledger never issued is refused, whatever else the row says.
        if let Some((_, _, cited)) = claim {
            let unknown: Vec<String> = cited
                .iter()
                .filter(|id| !evidence.iter().any(|row| row["id"] == **id))
                .cloned()
                .collect();
            if !unknown.is_empty() {
                unsupported.push(serde_json::json!({
                    "family": family,
                    "problem": "unknown_evidence_id",
                    "evidenceIds": unknown,
                }));
            }
        }
        if proven {
            runtime.credit_family(&family);
            covered.push(family.clone());
            ledger_rows.push(serde_json::json!({
                "family": family,
                "label": agent_coverage_family_label(&family),
                "status": "covered",
                "reason": "本轮已执行的请求与结论支持该覆盖族",
                "evidence": evidence,
            }));
            if claim.is_some_and(|(status, _, _)| status == "not_applicable") {
                unsupported.push(serde_json::json!({
                    "family": family,
                    "problem": "declared_not_applicable_with_evidence",
                }));
            }
            continue;
        }
        if claim.is_some_and(|(status, _, _)| status == "covered") {
            unsupported.push(serde_json::json!({
                "family": family,
                "problem": "covered_without_evidence",
                "evidence": evidence,
            }));
        }
        if claim.is_some_and(|(status, _, _)| status == "not_applicable") && evidence.is_empty() {
            let reason = claim
                .map(|(_, reason, _)| reason.clone())
                .unwrap_or_default();
            // "本轮没做" is not a reason the family cannot exist. Leave it open.
            if !agent_not_applicable_is_real(&reason) {
                continue;
            }
            runtime.not_applicable.insert(family.clone());
            not_applicable.push(serde_json::json!({
                "family": family,
                "label": agent_coverage_family_label(&family),
                "status": "not_applicable",
                "reason": if reason.is_empty() { "未说明为什么不适用".to_string() } else { reason },
            }));
            continue;
        }
        // Worked but not proven is its own display state: it is neither covered nor
        // "no risk found" (§9.5).
        let reason_code = evidence
            .iter()
            .filter_map(|row| row["reasonCode"].as_str())
            .find(|code| !code.is_empty())
            .unwrap_or("no_executed_request");
        // The status is Rust's; the model's own explanation is kept as text when it
        // offered one, because it is often more specific than the derived sentence.
        let proposed = claim
            .filter(|(status, _, _)| status != "covered")
            .map(|(_, reason, _)| reason.clone())
            .unwrap_or_default();
        let row = serde_json::json!({
            "family": family,
            "label": agent_coverage_family_label(&family),
            "status": if evidence.is_empty() { "not_covered" } else { "insufficient_evidence" },
            "reason": if proposed.is_empty() {
                agent_coverage_gap_text(&family, reason_code)
            } else {
                proposed
            },
            "reasonCode": reason_code,
            "evidence": evidence,
        });
        if evidence.is_empty() {
            not_covered.push(row.clone());
        } else {
            insufficient.push(row.clone());
        }
        ledger_rows.push(row);
    }
    runtime.uncovered_families = not_covered
        .iter()
        .chain(insufficient.iter())
        .chain(not_applicable.iter())
        .cloned()
        .collect();
    runtime.exclusions = string_array(arguments, "exclusions");
    runtime.manual_suggestions = string_array(arguments, "manualDeepDiveSuggestions");
    runtime.coverage_claims = ledger_rows.clone();
    runtime.finished = Some(serde_json::json!({
        "coveredFamilies": covered,
        "insufficientEvidenceFamilies": insufficient,
        "notApplicableFamilies": not_applicable,
        "notCoveredFamilies": not_covered,
        "coverage": ledger_rows,
        "stopReason": value_first(arguments, &["stopReason"]),
    }));
    serde_json::json!({
        "accepted": true,
        "coveredFamilies": covered,
        "insufficientEvidenceFamilies": insufficient.iter().filter_map(|row| row.get("family")).cloned().collect::<Vec<_>>(),
        "notApplicableFamilies": not_applicable.iter().filter_map(|row| row.get("family")).cloned().collect::<Vec<_>>(),
        "notCoveredFamilies": not_covered.iter().filter_map(|row| row.get("family")).cloned().collect::<Vec<_>>(),
        "coverage": ledger_rows,
        "unsupportedCoverageClaims": unsupported,
        "confirmedFindings": runtime.confirmed_findings,
        "targetUrl": context.target_url,
    })
}

/// Plain language for a gap the ledger can prove. Never "no risk found" (§9.5).
fn agent_coverage_gap_text(family: &str, reason_code: &str) -> String {
    let cause = match reason_code {
        "anonymous_only" => "只有匿名请求，缺少认证身份的对照",
        "no_identity_contrast" => "只有一个身份，缺少 A/B 对照",
        "login_page_only" => "只访问了登录入口，未验证已登录会话",
        "no_reflection_point" => "没有观察到输入在响应中的反射链路",
        "discovery_round_closed" => "本轮目录发现没有新增端点，仅关闭该轮",
        "no_response_structure" => "请求已发出，但未取得可判读的结构化响应",
        "no_executed_request" => "本轮没有该族的已执行请求",
        other => other,
    };
    format!("{cause}（{family} 证据不足，不能记为已覆盖）")
}

fn string_array(arguments: &JsonValue, key: &str) -> Vec<String> {
    arguments
        .get(key)
        .and_then(JsonValue::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn agent_tool_error(message: &str, code: &str) -> JsonValue {
    serde_json::json!({"error": message, "code": code})
}
