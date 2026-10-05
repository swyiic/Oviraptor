//! Explicit task/project/report imports are historical snapshots, never database
//! backups. Even a locally exported Reviewer decision loses execution authority
//! on import. The byte-exact document is retained by the shared sealed CAS.
use super::{ParseContext, FINDING_FIELDS, RUN_STATE_FIELDS};
use crate::artifact_import::canonical::{
    canonical_json, CanonicalRecord, Producer, RecordInput, RecordKind,
};
use crate::artifact_import::scope::Scope;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub(crate) struct ScopedRecords {
    pub scope: Scope,
    pub records: BTreeMap<String, CanonicalRecord>,
}

fn identifier(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| format!("invalid_bundle_identifier:{field}"))
}

fn array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], String> {
    match value.get(field) {
        None => Ok(&[]),
        Some(Value::Array(rows)) => Ok(rows),
        _ => Err(format!("invalid_bundle_array:{field}")),
    }
}

#[allow(clippy::too_many_arguments)]
fn insert(
    context: &ParseContext<'_>,
    group: &mut ScopedRecords,
    format: &str,
    collection: &str,
    pointer: String,
    kind: RecordKind,
    mut payload: Map<String, Value>,
    known: &'static [&'static str],
) -> Result<(), String> {
    let row_key = [
        "recordKey",
        "opportunityKey",
        "findingKey",
        "candidateId",
        "id",
    ]
    .iter()
    .find_map(|key| {
        payload
            .get(*key)
            .filter(|v| !v.is_null())
            .map(canonical_json)
    })
    .unwrap_or_else(|| canonical_json(&Value::Object(payload.clone())));
    let identity = canonical_json(&json!([
        row_key,
        payload.get("targetUrl"),
        payload.get("url"),
        payload.get("stage"),
        payload.get("kind")
    ]));
    payload.insert("sourceScanId".into(), json!(group.scope.scan_id));
    let record = CanonicalRecord::new(RecordInput {
        kind,
        adapter: "sentinel_bundle",
        producer: Producer::new("oviraptor-snapshot", "", format),
        bundle_id: context.bundle_id.into(),
        source_artifact_id: context
            .manifest
            .content_hash("sentinel-bundle.json")
            .unwrap_or_default()
            .into(),
        pointer,
        identity: vec![canonical_json(&json!([
            group.scope.scan_id,
            collection,
            identity
        ]))],
        payload,
        known,
    });
    // Do not run the cross-format fuzzy finding merge on exported database rows:
    // two distinct row identities must not be collapsed into one candidate.
    if group
        .records
        .insert(record.logical_key.clone(), record)
        .is_some()
    {
        return Err(format!("duplicate_bundle_record:{collection}"));
    }
    Ok(())
}

pub(crate) fn parse(
    context: &ParseContext<'_>,
    value: &Value,
) -> Result<Vec<ScopedRecords>, String> {
    if crate::artifact_import::report_bundle::is_bundle(value) {
        return super::source_report::parse_bundle(context, "sentinel-bundle.json", value)
            .map(|group| vec![group]);
    }
    if super::source_report::is_native(value) {
        return super::source_report::parse(context, "sentinel-bundle.json", value, None)
            .map(|group| vec![group]);
    }
    let format = value
        .get("format")
        .and_then(Value::as_str)
        .ok_or("missing_bundle_format")?;
    let project = matches!(
        format,
        "oviraptor-sentinel-project-v2" | "asset-atlas-sentinel-project-v2"
    );
    if !project && !matches!(format, "oviraptor-sentinel-v1" | "asset-atlas-sentinel-v1") {
        return Err("unsupported_sentinel_bundle_format".into());
    }
    let scans: Vec<&Value> = if project {
        if !value.get("project").is_some_and(Value::is_object) || value.get("scans").is_none() {
            return Err("invalid_project_bundle".into());
        }
        array(value, "scans")?.iter().collect()
    } else {
        vec![value.get("scan").ok_or("missing_bundle_scan")?]
    };
    let mut groups = BTreeMap::new();
    for (index, scan) in scans.iter().enumerate() {
        let id = identifier(scan, "id")?;
        let attempt = match scan.get("attemptCount") {
            None => 0,
            Some(number) => number
                .as_i64()
                .filter(|n| *n >= 0)
                .ok_or("invalid_bundle_attempt")?,
        };
        let mut group = ScopedRecords {
            scope: Scope {
                scan_id: id.clone(),
                attempt_number: attempt,
            },
            records: BTreeMap::new(),
        };
        let mut payload = scan.as_object().ok_or("invalid_bundle_scan")?.clone();
        payload.insert(
            "run_name".into(),
            json!(scan.get("taskName").and_then(Value::as_str).unwrap_or(&id)),
        );
        payload.insert(
            "originalStatus".into(),
            scan.get("status").cloned().unwrap_or(Value::Null),
        );
        payload.insert("status".into(), json!("imported"));
        if project {
            payload.insert("sourceProject".into(), value["project"].clone());
        }
        insert(
            context,
            &mut group,
            format,
            "scan",
            if project {
                format!("/scans/{index}")
            } else {
                "/scan".into()
            },
            RecordKind::RunState,
            payload,
            RUN_STATE_FIELDS,
        )?;
        if groups.insert(id, group).is_some() {
            return Err("duplicate_bundle_scan".into());
        }
    }
    let collections: &[&str] = &[
        "targets",
        "checkpoints",
        "findings",
        "validations",
        "opportunities",
        "fuseZone",
    ];
    let mut total = groups.len();
    for collection in collections {
        for (index, item) in array(value, collection)?.iter().enumerate() {
            total += 1;
            if total > context.limits.records {
                return Err("bundle_record_limit".into());
            }
            let mut payload = item.as_object().ok_or("invalid_bundle_row")?.clone();
            let source_key = if *collection == "fuseZone" {
                "sourceScanId"
            } else {
                "scanId"
            };
            let scan_id = if project {
                match item
                    .get(source_key)
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                {
                    Some(id) => id.to_string(),
                    None if *collection == "fuseZone" => {
                        // Project-wide historical archive rows have no live task.
                        let id = format!("project-archive-{}", &context.bundle_id[7..]);
                        groups.entry(id.clone()).or_insert_with(|| ScopedRecords {
                            scope: Scope {
                                scan_id: id.clone(),
                                attempt_number: 0,
                            },
                            records: BTreeMap::new(),
                        });
                        id
                    }
                    None => return Err("missing_bundle_row_scan".into()),
                }
            } else {
                let id = identifier(scans[0], "id")?;
                if item
                    .get(source_key)
                    .is_some_and(|v| v.as_str() != Some(&id))
                {
                    return Err("foreign_bundle_row_scan".into());
                }
                id
            };
            let group = groups.get_mut(&scan_id).ok_or("foreign_bundle_row_scan")?;
            let is_finding = *collection == "findings";
            payload.insert(
                "endpoint".into(),
                item.get("targetUrl")
                    .or_else(|| item.get("url"))
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            if !is_finding {
                payload.insert("collection".into(), json!(collection));
            }
            insert(
                context,
                group,
                format,
                collection,
                format!("/{collection}/{index}"),
                if is_finding {
                    RecordKind::FindingCandidate
                } else {
                    RecordKind::EvidenceNote
                },
                payload,
                if is_finding {
                    FINDING_FIELDS
                } else {
                    &["title", "endpoint", "collection"]
                },
            )?;
        }
    }
    // Empty projects are still retained and discoverable as historical objects.
    if groups.is_empty() {
        let id = format!("project-archive-{}", &context.bundle_id[7..]);
        let mut group = ScopedRecords {
            scope: Scope {
                scan_id: id,
                attempt_number: 0,
            },
            records: BTreeMap::new(),
        };
        let payload = json!({"run_name":value["project"]["name"],"status":"imported","sourceProject":value["project"]}).as_object().unwrap().clone();
        insert(
            context,
            &mut group,
            format,
            "project",
            "/project".into(),
            RecordKind::RunState,
            payload,
            RUN_STATE_FIELDS,
        )?;
        groups.insert(group.scope.scan_id.clone(), group);
    }
    if groups.values().map(|g| g.records.len()).sum::<usize>() > context.limits.records {
        return Err("bundle_record_limit".into());
    }
    Ok(groups.into_values().collect())
}
