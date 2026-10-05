//! Rows for the canonical import (§9.9). Original revisions are append-only: the
//! schema itself refuses an UPDATE or DELETE, so "history is kept" is enforced by the
//! database rather than by convention.

use crate::artifact_import::canonical::CanonicalRecord;
use crate::artifact_import::diagnostics::Diagnostic;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleRow {
    pub id: i64,
    pub bundle_id: String,
    pub source_path: String,
    pub signature: String,
}

/// Every membership must retain the source directory that supplied it, even
/// when another directory later reuses the same content-addressed bundle.
pub struct MembershipOrigin<'a> {
    pub scope_key: &'a str,
    pub source_path: &'a Path,
    pub attempt_number: i64,
}

pub fn upsert_source(connection: &Connection, root: &Path) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO import_sources(root_path) VALUES(?1) \
             ON CONFLICT(root_path) DO UPDATE SET last_seen_at=datetime('now','localtime')",
            [root.to_string_lossy().as_ref()],
        )
        .map_err(|error| format!("无法记录导入来源：{error}"))?;
    Ok(())
}

pub fn find_bundle(connection: &Connection, bundle_id: &str) -> Result<Option<BundleRow>, String> {
    connection
        .query_row(
            "SELECT id,bundle_id,source_path,signature FROM import_bundles WHERE bundle_id=?1",
            [bundle_id],
            |row| {
                Ok(BundleRow {
                    id: row.get(0)?,
                    bundle_id: row.get(1)?,
                    source_path: row.get(2)?,
                    signature: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("无法查询已导入 bundle：{error}"))
}

#[allow(clippy::too_many_arguments)]
pub fn write_bundle(
    connection: &Connection,
    bundle_id: &str,
    source_path: &Path,
    root_path: &Path,
    attempt_key: &str,
    attempt_number: i64,
    signature: &str,
    status: &str,
    record_count: usize,
    revision_count: usize,
) -> Result<i64, String> {
    connection
        .execute(
            "INSERT INTO import_bundles(
                bundle_id,source_path,root_path,attempt_key,attempt_number,signature,status,
                record_count,revision_count,first_seen_at,last_import_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,datetime('now','localtime'),datetime('now','localtime'))
             ON CONFLICT(bundle_id) DO UPDATE SET
                source_path=excluded.source_path,
                root_path=excluded.root_path,
                attempt_key=excluded.attempt_key,
                attempt_number=excluded.attempt_number,
                signature=excluded.signature,
                status=excluded.status,
                record_count=excluded.record_count,
                revision_count=excluded.revision_count,
                last_import_at=datetime('now','localtime')",
            params![
                bundle_id,
                source_path.to_string_lossy(),
                root_path.to_string_lossy(),
                attempt_key,
                attempt_number,
                signature,
                status,
                record_count as i64,
                revision_count as i64
            ],
        )
        .map_err(|error| format!("无法写入 bundle 行：{error}"))?;
    let id: i64 = connection
        .query_row(
            "SELECT id FROM import_bundles WHERE bundle_id=?1",
            [bundle_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法读回 bundle 行：{error}"))?;
    Ok(id)
}

pub fn replace_bundle_files(
    connection: &Connection,
    bundle_row_id: i64,
    files: &[(String, String, u64, i64)],
) -> Result<(), String> {
    // Files are a projection of what the bundle contained at import time, so the
    // listing may be rebuilt; revisions never are.
    connection
        .execute(
            "DELETE FROM import_bundle_files WHERE bundle_row_id=?1",
            [bundle_row_id],
        )
        .map_err(|error| format!("无法清理 bundle 文件列表：{error}"))?;
    for (relative_path, content_hash, bytes, object_id) in files {
        connection
            .execute(
                "INSERT INTO import_bundle_files(bundle_row_id,relative_path,content_hash,bytes,artifact_object_id) \
                 VALUES(?1,?2,?3,?4,?5)",
                params![bundle_row_id, relative_path, content_hash, *bytes as i64, object_id],
            )
            .map_err(|error| format!("无法写入 bundle 文件 {relative_path}：{error}"))?;
    }
    Ok(())
}

pub fn upsert_object(
    connection: &Connection,
    content_hash: &str,
    bytes: u64,
    storage_path: &Path,
    display_text: &str,
    secret_material: bool,
) -> Result<i64, String> {
    connection
        .execute(
            "INSERT OR IGNORE INTO artifact_objects(content_hash,bytes,storage_path,display_text,secret_material) \
             VALUES(?1,?2,?3,?4,?5)",
            params![
                content_hash,
                bytes as i64,
                storage_path.to_string_lossy(),
                display_text,
                i64::from(secret_material)
            ],
        )
        .map_err(|error| format!("无法写入对象 {content_hash}：{error}"))?;
    connection
        .query_row(
            "SELECT id FROM artifact_objects WHERE content_hash=?1",
            [content_hash],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法读回对象 {content_hash}：{error}"))
}

/// Returns `(revision id, whether it was new)`. An existing revision is reused, so a
/// repeated import cannot add a second semantic row.
pub fn insert_revision(
    connection: &Connection,
    record: &CanonicalRecord,
    envelope_json: &str,
) -> Result<(i64, bool), String> {
    let inserted = connection
        .execute(
            "INSERT OR IGNORE INTO import_record_revisions(
                record_key,logical_key,record_kind,revision_hash,adapter,envelope_json
             ) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                record.logical_key,
                record.logical_key,
                record.record_kind.as_str(),
                record.revision_hash,
                record.provenance.import_adapter,
                envelope_json
            ],
        )
        .map_err(|error| format!("无法写入记录修订：{error}"))?;
    let id: i64 = connection
        .query_row(
            "SELECT id FROM import_record_revisions WHERE record_key=?1 AND revision_hash=?2",
            params![record.logical_key, record.revision_hash],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法读回记录修订：{error}"))?;
    Ok((id, inserted > 0))
}

/// Highest attempt number ever recorded in a scope, tombstones included. A scope
/// whose findings were all deleted still has to remember that attempt 2 happened, or
/// an older attempt re-imported later would resurrect them (§IDM-011).
pub fn scope_attempt_ceiling(connection: &Connection, scope_key: &str) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(attempt_number),-1) FROM import_projection_memberships WHERE scope_key=?1",
            [scope_key],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法读取作用域 attempt 上限：{error}"))
}

/// §12 Stage 2 缺口 5: a scan the user deleted must never gain a new current
/// projection, even while its result files are still on disk. Only the exact
/// task identity is fenced; producer aliases cannot hide another task.
pub fn scan_marked_deleted(connection: &Connection, scan_id: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT 1 FROM sentinel_deleted_scans \
             WHERE scan_id=?1 LIMIT 1",
            [scan_id],
            |_| Ok(true),
        )
        .optional()
        .map(|found| found.unwrap_or(false))
        .map_err(|error| format!("无法读取任务删除登记：{error}"))
}

/// `(membership id, revision id, attempt number)` currently projected in a scope.
pub fn current_memberships(
    connection: &Connection,
    scope_key: &str,
) -> Result<Vec<(i64, i64, i64, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT m.id, m.revision_id, m.attempt_number, r.logical_key
             FROM import_projection_memberships m
             JOIN import_record_revisions r ON r.id=m.revision_id
             WHERE m.scope_key=?1 AND m.current=1",
        )
        .map_err(|error| format!("无法读取投影成员：{error}"))?;
    let rows = statement
        .query_map([scope_key], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|error| format!("无法读取投影成员：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析投影成员：{error}"))
}

pub fn demote_membership(connection: &Connection, membership_id: i64) -> Result<(), String> {
    connection
        .execute(
            "UPDATE import_projection_memberships SET current=0, removed_at=datetime('now','localtime') \
             WHERE id=?1",
            [membership_id],
        )
        .map_err(|error| format!("无法撤销投影：{error}"))?;
    Ok(())
}

pub fn add_membership(
    connection: &Connection,
    origin: &MembershipOrigin<'_>,
    record: &CanonicalRecord,
    revision_id: i64,
    bundle_row_id: i64,
    adopted_from: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO import_projection_memberships(
                scope_key,source_path,record_key,logical_key,revision_id,bundle_row_id,attempt_number,current,tombstone,adopted_from
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,1,0,?8)",
            params![
                origin.scope_key,
                origin.source_path.to_string_lossy(),
                record.logical_key,
                record.logical_key,
                revision_id,
                bundle_row_id,
                origin.attempt_number,
                adopted_from
            ],
        )
        .map_err(|error| format!("无法写入投影成员：{error}"))?;
    Ok(())
}

/// A logical key that disappeared gets a tombstone so an older bundle re-imported
/// later cannot bring it back (§IDM-007/§IDM-011).
pub fn tombstone(
    connection: &Connection,
    origin: &MembershipOrigin<'_>,
    logical_key: &str,
    revision_id: i64,
    bundle_row_id: i64,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO import_projection_memberships(
                scope_key,source_path,record_key,logical_key,revision_id,bundle_row_id,attempt_number,current,tombstone
             ) VALUES(?1,?2,?3,?3,?4,?5,?6,0,1)",
            params![
                origin.scope_key,
                origin.source_path.to_string_lossy(),
                logical_key,
                revision_id,
                bundle_row_id,
                origin.attempt_number
            ],
        )
        .map_err(|error| format!("无法写入 tombstone：{error}"))?;
    Ok(())
}

pub fn record_diagnostic(
    connection: &Connection,
    bundle_row_id: Option<i64>,
    root_path: &str,
    diagnostic: &Diagnostic,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO import_diagnostics(bundle_row_id,root_path,source_path,code,severity,record_pointer,detail) \
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                bundle_row_id,
                root_path,
                diagnostic.source_path,
                diagnostic.code,
                diagnostic.severity.as_str(),
                diagnostic.record_pointer,
                bounded_detail(&diagnostic.detail)
            ],
        )
        .map_err(|error| format!("无法写入诊断：{error}"))?;
    Ok(())
}

/// Diagnostics are stored for the UI, so a credential inside a source error text is
/// masked and the row is capped.
fn bounded_detail(text: &str) -> String {
    let redacted = crate::agent_runtime::secrets::redact_text_with(text, None);
    redacted.chars().take(1200).collect()
}
