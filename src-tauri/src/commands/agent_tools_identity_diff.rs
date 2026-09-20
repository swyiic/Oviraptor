fn agent_tool_compare_identities(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    arguments: &JsonValue,
) -> JsonValue {
    if context.identities.len() < 2 {
        return serde_json::json!({
            "status": "insufficient_evidence",
            "reason": "当前任务只有一个身份或为匿名任务，无法做身份差异比较",
            "identityCount": context.identities.len(),
        });
    }
    let left = value_first(arguments, &["leftIdentity"]);
    let right = value_first(arguments, &["rightIdentity"]);
    if left == right {
        return serde_json::json!({"error": "两侧身份不能相同", "code": "same_identity"});
    }
    let method = value_first(arguments, &["method"]).to_ascii_uppercase();
    let path = value_first(arguments, &["path"]);
    let Ok(base) = reqwest::Url::parse(&context.target_url) else {
        return serde_json::json!({"error": "目标 URL 无法解析", "code": "invalid_url"});
    };
    let url = if path.starts_with("http") {
        path.clone()
    } else {
        let normalized_path = if path.starts_with('/') { path.clone() } else { format!("/{path}") };
        let mut joined = base.clone();
        joined.set_path(&normalized_path);
        joined.to_string()
    };
    let family = value_first(arguments, &["family"]);
    let contract_key = value_first(arguments, &["contractKey"]);
    // Resolve both handles before touching the target: a comparison that cannot
    // run must not spend one side's request on the way out (§8).
    for identity in [left.as_str(), right.as_str()] {
        if agent_identity_of(context, identity).is_none() {
            return serde_json::json!({
                "status": "insufficient_evidence",
                "code": "identity_not_found",
                "reason": format!("{identity} 不是当前任务的身份句柄"),
            });
        }
    }
    let mut sides = Vec::new();
    for identity in [left.as_str(), right.as_str()] {
        match agent_http_request(
            context,
            runtime,
            identity,
            &method,
            &url,
            Vec::new(),
            None,
            None,
            &contract_key,
            &family,
            ScopeSource::IdentityComparison,
            "compare_identities",
        ) {
            Ok(response) => sides.push((identity.to_string(), response)),
            Err(error) => {
                if value_first(&error, &["code"]) == "protected" {
                    return error;
                }
                return serde_json::json!({
                    "status": "insufficient_evidence",
                    "reason": format!("{identity} 一侧未采集成功：{}", value_first(&error, &["error"])),
                    "code": value_first(&error, &["code"]),
                });
            }
        }
    }
    let (left_key, left_response) = sides[0].clone();
    let (right_key, right_response) = sides[1].clone();
    let left_status = left_response.get("status").and_then(JsonValue::as_i64).unwrap_or(0);
    let right_status = right_response.get("status").and_then(JsonValue::as_i64).unwrap_or(0);
    // §8: an A/B claim is only worth making about two complete, comparable
    // records this chain actually executed.
    let pair = match agent_ab_pair_evidence(runtime, &left_response, &right_response) {
        Ok(pair) => pair,
        Err(reason) => {
            return serde_json::json!({
                "status": "insufficient_evidence",
                "code": "incomplete_ab_evidence",
                "reason": reason,
                "endpoint": format!("{}|{}", method, normalized_investigation_path(&path)),
            })
        }
    };
    let (left_trace, right_trace) = (pair.0.clone(), pair.1.clone());
    let field_diff = agent_response_field_difference(&left_response, &right_response);
    let signature = format!("{}|{}", method, normalized_investigation_path(&path));
    let differs = agent_difference_is_material(&field_diff, left_status, right_status)
        && sides.iter().all(|(_, side)| agent_redirect_credits_coverage(side));
    if differs && runtime.record_comparison(&signature) {
        runtime.last_progress.new_identity_differences += 1;
    }
    if agent_redirect_credits_coverage(&sides[0].1)
        && agent_redirect_credits_coverage(&sides[1].1)
        && runtime.credit_family(&family)
    {
        runtime.last_progress.new_families += 1;
    }
    if agent_redirect_credits_coverage(&sides[0].1) && agent_redirect_credits_coverage(&sides[1].1) {
        let (result, reason_code) = if differs {
            ("covered", "ab_contrast_observed")
        } else {
            ("partial", "no_material_difference")
        };
        for trace in [&left_trace, &right_trace] {
            runtime.note_coverage(&family, "identity_pair", &contract_key, &trace.id, result, reason_code);
        }
    }
    let difference_artifact = match agent_write_diff_record(
        context,
        runtime.target_requests,
        &serde_json::json!({
            "endpoint": signature,
            "method": method,
            "path": normalized_investigation_path(&path),
            "contractKey": contract_key,
            "left": {
                "requestId": left_trace.id,
                "identity": left_trace.identity,
                "artifactId": left_trace.artifact_id,
                "structureHash": left_trace.structure_hash,
                "status": left_status,
                "parameters": left_trace.parameters,
            },
            "right": {
                "requestId": right_trace.id,
                "identity": right_trace.identity,
                "artifactId": right_trace.artifact_id,
                "structureHash": right_trace.structure_hash,
                "status": right_status,
                "parameters": right_trace.parameters,
            },
            "materialDifference": differs,
            "fieldDifference": field_diff,
        }),
    ) {
        Ok(name) => name,
        Err(error) => {
            return serde_json::json!({"error": error, "code": "evidence_write_failed"})
        }
    };
    runtime.note_artifact(&difference_artifact);
    let authorization_rows = field_diff
        .get("materialAuthorizationDiffs")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let result = serde_json::json!({
        "endpoint": signature,
        "left": {
            "identity": agent_identity_label(&context.identities, &left_key),
            "label": left_key,
            "status": left_status,
            "sha256": value_first(&left_response, &["bodySha256"]),
            "bytes": left_response.get("bytes"),
            "requestId": left_trace.id,
            "artifactId": left_trace.artifact_id,
            "structureHash": left_trace.structure_hash,
        },
        "right": {
            "identity": agent_identity_label(&context.identities, &right_key),
            "label": right_key,
            "status": right_status,
            "sha256": value_first(&right_response, &["bodySha256"]),
            "bytes": right_response.get("bytes"),
            "requestId": right_trace.id,
            "artifactId": right_trace.artifact_id,
            "structureHash": right_trace.structure_hash,
        },
        "statusDiffers": left_status != right_status,
        "materialDifference": differs,
        "materialAuthorizationDiffs": authorization_rows,
        "subjectDiffs": field_diff.get("subjectDiffs").cloned().unwrap_or_else(|| JsonValue::Array(Vec::new())),
        "fieldDifference": field_diff,
        "responseDifferenceArtifactId": difference_artifact,
        "ownershipHints": value_first(arguments, &["ownershipHints"]),
        "verdictHint": if differs { "compare_then_record_hypothesis" } else { "no_difference" },
        "note": "差异本身不是漏洞；需要 record_hypothesis_result 引用两侧 request id 才能确认",
    });
    if let Err(error) = persist_agent_evidence(context, &result, "compare_identities") {
        return serde_json::json!({"error": error, "code": "evidence_write_failed"});
    }
    result
}

/// §8: the hard conditions behind a usable A/B pair — two distinct executed
/// requests of this chain, same method, host, path and business parameters, two
/// different task-level identities, and a recorded artifact and structure hash on
/// each side.
fn agent_ab_pair_evidence(
    runtime: &AgentToolRuntime,
    left: &JsonValue,
    right: &JsonValue,
) -> Result<(AgentRequestTrace, AgentRequestTrace), String> {
    let trace_of = |side: &JsonValue| runtime.request(&value_first(side, &["requestId"]));
    let (Some(left_trace), Some(right_trace)) = (trace_of(left), trace_of(right)) else {
        return Err("两侧缺少本执行链已发出的完整请求记录".to_string());
    };
    if left_trace.id == right_trace.id {
        return Err("两侧是同一条请求记录，不构成 A/B".to_string());
    }
    for (label, trace) in [("左", left_trace), ("右", right_trace)] {
        if trace.artifact_id.is_empty() || trace.structure_hash.is_empty() || trace.status <= 0 {
            return Err(format!("{label} 一侧缺少响应 status、artifact 或结构指纹"));
        }
    }
    if left_trace.identity.is_empty() || left_trace.identity == right_trace.identity {
        return Err("两侧必须是两个不同的任务级身份句柄".to_string());
    }
    if left_trace.method != right_trace.method
        || left_trace.origin != right_trace.origin
        || left_trace.path != right_trace.path
    {
        return Err("两侧的 method 或规范化 host+path 不一致".to_string());
    }
    if agent_query_parameter_names(&value_first(left, &["url"]))
        != agent_query_parameter_names(&value_first(right, &["url"]))
    {
        return Err("两侧的业务参数集合不一致，比较的不是同一个请求".to_string());
    }
    if left_trace.artifact_id == right_trace.artifact_id {
        return Err("两侧指向同一个响应 artifact".to_string());
    }
    Ok((left_trace.clone(), right_trace.clone()))
}

/// The business parameter *names* of a URL, sorted; values never enter the
/// comparison (§8).
fn agent_query_parameter_names(url: &str) -> Vec<String> {
    reqwest::Url::parse(url)
        .map(|parsed| {
            let mut names: Vec<String> =
                parsed.query_pairs().map(|(key, _)| key.to_string()).collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

/// The six §8 classes a diffed response field can belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentFieldClass {
    /// Rotating per-request noise: request ids, timestamps, nonces.
    Volatile,
    /// A credential or personal value; never compared for materiality.
    Secret,
    /// Identifies whose object this is (userId, accountId, ...).
    Subject,
    /// Carries the authorization decision (role, permissionId, tenantId, ...).
    Authorization,
    /// Ordinary business content with a readable value class.
    Business,
    /// Structure we cannot read a decision out of (nested object, opaque blob).
    Unknown,
}

impl AgentFieldClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Volatile => "volatile",
            Self::Secret => "secret",
            Self::Subject => "subject",
            Self::Authorization => "authorization",
            Self::Business => "business",
            Self::Unknown => "unknown",
        }
    }
}

/// Trailing name of a flattened path: `items[].role`, `permissions[]` and
/// `data.role` all end at the field name the class rules are written against.
fn agent_field_leaf(path: &str) -> String {
    let is_name = |character: char| character.is_ascii_alphanumeric() || character == '_';
    let trimmed = path.trim_end_matches(|character: char| !is_name(character));
    trimmed
        .chars()
        .rev()
        .take_while(|character| is_name(*character))
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}
/// What an executed chain may claim about one coverage family (§9.3). The rules
/// are deliberately stricter than "a request carrying this family tag happened":
/// a single anonymous call cannot prove authorization, a login page cannot prove
/// a session, and an unrecovered input cannot prove reflection.
fn agent_family_sufficiency(runtime: &AgentToolRuntime, family: &str) -> (&'static str, &'static str) {
    let requests: Vec<&AgentRequestTrace> = runtime
        .requests
        .iter()
        .filter(|trace| trace.family == family)
        .collect();
    let identities: HashSet<&str> = requests.iter().map(|trace| trace.identity.as_str()).collect();
    let saw_anonymous = identities.contains(AGENT_ANONYMOUS_IDENTITY);
    let saw_authenticated = identities.iter().any(|id| *id != AGENT_ANONYMOUS_IDENTITY);
    match family {
        "authorization" => {
            // A contrast is what makes an authorization claim: two identities, or
            // at least an anonymous and an authenticated view of the same call.
            if identities.len() >= 2 || (saw_anonymous && saw_authenticated) {
                ("covered", "identity_contrast_observed")
            } else if saw_anonymous {
                ("partial", "anonymous_only")
            } else {
                ("partial", "no_identity_contrast")
            }
        }
        "authentication_session" => {
            let served = requests.iter().any(|trace| {
                !agent_is_login_page_path(&trace.path)
                    && (200..400).contains(&trace.status)
                    && trace.identity != AGENT_ANONYMOUS_IDENTITY
            });
            if served {
                ("covered", "authenticated_response_observed")
            } else {
                ("partial", "login_page_only")
            }
        }
        "input_reflection_xss" => {
            if requests
                .iter()
                .any(|trace| runtime.reflections.contains(&trace.id))
            {
                ("covered", "reflection_point_observed")
            } else {
                ("partial", "no_reflection_point")
            }
        }
        "hidden_interface_discovery" => {
            let verified = requests.iter().any(|trace| {
                trace.tool == "targeted_discovery" && (200..400).contains(&trace.status)
            });
            if verified {
                ("covered", "discovery_verified")
            } else if runtime.discovery_rounds > 0 {
                ("partial", "discovery_round_closed")
            } else {
                ("partial", "no_executed_request")
            }
        }
        _ => {
            if requests
                .iter()
                .any(|trace| (200..500).contains(&trace.status) && !trace.structure_hash.is_empty())
            {
                ("covered", "response_observed")
            } else if requests.is_empty() {
                ("partial", "no_executed_request")
            } else {
                ("partial", "no_response_structure")
            }
        }
    }
}

fn agent_is_login_page_path(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    AGENT_LOGIN_SEGMENTS
        .iter()
        .any(|segment| lowered.split(['/', '.']).any(|part| part == *segment))
}

/// A request whose response echoed one of its own parameter values: the input →
/// response chain an XSS claim has to have (§9.3).
fn agent_reflects_input(
    request: &AgentHttpRequest,
    fields: Option<&serde_json::Map<String, JsonValue>>,
) -> bool {
    let Some(fields) = fields else { return false };
    let fingerprints: Vec<&str> = fields.values().filter_map(JsonValue::as_str).collect();
    let mut values: Vec<String> = reqwest::Url::parse(&request.url)
        .map(|parsed| {
            parsed
                .query_pairs()
                .map(|(_, value)| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(body) = request.body.as_deref() {
        values.extend(body.split('&').filter_map(|pair| pair.split_once('=').map(|(_, v)| v.to_string())));
    }
    values.iter().any(|value| {
        let probe = value.trim().to_ascii_lowercase();
        probe.chars().count() >= 3
            && fingerprints
                .iter()
                .any(|row| row == &format!("text:{probe}"))
    })
}

fn agent_field_class(path: &str, fingerprint: &str) -> AgentFieldClass {
    let leaf = agent_field_leaf(path);
    let normalized = leaf.to_ascii_lowercase();
    let compact = normalized.replace(['_', '-'], "");
    if AGENT_VOLATILE_FIELDS.contains(&normalized.as_str())
        || AGENT_VOLATILE_FIELDS.contains(&compact.as_str())
        || normalized.ends_with("_at")
        || normalized.ends_with("time")
        || normalized.ends_with("stamp")
        || agent_looks_like_timestamp(fingerprint)
    {
        return AgentFieldClass::Volatile;
    }
    if crate::agent_runtime::secrets::is_secret_field(&leaf)
        || is_agent_token_field(&normalized)
        || is_agent_token_field(&compact)
    {
        return AgentFieldClass::Secret;
    }
    if AGENT_AUTHORIZATION_FIELDS.contains(&normalized.as_str())
        || AGENT_AUTHORIZATION_FIELDS.contains(&compact.as_str())
    {
        return AgentFieldClass::Authorization;
    }
    if AGENT_SUBJECT_FIELDS.contains(&normalized.as_str())
        || AGENT_SUBJECT_FIELDS.contains(&compact.as_str())
    {
        return AgentFieldClass::Subject;
    }
    // A readable short literal is business content; a blob, an array of objects or
    // an opaque token tells us nothing about who is allowed to do what.
    if ["num:", "bool:", "text:", "empty"]
        .iter()
        .any(|prefix| fingerprint.starts_with(prefix))
    {
        AgentFieldClass::Business
    } else {
        AgentFieldClass::Unknown
    }
}

/// Compare normalized field fingerprints instead of raw bytes or top-level key
/// lists: volatile ids, re-issued credentials and per-account objects are split
/// away from the differences that can carry a security verdict (§8).
fn agent_response_field_difference(left: &JsonValue, right: &JsonValue) -> JsonValue {
    let fingerprints = |response: &JsonValue| -> std::collections::BTreeMap<String, String> {
        response
            .get("fields")
            .and_then(JsonValue::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|text| (key.clone(), text.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let left_fields = fingerprints(left);
    let right_fields = fingerprints(right);
    if left_fields.is_empty() && right_fields.is_empty() {
        return JsonValue::Null;
    }
    let mut only_on_left: Vec<JsonValue> = Vec::new();
    let mut only_on_right: Vec<JsonValue> = Vec::new();
    let mut value_differs: Vec<JsonValue> = Vec::new();
    let mut authorization: Vec<JsonValue> = Vec::new();
    let mut subject: Vec<JsonValue> = Vec::new();
    let mut volatile = 0usize;
    let mut secrets = 0usize;
    let mut file = |path: &str, left: &str, right: &str, class: AgentFieldClass| {
        let row = serde_json::json!({"path": path, "class": class.as_str(), "left": left, "right": right});
        match class {
            AgentFieldClass::Volatile => volatile += 1,
            AgentFieldClass::Secret => secrets += 1,
            AgentFieldClass::Subject => subject.push(row),
            AgentFieldClass::Authorization => authorization.push(row),
            AgentFieldClass::Business | AgentFieldClass::Unknown => {
                if left == ABSENT_FIELD {
                    only_on_right.push(row);
                } else if right == ABSENT_FIELD {
                    only_on_left.push(row);
                } else {
                    value_differs.push(row);
                }
            }
        }
    };
    for (key, left_value) in &left_fields {
        let class = agent_field_class(key, left_value);
        match right_fields.get(key) {
            Some(right_value) if left_value != right_value => {
                file(key, left_value, right_value, class)
            }
            Some(_) => {}
            None => file(key, left_value, ABSENT_FIELD, class),
        }
    }
    for (key, right_value) in &right_fields {
        if !left_fields.contains_key(key) {
            file(key, ABSENT_FIELD, right_value, agent_field_class(key, right_value));
        }
    }
    let capped = |rows: Vec<JsonValue>, limit: usize| -> Vec<JsonValue> {
        rows.into_iter().take(limit).collect()
    };
    serde_json::json!({
        "onlyOnLeft": capped(only_on_left, 24),
        "onlyOnRight": capped(only_on_right, 24),
        "valueDiffers": capped(value_differs, 24),
        "materialAuthorizationDiffs": capped(authorization, 24),
        "subjectDiffs": capped(subject, 12),
        "ignoredVolatile": volatile,
        "ignoredSecrets": secrets,
    })
}

const ABSENT_FIELD: &str = "absent";

/// Only a status split, a field one identity cannot see, a business value change
/// or an authorization field change is material. Per-account object ids and
/// re-issued credentials are expected between two identities and never qualify.
fn agent_difference_is_material(diff: &JsonValue, left_status: i64, right_status: i64) -> bool {
    if left_status != right_status {
        return true;
    }
    if diff.is_null() {
        return false;
    }
    [
        "onlyOnLeft",
        "onlyOnRight",
        "valueDiffers",
        "materialAuthorizationDiffs",
    ]
    .iter()
    .any(|key| {
        diff.get(*key)
            .and_then(JsonValue::as_array)
            .map(|rows| !rows.is_empty())
            .unwrap_or(false)
    })
}

/// `2026-09-19T00:00:00` and friends are clock noise whatever the field is
/// called (§8).
fn agent_looks_like_timestamp(fingerprint: &str) -> bool {
    let Some(value) = fingerprint.strip_prefix("text:") else {
        return false;
    };
    let bytes = value.as_bytes();
    bytes.len() >= 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

fn is_agent_token_field(normalized: &str) -> bool {
    AGENT_TOKEN_FIELDS.contains(&normalized)
}

/// Flattened `path -> fingerprint` map of a JSON body. Values are reduced to a
/// class plus, for short literals, the value itself: business codes and roles
/// stay comparable, while opaque strings and JWTs collapse to a class so a
/// re-issued token is not mistaken for a policy change.
fn agent_field_fingerprints(
    text: &str,
    content_type: &str,
    redaction: &crate::agent_runtime::secrets::RedactionContext,
) -> JsonValue {
    if !(content_type.contains("json")
        || text.trim_start().starts_with('{')
        || text.trim_start().starts_with('['))
    {
        return JsonValue::Null;
    }
    let Ok(parsed) = serde_json::from_str::<JsonValue>(text) else {
        return JsonValue::Null;
    };
    let mut fields = serde_json::Map::new();
    agent_flatten_value("", &parsed, 0, redaction, &mut fields);
    if fields.is_empty() {
        return JsonValue::Null;
    }
    JsonValue::Object(fields)
}

fn agent_flatten_value(
    prefix: &str,
    value: &JsonValue,
    depth: usize,
    redaction: &crate::agent_runtime::secrets::RedactionContext,
    fields: &mut serde_json::Map<String, JsonValue>,
) {
    if fields.len() >= 120 {
        return;
    }
    match value {
        JsonValue::Object(map) if depth < 4 => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                agent_flatten_value(&path, child, depth + 1, redaction, fields);
            }
        }
        JsonValue::Array(rows) if depth < 4 => {
            if rows.is_empty() {
                fields.insert(prefix.to_string(), JsonValue::String("array(0)".to_string()));
                return;
            }
            let mut merged: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
                Default::default();
            for row in rows.iter().take(5) {
                let mut inner = serde_json::Map::new();
                agent_flatten_value("", row, depth + 1, redaction, &mut inner);
                for (key, fingerprint) in inner {
                    let text = fingerprint.as_str().unwrap_or_default().to_string();
                    merged.entry(key).or_default().insert(text);
                }
            }
            for (key, variants) in merged {
                let path = if prefix.is_empty() {
                    format!("[]{key}")
                } else {
                    format!("{prefix}[].{key}")
                };
                let joined = variants.into_iter().collect::<Vec<_>>().join(",");
                fields.insert(path, JsonValue::String(format!("array<{}>", joined)));
            }
            fields.insert(
                if prefix.is_empty() {
                    "[]".to_string()
                } else {
                    format!("{prefix}[]")
                },
                JsonValue::String(format!("array({})", rows.len())),
            );
        }
        other => {
            if prefix.is_empty() {
                fields.insert(
                    "$".to_string(),
                    JsonValue::String(agent_value_fingerprint(other, redaction)),
                );
            } else {
                fields.insert(
                    prefix.to_string(),
                    JsonValue::String(agent_value_fingerprint(other, redaction)),
                );
            }
        }
    }
}

fn agent_value_fingerprint(
    value: &JsonValue,
    redaction: &crate::agent_runtime::secrets::RedactionContext,
) -> String {
    match value {
        JsonValue::Null => "null".to_string(),
        JsonValue::Bool(flag) => format!("bool:{flag}"),
        JsonValue::Number(number) => format!("num:{number}"),
        JsonValue::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return "empty".to_string();
            }
            use crate::agent_runtime::secrets::{looks_like_api_key, looks_like_bearer, pii_kind};
            // A credential or personal value never leaves this stage as text, but
            // it keeps a run-stable fingerprint so A/B can still compare (§6.2).
            if let Some(kind) = pii_kind(trimmed) {
                return redaction.marker(kind, trimmed);
            }
            if trimmed.matches('.').count() == 2 && trimmed.starts_with("ey") {
                return redaction.marker("assertion", trimmed);
            }
            if looks_like_bearer(trimmed) {
                return redaction.marker("bearer", trimmed);
            }
            if looks_like_api_key(trimmed) {
                return redaction.marker("opaque", trimmed);
            }
            let lowered = trimmed.to_ascii_lowercase();
            if lowered.len() >= 24
                && lowered.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            {
                return "opaque".to_string();
            }
            if trimmed.len() <= 48 {
                return format!("text:{lowered}");
            }
            "prose".to_string()
        }
        JsonValue::Array(_) => "array".to_string(),
        JsonValue::Object(_) => "object".to_string(),
    }
}
