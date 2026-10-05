//! One atomically published JSON container, not independently published files.
//! Digests bind canonical document content, not authenticity or execution grants.
use super::canonical::{canonical_json, sha256_hex};
use serde_json::{json, Value};

pub(crate) const FORMAT: &str = "oviraptor-source-review-bundle-v1";

pub(crate) fn is_bundle(value: &Value) -> bool {
    value["format"].as_str() == Some(FORMAT)
}

fn entry(name: &str, document: &Value) -> Value {
    let bytes = canonical_json(document);
    json!({"name":name,"bytes":bytes.len(),"sha256":sha256_hex(bytes.as_bytes())})
}

pub(crate) fn build(report: Value, sarif: Value) -> Result<Value, String> {
    let manifest = json!({"json":entry("source-review.json", &report),
        "sarif":entry("source-review.sarif", &sarif)});
    let container = json!({"format":FORMAT,"schemaVersion":1,
        "digestEncoding":"oviraptor-canonical-json-v1", "executionEligible":false,
        "manifest":manifest,"documents":{"json":report,"sarif":sarif}});
    validate(&container)?;
    Ok(container)
}

pub(crate) fn validate(value: &Value) -> Result<(&Value, &Value), String> {
    if !is_bundle(value)
        || value["schemaVersion"] != 1
        || value["digestEncoding"] != "oviraptor-canonical-json-v1"
    {
        return Err("unsupported_source_bundle_schema".into());
    }
    for field in ["documents", "manifest"] {
        let object = value[field]
            .as_object()
            .ok_or("invalid_source_bundle_members")?;
        if object.len() != 2 || !object.contains_key("json") || !object.contains_key("sarif") {
            return Err("invalid_source_bundle_members".into());
        }
    }
    let report = &value["documents"]["json"];
    let sarif = &value["documents"]["sarif"];
    for (key, name, document) in [
        ("json", "source-review.json", report),
        ("sarif", "source-review.sarif", sarif),
    ] {
        if value["manifest"][key] != entry(name, document) {
            return Err("source_bundle_digest_mismatch".into());
        }
    }
    if report["format"] != "oviraptor-source-review-v1"
        || report["schemaVersion"] != 1
        || sarif["version"] != "2.1.0"
        || sarif["runs"].as_array().map(Vec::len) != Some(1)
    {
        return Err("invalid_source_bundle_documents".into());
    }
    let findings = report["review"]["findings"]
        .as_array()
        .ok_or("invalid_source_bundle_findings")?;
    let run = &sarif["runs"][0];
    let results = run["results"]
        .as_array()
        .ok_or("invalid_source_bundle_results")?;
    let mut summary = report.clone();
    summary["review"]
        .as_object_mut()
        .ok_or("invalid_source_bundle_review")?
        .remove("findings");
    if run["properties"]["oviraptorSourceReview"] != summary
        || results.len() != findings.len()
        || results
            .iter()
            .zip(findings)
            .any(|(result, finding)| &result["properties"]["oviraptorSourceFinding"] != finding)
    {
        return Err("inconsistent_source_bundle_documents".into());
    }
    Ok((report, sarif))
}
