// §Q3 — the passive web walker, executed as one work item per authorized origin. It is the
// only place in this slice that opens a socket at a target, and every send goes through the
// Q2 client, which authorizes the hop, charges the budget and redacts the evidence first.

use super::super::observations::NewObservation;
use super::super::orchestrator::ItemOutcome;
use super::super::redaction::{content_hash, redact_json};
use super::super::rules::{
    findings_for, natural_links, natural_links_bounded, resource_kind, KIND_TRANSPORT,
};
use super::super::scope::{ExposureDenial, ExposureGrant, ExposureScope};
use super::super::state::{utc_now, SourceStatus, WorkItemStatus};
use super::super::work_items::WorkItem;
use super::http::{FetchBlock, PassiveClient, PassiveFetch, RequestMeter};
use serde_json::json;
use std::collections::HashSet;

#[path = "walk_checkpoint.rs"]
mod walk_checkpoint;
use walk_checkpoint::{WalkCheckpoint, MAX_TRACKED_URLS};

#[cfg(test)]
#[path = "walk_checkpoint_tests.rs"]
mod walk_checkpoint_tests;

/// Per-origin ceilings. They are named gaps instead of a silent stop, and an attempt costs
/// even when the response is an error or a transport failure.
#[derive(Clone, Copy, Debug)]
pub struct WalkBudget {
    pub max_attempts: usize,
    pub max_queued: usize,
}

impl Default for WalkBudget {
    fn default() -> Self {
        Self {
            max_attempts: 20,
            max_queued: 80,
        }
    }
}

fn seed_urls(base: &reqwest::Url) -> Vec<reqwest::Url> {
    [
        "/",
        "/robots.txt",
        "/sitemap.xml",
        "/.well-known/security.txt",
    ]
    .iter()
    .filter_map(|path| base.join(path).ok())
    .collect()
}

/// The grant origin of a URL, or `None` when it is not an http(s) origin at all.
fn origin_of(candidate: &reqwest::Url) -> Option<String> {
    ExposureGrant::from_url(candidate).map(|grant| grant.origin)
}

fn inside_origin(origin: &str, candidate: &reqwest::Url) -> bool {
    origin_of(candidate).as_deref() == Some(origin)
}

fn observation_for(fetch: &PassiveFetch, asset_id: Option<i64>, item: &WorkItem) -> NewObservation {
    NewObservation {
        source_key: item.source_key.clone(),
        kind: resource_kind(&fetch.final_url, &fetch.content_type).to_string(),
        url: fetch.requested_url.clone(),
        final_url: fetch.final_url.clone(),
        status_code: i64::from(fetch.status),
        content_type: fetch.content_type.clone(),
        body_bytes: fetch.body.len() as i64,
        truncated: fetch.truncated,
        boundary: fetch.boundary.clone(),
        body: fetch.body.clone(),
        headers: redact_json(&json!({
            "contentType": fetch.content_type,
            "contentHash": content_hash(&fetch.body),
            "headersHash": fetch.headers_hash,
        })),
        asset_id,
        current_accessible: Some(fetch.status < 400),
        indexed_at: String::new(),
        method: "GET".to_string(),
        extra: json!({
            "origin": item.origin,
            "workKey": item.work_key,
            "hops": fetch.hops,
            "requestsCharged": fetch.requests_charged,
            "elapsedMs": fetch.elapsed_ms,
            "observedAt": fetch.observed_at,
            "tlsValidation": fetch.tls_validation,
            "httpDowngrade": fetch.http_downgrade,
        }),
    }
}

fn remember_transport_warnings(reason: &mut String, fetch: &PassiveFetch) {
    let had_http_warning = reason.contains("same_host_http");
    if let Some(tls) = &fetch.tls_validation {
        *reason = format!(
            "tls_certificate_exception：已继续采集公开响应，TLS 身份未经验证；{}",
            tls.diagnostic
        );
    }
    if (fetch.http_downgrade.is_some() || had_http_warning) && !reason.contains("same_host_http") {
        if !reason.is_empty() {
            reason.push_str("；");
        }
        reason.push_str("same_host_http：已跟随同域 HTTP 公开跳转；传输未加密，响应身份未经验证");
    }
}

fn reference_observation(
    item: &WorkItem,
    url: &str,
    kind: &str,
    detail: serde_json::Value,
) -> NewObservation {
    NewObservation {
        source_key: item.source_key.clone(),
        kind: kind.to_string(),
        url: url.to_string(),
        asset_id: item.asset_id,
        extra: detail,
        ..NewObservation::new(&item.source_key, kind, url)
    }
}

/// One origin: breadth-first over natural links that stay inside the authorized origin, under
/// both the run-wide request/byte budget and this origin's attempt ceiling.
pub fn execute(
    client: &PassiveClient,
    scope: &ExposureScope,
    item: &WorkItem,
    meter: &mut RequestMeter,
    budget: &WalkBudget,
    revalidate: &dyn Fn() -> Result<(), ExposureDenial>,
    cancelled: &dyn Fn() -> bool,
) -> ItemOutcome {
    let mut outcome = ItemOutcome::default();
    let base = match reqwest::Url::parse(&format!("{}/", item.origin)) {
        Ok(value) => value,
        Err(error) => {
            outcome.status = Some(WorkItemStatus::Failed);
            outcome.source_status = Some(SourceStatus::Failed);
            outcome.reason = format!("origin 无法解析：{error}");
            return outcome;
        }
    };
    if let Err(denial) = revalidate() {
        outcome.status = Some(WorkItemStatus::BlockedAuthorization);
        outcome.reason = format!("{}:{}", denial.code, denial.message);
        return outcome;
    }
    if let Err(denial) = scope.authorize(&base, &utc_now()) {
        outcome.status = Some(WorkItemStatus::BlockedAuthorization);
        outcome.reason = format!("{}:{}", denial.code, denial.message);
        return outcome;
    }

    let mut queue: Vec<reqwest::Url> = seed_urls(&base);
    let mut visited: HashSet<String> = HashSet::new();
    let mut attempts = 0usize;
    let mut successes = 0usize;
    let mut stopped_by_budget = false;
    let mut outside_references = 0usize;

    while let Some(url) = queue.first().cloned() {
        queue.remove(0);
        if !visited.insert(url.as_str().to_string()) {
            continue;
        }
        if cancelled() {
            outcome.status = Some(WorkItemStatus::Cancelled);
            outcome.source_status = Some(SourceStatus::Cancelled);
            outcome.reason = "用户取消；已保存的观察保留".to_string();
            return outcome;
        }
        if !inside_origin(&item.origin, &url) {
            // A discovered link outside the origin is recorded as a reference, never fetched.
            outside_references += 1;
            outcome.observations.push(reference_observation(
                item,
                url.as_str(),
                "reference_out_of_scope",
                json!({"reason": "origin 未在授权范围内，只记录外链"}),
            ));
            continue;
        }
        if attempts >= budget.max_attempts {
            stopped_by_budget = true;
            break;
        }
        // The certificate exception is a second network attempt, not a free retry.
        // Bound both sends by this origin's ceiling as well as the run-wide meter.
        let before = meter.requests_used;
        let mut limited = meter.clone();
        limited.max_requests = limited
            .max_requests
            .min(before.saturating_add((budget.max_attempts - attempts) as i64));
        let fetched = client.fetch(scope, &url, &mut limited, revalidate, cancelled);
        meter.requests_used = limited.requests_used;
        meter.bytes_used = limited.bytes_used;
        attempts += limited.requests_used.saturating_sub(before) as usize;
        match fetched {
            Ok(fetch) => {
                successes += 1;
                remember_transport_warnings(&mut outcome.source_reason, &fetch);
                outcome
                    .observations
                    .push(observation_for(&fetch, item.asset_id, item));
                outcome.findings.extend(findings_for(
                    &fetch.final_url,
                    &fetch.content_type,
                    &fetch.body,
                    item.asset_id,
                ));
                let followable = fetch.content_type.contains("html")
                    || fetch.content_type.is_empty()
                    || fetch.content_type.contains("text/plain");
                if followable && fetch.status < 400 && !fetch.truncated {
                    let text = String::from_utf8_lossy(&fetch.body).to_string();
                    for link in natural_links(&text, &url) {
                        let Ok(parsed) = reqwest::Url::parse(&link) else {
                            continue;
                        };
                        if queue.len() + visited.len() >= budget.max_queued {
                            stopped_by_budget = true;
                            break;
                        }
                        if !visited.contains(&link) {
                            queue.push(parsed);
                        }
                    }
                }
            }
            Err(block) => match block {
                FetchBlock::Cancelled => {
                    outcome.status = Some(WorkItemStatus::Cancelled);
                    outcome.source_status = Some(SourceStatus::Cancelled);
                    outcome.reason = "用户取消；已保存的观察保留".to_string();
                    outcome.requests = meter.requests_used;
                    outcome.bytes = meter.bytes_used;
                    return outcome;
                }
                FetchBlock::Denied(denial) => {
                    outcome.requests = meter.requests_used;
                    outcome.bytes = meter.bytes_used;
                    if matches!(
                        denial.code,
                        "request_budget_exhausted" | "byte_budget_exhausted"
                    ) {
                        stopped_by_budget = true;
                        outcome.reason = format!("配额：{}", denial.message);
                        break;
                    }
                    if matches!(
                        denial.code,
                        "authorization_revoked"
                            | "scope_expired"
                            | "host_not_granted"
                            | "project_inactive"
                            | "userinfo_denied"
                            | "scheme_denied"
                            | "private_destination_denied"
                    ) {
                        outcome.status = Some(WorkItemStatus::BlockedAuthorization);
                        outcome.source_status = Some(SourceStatus::Partial);
                        outcome.source_reason = denial.code.to_string();
                        outcome.reason = format!("{}:{}", denial.code, denial.message);
                        return outcome;
                    }
                    outcome.observations.push(reference_observation(
                        item,
                        url.as_str(),
                        KIND_TRANSPORT,
                        denial.as_json(),
                    ));
                }
                FetchBlock::RedirectStopped {
                    from,
                    location,
                    denial,
                    ..
                } => {
                    outcome.observations.push(reference_observation(
                        item,
                        &from,
                        "redirect_stopped",
                        json!({"target": location, "denial": denial.as_json()}),
                    ));
                    outcome.reason =
                        format!("跳出授权 origin 已停止：{}:{}", denial.code, denial.message);
                }
                FetchBlock::Transport { message, .. } => {
                    outcome.observations.push(reference_observation(
                        item,
                        url.as_str(),
                        KIND_TRANSPORT,
                        json!({"error": message}),
                    ));
                    if attempts == 1 {
                        outcome.source_status = Some(SourceStatus::Failed);
                        outcome.status = Some(WorkItemStatus::Failed);
                        outcome.reason = format!("首页不可达：{message}");
                        outcome.requests = meter.requests_used;
                        outcome.bytes = meter.bytes_used;
                        return outcome;
                    }
                }
            },
        }
        outcome.requests = meter.requests_used;
        outcome.bytes = meter.bytes_used;
    }

    outcome.requests = meter.requests_used;
    outcome.bytes = meter.bytes_used;
    outcome.observed = outcome.observations.len() as i64;
    if stopped_by_budget {
        outcome.source_status = Some(SourceStatus::Partial);
        outcome.status = Some(WorkItemStatus::BlockedQuota);
        if outcome.reason.is_empty() {
            outcome.reason = format!(
                "达到本源 {0} 次尝试上限，仍有 {1} 个候选未读取",
                budget.max_attempts,
                queue.len()
            );
        }
    } else if successes == 0 {
        outcome.source_status = Some(SourceStatus::Failed);
        outcome.status = Some(WorkItemStatus::Failed);
        if outcome.reason.is_empty() {
            outcome.reason = "本源没有任何可读响应，只有传输失败".to_string();
        }
    } else {
        outcome.source_status = Some(if outcome.reason.is_empty() {
            SourceStatus::Completed
        } else {
            // A walk that stopped at an out-of-origin redirect did not cover the origin fully.
            SourceStatus::Partial
        });
        outcome.status = Some(WorkItemStatus::Completed);
        if !outcome.reason.is_empty() && outside_references > 0 {
            // A stopped redirect is a fact about coverage, not a completion defect.
            outcome.reason = format!("{}；外链引用 {outside_references} 个未跟随", outcome.reason);
        } else {
            outcome.reason = String::new();
        }
    }
    outcome
}

/// Read-only recovery projection. A damaged or foreign-origin cursor is not treated as
/// an empty frontier; in-flight URLs count once so recovery can reconcile them without send.
pub fn cursor_pending_count(raw: &str, origin: &str) -> Result<usize, String> {
    Ok(WalkCheckpoint::decode(raw, origin)?.pending())
}

/// Bounded progress metadata only; raw queued or unknown URLs stay in local storage.
pub fn cursor_progress_counts(raw: &str, origin: &str) -> Result<(usize, usize, i64), String> {
    let state = WalkCheckpoint::decode(raw, origin)?;
    Ok((
        state.unknown.len(),
        state.deferred_discovery_count,
        state.unknown_reserved_bytes,
    ))
}

pub fn validate_cursor_for(
    raw: &str,
    item: &WorkItem,
    scope: &ExposureScope,
) -> Result<usize, String> {
    let state = WalkCheckpoint::decode(raw, &item.origin)?;
    state.validate_binding(scope, item)?;
    Ok(state.pending())
}

fn checkpoint_outcome(
    mut outcome: ItemOutcome,
    state: &WalkCheckpoint,
    requests: i64,
    bytes: i64,
) -> Result<ItemOutcome, String> {
    outcome.requests = requests;
    outcome.bytes = bytes;
    outcome.observed = outcome.observations.len() as i64;
    outcome.cursor_json = Some(state.encode()?);
    outcome.source_reason = state.source_reason.clone();
    Ok(outcome)
}

fn finish_checkpoint(state: &WalkCheckpoint, mut outcome: ItemOutcome) -> ItemOutcome {
    let mut gaps = Vec::new();
    if !state.coverage_reason.is_empty() {
        gaps.push(state.coverage_reason.clone());
    }
    if state.queue_truncated {
        gaps.push(format!(
            "自然链接数量达到本轮队列安全上限；{} 条新发现引用延期，未计为已采集",
            state.deferred_discovery_count
        ));
    }
    if !state.unknown.is_empty() {
        gaps.push(format!(
            "{} 个在途请求结果未知，已保留记录且没有自动重放",
            state.unknown.len()
        ));
    }
    outcome.reason = gaps.join("；");
    if state.successes == 0 {
        outcome.status = Some(WorkItemStatus::Failed);
        outcome.source_status = Some(SourceStatus::Failed);
        if outcome.reason.is_empty() {
            outcome.reason = "本源没有任何可读响应，传输证据已保留".to_string();
        }
    } else {
        outcome.status = Some(WorkItemStatus::Completed);
        outcome.source_status = Some(if outcome.reason.is_empty() {
            SourceStatus::Completed
        } else {
            SourceStatus::Partial
        });
    }
    outcome
}

/// Executes at most one queued URL. Before the first socket, `checkpoint` must durably
/// store the in-flight URL guarded by the current claim. The caller then atomically stores
/// returned observations, the returned cursor, and the byte usage before requeuing the
/// item. `before_send` journals every actual strict/fallback/redirect network attempt.
/// This small boundary makes pause/restart resume the frontier rather than the origin.
#[allow(clippy::too_many_arguments)]
pub fn execute_checkpointed(
    client: &PassiveClient,
    scope: &ExposureScope,
    item: &WorkItem,
    meter: &mut RequestMeter,
    budget: &WalkBudget,
    revalidate: &dyn Fn() -> Result<(), ExposureDenial>,
    cancelled: &dyn Fn() -> bool,
    raw_cursor: &str,
    saved_urls: &[String],
    checkpoint: &dyn Fn(&str) -> Result<(), String>,
    before_send: &dyn Fn() -> Result<(), ExposureDenial>,
) -> Result<ItemOutcome, String> {
    let mut outcome = ItemOutcome::default();
    let base = reqwest::Url::parse(&format!("{}/", item.origin))
        .map_err(|error| format!("[exposure_walk_origin_invalid] {error}"))?;
    let mut state = if raw_cursor.is_empty() {
        if item.attempts > 1 || item.requests > 0 || item.observations > 0 || !saved_urls.is_empty()
        {
            return Err("[exposure_walk_legacy_checkpoint_missing] 旧工作项没有 URL 断点，不能证明哪些请求尚未发出；保留历史结果，请新建一轮采集".to_string());
        }
        WalkCheckpoint::new(
            scope,
            item,
            seed_urls(&base)
                .into_iter()
                .map(|url| url.to_string())
                .collect(),
        )
    } else {
        let state = WalkCheckpoint::decode(raw_cursor, &item.origin)?;
        state.validate_binding(scope, item)?;
        state
    };
    state.reconcile_saved(saved_urls);
    // Duplicate saved results are never fetched again even if the earlier terminal write
    // did not run. In-flight without a saved result is quarantined as unknown above.
    let before_requests = meter.requests_used;
    let before_bytes = meter.bytes_used;
    if cancelled() {
        outcome.status = Some(WorkItemStatus::Cancelled);
        outcome.source_status = Some(SourceStatus::Cancelled);
        outcome.reason = "已暂停；未发送 URL 的队列和已保存的证据保留".to_string();
        outcome.continuation = state.pending() > 0;
        return checkpoint_outcome(outcome, &state, 0, 0);
    }
    if let Err(denial) = revalidate().and_then(|_| scope.authorize(&base, &utc_now()).map(|_| ())) {
        outcome.status = Some(WorkItemStatus::BlockedAuthorization);
        outcome.source_status = Some(SourceStatus::Partial);
        outcome.reason = format!("{}:{}", denial.code, denial.message);
        return checkpoint_outcome(outcome, &state, 0, 0);
    }
    let Some(raw_url) = state.queue.first().cloned() else {
        return checkpoint_outcome(finish_checkpoint(&state, outcome), &state, 0, 0);
    };
    let url = reqwest::Url::parse(&raw_url)
        .map_err(|_| "[exposure_walk_cursor_invalid] 队列 URL 无法解析".to_string())?;
    if !scope.navigation_allowed(&base, &url) {
        state.queue.remove(0);
        state.visited.push(raw_url.clone());
        outcome.observations.push(reference_observation(
            item,
            &raw_url,
            "reference_out_of_scope",
            json!({"reason":"未在冻结导航范围内，只记录外链"}),
        ));
    } else {
        // A run-wide hard limit does not discard the queued URL or mark it visited.
        if meter.remaining_requests() == 0
            || meter.bytes_used >= meter.max_bytes
            || budget.max_attempts == 0
        {
            outcome.status = Some(WorkItemStatus::BlockedQuota);
            outcome.source_status = Some(SourceStatus::Partial);
            outcome.reason = "本轮总请求或字节预算已用尽；未发送 URL 的断点保留".to_string();
            return checkpoint_outcome(outcome, &state, 0, 0);
        }
        state.queue.remove(0);
        state.in_flight = Some(raw_url.clone());
        checkpoint(&state.encode()?)?;
        let mut limited = meter.clone();
        limited.max_requests = limited
            .max_requests
            .min(before_requests.saturating_add(budget.max_attempts as i64));
        let fetched = client.fetch_navigation_with_charge(
            scope,
            &base,
            &url,
            &mut limited,
            revalidate,
            cancelled,
            before_send,
        );
        meter.requests_used = limited.requests_used;
        meter.bytes_used = limited.bytes_used;
        let sent = meter.requests_used.saturating_sub(before_requests);
        match fetched {
            Ok(fetch) => {
                state.in_flight = None;
                state.visited.push(raw_url.clone());
                state.successes += 1;
                remember_transport_warnings(&mut state.source_reason, &fetch);
                outcome
                    .observations
                    .push(observation_for(&fetch, item.asset_id, item));
                outcome.findings.extend(findings_for(
                    &fetch.final_url,
                    &fetch.content_type,
                    &fetch.body,
                    item.asset_id,
                ));
                if fetch.status < 400
                    && !fetch.truncated
                    && (fetch.content_type.contains("html")
                        || fetch.content_type.is_empty()
                        || fetch.content_type.contains("text/plain"))
                {
                    let final_base =
                        reqwest::Url::parse(&fetch.final_url).unwrap_or_else(|_| url.clone());
                    let known: HashSet<String> = state
                        .queue
                        .iter()
                        .chain(&state.visited)
                        .chain(&state.unknown)
                        .cloned()
                        .collect();
                    let mut additions = HashSet::new();
                    let extracted = natural_links_bounded(
                        &String::from_utf8_lossy(&fetch.body),
                        &final_base,
                        MAX_TRACKED_URLS,
                    );
                    if extracted.truncated {
                        state.queue_truncated = true;
                        state.deferred_discovery_count = state
                            .deferred_discovery_count
                            .saturating_add(extracted.deferred_count);
                    }
                    for link in extracted.links {
                        let Ok(mut parsed) = reqwest::Url::parse(&link) else {
                            continue;
                        };
                        parsed.set_fragment(None);
                        if !parsed.username().is_empty() || parsed.password().is_some() {
                            continue;
                        }
                        let canonical = parsed.to_string();
                        if known.contains(&canonical) || !additions.insert(canonical.clone()) {
                            continue;
                        }
                        if state.queue.len() + state.visited.len() + state.unknown.len()
                            >= MAX_TRACKED_URLS
                        {
                            state.queue_truncated = true;
                            state.deferred_discovery_count =
                                state.deferred_discovery_count.saturating_add(1);
                            continue;
                        }
                        state.queue.push(canonical);
                    }
                }
            }
            Err(FetchBlock::Cancelled) => {
                state.in_flight = None;
                // The blocking client reports cancellation only between completed
                // transport steps, never while an HTTP send is unresolved. Resume this
                // known boundary; only crash-recovered inFlight becomes unknown.
                state.queue.insert(0, raw_url);
                outcome.status = Some(WorkItemStatus::Cancelled);
                outcome.source_status = Some(SourceStatus::Cancelled);
                outcome.reason =
                    "已暂停；当前公开 GET 在已知请求边界结束，URL 断点和已消耗次数保留".to_string();
                outcome.continuation = state.pending() > 0;
                return checkpoint_outcome(
                    outcome,
                    &state,
                    sent,
                    meter.bytes_used.saturating_sub(before_bytes),
                );
            }
            Err(FetchBlock::Denied(denial)) => {
                if matches!(
                    denial.code,
                    "exposure_claim_changed" | "request_journal_failed"
                ) {
                    // Keep the pre-send checkpoint authoritative. A lost owner must not
                    // persist a terminal state or overwrite its replacement's cursor.
                    return Err(format!("[{}] {}", denial.code, denial.message));
                }
                state.in_flight = None;
                if denial.code == "exposure_paused" {
                    // The send journal refused the next request before a socket was
                    // opened. Earlier TLS failures/redirect responses are already known.
                    state.queue.insert(0, raw_url);
                    outcome.status = Some(WorkItemStatus::Cancelled);
                    outcome.source_status = Some(SourceStatus::Cancelled);
                    outcome.reason = format!(
                        "{}:{}；已知请求边界暂停，当前 URL 和已消耗次数保留",
                        denial.code, denial.message
                    );
                    outcome.continuation = state.pending() > 0;
                    return checkpoint_outcome(
                        outcome,
                        &state,
                        sent,
                        meter.bytes_used.saturating_sub(before_bytes),
                    );
                }
                if sent == 0 {
                    state.queue.insert(0, raw_url.clone());
                } else {
                    state.visited.push(raw_url.clone());
                    outcome.observations.push(reference_observation(
                        item,
                        &raw_url,
                        KIND_TRANSPORT,
                        denial.as_json(),
                    ));
                }
                outcome.status = Some(
                    if matches!(
                        denial.code,
                        "request_budget_exhausted" | "byte_budget_exhausted"
                    ) {
                        WorkItemStatus::BlockedQuota
                    } else {
                        WorkItemStatus::BlockedAuthorization
                    },
                );
                outcome.source_status = Some(SourceStatus::Partial);
                outcome.reason = format!("{}:{}；尚未发送的队列保留", denial.code, denial.message);
                return checkpoint_outcome(
                    outcome,
                    &state,
                    sent,
                    meter.bytes_used.saturating_sub(before_bytes),
                );
            }
            Err(FetchBlock::RedirectStopped {
                from,
                location,
                denial,
                ..
            }) => {
                state.in_flight = None;
                state.visited.push(raw_url.clone());
                outcome.observations.push(reference_observation(
                    item,
                    &from,
                    "redirect_stopped",
                    json!({"target":location,"denial":denial.as_json()}),
                ));
                state.coverage_reason =
                    format!("部分跳转被范围拦截：{}:{}", denial.code, denial.message);
            }
            Err(FetchBlock::Transport { message, .. }) => {
                state.in_flight = None;
                state.visited.push(raw_url.clone());
                outcome.observations.push(reference_observation(
                    item,
                    &raw_url,
                    KIND_TRANSPORT,
                    json!({"error":message}),
                ));
                state.coverage_reason =
                    "部分 URL 发生传输错误；详细错误和其余队列均已保留".to_string();
            }
        }
    }
    let requests = meter.requests_used.saturating_sub(before_requests);
    let bytes = meter.bytes_used.saturating_sub(before_bytes);
    if state.queue.is_empty() {
        outcome = finish_checkpoint(&state, outcome);
    } else {
        outcome.status = Some(WorkItemStatus::Pending);
        outcome.continuation = true;
        // A batch boundary is not a coverage gap. Source aggregation is emitted only after
        // this origin is drained, paused, or stopped by the run-wide hard limit.
    }
    checkpoint_outcome(outcome, &state, requests, bytes)
}
