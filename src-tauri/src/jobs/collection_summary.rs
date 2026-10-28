use crate::{asset_models::CollectionSummary, db};
use rusqlite::params;
use serde_json::Value;
use std::{fs, path::Path};

fn count(value: &Value, name: &str) -> u64 {
    value.get(name).and_then(Value::as_u64).unwrap_or(0)
}

fn codes(value: &Value, name: &str) -> Vec<String> {
    let mut result = value
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 80
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
        .take(32)
        .map(str::to_string)
        .collect::<Vec<_>>();
    result.sort();
    result.dedup();
    result
}

pub(super) fn binding(value: &Value) -> Option<&str> {
    value
        .get("binding_sha256")
        .and_then(Value::as_str)
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Coverage is a proof about every planned query, not the execution's success flag.
/// Missing fields and contradictory receipts must remain unknown.
pub(super) fn proven_complete(value: &Value) -> bool {
    let Some(queries) = value
        .get("queries")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
    else {
        return false;
    };
    value.get("complete").and_then(Value::as_bool) == Some(true)
        && value.get("coverage_complete").and_then(Value::as_bool) == Some(true)
        && value.get("completed_queries").and_then(Value::as_u64) == Some(queries)
        && [
            "failed_queries",
            "truncated_queries",
            "partial_queries",
            "invalid_records",
        ]
        .iter()
        .all(|key| value.get(key).and_then(Value::as_u64) == Some(0))
        && value
            .get("field_coverage_complete")
            .and_then(Value::as_bool)
            == Some(true)
        && value
            .get("coverage_reasons")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        && value
            .get("error_categories")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
}

pub(super) fn from_collection(value: &Value) -> CollectionSummary {
    let execution_complete = value.get("complete").and_then(Value::as_bool) == Some(true);
    let coverage_complete = proven_complete(value);
    let mut reasons = codes(value, "coverage_reasons");
    if !coverage_complete && reasons.is_empty() {
        reasons.push("coverage_unproven".into());
    }
    CollectionSummary {
        schema_version: 1,
        execution_complete,
        coverage_complete,
        queries: count(value, "queries"),
        completed_queries: count(value, "completed_queries"),
        failed_queries: count(value, "failed_queries"),
        truncated_queries: count(value, "truncated_queries"),
        partial_queries: count(value, "partial_queries"),
        invalid_records: value
            .get("invalid_records")
            .or_else(|| value.get("rejected_rows"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        candidates: count(value, "candidates"),
        imported_candidates: Some(0),
        probed_assets: None,
        reused_pages: count(value, "reused_pages"),
        error_categories: codes(value, "error_categories"),
        reasons,
        retention: None,
    }
}

pub(super) fn retention(
    report: &super::collection_retention::RetentionReport,
    limits: super::collection_retention::Limits,
) -> crate::asset_models::CollectionRetentionSummary {
    crate::asset_models::CollectionRetentionSummary {
        scope: "owned_collection_files_and_control_metadata_only".into(),
        run_bytes: report.run_bytes,
        project_bytes: report.project_bytes,
        global_bytes: report.global_bytes,
        // The UI's retention scope is all registered collection directories.
        // Count removals there, including another expired run reclaimed now.
        expired_files: report.global_expired_files,
        expired_bytes: report.global_expired_bytes,
        unmanaged_runs: report.unmanaged_runs as u64,
        unknown_files: report.unknown_files as u64,
        history_truncated: report.history_truncated,
        max_run_bytes: limits.run_bytes,
        max_project_bytes: limits.project_bytes,
        max_global_bytes: limits.global_bytes,
        max_pages: limits.pages as u64,
        max_age_seconds: limits.age_seconds as u64,
    }
}

/// Only accept the collector's bounded atomic receipt, never infer coverage
/// from a file's presence, candidate count, error text or progress percentage.
pub(super) fn failed_receipt(directory: &Path) -> CollectionSummary {
    let path = directory.join("collection_state.json");
    let receipt = fs::metadata(&path)
        .ok()
        .filter(|m| m.len() <= 8 * 1024 * 1024)
        .and_then(|_| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    let mut summary = from_collection(&receipt);
    summary.execution_complete = false;
    summary.coverage_complete = false;
    if !summary
        .reasons
        .iter()
        .any(|code| code == "collection_failed")
    {
        summary.reasons.push("collection_failed".into());
    }
    summary
}

pub(super) fn save_on(
    connection: &rusqlite::Connection,
    run_id: i64,
    summary: &CollectionSummary,
) -> Result<(), String> {
    let encoded = serde_json::to_string(summary).map_err(|e| e.to_string())?;
    if encoded.len() > 16 * 1024 {
        return Err("资产收集摘要超出保存上限".into());
    }
    let changed = connection
        .execute(
            "UPDATE runs SET collection_summary_json=?1 WHERE id=?2",
            params![encoded, run_id],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("资产收集摘要对应任务不存在".into());
    }
    Ok(())
}

pub(super) fn save(db_path: &Path, run_id: i64, summary: &CollectionSummary) -> Result<(), String> {
    save_on(&db::open(db_path)?, run_id, summary)
}

pub(super) fn probe_count(value: &Value) -> Option<u64> {
    value
        .get("stages")?
        .as_object()?
        .values()
        .try_fold(0u64, |sum, stage| {
            if stage.get("complete").and_then(Value::as_bool) != Some(true) {
                return None;
            }
            let counts = stage.get("counts")?.as_object()?;
            let selected = stage.get("selected_rows")?.as_u64()?;
            let total = counts
                .values()
                .try_fold(0u64, |n, v| n.checked_add(v.as_u64()?))?;
            if total != selected {
                return None;
            }
            sum.checked_add(total.checked_sub(counts.get("skipped")?.as_u64()?)?)
        })
}

