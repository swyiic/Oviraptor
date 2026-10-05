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
            "Replay one captured request with its recorded method, URL, headers, cookies and body. Read-only by default; a write method needs contractKey, attempt, cleanup and recoveryCondition.",
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
            "Verify a bounded round of path candidates that must already appear in links, forms, JS call sites, observed business vocabulary, product routes or local knowledge.",
            agent_strict_schema(&["identity","words"], serde_json::json!({
                "identity": {"type":"string","minLength":1},
                "words": {"type":"array","items":{"type":"string","minLength":1,"maxLength":64},"minItems":1,"maxItems":24},
                "method": {"type":"string","enum":["GET","HEAD","OPTIONS"]},
                "family": {"type":"string","enum": AGENT_COVERAGE_FAMILIES.to_vec()}
            })),
        ),
        agent_string_tool(
            "browser_action",
            "Inspect one located read-only action or route. In multi-agent runs the browser sandbox is unavailable; only its located URL may be fetched once through the HTTP Broker, explicitly labelled browserDriven=false. This does not execute the DOM action or observe a network delta.",
            agent_strict_schema(&["identity","actionKey"], serde_json::json!({
                "identity": {"type":"string","minLength":1},
                "actionKey": {"type":"string","minLength":1},
                "family": {"type":"string","enum": AGENT_COVERAGE_FAMILIES.to_vec()}
            })),
        ),
        agent_string_tool(
            "record_hypothesis_result",
            "Close one hypothesis with a verdict. confirmed requires control request, test request, response difference, impact and reproduction steps.",
            agent_strict_schema(&["hypothesisKey","status"], serde_json::json!({
                "hypothesisKey": {"type":"string","minLength":1},
                "status": {"type":"string","enum":["confirmed","rejected","exhausted","insufficient_evidence"]},
                "contractKey": {"type":"string"},
                "family": {"type":"string","enum": AGENT_COVERAGE_FAMILIES.to_vec()},
                "title": {"type":"string"},
                "severity": {"type":"string","enum":["critical","high","medium","low","informational"]},
                "cwe": {"type":"string"},
                "cvss": {"type":"string"},
                "confidence": {"type":"number","minimum":0,"maximum":1},
                "confidenceRationale": {"type":"string"},
                "controlRequestId": {"type":"string","minLength":1,"maxLength":64},
                "testRequestId": {"type":"string","minLength":1,"maxLength":64},
                "responseDifferenceArtifactId": {"type":"string","minLength":1,"maxLength":64},
                "impact": {"type":"string"},
                "reproductionSteps": {"type":"string"},
                "counterEvidenceCheck": {"type":"string"},
                "severityChangeConditions": {"type":"string"},
                "remediation": {"type":"string"},
                "fixVerification": {"type":"string"}
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
    agent_tool_specs()
        .into_iter()
        .filter(|spec| source || !agent_is_source_tool(spec.name))
        .collect()
}
