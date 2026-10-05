const VIEW_DIAGNOSTICS: usize = 200;
const VIEW_BUNDLES: usize = 50;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactImportBundle {
    source_path: String,
    bundle_id: String,
    status: String,
    records: usize,
    revisions: usize,
    revoked: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactImportDiagnostic {
    code: String,
    severity: String,
    source_path: String,
    record_pointer: String,
    detail: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactImportReport {
    bundles: usize,
    imported: usize,
    unchanged: usize,
    failed: usize,
    records: usize,
    revisions: usize,
    revoked: usize,
    sealed_objects: usize,
    canonical_candidates: usize,
    legacy_findings: usize,
    matched: usize,
    only_canonical: usize,
    only_legacy: usize,
    bundle_rows: Vec<ArtifactImportBundle>,
    diagnostics: Vec<ArtifactImportDiagnostic>,
}

fn count(connection: &rusqlite::Connection, sql: &str) -> usize {
    connection
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .unwrap_or(0)
        .max(0) as usize
}

fn status_count(connection: &rusqlite::Connection, status: &str) -> usize {
    connection
        .query_row(
            "SELECT COUNT(*) FROM import_bundles WHERE status=?1",
            [status],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as usize
}

/// Originals that were written sealed (§9.4). Counts only, never the bytes.
fn sealed_count(connection: &rusqlite::Connection) -> usize {
    count(
        connection,
        "SELECT COUNT(*) FROM artifact_objects WHERE secret_material=1",
    )
}

impl ArtifactImportReport {
    #[cfg(test)]
    fn build(
        summary: &crate::artifact_import::ImportSummary,
        connection: &rusqlite::Connection,
    ) -> Self {
        let counted = |status: crate::artifact_import::BundleStatus| {
            summary
                .outcomes
                .iter()
                .filter(|outcome| outcome.status == status)
                .count()
        };
        let mut diagnostics: Vec<ArtifactImportDiagnostic> = summary
            .outcomes
            .iter()
            .flat_map(|outcome| outcome.diagnostics.iter())
            .chain(summary.discovery_diagnostics.iter())
            .map(|diagnostic| ArtifactImportDiagnostic {
                code: diagnostic.code.clone(),
                severity: diagnostic.severity.as_str().to_string(),
                source_path: diagnostic.source_path.clone(),
                record_pointer: diagnostic.record_pointer.clone(),
                detail: diagnostic.detail.clone(),
            })
            .collect();
        diagnostics.sort_by(|left, right| {
            (&left.code, &left.source_path, &left.record_pointer).cmp(&(
                &right.code,
                &right.source_path,
                &right.record_pointer,
            ))
        });
        let sealed_objects = sealed_count(connection);
        Self {
            bundles: summary.outcomes.len(),
            imported: counted(crate::artifact_import::BundleStatus::Imported),
            unchanged: counted(crate::artifact_import::BundleStatus::Unchanged),
            failed: counted(crate::artifact_import::BundleStatus::Failed),
            records: summary.outcomes.iter().map(|outcome| outcome.records).sum(),
            revisions: summary
                .outcomes
                .iter()
                .map(|outcome| outcome.revisions)
                .sum(),
            revoked: summary.outcomes.iter().map(|outcome| outcome.revoked).sum(),
            sealed_objects,
            canonical_candidates: summary.shadow.canonical_candidates,
            legacy_findings: summary.shadow.legacy_findings,
            matched: summary.shadow.matched,
            only_canonical: summary.shadow.only_canonical,
            only_legacy: summary.shadow.only_legacy,
            bundle_rows: summary
                .outcomes
                .iter()
                .map(|outcome| ArtifactImportBundle {
                    source_path: outcome.source_path.clone(),
                    bundle_id: outcome.bundle_id.clone(),
                    status: outcome.status.as_str().to_string(),
                    records: outcome.records,
                    revisions: outcome.revisions,
                    revoked: outcome.revoked,
                })
                .collect(),
            diagnostics,
        }
    }

    /// Everything here is a SELECT. The shadow comparison is read-only too, so
    /// asking for status can never advance it (§IDM-013, 缺口 6).
    fn view(connection: &rusqlite::Connection) -> Result<Self, String> {
        let mut statement = connection
            .prepare(
                "SELECT b.source_path,b.bundle_id,b.status,b.record_count,b.revision_count,
                        (SELECT COUNT(*) FROM import_projection_memberships t
                          WHERE t.bundle_row_id=b.id AND t.tombstone=1)
                 FROM import_bundles b ORDER BY b.id DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let bundle_rows = statement
            .query_map([VIEW_BUNDLES as i64], |row| {
                Ok(ArtifactImportBundle {
                    source_path: row.get(0)?,
                    bundle_id: row.get(1)?,
                    status: row.get(2)?,
                    records: row.get::<_, i64>(3)? as usize,
                    revisions: row.get::<_, i64>(4)? as usize,
                    revoked: row.get::<_, i64>(5)? as usize,
                })
            })
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .collect();
        let mut statement = connection
            .prepare(
                "SELECT code,severity,source_path,record_pointer,detail
                 FROM import_diagnostics ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let diagnostics = statement
            .query_map([VIEW_DIAGNOSTICS as i64], |row| {
                Ok(ArtifactImportDiagnostic {
                    code: row.get(0)?,
                    severity: row.get(1)?,
                    source_path: row.get(2)?,
                    record_pointer: row.get(3)?,
                    detail: row.get(4)?,
                })
            })
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .collect();
        let shadow = crate::artifact_import::shadow_report(connection).unwrap_or_default();
        Ok(Self {
            bundles: count(connection, "SELECT COUNT(*) FROM import_bundles"),
            imported: status_count(connection, "imported"),
            unchanged: status_count(connection, "unchanged"),
            failed: status_count(connection, "failed"),
            records: count(
                connection,
                "SELECT COALESCE(SUM(record_count),0) FROM import_bundles",
            ),
            revisions: count(connection, "SELECT COUNT(*) FROM import_record_revisions"),
            revoked: count(
                connection,
                "SELECT COUNT(*) FROM import_projection_memberships WHERE tombstone=1",
            ),
            sealed_objects: sealed_count(connection),
            canonical_candidates: shadow.canonical_candidates,
            legacy_findings: shadow.legacy_findings,
            matched: shadow.matched,
            only_canonical: shadow.only_canonical,
            only_legacy: shadow.only_legacy,
            bundle_rows,
            diagnostics,
        })
    }
}

/// Read-only status of the canonical historical import, including the shadow compare
/// against the legacy writer (§12 Stage 2: the new importer is not yet the only one).
/// The connection itself is opened `READ_ONLY`, so SQLite refuses even an accidental
/// write here (缺口 6).
#[tauri::command]
pub async fn get_artifact_import_status(
    state: State<'_, AppState>,
) -> Result<ArtifactImportReport, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|error| format!("历史导入状态只能读取现有数据库：{error}"))?;
        ArtifactImportReport::view(&connection)
    })
    .await
    .map_err(|error| format!("历史导入状态线程失败：{error}"))?
}
