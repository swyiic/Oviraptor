// Versioned durable frontier for one frozen public-origin walk. This is data, not an
// authorization: every navigation is still checked by ExposureScope immediately before send.
use super::super::super::orchestrator::{scope_hash_of, SCOPE_HASH_PREFIX};
use super::super::super::scope::ExposureScope;
use super::super::super::work_items::WorkItem;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const CURSOR_VERSION: u8 = 1;
const MAX_CURSOR_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TRACKED_URLS: usize = 10_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WalkCheckpoint {
    pub version: u8,
    pub run_id: i64,
    pub project_id: i64,
    pub work_key: String,
    pub input_hash: String,
    pub origin: String,
    pub scope_hash: String,
    pub queue: Vec<String>,
    pub visited: Vec<String>,
    pub in_flight: Option<String>,
    pub unknown: Vec<String>,
    pub successes: usize,
    pub source_reason: String,
    pub coverage_reason: String,
    pub queue_truncated: bool,
    #[serde(default)]
    pub deferred_discovery_count: usize,
    #[serde(default)]
    pub reserved_bytes: i64,
    #[serde(default)]
    pub unknown_reserved_bytes: i64,
}

fn invalid(message: &str) -> String {
    format!("[exposure_walk_cursor_invalid] {message}")
}

fn validate_url(raw: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(raw).map_err(|_| invalid("URL 无法解析"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || raw.len() > 4096
    {
        return Err(invalid("URL 格式不适合公开 GET"));
    }
    Ok(())
}

impl WalkCheckpoint {
    pub fn new(scope: &ExposureScope, item: &WorkItem, queue: Vec<String>) -> Self {
        Self {
            version: CURSOR_VERSION,
            run_id: item.run_id,
            project_id: item.project_id,
            work_key: item.work_key.clone(),
            input_hash: item.input_hash.clone(),
            origin: item.origin.clone(),
            scope_hash: scope_hash_of(scope),
            queue,
            visited: Vec::new(),
            in_flight: None,
            unknown: Vec::new(),
            successes: 0,
            source_reason: String::new(),
            coverage_reason: String::new(),
            queue_truncated: false,
            deferred_discovery_count: 0,
            reserved_bytes: 0,
            unknown_reserved_bytes: 0,
        }
    }

    pub fn decode(raw: &str, origin: &str) -> Result<Self, String> {
        if raw.is_empty() || raw.len() > MAX_CURSOR_BYTES {
            return Err(invalid("游标为空或超过大小限制"));
        }
        let value: Self = serde_json::from_str(raw).map_err(|_| invalid("版本结构无法读取"))?;
        if value.version != CURSOR_VERSION
            || value.origin != origin
            || value.run_id <= 0
            || value.project_id <= 0
            || value.work_key.is_empty()
            || value
                .scope_hash
                .strip_prefix(SCOPE_HASH_PREFIX)
                .is_none_or(|hash| {
                    hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            || value.queue.len()
                + value.visited.len()
                + value.unknown.len()
                + usize::from(value.in_flight.is_some())
                > MAX_TRACKED_URLS
            || value.successes > value.visited.len()
            || value.reserved_bytes < 0
            || value.unknown_reserved_bytes < 0
            || (value.in_flight.is_none() && value.reserved_bytes != 0)
            || value.source_reason.len() > 16_384
            || value.coverage_reason.len() > 16_384
        {
            return Err(invalid("版本、范围或计数不匹配"));
        }
        validate_url(&format!("{origin}/"))?;
        let mut seen = HashSet::new();
        for url in value
            .queue
            .iter()
            .chain(&value.visited)
            .chain(&value.unknown)
        {
            validate_url(url)?;
            if !seen.insert(url) {
                return Err(invalid("队列包含重复或相互矛盾的 URL"));
            }
        }
        if let Some(url) = &value.in_flight {
            validate_url(url)?;
            if seen.contains(url) {
                return Err(invalid("在途 URL 同时存在于其他队列"));
            }
        }
        Ok(value)
    }

    pub fn validate_binding(&self, scope: &ExposureScope, item: &WorkItem) -> Result<(), String> {
        if self.run_id != item.run_id
            || self.project_id != item.project_id
            || self.work_key != item.work_key
            || self.input_hash != item.input_hash
            || self.origin != item.origin
            || self.scope_hash != scope_hash_of(scope)
            || self.reserved_bytes > scope.max_response_bytes
            || self.unknown_reserved_bytes > scope.max_total_bytes
        {
            return Err(
                "[exposure_walk_cursor_scope_changed] 游标不属于当前工作项或冻结范围".to_string(),
            );
        }
        let anchor = reqwest::Url::parse(&format!("{}/", item.origin))
            .map_err(|_| invalid("origin 无法解析"))?;
        // Queued external references are allowed as evidence, but never sent by the walker.
        // The binding check deliberately does not turn such a reference into a grant.
        for url in self.in_flight.iter().chain(&self.unknown) {
            let candidate = reqwest::Url::parse(url).map_err(|_| invalid("URL 无法解析"))?;
            if !scope.navigation_allowed(&anchor, &candidate) {
                return Err(invalid("已发送 URL 越过冻结导航范围"));
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<String, String> {
        let raw = serde_json::to_string(self).map_err(|error| invalid(&error.to_string()))?;
        if raw.len() > MAX_CURSOR_BYTES {
            return Err(invalid("游标超过大小限制"));
        }
        Ok(raw)
    }

    pub fn pending(&self) -> usize {
        self.queue.len() + usize::from(self.in_flight.is_some())
    }

    pub fn reconcile_saved(&mut self, saved_urls: &[String]) {
        let saved: HashSet<&str> = saved_urls.iter().map(String::as_str).collect();
        let completed: Vec<String> = self
            .queue
            .iter()
            .filter(|url| saved.contains(url.as_str()))
            .cloned()
            .collect();
        self.queue.retain(|url| !saved.contains(url.as_str()));
        for url in completed {
            if !self.visited.contains(&url) {
                self.visited.push(url);
            }
        }
        if let Some(url) = self.in_flight.take() {
            if saved.contains(url.as_str()) {
                if !self.visited.contains(&url) {
                    self.visited.push(url);
                }
            } else {
                // The request may have reached the remote peer. Lack of an observation
                // does not mean it was never sent, so it is not put back in the frontier.
                self.unknown.push(url);
                self.unknown_reserved_bytes = self
                    .unknown_reserved_bytes
                    .saturating_add(self.reserved_bytes);
            }
            self.reserved_bytes = 0;
        }
    }
}
