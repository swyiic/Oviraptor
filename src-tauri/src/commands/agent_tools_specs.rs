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
            "Execute one locatable action or route from the captured frontend action graph and report the network delta it produced.",
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
    ]
}
