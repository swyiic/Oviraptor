/// Native agent tool registry (§5).
///
/// Every tool validates its own arguments locally: the model can never widen
/// the authorized scope, mix identities, exceed attempt limits or skip the
/// cleanup requirement for a write request.
const AGENT_MAX_INSPECT_ITEMS: usize = 24;
const AGENT_MAX_PREVIEW_CHARS: usize = 1_400;
/// Wall-clock budget for one browser-driven action, including the probe's own
/// navigation and exploration floors.
const AGENT_BROWSER_ACTION_SECONDS: u64 = 150;
const AGENT_HTTP_DIRECTORY: &str = "agent-http";
/// Response-difference records: the thing a confirmed finding must cite (§10.2).
const AGENT_DIFF_DIRECTORY: &str = "agent-diff";
const AGENT_MAX_RESPONSE_BYTES: usize = 262_144;
/// Consecutive HTTP 429 answers that mean this run will not be answered (§11).
const AGENT_PERSISTENT_RATE_LIMITS: usize = 2;
const AGENT_RESPONSE_CHUNK_BYTES: usize = 8_192;
const AGENT_READ_METHODS: [&str; 3] = ["GET", "HEAD", "OPTIONS"];
/// The task-level handle an unauthenticated call is replayed with.
const AGENT_ANONYMOUS_IDENTITY: &str = "anonymous";
/// Paths that only prove a login screen was reached, never that a session works.
const AGENT_LOGIN_SEGMENTS: [&str; 6] = ["login", "signin", "logout", "register", "signup", "sso"];
const AGENT_DENY_METHODS: [&str; 5] = ["UNKNOWN", "CONNECT", "TRACE", "TRACK", ""];
const AGENT_CREDENTIAL_HEADERS: [&str; 3] = ["cookie", "authorization", "proxy-authorization"];
/// Static asset suffixes. A JS file served by an authorized host is a resource
/// reference, not a business API (§5.3).
const AGENT_STATIC_SUFFIXES: [&str; 16] = [
    ".js", ".mjs", ".css", ".map", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp",
    ".avif", ".woff", ".woff2", ".ttf", ".eot",
];
/// Keys that legitimately differ between two identities on the same endpoint.
/// Reporting them as a difference is what produced the A/B false positives.
const AGENT_VOLATILE_FIELDS: [&str; 18] = [
    "requestid", "request_id", "traceid", "trace_id", "correlationid", "correlation_id", "spanid",
    "nonce", "timestamp", "ts", "server_time", "servertime", "csrf", "xsrf", "_t",
    "createdat", "updatedat", "deletedat",
];
/// Per-account material: a different value here means each identity sees its own
/// object. It is shown separately so ownership can be confirmed, and it is never
/// an authorization difference by itself (§8).
const AGENT_SUBJECT_FIELDS: [&str; 16] = [
    "id", "_id", "uuid", "guid", "key", "uid", "userid", "user_id", "accountid", "account_id",
    "ownerid", "owner_id", "email", "phone", "mobile", "username",
];
/// Fields whose value *is* the authorization decision. A difference here is
/// material evidence and must never be filed under ownership or ignored noise (§8).
const AGENT_AUTHORIZATION_FIELDS: [&str; 31] = [
    "role", "roleid", "roles", "roleids", "permission", "permissionid", "permissions",
    "permissionids", "perm", "perms", "tenant", "tenantid", "organization", "organizationid",
    "org", "orgid", "department", "departmentid", "dept", "deptid", "policy", "policyid",
    "scope", "scopes", "group", "groupid", "groups", "privilege", "privileges", "authority",
    "acl",
];
const AGENT_TOKEN_FIELDS: [&str; 8] = [
    "token", "accesstoken", "access_token", "refreshtoken", "refresh_token", "authtoken",
    "authorization", "sessiontoken",
];

#[derive(Default)]
struct AgentToolRuntime {
    endpoints: HashSet<String>,
    response_shapes: HashSet<String>,
    parameter_signatures: HashSet<String>,
    verdict_keys: HashSet<String>,
    /// Families that a real, executed request has produced evidence for. A model
    /// claim alone never enters this set (§8 progress rules).
    families: HashSet<String>,
    contract_attempts: HashMap<String, i64>,
    contract_outcomes: HashMap<String, String>,
    requests: Vec<AgentRequestTrace>,
    comparisons: HashSet<String>,
    discovery_rounds: usize,
    target_requests: usize,
    last_progress: AgentProgressDelta,
    protected_stop: Option<AgentStop>,
    /// Consecutive HTTP 429 answers inside this attempt. One throttle is work to
    /// postpone; a run the site keeps refusing is a protection stop (§11).
    rate_limited_streak: usize,
    /// Per-run salt so a redacted value stays comparable inside the run and
    /// unlinkable across runs (§6.2).
    redaction: crate::agent_runtime::secrets::RedactionContext,
    /// Artifact written by the most recent tool execution, so the tool result can
    /// point at the raw bytes without carrying them (§6.1).
    last_artifact_id: String,
    /// Fingerprint of the most recent schema rejection and how often it repeated
    /// unchanged (§7: the same malformed call is no progress).
    last_argument_error: String,
    repeated_argument_errors: usize,
    /// The coverage ledger Rust owns (§9.2): every entry cites request records and
    /// tool invocations this chain really performed.
    coverage: Vec<CoverageEvidence>,
    invocations: usize,
    /// Invocation id of the call being executed, so a record written during it can
    /// carry the same id.
    current_invocation: String,
    /// Request ids whose response echoed one of the request's own parameter
    /// values — the input→response chain an XSS claim needs (§9.3).
    reflections: HashSet<String>,
    /// hypothesisKey -> verdict id, so a finding cites the verdict that earned it.
    verdict_ids: HashMap<String, String>,
    /// Refreshed each round by the loop; lets a tool stop touching the target the
    /// moment the user pauses or cancels.
    cancel: Option<CancelToken>,
    confirmed_findings: i64,
    uncovered_families: Vec<JsonValue>,
    /// The derived coverage ledger as it was handed to the model at close-out.
    coverage_claims: Vec<JsonValue>,
    /// Families the closing ledger declares inapplicable, with an honest reason.
    not_applicable: HashSet<String>,
    manual_suggestions: Vec<String>,
    exclusions: Vec<String>,
    finished: Option<JsonValue>,
}

impl AgentToolRuntime {
    fn take_progress(&mut self) -> AgentProgressDelta {
        std::mem::take(&mut self.last_progress)
    }

    /// Rebuild the working set from a checkpoint so a resumed attempt keeps the
    /// HTTP budget it already spent and the contracts it already executed.
    ///
    /// Fields are assigned after `default()` on purpose: a literal with all 18
    /// of them would hide what a resume actually restores.
    #[allow(clippy::field_reassign_with_default)]
    fn restore(state: &NativeAgentState) -> Self {
        let mut runtime = Self::default();
        runtime.discovery_rounds = state.discovery_rounds.max(0) as usize;
        runtime.target_requests = state.target_requests.max(0) as usize;
        runtime.confirmed_findings = state.confirmed_findings;
        runtime.verdict_keys = state.verdict_keys.iter().cloned().collect();
        runtime.requests = state.observed_requests.clone();
        for trace in &runtime.requests {
            runtime
                .endpoints
                .insert(format!("{}|{}", trace.method, trace.path));
        }
        for (key, attempts) in &state.contract_attempts {
            runtime.contract_attempts.insert(key.clone(), *attempts);
        }
        for (key, outcome) in &state.contract_outcomes {
            runtime.contract_outcomes.insert(key.clone(), outcome.clone());
        }
        runtime.families = state
            .observed_requests
            .iter()
            .map(|trace| trace.family.clone())
            .filter(|family| !family.is_empty())
            .collect();
        runtime.coverage = state.coverage_evidence.clone();
        // Coverage is Rust's to derive, so a family inherited without any ledger
        // entry is rebuilt from the inherited requests: a checkpoint written before
        // this ledger existed cannot leave a family unaccounted for (§9.1).
        let mut inherited: HashMap<String, (String, Vec<String>)> = HashMap::new();
        for trace in &runtime.requests {
            if trace.family.is_empty() {
                continue;
            }
            let entry = inherited
                .entry(trace.family.clone())
                .or_insert_with(|| (trace.contract_key.clone(), Vec::new()));
            entry.1.push(trace.id.clone());
        }
        for (family, (contract_key, request_ids)) in inherited {
            if runtime.coverage.iter().any(|row| row.family == family) {
                continue;
            }
            runtime.credit_coverage(&family, "request", &contract_key, &request_ids);
        }
        runtime
    }

    /// Open one invocation before the tool runs, so the audit row can be written in
    /// `running` state first (§4.2). `inv-<attempt>-<seq>` is unique across attempts
    /// without a persisted counter, so a resumed run can still cite an id its parent
    /// recorded (§9.2).
    fn begin_invocation(&mut self, attempt_number: i64) -> String {
        self.invocations += 1;
        self.set_invocation(&format!("inv-{attempt_number}-{:03}", self.invocations));
        self.current_invocation.clone()
    }

    /// Adopt an id the loop already allocated for this call.
    fn set_invocation(&mut self, invocation_id: &str) {
        self.current_invocation = invocation_id.to_string();
        let digits: String = invocation_id
            .chars()
            .rev()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        self.invocations = self.invocations.max(digits.parse::<usize>().unwrap_or(0));
    }

    /// Add or extend the evidence entry for one family. The id is assigned here and
    /// nowhere else, which is what lets `finish_target` check a model's citation.
    fn note_coverage(
        &mut self,
        family: &str,
        evidence_kind: &str,
        contract_key: &str,
        request_id: &str,
        result: &str,
        reason_code: &str,
    ) -> String {
        if family.is_empty() {
            return String::new();
        }
        let existing = self.coverage.iter_mut().find(|entry| {
            entry.family == family
                && entry.evidence_kind == evidence_kind
                && entry.contract_key == contract_key
        });
        if let Some(entry) = existing {
            if !request_id.is_empty() && !entry.request_record_ids.contains(&request_id.to_string()) {
                entry.request_record_ids.push(request_id.to_string());
            }
            if !self.current_invocation.is_empty()
                && !entry.tool_invocation_ids.contains(&self.current_invocation)
            {
                entry.tool_invocation_ids.push(self.current_invocation.clone());
            }
            // A stronger conclusion never regresses, and a weaker one never
            // overstates what the chain proved.
            if entry.result != "covered" && result == "covered" {
                entry.result = result.to_string();
                entry.reason_code = reason_code.to_string();
            }
            return entry.id.clone();
        }
        let id = format!("cov-{:04}", self.coverage.len() + 1);
        self.coverage.push(CoverageEvidence {
            id: id.clone(),
            family: family.to_string(),
            evidence_kind: evidence_kind.to_string(),
            contract_key: contract_key.to_string(),
            tool_invocation_ids: if self.current_invocation.is_empty() {
                Vec::new()
            } else {
                vec![self.current_invocation.clone()]
            },
            request_record_ids: if request_id.is_empty() {
                Vec::new()
            } else {
                vec![request_id.to_string()]
            },
            verdict_id: String::new(),
            result: result.to_string(),
            reason_code: reason_code.to_string(),
        });
        id
    }

    /// Derive the ledger entry for one family from the executed chain, then file
    /// it under the kind of work that produced it (§9.1).
    fn credit_coverage(
        &mut self,
        family: &str,
        evidence_kind: &str,
        contract_key: &str,
        request_ids: &[String],
    ) {
        if family.is_empty() {
            return;
        }
        let (result, reason_code) = agent_family_sufficiency(self, family);
        let ids: Vec<String> = if request_ids.is_empty() {
            vec![String::new()]
        } else {
            request_ids.to_vec()
        };
        for id in ids {
            self.note_coverage(family, evidence_kind, contract_key, &id, result, reason_code);
        }
    }

    /// Stable id for one recorded verdict, so a coverage entry and a finding can
    /// cite the same thing (§9.2, §10.1).
    fn verdict_id_for(&mut self, verdict_key: &str) -> String {
        if let Some(existing) = self.verdict_ids.get(verdict_key) {
            return existing.clone();
        }
        let id = format!("ver-{:04}", self.verdict_ids.len() + 1);
        self.verdict_ids.insert(verdict_key.to_string(), id.clone());
        id
    }

    fn has_coverage_evidence(&self, family: &str) -> bool {
        self.coverage.iter().any(|entry| {
            entry.family == family
                && entry.result == "covered"
                && !entry.request_record_ids.is_empty()
        })
    }

    fn has_family_evidence(&self, family: &str) -> bool {
        !family.is_empty() && self.requests.iter().any(|trace| trace.family == family)
    }

    /// Credit a coverage family only when executed work backs it.
    fn credit_family(&mut self, family: &str) -> bool {
        if !self.has_family_evidence(family) {
            return false;
        }
        self.families.insert(family.to_string())
    }

    /// A queue entry is satisfied only by executed work, never by a model claim.
    fn queue_key_satisfied(&self, context: &AgentRunContext, key: &str) -> bool {
        if let Some(family) = key.strip_prefix("family:") {
            // A family still open when the ledger closes is either worked or
            // honestly declared not applicable; both end the queue, and only
            // executed requests can make it *covered*.
            return self.has_family_evidence(family) || self.not_applicable.contains(family);
        }
        if let Some(contract) = key.strip_prefix("contract:") {
            if self.contract_outcomes.contains_key(contract) {
                return true;
            }
            // Attempts spent to the evidence contract's own cap are done even
            // without a verdict.
            self.attempts_for(contract)
                >= agent_evidence_contract_attempts(&context.evidence, contract)
        } else if let Some(endpoint) = key.strip_prefix("api:") {
            let (method, path) = endpoint.split_once('|').unwrap_or(("GET", endpoint));
            self.requests
                .iter()
                .any(|trace| trace.method == method && trace.path == path)
        } else {
            false
        }
    }

    fn record_endpoint(&mut self, method: &str, path: &str) -> bool {
        self.endpoints
            .insert(format!("{}|{}", method.to_ascii_uppercase(), path))
    }

    fn record_response_shape(&mut self, method: &str, path: &str, response: &JsonValue) -> bool {
        let key = format!(
            "{}|{}|{}|{}",
            method.to_ascii_uppercase(),
            path,
            response.get("status").and_then(JsonValue::as_i64).unwrap_or(0),
            response
                .get("bodySha256")
                .and_then(JsonValue::as_str)
                .map(|value| value.chars().take(8).collect::<String>())
                .unwrap_or_default()
        );
        self.response_shapes.insert(key)
    }

    fn record_parameters(&mut self, method: &str, path: &str, names: &[String]) -> usize {
        let mut added = 0usize;
        for name in names {
            if self
                .parameter_signatures
                .insert(format!("{method}|{path}|{}", name.to_ascii_lowercase()))
            {
                added += 1;
            }
        }
        added
    }

    /// Count a call that reached the network, including transport failures, so a
    /// flaky target cannot be hammered for free.
    fn spend_request(&mut self) {
        self.target_requests += 1;
    }

    /// The raw audit file for the exchange that was just recorded.
    fn note_artifact(&mut self, artifact_id: &str) {
        self.last_artifact_id = artifact_id.to_string();
    }

    /// §7: a rejected call is only progress in the negative sense — the same
    /// malformed arguments repeating must keep the no-progress streak running.
    fn note_argument_error(&mut self, fingerprint: &str) {
        if self.last_argument_error == fingerprint {
            self.repeated_argument_errors += 1;
        } else {
            self.last_argument_error = fingerprint.to_string();
            self.repeated_argument_errors = 1;
        }
    }

    fn clear_argument_error(&mut self) {
        self.last_argument_error.clear();
        self.repeated_argument_errors = 0;
    }

    fn repeats_argument_error(&self) -> bool {
        self.repeated_argument_errors >= 2
    }

    /// Bind the audit artifact to the exchange it came from. Coverage and finding
    /// claims cite request ids, so the request has to be able to point back at the
    /// bytes that proved it (§9.2, §10.1).
    fn attach_artifact(&mut self, artifact_id: &str, structure_hash: &str) {
        if let Some(trace) = self.requests.last_mut() {
            trace.artifact_id = artifact_id.to_string();
            trace.structure_hash = structure_hash.to_string();
        }
    }

    /// The trace a claim cites, if this execution chain really made that request.
    fn request(&self, request_id: &str) -> Option<&AgentRequestTrace> {
        self.requests.iter().find(|trace| trace.id == request_id)
    }

    /// The single place an executed exchange is accounted: HTTP spend, endpoint
    /// discovery and the contract attempt that produced it. The request id is
    /// assigned here and nowhere else, so every later reference resolves to a
    /// request this chain really made (§10.1).
    fn record_request(&mut self, mut trace: AgentRequestTrace) {
        if trace.id.is_empty() {
            trace.id = format!("req-{:04}", self.target_requests.max(1));
        }
        if trace.invocation_id == 0 {
            trace.invocation_id = self.invocations as i64;
        }
        if self.record_endpoint(&trace.method, &trace.path) {
            self.last_progress.new_endpoints += 1;
        }
        if !trace.contract_key.is_empty() {
            *self
                .contract_attempts
                .entry(trace.contract_key.clone())
                .or_insert(0) += 1;
        }
        self.requests.push(trace);
    }

    fn touched(&self, method: &str, path: &str) -> bool {
        let upper = method.to_ascii_uppercase();
        self.requests
            .iter()
            .any(|trace| trace.method == upper && trace.path == path)
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|token| token.is_cancelled())
            .unwrap_or(false)
    }

    fn attempts_for(&self, contract_key: &str) -> i64 {
        self.contract_attempts.get(contract_key).copied().unwrap_or(0)
    }

    /// Count a request the target really served, including one a browser session
    /// made on the agent's behalf: it spent budget and it produced evidence.
    fn note_observed_request(&mut self, trace: AgentRequestTrace) {
        self.spend_request();
        self.record_request(trace);
    }

    /// One material comparison per endpoint is enough to count as progress.
    fn record_comparison(&mut self, signature: &str) -> bool {
        self.comparisons.insert(signature.to_string())
    }
}
