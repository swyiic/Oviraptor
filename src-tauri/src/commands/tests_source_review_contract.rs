fn source_review_contract_response(contract:&JsonValue)->JsonValue {
    let required=&contract["requirements"];
    json!({"schemaVersion":1,"subject":"source_candidates","materialDigest":required["materialDigest"],
        "decisions":required["candidates"].as_array().unwrap().iter().map(|candidate|json!({
            "identity":candidate["identity"],"candidateDigest":candidate["candidateDigest"],
            "verdict":"insufficient_evidence","rationale":"Further independent source evidence is needed",
            "reasonCodes":["source_evidence_incomplete"],"evidenceRefs":[],"counterEvidenceRefs":[],
            "missingEvidence":["A verified source slice for this claim"],"confidence":0.5,"severity":"informational",
        })).collect::<Vec<_>>()})
}

#[test]
fn source_review_contract_binds_actual_material_and_rejects_invalid_model_decisions() {
    use crate::agent_runtime::multi_agent::source_review_contract::SourceReviewContract;
    let (root,connection,lease,result)=source_review_material_execution_fixture(None);
    assert!(result.is_ok(),"{result:?}");
    let text:String=connection.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",
        [&lease.root_run_id],|r|r.get(0)).unwrap();
    let event:JsonValue=serde_json::from_str(&text).unwrap();
    let capture=&event["reviewMaterial"];
    let contract=SourceReviewContract::from_material(&capture["material"],&lease.root_run_id).unwrap().unwrap();
    let frozen=contract.as_json();
    assert_eq!(frozen,capture["decisionContract"]);
    assert_eq!(frozen["requirements"]["materialDigest"],capture["digest"]);
    let response=source_review_contract_response(&frozen);
    let accepted=contract.bind_response("a-distinct-reviewer-run",&response.to_string()).unwrap();
    assert_eq!(accepted,response,"contract sorts decisions in canonical identity order");
    let baseline=connection.total_changes();
    for (label,mutated) in [
        ("stale_material",{let mut v=response.clone();v["materialDigest"]=json!("stale");v}),
        ("wrong_subject",{let mut v=response.clone();v["subject"]=json!("coverage_closure_review");v}),
        ("wrong_version",{let mut v=response.clone();v["schemaVersion"]=json!(2);v}),
        ("missing_candidate",{let mut v=response.clone();v["decisions"].as_array_mut().unwrap().pop();v}),
        ("extra_candidate",{let mut v=response.clone();let extra=v["decisions"][0].clone();v["decisions"].as_array_mut().unwrap().push(extra);v}),
        ("duplicate_candidate",{let mut v=response.clone();v["decisions"][1]=v["decisions"][0].clone();v}),
        ("stale_candidate",{let mut v=response.clone();v["decisions"][0]["candidateDigest"]=json!("stale");v}),
        ("foreign_identity",{let mut v=response.clone();v["decisions"][0]["identity"]=json!({"kind":"graph_candidate","nodeId":"other-root","revision":1});v}),
        ("extra_identity_field",{let mut v=response.clone();v["decisions"][0]["identity"]["revision"]=json!(999);v}),
        ("unknown_verdict",{let mut v=response.clone();v["decisions"][0]["verdict"]=json!("passed");v}),
        ("nan_confidence",{let mut v=response.clone();v["decisions"][0]["confidence"]=JsonValue::Null;v}),
        ("large_confidence",{let mut v=response.clone();v["decisions"][0]["confidence"]=json!(1.01);v}),
        ("negative_confidence",{let mut v=response.clone();v["decisions"][0]["confidence"]=json!(-0.1);v}),
        ("unknown_severity",{let mut v=response.clone();v["decisions"][0]["severity"]=json!("severe");v}),
        ("blank_rationale",{let mut v=response.clone();v["decisions"][0]["rationale"]=json!(" ");v}),
        ("missing_reason",{let mut v=response.clone();v["decisions"][0]["reasonCodes"]=json!([]);v}),
        ("missing_gap",{let mut v=response.clone();v["decisions"][0]["missingEvidence"]=json!([]);v}),
        ("fabricated_ref",{let mut v=response.clone();v["decisions"][0]["evidenceRefs"]=json!(["tool:not-in-this-material"]);v}),
        ("confirmed_without_proof",{let mut v=response.clone();v["decisions"][0]["verdict"]=json!("confirmed");v["decisions"][0]["missingEvidence"]=json!([]);v}),
        ("rejected_without_proof",{let mut v=response.clone();v["decisions"][0]["verdict"]=json!("rejected");v}),
        ("model_grants_tools",{let mut v=response.clone();v["toolsGranted"]=json!(["shell"]);v}),
        ("model_claims_authority",{let mut v=response.clone();v["reviewerRunId"]=json!(lease.root_run_id);v}),
    ] {
        assert!(contract.bind_response("a-distinct-reviewer-run",&mutated.to_string()).is_err(),"accepted {label}: {mutated}");
    }
    // Direct struct parsing must catch duplicate fields, including nested ones.
    for invalid in [
        response.to_string().replacen("\"schemaVersion\":1","\"schemaVersion\":1,\"schemaVersion\":1",1),
        response.to_string().replacen("\"confidence\":0.5","\"confidence\":0.5,\"confidence\":0.5",1),
        format!("```json\n{response}\n```"),
        " ".repeat(1_048_577),
    ] { assert!(contract.bind_response("a-distinct-reviewer-run",&invalid).is_err()); }
    let tool_author=capture["material"]["toolEvidence"][0]["authorRunId"].as_str().unwrap();
    for author in ["",lease.root_run_id.as_str(),tool_author] {
        assert_eq!(contract.bind_response(author,&response.to_string()).unwrap_err(),"source_review_author_not_independent");
    }
    // Merely binding output must never write a decision, create a run or turn
    // the unreviewed root into a CI-confirmed result.
    assert_eq!(connection.total_changes(),baseline);
    assert_eq!(result.unwrap()["independentReviewCompleted"],false);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs WHERE role='evidence_reviewer'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_review_contract_keeps_identity_namespaces_and_requires_real_supporting_refs() {
    use crate::agent_runtime::multi_agent::source_review_contract::SourceReviewContract;
    let (root,connection,lease,result)=source_review_material_execution_fixture(None);
    assert!(result.is_ok(),"{result:?}");
    let text:String=connection.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",
        [&lease.root_run_id],|r|r.get(0)).unwrap();
    let event:JsonValue=serde_json::from_str(&text).unwrap();
    let material=&event["reviewMaterial"]["material"];
    let contract=SourceReviewContract::from_material(material,&lease.root_run_id).unwrap().unwrap();
    let frozen=contract.as_json();
    let required=frozen["requirements"]["candidates"].as_array().unwrap();
    let mut response=source_review_contract_response(&frozen);
    let analyzer=required.iter().position(|c|c["identity"]["kind"]=="analyzer_revision").unwrap();
    let graph=required.iter().position(|c|c["identity"]["kind"]=="graph_candidate").unwrap();
    assert!(required[analyzer]["identity"].get("revision").is_none());
    assert!(required[analyzer]["identity"]["revisionHash"].as_str().unwrap().starts_with("sha256:"),"preserve canonical importer hash namespace");
    assert!(required[graph]["identity"].get("revisionHash").is_none());
    assert!(required[graph]["evidenceRefs"].as_array().unwrap().is_empty(),"inventory and self-submission must not count as proof");
    response["decisions"][analyzer]["verdict"]=json!("confirmed");
    response["decisions"][analyzer]["missingEvidence"]=json!([]);
    response["decisions"][analyzer]["evidenceRefs"]=required[analyzer]["evidenceRefs"].clone();
    assert!(contract.bind_response("distinct-reviewer",&response.to_string()).is_ok(),"structural admission only, not independent execution or publication");
    let mut gap=response.clone();gap["decisions"][analyzer]["missingEvidence"]=json!(["still missing proof"]);
    assert_eq!(contract.bind_response("distinct-reviewer",&gap.to_string()).unwrap_err(),"source_review_confirmed_evidence_missing");
    let mut foreign=response.clone();foreign["decisions"][graph]["evidenceRefs"]=required[analyzer]["evidenceRefs"].clone();
    assert_eq!(contract.bind_response("distinct-reviewer",&foreign.to_string()).unwrap_err(),"source_review_evidence_reference_invalid");
    let mut reordered=response.clone();reordered["decisions"].as_array_mut().unwrap().reverse();
    assert_eq!(contract.bind_response("distinct-reviewer",&response.to_string()).unwrap(),contract.bind_response("distinct-reviewer",&reordered.to_string()).unwrap());
    for malformed in [
        {let mut m=material.clone();let duplicate=m["candidates"][0].clone();m["candidates"].as_array_mut().unwrap().push(duplicate);m},
        {let mut m=material.clone();m["rootRunId"]=json!("other-root");m},
        {let mut m=material.clone();m["toolEvidence"][0]["rootRunId"]=json!("other-root");m},
    ] { assert!(SourceReviewContract::from_material(&malformed,&lease.root_run_id).is_err()); }
    let mut empty=material.clone();empty["candidates"]=json!([]);
    assert!(SourceReviewContract::from_material(&empty,&lease.root_run_id).unwrap().is_none(),"no fabricated candidate review task");
    drop(connection);fs::remove_dir_all(root).unwrap();
}
