//! Native exports share exact historical identities across JSON and SARIF.
//! Exported review claims are data, never execution authority.
use super::{sentinel_bundle::ScopedRecords, ParseContext, FINDING_FIELDS, RUN_STATE_FIELDS};
use crate::artifact_import::{
    canonical::{canonical_json, CanonicalRecord, Producer, RecordInput, RecordKind},
    reconcile,
    scope::Scope,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const FORMAT: &str = "oviraptor-source-review-v1";

pub(crate) fn is_json_name(name: &str) -> bool {
    name == "source-review.json" || (name.starts_with("source-review-") && name.ends_with(".json"))
}

pub(crate) fn is_native(value: &Value) -> bool {
    value.get("format").and_then(Value::as_str) == Some(FORMAT)
}

fn identifier<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
        .ok_or_else(|| format!("invalid_source_report_identity:{key}"))
}

pub(crate) fn parse(
    context: &ParseContext<'_>,
    path: &str,
    bundle: &Value,
    sarif_run: Option<(&Value, usize)>,
) -> Result<ScopedRecords, String> {
    parse_at(context, path, bundle, sarif_run, "")
}

fn parse_at(
    context: &ParseContext<'_>,
    path: &str,
    bundle: &Value,
    sarif_run: Option<(&Value, usize)>,
    prefix: &str,
) -> Result<ScopedRecords, String> {
    if !is_native(bundle) || bundle["schemaVersion"] != 1 {
        return Err("unsupported_source_report_schema".into());
    }
    let review = &bundle["review"];
    let scan = identifier(review, "scanId")?;
    let root = identifier(review, "rootRunId")?;
    let material = identifier(review, "materialDigest")?;
    let attempt = review["attemptNumber"]
        .as_i64()
        .filter(|a| *a > 0)
        .ok_or("invalid_source_report_attempt")?;
    let (rows, pointer) = if let Some((run, index)) = sarif_run {
        (
            run["results"]
                .as_array()
                .ok_or("invalid_source_report_results")?,
            format!("{prefix}/runs/{index}"),
        )
    } else {
        (
            review["findings"]
                .as_array()
                .ok_or("invalid_source_report_findings")?,
            format!("{prefix}/review"),
        )
    };
    if rows.len().saturating_add(1) > context.limits.records {
        return Err("bundle_record_limit".into());
    }
    if review["counts"]["confirmed"].as_u64() != Some(rows.len() as u64) {
        return Err("source_report_count_mismatch".into());
    }
    let artifact = context
        .manifest
        .content_hash(path)
        .ok_or("source_report_artifact_missing")?;
    let make = |kind, pointer: String, identity: Value, payload: Value, known| {
        CanonicalRecord::new(RecordInput {
            kind,
            adapter: "source_report",
            producer: Producer::new("oviraptor-snapshot", "", FORMAT),
            bundle_id: context.bundle_id.into(),
            source_artifact_id: artifact.into(),
            pointer,
            // A single exact dimension is deliberately ineligible for fuzzy merging.
            identity: vec![canonical_json(&identity)],
            payload: payload.as_object().expect("constructed object").clone(),
            known,
        })
    };
    let mut summary = review.clone();
    let summary_map = summary
        .as_object_mut()
        .ok_or("invalid_source_report_review")?;
    summary_map.remove("findings");
    summary_map.insert("originalStatus".into(), review["status"].clone());
    summary_map.insert("status".into(), json!("imported"));
    summary_map.insert("run_name".into(), json!(scan));
    summary_map.insert("sourceScanId".into(), json!(scan));
    if let Some((run, _)) = sarif_run {
        summary_map.insert("sarif_run_properties".into(), run["properties"].clone());
        summary_map.insert("sarif_tool".into(), run["tool"]["driver"].clone());
    }
    let mut records = vec![make(
        RecordKind::RunState,
        pointer.clone(),
        json!([FORMAT, scan, attempt, root, material, "summary", artifact, path, pointer]),
        summary,
        RUN_STATE_FIELDS,
    )];
    let mut seen = BTreeSet::new();
    for (index, input) in rows.iter().enumerate() {
        let row = if sarif_run.is_some() {
            &input["properties"]["oviraptorSourceFinding"]
        } else {
            input
        };
        let decision = identifier(row, "sourceDecisionId")?;
        if row["id"].as_str() != Some(decision) || !seen.insert(decision) {
            return Err("duplicate_or_inconsistent_source_decision".into());
        }
        for field in ["scanId", "attemptNumber", "rootRunId", "materialDigest"] {
            if row[field] != review[field] {
                return Err(format!("foreign_source_report_row:{field}"));
            }
        }
        identifier(row, "candidateDigest")?;
        if row["reviewState"] != "confirmed" {
            return Err("invalid_source_report_verdict".into());
        }
        let mut payload = row.clone();
        payload["sourceScanId"] = json!(scan);
        payload["rule_id"] = json!(decision);
        if sarif_run.is_some() {
            if input["kind"] != "fail"
                || !matches!(input["level"].as_str(), Some("error" | "warning"))
                || input["ruleId"] != row["sourceDecisionId"]
                || input["partialFingerprints"]["oviraptorSourceDecisionId"]
                    != row["sourceDecisionId"]
                || input["partialFingerprints"]["candidateDigest"] != row["candidateDigest"]
            {
                return Err("inconsistent_source_sarif_result".into());
            }
            payload["sarif_result"] = input.clone();
        }
        records.push(make(
            RecordKind::FindingCandidate,
            format!(
                "{pointer}/{}/{index}",
                if sarif_run.is_some() {
                    "results"
                } else {
                    "findings"
                }
            ),
            json!([FORMAT, scan, attempt, root, material, decision]),
            payload,
            FINDING_FIELDS,
        ));
    }
    Ok(ScopedRecords {
        scope: Scope {
            scan_id: scan.into(),
            attempt_number: attempt,
        },
        records: reconcile::merge_by_logical_key(records),
    })
}

pub(crate) fn parse_bundle(
    context: &ParseContext<'_>,
    path: &str,
    value: &Value,
) -> Result<ScopedRecords, String> {
    let [json, sarif] = parse_bundle_documents(context, path, value)?;
    Ok(ScopedRecords {
        scope: json.scope,
        records: reconcile::merge_by_logical_key(
            json.records
                .into_values()
                .chain(sarif.records.into_values())
                .collect(),
        ),
    })
}

fn parse_bundle_documents(
    context: &ParseContext<'_>,
    path: &str,
    value: &Value,
) -> Result<[ScopedRecords; 2], String> {
    let (report, sarif) = crate::artifact_import::report_bundle::validate(value)?;
    let run = &sarif["runs"][0];
    // Count both input documents before their shared candidates are merged.
    let count = report["review"]["findings"]
        .as_array()
        .ok_or("invalid_source_bundle_findings")?
        .len();
    if count.saturating_add(1).saturating_mul(2) > context.limits.records {
        return Err("bundle_record_limit".into());
    }
    let json = parse_at(context, path, report, None, "/documents/json")?;
    let sarif = parse_at(
        context,
        path,
        &run["properties"]["oviraptorSourceReview"],
        Some((run, 0)),
        "/documents/sarif",
    )?;
    Ok([json, sarif])
}

pub(crate) fn parse_directory(context: &ParseContext<'_>) -> Result<Vec<ScopedRecords>, String> {
    let mut groups = Vec::new();
    let mut count = 0usize;
    let mut push = |group: ScopedRecords| -> Result<(), String> {
        count = count.saturating_add(group.records.len());
        if count > context.limits.records {
            return Err("bundle_record_limit".into());
        }
        groups.push(group);
        Ok(())
    };
    for (path, bytes) in context.payloads {
        let file = std::path::Path::new(path);
        let native_json = is_json_name(file.file_name().and_then(|s| s.to_str()).unwrap_or(""));
        let sarif = file
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("sarif"));
        if !native_json && !sarif {
            continue;
        }
        let value: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) if !native_json => continue, // Ordinary SARIF diagnostics belong to its adapter.
            Err(_) => return Err("invalid_source_report_json".into()),
        };
        if native_json {
            if crate::artifact_import::report_bundle::is_bundle(&value) {
                // Keep separate inputs until the service counts generic + native records.
                for group in parse_bundle_documents(context, path, &value)? {
                    push(group)?;
                }
            } else {
                push(parse(context, path, &value, None)?)?;
            }
        } else if let Some(runs) = value["runs"].as_array() {
            for (index, run) in runs.iter().enumerate() {
                let bundle = &run["properties"]["oviraptorSourceReview"];
                if is_native(bundle) {
                    if value["version"] != "2.1.0" {
                        return Err("invalid_source_sarif_version".into());
                    }
                    push(parse(context, path, bundle, Some((run, index)))?)?;
                }
            }
        }
    }
    Ok(groups)
}

fn legacy_identity(kind: &str, extensions: &Value) -> Option<String> {
    let (row, kind) = if kind == "finding_candidate" {
        (
            &extensions["sarif_result"]["properties"]["oviraptorSourceFinding"],
            "finding",
        )
    } else if kind == "run_state" {
        let bundle = &extensions["sarif_run_properties"]["oviraptorSourceReview"];
        if !is_native(bundle) {
            return None;
        }
        (&bundle["review"], "summary")
    } else {
        return None;
    };
    Some(canonical_json(&json!([
        kind,
        identifier(row, "scanId").ok()?,
        row["attemptNumber"].as_i64()?,
        identifier(row, "rootRunId").ok()?,
        identifier(row, "materialDigest").ok()?,
        if kind == "finding" {
            identifier(row, "sourceDecisionId").ok()?
        } else {
            ""
        }
    ])))
}

/// Retire only old generic-SARIF projections proven to originate from the exact
/// same bytes and native report identities being re-imported. This also covers
/// moved files; unrelated records in the old directory scope remain untouched.
/// Immutable revisions and originals remain available for audit.
pub(crate) fn retire_legacy_memberships(
    db: &rusqlite::Connection,
    groups: &[ScopedRecords],
) -> Result<usize, String> {
    let mut identities = BTreeMap::<String, BTreeSet<String>>::new();
    for record in groups.iter().flat_map(|g| g.records.values()) {
        if record.provenance.import_adapter != "source_report" {
            continue;
        }
        if let Some(identity) = legacy_identity(record.record_kind.as_str(), &record.extensions) {
            for artifact in &record.contributing_artifacts {
                identities
                    .entry(artifact.clone())
                    .or_default()
                    .insert(identity.clone());
            }
        }
    }
    if identities.is_empty() {
        return Ok(0);
    }
    let mut revoked = 0;
    let artifacts =
        serde_json::to_string(&identities.keys().collect::<Vec<_>>()).map_err(|e| e.to_string())?;
    let mut query = db
            .prepare(
                "SELECT m.id,r.envelope_json FROM import_projection_memberships m
            JOIN import_record_revisions r ON r.id=m.revision_id
            WHERE m.current=1 AND r.adapter='sarif'
            AND json_extract(r.envelope_json,'$.provenance.sourceArtifactId') IN (SELECT value FROM json_each(?1))
            AND json_extract(r.envelope_json,'$.provenance.adapterVersion')<3",
            )
            .map_err(|e| e.to_string())?;
    let rows = query
        .query_map([artifacts], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for (id, text) in rows {
        let envelope: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let expected = envelope["provenance"]["sourceArtifactId"]
            .as_str()
            .and_then(|artifact| identities.get(artifact));
        if !legacy_identity(
            envelope["recordKind"].as_str().unwrap_or(""),
            &envelope["extensions"],
        )
        .is_some_and(|identity| expected.is_some_and(|set| set.contains(&identity)))
        {
            continue;
        }
        // An old fuzzy merge may contain other findings. Never silently
        // erase those while rebinding the native component.
        if envelope["conflicts"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        {
            return Err("source_report_legacy_merge_ambiguous".into());
        }
        crate::artifact_import::store::demote_membership(db, id)?;
        revoked += 1;
    }
    Ok(revoked)
}
