//! §9.7 — merge records that independent adapters produced into one semantic row per
//! logical key, then reconcile that set against what the scope already projects
//! (§9.9). Revisions are only ever added; membership rows are the current view.

use super::canonical::{self, CanonicalRecord, RecordKind};
use super::diagnostics::Diagnostic;
use super::store;
use rusqlite::Connection;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub revisions: usize,
    pub unchanged: usize,
    pub revoked: usize,
    pub adopted: usize,
    pub stale_ignored: usize,
}

/// Adapter preference when two formats disagree: the adopted value comes from the
/// earlier adapter and every other value is still recorded as a conflict (§9.7).
const ADAPTER_PRIORITY: &[&str] = &["sarif", "frontend_recon", "sentinel_stages"];

/// The record that speaks first inside one group: adapter preference, then position.
fn record_rank(record: &CanonicalRecord) -> (usize, String) {
    (
        priority(&record.provenance.import_adapter),
        record.provenance.source_record_pointer.clone(),
    )
}

fn group_rank(group: &[CanonicalRecord]) -> (usize, String) {
    group
        .iter()
        .min_by_key(|record| record_rank(record))
        .map(record_rank)
        .unwrap_or((usize::MAX, String::new()))
}

fn priority(adapter: &str) -> usize {
    ADAPTER_PRIORITY
        .iter()
        .position(|known| *known == adapter)
        .unwrap_or(ADAPTER_PRIORITY.len())
}

/// Groups by record kind + logical key and folds each group into one record.
pub fn merge_by_logical_key(records: Vec<CanonicalRecord>) -> BTreeMap<String, CanonicalRecord> {
    let mut groups = BTreeMap::<String, Vec<CanonicalRecord>>::new();
    for record in records {
        groups
            .entry(format!(
                "{}\u{1}{}",
                record.record_kind.as_str(),
                record.logical_key
            ))
            .or_default()
            .push(record);
    }
    let mut merged = BTreeMap::<String, CanonicalRecord>::new();
    // Cross-format merging looks for a host among the groups already folded, so the
    // order groups are visited in decides who adopts whom. Hash order would make that
    // depend on a digest; adapter priority and pointer do not.
    let mut ordered: Vec<(String, Vec<CanonicalRecord>)> = groups.into_iter().collect();
    ordered.sort_by(|left, right| {
        group_rank(&left.1)
            .cmp(&group_rank(&right.1))
            .then_with(|| left.0.cmp(&right.0))
    });
    for (key, mut group) in ordered {
        group.sort_by_key(record_rank);
        let mut primary = group.remove(0);
        for other in group {
            primary.merge_from(&other);
        }
        // §9.7: the stored hash must describe what this merge actually produced.
        primary.finalize_revision_hash();
        if primary.record_kind == RecordKind::FindingCandidate {
            // 缺口 8: a format that states *less* about the same finding (no region,
            // no method) still belongs to it, as long as nothing they both state
            // disagrees. Groups are visited in a deterministic order, so which record
            // absorbs which never depends on the source array's order.
            let fingerprint = primary.fingerprint.clone();
            let host = merged
                .iter()
                .find(|(_, row)| {
                    row.record_kind == RecordKind::FindingCandidate
                        && canonical::fingerprints_merge(&row.fingerprint, &fingerprint)
                })
                .map(|(existing_key, _)| existing_key.clone());
            if let Some(host) = host {
                if let Some(row) = merged.get_mut(&host) {
                    row.merge_from(&primary);
                    row.finalize_revision_hash();
                    continue;
                }
            }
        }
        merged.insert(key, primary);
    }
    merged
}

/// §IDM-012: a record the legacy importer already wrote is referenced, not duplicated.
fn adopted_from_legacy(connection: &Connection, logical_key: &str) -> bool {
    connection
        .query_row(
            "SELECT 1 FROM sentinel_findings WHERE record_key=?1 LIMIT 1",
            [logical_key],
            |_| Ok(()),
        )
        .is_ok()
}

pub fn attempt_number_of(attempt_key: &str) -> i64 {
    attempt_key
        .split('-')
        .nth(1)
        .and_then(|digits| digits.parse::<i64>().ok())
        .unwrap_or(0)
}

pub fn apply(
    connection: &Connection,
    scope: &super::scope::Scope,
    bundle_row_id: i64,
    source_path: &std::path::Path,
    records: &BTreeMap<String, CanonicalRecord>,
    envelope_json: &dyn Fn(&CanonicalRecord) -> String,
    limits: &super::limits::Limits,
) -> Result<(ReconcileReport, Vec<Diagnostic>), String> {
    let attempt_number = scope.attempt_number;
    let scope_key = scope.reconcile_key();
    let origin = store::MembershipOrigin {
        scope_key: &scope_key,
        source_path,
        attempt_number,
    };
    let existing = store::current_memberships(connection, &scope_key)?;
    // Tombstones belong to the attempt ceiling as well: a scope that projected
    // attempt 2 and then lost every finding must still refuse a rollback to 1.
    let highest = store::scope_attempt_ceiling(connection, &scope_key)?.max(
        existing
            .iter()
            .map(|(_, _, attempt, _)| *attempt)
            .max()
            .unwrap_or(i64::MIN),
    );
    let mut report = ReconcileReport::default();
    let mut diagnostics = Vec::new();
    if attempt_number < highest {
        // An older attempt may never roll back what a newer one already projected.
        report.stale_ignored = records.len();
        diagnostics.push(Diagnostic::warning(
            "older_attempt_ignored",
            &scope_key,
            format!("第 {attempt_number} 次结果早于已投影的第 {highest} 次，本轮只登记不覆盖"),
        ));
        return Ok((report, diagnostics));
    }
    if store::scan_marked_deleted(connection, &scope.scan_id)? {
        // The user deleted this task. Its files may still be on disk, but nothing may
        // put them back on the current projection surface (§12 Stage 2).
        report.stale_ignored = records.len();
        diagnostics.push(Diagnostic::warning(
            "deleted_scan_not_projected",
            &scope.scan_id,
            format!("任务已删除，本批 {} 条记录只登记不投影", records.len()),
        ));
        return Ok((report, diagnostics));
    }
    let mut touched = Vec::new();
    for record in records.values() {
        let envelope = envelope_for(record, envelope_json, limits);
        let (revision_id, fresh) = store::insert_revision(connection, record, &envelope)?;
        if fresh {
            report.revisions += 1;
        } else {
            report.unchanged += 1;
        }
        let current = existing
            .iter()
            .find(|(_, _, _, logical)| *logical == record.logical_key)
            .map(|(id, revision, attempt, _)| (*id, *revision, *attempt));
        match current {
            Some((_, existing_revision, _)) if existing_revision == revision_id => {
                // Same semantic content already projected: no second row.
            }
            Some((membership_id, _, _)) => {
                store::demote_membership(connection, membership_id)?;
                let adopted = if adopted_from_legacy(connection, &record.logical_key) {
                    report.adopted += 1;
                    "legacy_sentinel_findings"
                } else {
                    ""
                };
                store::add_membership(
                    connection,
                    &origin,
                    record,
                    revision_id,
                    bundle_row_id,
                    adopted,
                )?;
            }
            None => {
                let adopted = if adopted_from_legacy(connection, &record.logical_key) {
                    report.adopted += 1;
                    "legacy_sentinel_findings"
                } else {
                    ""
                };
                store::add_membership(
                    connection,
                    &origin,
                    record,
                    revision_id,
                    bundle_row_id,
                    adopted,
                )?;
            }
        }
        touched.push(record.logical_key.clone());
    }
    // §IDM-007: a key that this bundle no longer contains is revoked, and the
    // tombstone keeps an older bundle from bringing it back later.
    for (membership_id, revision_id, membership_attempt, logical) in &existing {
        if touched.contains(logical) || membership_attempt > &attempt_number {
            continue;
        }
        store::demote_membership(connection, *membership_id)?;
        store::tombstone(connection, &origin, logical, *revision_id, bundle_row_id)?;
        report.revoked += 1;
    }
    Ok((report, diagnostics))
}

/// §COR-008: credential values never reach a database column. The stored envelope
/// keeps field names, hashes and provenance, and the byte-identical original stays
/// in the content-addressed object store.
fn envelope_for(
    record: &CanonicalRecord,
    envelope_json: &dyn Fn(&CanonicalRecord) -> String,
    limits: &super::limits::Limits,
) -> String {
    let text = envelope_json(record);
    let redacted = serde_json::from_str::<serde_json::Value>(&text)
        .map(|value| crate::agent_runtime::secrets::redact_json(&value).to_string())
        .unwrap_or(text.clone());
    if redacted == text {
        return text;
    }
    redacted.chars().take(limits.text_chars).collect()
}
