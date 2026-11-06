fn agent_string_tool(
    name: &'static str,
    description: &'static str,
    parameters: JsonValue,
) -> AgentToolSpec {
    AgentToolSpec::new(name, description, parameters)
}

fn agent_strict_schema(required: &[&str], properties: JsonValue) -> JsonValue {
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn agent_tool_specs() -> Vec<AgentToolSpec> {
    vec![
        agent_string_tool(
            "inspect_evidence",
            "Read the Oviraptor-compressed evidence bundle for this target. It never reads arbitrary files.",
            agent_strict_schema(&[], serde_json::json!({
                "kind": {"type":"string","enum":["api","action","identity","sensitive","route","opportunity","all"]},
                "query": {"type":"string","minLength":1,"maxLength":160},
                "limit": {"type":"integer","minimum":1,"maximum":24}
            })),
        ),
        agent_string_tool(
            "replay_http",
            "Replay one captured request with its recorded method, URL, headers, cookies and body. Read-only comparisons need no confidence score. POST/PUT/PATCH/DELETE stay disabled until the operator opens the POST switch, then one in-person approval covers a bounded comparison series (default 6, max 8) on that contract. An HTML marker is sent only after the frontend render sink is HTML (escaped sinks are ignored, unmapped fields are not probed, and HTML sinks are recorded without automatic bypasses) and the operator has confirmed. A refused write does not stop other tests.",
            agent_strict_schema(&["identity","method","url"], serde_json::json!({
                "identity": {"type":"string","minLength":1},
                "method": {"type":"string","enum":["GET","HEAD","POST","PUT","PATCH","DELETE"]},
                "url": {"type":"string","format":"url"},
                "headers": {"type":"object","additionalProperties":{"type":"string"}},
                "body": {"type":"string"},
                "contentType": {"type":"string"},
                "contractKey": {"type":"string"},
                "attempt": {"type":"integer","minimum":1,"maximum":3},
                "cleanup": {"type":"string"},
                "recoveryCondition": {"type":"string"},
                "confidence": {"type":"number","minimum":0,"maximum":1},
                "confidenceRationale": {"type":"string","minLength":12,"maxLength":400},
                "family": {"type":"string","enum": AGENT_COVERAGE_FAMILIES.to_vec()}
            })),
        ),
        agent_string_tool(
            "compare_identities",
            "Replay one read-only endpoint under two task-scoped identities and compare status, field ownership and normalized value fingerprints. Volatile ids, timestamps and re-issued tokens never count as a difference.",
            agent_strict_schema(&["leftIdentity","rightIdentity","method","path"], serde_json::json!({
                "leftIdentity": {"type":"string","minLength":1},
                "rightIdentity": {"type":"string","minLength":1},
                "method": {"type":"string","enum":["GET","HEAD"]},
                "path": {"type":"string","minLength":1,"maxLength":512},
                "family": {"type":"string","enum": AGENT_COVERAGE_FAMILIES.to_vec()},
                "contractKey": {"type":"string"},
                "ownershipHints": {"type":"array","items":{"type":"string","minLength":1},"maxItems":12}
            })),
        ),
        agent_string_tool(
            "targeted_discovery",
            "Verify a bounded round of path candidates. Words already present in links, forms, JS call sites, routes or local knowledge are probed as-is. The scan's configured cloud model may also expand those names and the domain into pinyin, initials or English path segments; unrelated dictionaries are not added.",
            agent_strict_schema(&["identity","words"], serde_json::json!({
                "identity": {"type":"string","minLength":1},
                "words": {"type":"array","items":{"type":"string","minLength":1,"maxLength":64},"minItems":1,"maxItems":24},
                "method": {"type":"string","enum":["GET","HEAD","OPTIONS"]},
                "family": {"type":"string","enum": AGENT_COVERAGE_FAMILIES.to_vec()}
            })),
        ),
        agent_string_tool(
            "browser_action",
            "Open the located read-only page in the task browser runtime (headed only when OVIRAPTOR_BROWSER_HEADED=1; otherwise headless Chrome/Edge). Use this when a field has no render sink yet. It does not submit forms or uploads. If the runtime is missing, the same URL is fetched once and labelled browserDriven=false.",
            agent_strict_schema(&["identity","actionKey"], serde_json::json!({
                "identity": {"type":"string","minLength":1},
                "actionKey": {"type":"string","minLength":1},
                "family": {"type":"string","enum": AGENT_COVERAGE_FAMILIES.to_vec()}
            })),
        ),
        agent_string_tool(
            "plan_ssrf_channel",
            "Choose one SSRF proof channel from the latest response and do not send it. Use the configured callback when the response does not reveal an internal address. Use one internal host only when internal_scope is authorized and that exact address already appeared in the response or saved evidence. Never scan a range and never invent an address that was not observed.",
            agent_strict_schema(&[], serde_json::json!({
                "response": {"type":"string","maxLength":4000},
                "reason": {"type":"string","maxLength":400},
                "url": {"type":"string","format":"url"},
                "param": {"type":"string","maxLength":128},
                "identity": {"type":"string","minLength":1}
            })),
        ),
        agent_string_tool(
            "record_hypothesis_result",
            "Close one hypothesis with a verdict. confirmed requires control request, test request, response difference, impact and reproduction steps. For a confirmed finding also fill `description` (one or two plain sentences on what the flaw is) and `technicalAnalysis` (how the two requests prove it) — both render directly on the finding card; leave them empty when the verdict is not confirmed. Also fill `businessImpact`: in plain language a non-security developer understands, what this actually lets an attacker do to the business (concrete consequences, not CWE codes) — it is the headline shown to developers. If this finding is one link in a larger attack chain, fill `chainContext`: what finding or condition enables it, and what it in turn enables next (e.g. “the leaked token from the info-leak finding lets this IDOR read any user, which enables bulk export”) — you may reason freely and propose links here.",
            agent_strict_schema(&["hypothesisKey","status"], serde_json::json!({
                "hypothesisKey": {"type":"string","minLength":1},
                "status": {"type":"string","enum":["confirmed","rejected","exhausted","insufficient_evidence"]},
                "contractKey": {"type":"string"},
                "family": {"type":"string","enum": AGENT_COVERAGE_FAMILIES.to_vec()},
                "title": {"type":"string"},
                "description": {"type":"string","maxLength":1200},
                "technicalAnalysis": {"type":"string","maxLength":2000},
                "businessImpact": {"type":"string","maxLength":2000},
                "chainContext": {"type":"string","maxLength":2000},
                "severity": {"type":"string","enum":["critical","high","medium","low","informational"]},
                "cwe": {"type":"string"},
                "cvss": {"type":"string"},
                "confidence": {"type":"number","minimum":0,"maximum":1},
                "confidenceRationale": {"type":"string"},
                "controlRequestId": {"type":"string","minLength":1,"maxLength":64},
                "testRequestId": {"type":"string","minLength":1,"maxLength":64},
                "responseDifferenceArtifactId": {"type":"string","minLength":1,"maxLength":64},
                "objectControlReceiptId": {"type":"string","minLength":1,"maxLength":64},
                "impact": {"type":"string"},
                "reproductionSteps": {"type":"string"},
                "counterEvidenceCheck": {"type":"string"},
                "severityChangeConditions": {"type":"string"},
                "remediation": {"type":"string"},
                "fixVerification": {"type":"string"}
            })),
        ),
        agent_string_tool(
            "call_mcp_tool",
            "Invoke one tool on an operator-configured MCP server (stdio). Both the server and tool must be enabled and on the workspace MCP allowlist (default deny). Pass the tool arguments object exactly as tools/list describes.",
            agent_strict_schema(&["serverId", "tool"], serde_json::json!({
                "serverId": {"type":"string","minLength":1,"maxLength":64},
                "tool": {"type":"string","minLength":1,"maxLength":128},
                "arguments": {"type":"object"}
            })),
        ),
        agent_string_tool(
            "ask_operator",
            "Ask the human operator a structured question when you need credentials, scope clarification, or a fork in the test plan. Optional `options` (max 8) become clickable choices in the UI. Pass `questionId` on retry to read the answer; do not recreate the same question. The operator's answer is authoritative.",
            agent_strict_schema(&["question"], serde_json::json!({
                "question": {"type":"string","minLength":1,"maxLength":800},
                "context": {"type":"string","maxLength":600},
                "options": {"type":"array","items":{"type":"string","minLength":1,"maxLength":120},"maxItems":8},
                "questionId": {"type":"string","minLength":1,"maxLength":64}
            })),
        ),
        agent_string_tool(
            "finish_target",
            "Close the target with the coverage ledger, uncovered reasons, exclusions and manual deep-dive suggestions.",
            agent_strict_schema(&["coverage","stopReason"], serde_json::json!({
                "coverage": {"type":"array","items":{
                    "type":"object",
                    "properties":{
                        "family":{"type":"string","enum": AGENT_COVERAGE_FAMILIES.to_vec()},
                        "status":{"type":"string","enum":["covered","partial","not_applicable"]},
                        "reason":{"type":"string","minLength":1},
                        "evidenceIds":{"type":"array","items":{"type":"string","minLength":1},"maxItems":8}
                    },
                    "required":["family","status"],
                    "additionalProperties":false
                },"minItems":1,"maxItems":7},
                "stopReason": {"type":"string","minLength":1},
                "exclusions": {"type":"array","items":{"type":"string","minLength":1},"maxItems":12},
                "manualDeepDiveSuggestions": {"type":"array","items":{"type":"string","minLength":1},"maxItems":8}
            })),
        ),
        // Track C (code_exec): prove a command-injection finding on the authorized
        // target. Advertised ONLY to a scan that authorized the code_exec class
        // (filtered in agent_tool_specs_for), so a normal scan never sees it and its
        // high-risk capability class never trips the scheduling floor.
        agent_string_tool(
            "probe_command_execution",
            "Prove a command-injection finding on the authorized target by delivering ONE command through the vulnerable request and capturing its output as evidence. Gated: the task must have authorized the code_exec class; harmless identity commands (id, whoami, ifconfig, uname, hostname, pwd, ip, arch) run automatically; any other command, including destructive ones, needs the operator's single-use approval and is never auto-run. v1 delivers through read methods only (GET/HEAD/OPTIONS) — put the placeholder {{CMD}} where the command goes in the url or a header value. Never runs anything off-target or on your host.",
            agent_strict_schema(&["command", "url"], serde_json::json!({
                "command": {"type":"string","minLength":1,"maxLength":512},
                "method": {"type":"string","enum":["GET","HEAD","OPTIONS"]},
                "url": {"type":"string","format":"url"},
                "headers": {"type":"object","additionalProperties":{"type":"string"}},
                "identity": {"type":"string","minLength":1},
                "family": {"type":"string","minLength":1,"maxLength":60},
                "placeholder": {"type":"string","minLength":1,"maxLength":32}
            })),
        ),
        // Track C (target_write): SQL-injection DETECTOR. Sends a tiny fixed
        // differential probe set at one injection point and classifies error- /
        // boolean- / time-based injectability. Advertised ONLY to a scan that
        // authorized target_write (filtered in agent_tool_specs_for).
        agent_string_tool(
            "probe_sql_injection",
            "Detect SQL injection at ONE authorized GET parameter. Sends the unmodified value first, then asks the scan model what single next value to compare, based on the response. No fixed payload list and no data dump. Stop when the model stops. Give `url` and either `param` or a {{INJ}} placeholder; `identity` selects the session.",
            agent_strict_schema(&["url", "identity"], serde_json::json!({
                "url": {"type":"string","format":"url"},
                "identity": {"type":"string","minLength":1},
                "param": {"type":"string","minLength":1,"maxLength":128},
                "placeholder": {"type":"string","minLength":1,"maxLength":32},
                "dbms": {"type":"string","enum":["mysql","postgres","mssql"]}
            })),
        ),
        // Track C (code_exec + egress): target-scoped network PoC sandbox. Runs
        // Python whose ONLY network reach is the authorized target, via a
        // deterministic loopback proxy. Advertised only to a scan that authorized
        // BOTH code_exec and egress (filtered in agent_tool_specs_for).
        agent_string_tool(
            "run_target_poc",
            "Run a short Python PoC in a sandbox whose ONLY network reach is the authorized target (every other host/port and DNS are denied). Use this to send a custom, non-HTTP-tool PoC at the target — read ./target.json for proxyHost/proxyPort (connect there) and targetHost/targetPort (use targetHost for TLS SNI and the Host header); you reach the target through the local proxy. Writes are confined to scratch; CPU/memory/time/output bounded. Gated: the task must have authorized BOTH code_exec and egress, and each distinct script needs the operator's single-use approval. The destination is the scan's authorized target only — you cannot choose another host. For plain HTTP use the normal HTTP tools; for local-only compute use run_analysis_script.",
            agent_strict_schema(&["language", "script"], serde_json::json!({
                "language": {"type":"string","enum":["python"]},
                "script": {"type":"string","minLength":1,"maxLength":8000},
                "input": {"type":"string","maxLength":16000},
                "purpose": {"type":"string","minLength":1,"maxLength":200}
            })),
        ),
        // Track C (code_exec): sandboxed local analysis script. No network; computes
        // over data already captured. Advertised only to a scan that authorized
        // code_exec (filtered in agent_tool_specs_for).
        agent_string_tool(
            "run_analysis_script",
            "Run a short Python script in a no-network sandbox (writes confined to scratch, CPU/memory/time/output bounded) to COMPUTE over data you already have: decode a token, derive a ciphertext from a leaked public key, brute a local hash, parse a .map. It CANNOT reach the target or the host — to send anything to the target use the HTTP tools. Gated: the task must have authorized code_exec; because the sandbox has no network and writes only to scratch (non-destructive), once that class is authorized it runs automatically with no separate per-script approval. Optional `input` is written next to the script as ./input.dat.",
            agent_strict_schema(&["language", "script"], serde_json::json!({
                "language": {"type":"string","enum":["python"]},
                "script": {"type":"string","minLength":1,"maxLength":8000},
                "input": {"type":"string","maxLength":16000},
                "purpose": {"type":"string","minLength":1,"maxLength":200}
            })),
        ),
        // §10.2 item 6 — the source surface. These exist in the registry so a call is
        // schema-checked before it reaches the broker, and they are only advertised to a
        // run that actually froze a repository snapshot.
        agent_string_tool(
            "repo.inventory",
            "List the frozen snapshot's file manifest: repo-relative paths with their content hashes. Never reads outside the snapshot.",
            agent_strict_schema(&[], serde_json::json!({
                "prefix": {"type":"string","minLength":1,"maxLength":512},
                "limit": {"type":"integer","minimum":1,"maximum":100}
            })),
        ),
        agent_string_tool(
            "repo.search",
            "Search a literal string through the frozen snapshot. Returns path, line and a short preview, capped.",
            agent_strict_schema(&["query"], serde_json::json!({
                "query": {"type":"string","minLength":1,"maxLength":160},
                "prefix": {"type":"string","minLength":1,"maxLength":512},
                "limit": {"type":"integer","minimum":1,"maximum":100},
                "ignoreCase": {"type":"boolean"}
            })),
        ),
        agent_string_tool(
            "repo.read_slice",
            "Read a bounded line slice of one snapshot file. The bytes must still hash to what the snapshot recorded.",
            agent_strict_schema(&["path"], serde_json::json!({
                "path": {"type":"string","minLength":1,"maxLength":512},
                "startLine": {"type":"integer","minimum":1,"maximum":1000000},
                "maxLines": {"type":"integer","minimum":1,"maximum":400}
            })),
        ),
        agent_string_tool(
            "git.changed_files",
            "Files changed between the frozen diff base and HEAD. Refused when the base is not a proven ancestor.",
            agent_strict_schema(&[], serde_json::json!({})),
        ),
        agent_string_tool(
            "analyzer.list_results",
            "Analyzer runs and the candidate findings the canonical importer produced for this scan. An absent analyzer is a gap, not a pass.",
            agent_strict_schema(&[], serde_json::json!({
                "engine": {"type":"string","enum":["semgrep","codeql"]},
                "limit": {"type":"integer","minimum":1,"maximum":100}
            })),
        ),
        agent_string_tool(
            "analyzer.get_result",
            "Read back one imported record in full, by the key analyzer.list_results reported.",
            agent_strict_schema(&["key"], serde_json::json!({
                "key": {"type":"string","minLength":1,"maxLength":128}
            })),
        ),
        agent_string_tool(
            "callgraph.get_slice",
            "Reserved: no call-graph analyzer is configured, so this answers with an explicit coverage gap.",
            agent_strict_schema(&[], serde_json::json!({
                "path": {"type":"string","minLength":1,"maxLength":512},
                "symbol": {"type":"string","minLength":1,"maxLength":160}
            })),
        ),
        agent_string_tool(
            "dependency.get_record",
            "Declared dependencies of one manifest inside the snapshot. Transitive closure needs a lockfile or SBOM and is not claimed here.",
            agent_strict_schema(&["manifest"], serde_json::json!({
                "manifest": {"type":"string","minLength":1,"maxLength":512},
                "name": {"type":"string","minLength":1,"maxLength":160}
            })),
        ),
        agent_string_tool(
            "evidence.submit_candidate",
            "Record a suspicion as a candidate. It is never a finding until the reviewer answers it; a confirmed field in the arguments is refused.",
            agent_strict_schema(&["title", "rationale", "path", "line"], serde_json::json!({
                "title": {"type":"string","minLength":1,"maxLength":160},
                "rationale": {"type":"string","minLength":1,"maxLength":600},
                "severity": {"type":"string","enum":["critical","high","medium","low","informational"]},
                "rule": {"type":"string","minLength":1,"maxLength":120},
                "cwe": {"type":"string","minLength":1,"maxLength":32},
                "path": {"type":"string","minLength":1,"maxLength":512},
                "line": {"type":"integer","minimum":1,"maximum":10000000},
                "sourceAnalyzer": {"type":"string","minLength":1,"maxLength":60}
            })),
        ),
        agent_string_tool(
            "assignment.finish",
            "Close this source assignment with a summary and the coverage gaps actually observed.",
            agent_strict_schema(&["summary"], serde_json::json!({
                "summary": {"type":"string","minLength":1,"maxLength":600},
                "gaps": {"type":"array","items":{"type":"string","minLength":1,"maxLength":160},"maxItems":24}
            })),
        ),
    ]
}

/// The §10.2 item 6 source surface, kept in one place so the registry, the broker and
/// the per-run advertisement cannot drift apart.
fn agent_source_tool_names() -> &'static [&'static str] {
    crate::native_pipeline::tools::SOURCE_TOOLS
}

fn agent_is_source_tool(name: &str) -> bool {
    agent_source_tool_names().contains(&name)
}

/// Only a run that froze a repository snapshot is offered the source tools; a web run
/// sees exactly the tool set it saw before Stage 4.
fn agent_tool_specs_for(context: &AgentRunContext) -> Vec<AgentToolSpec> {
    if context.target_url.starts_with("source:") {
        let Ok(connection)=db::open(&context.db_path) else { return Vec::new(); };
        let Ok(authority)=agent_native_source_tool_authority(&connection,context,"repo.inventory") else { return Vec::new(); };
        return agent_tool_specs().into_iter().filter(|spec|authority.tools.iter().any(|name|name==spec.name)).collect();
    }
    let source = agent_run_has_frozen_source(context);
    let code_exec_authorized = agent_scan_code_exec_authorized(context);
    let target_write_authorized = agent_scan_target_write_authorized(context);
    let egress_authorized = agent_scan_egress_authorized(context);
    let Ok(connection) = db::open(&context.db_path) else {
        return agent_tool_specs()
            .into_iter()
            .filter(|spec| spec.name != "call_mcp_tool")
            .filter(|spec| source || !agent_is_source_tool(spec.name))
            .collect();
    };
    let mcp_surface = scan_has_mcp_surface(&connection);
    agent_tool_specs()
        .into_iter()
        .filter(|spec| spec.name != "call_mcp_tool" || mcp_surface)
        .filter(|spec| source || !agent_is_source_tool(spec.name))
        .filter(|spec| spec.name != "browser_action" || (!agent_channel::preflight_snapshot_excludes_browser(&context.db_path, &context.scan_id, context.attempt_number) && context.browser.is_some()))
        // The RCE probe and the sandboxed analysis script are advertised only to a scan
        // that authorized code_exec, so a normal scan never leases them and their
        // high-risk class never reaches the scheduling floor.
        .filter(|spec| !matches!(spec.name, "probe_command_execution" | "run_analysis_script") || code_exec_authorized)
        // The SQL-injection detector is advertised only to a scan that authorized
        // target_write, for the same reason.
        .filter(|spec| spec.name != "probe_sql_injection" || target_write_authorized)
        // The target-scoped network PoC sandbox needs BOTH code_exec and egress.
        .filter(|spec| spec.name != "run_target_poc" || (code_exec_authorized && egress_authorized))
        .collect()
}

/// Whether this scan attempt authorized the code_exec high-risk class. Fail-closed:
/// any storage error means not authorized, so the RCE probe stays hidden.
fn agent_scan_code_exec_authorized(context: &AgentRunContext) -> bool {
    let Ok(connection) = db::open(&context.db_path) else {
        return false;
    };
    scan_capability_class_authorized(&connection, &context.scan_id, context.attempt_number, "code_exec")
        .unwrap_or(false)
}

/// Whether this scan attempt authorized the target_write high-risk class. Fail-
/// closed: any storage error means not authorized, so the SQLi detector stays
/// hidden.
fn agent_scan_target_write_authorized(context: &AgentRunContext) -> bool {
    let Ok(connection) = db::open(&context.db_path) else {
        return false;
    };
    scan_capability_class_authorized(&connection, &context.scan_id, context.attempt_number, "target_write")
        .unwrap_or(false)
}

/// Whether this scan attempt authorized the egress high-risk class. Fail-closed, so
/// the target-scoped network PoC sandbox stays hidden without it.
fn agent_scan_egress_authorized(context: &AgentRunContext) -> bool {
    let Ok(connection) = db::open(&context.db_path) else {
        return false;
    };
    scan_capability_class_authorized(&connection, &context.scan_id, context.attempt_number, "egress")
        .unwrap_or(false)
}
