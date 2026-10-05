/// A deliberately small, redacted read model of the canonical historical
/// projection. It is NOT a SentinelFinding: imported claims are unreviewed and
/// cannot be used by the native Reviewer or as executable coverage credit.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalImportPreview {
    pub membership_id: i64,
    pub scan_id: String,
    pub attempt_number: i64,
    pub kind: String,
    pub title: String,
    pub severity: String,
    pub target: String,
    pub producer: String,
    pub review_state: String,
    pub read_only: bool,
    pub execution_eligible: bool,
}

/// Read current canonical memberships for the exact task identity only.
/// Shared source directories do not establish identity aliases.
/// Full envelopes and CAS originals are never returned by this surface because
/// they can contain historical credential material.
fn historical_import_previews(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Vec<HistoricalImportPreview>, String> {
    let mut statement = connection.prepare(
        "SELECT m.id,m.attempt_number,r.record_kind, \
             CASE WHEN json_type(r.envelope_json,'$.payload.title')='text' THEN json_extract(r.envelope_json,'$.payload.title') ELSE '' END, \
             CASE WHEN json_type(r.envelope_json,'$.payload.severity')='text' THEN json_extract(r.envelope_json,'$.payload.severity') ELSE '' END, \
             CASE WHEN json_type(r.envelope_json,'$.payload.endpoint')='text' THEN json_extract(r.envelope_json,'$.payload.endpoint') \
                  WHEN json_type(r.envelope_json,'$.payload.target')='text' THEN json_extract(r.envelope_json,'$.payload.target') ELSE '' END, \
             CASE WHEN json_type(r.envelope_json,'$.producer.name')='text' THEN json_extract(r.envelope_json,'$.producer.name') ELSE '' END \
         FROM import_projection_memberships m \
         JOIN import_record_revisions r ON r.id=m.revision_id \
         JOIN import_bundles b ON b.id=m.bundle_row_id \
         JOIN sentinel_scans s ON s.id=?1 \
         WHERE m.current=1 AND m.tombstone=0 \
           AND b.status IN ('imported','unchanged') \
           AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d \
               WHERE d.scan_id=s.id) \
           AND m.scope_key='scan='||s.id \
           AND r.record_kind IN ('finding_candidate','coverage','run_state','evidence_note') \
           AND json_extract(r.envelope_json,'$.canonicalSchema')='oviraptor.artifact.v1' \
           AND json_type(r.envelope_json,'$.claim.readOnly')='true' \
           AND json_type(r.envelope_json,'$.claim.executionEligible')='false' \
           AND json_extract(r.envelope_json,'$.claim.authority')='historical_external' \
           AND json_extract(r.envelope_json,'$.claim.reviewState')='unreviewed' \
           AND json_extract(r.envelope_json,'$.claim.readOnly')=1 \
           AND json_extract(r.envelope_json,'$.claim.executionEligible')=0 \
         ORDER BY m.attempt_number DESC,m.id DESC LIMIT 300"
    ).map_err(|error| format!("无法准备历史预览：{error}"))?;
    let rows = statement
        .query_map([scan_id], |row| {
            let clean =
                |value: String| crate::agent_runtime::secrets::redact_text_with(&value, None);
            Ok(HistoricalImportPreview {
                membership_id: row.get(0)?,
                scan_id: scan_id.to_string(),
                attempt_number: row.get(1)?,
                kind: row.get(2)?,
                title: clean(row.get(3)?),
                severity: clean(row.get(4)?),
                target: clean(row.get(5)?),
                producer: clean(row.get(6)?),
                review_state: "unreviewed".into(),
                read_only: true,
                execution_eligible: false,
            })
        })
        .map_err(|error| format!("无法读取历史预览：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析历史预览：{error}"))?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_historical_import_previews(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<Vec<HistoricalImportPreview>, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|error| format!("历史预览只能读取现有数据库：{error}"))?;
        historical_import_previews(&connection, &scan_id)
    })
    .await
    .map_err(|error| format!("历史预览线程失败：{error}"))?
}

/// The task list used to depend on the old result synchronizer creating a
/// `sentinel_scans` row. Canonical imports must remain discoverable even when
/// that writer is removed. This is a separate, read-only ledger: no imported
/// claim is represented as a Native scan or a confirmed finding.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalImportRun {
    pub row_id: i64,
    pub bundle_id: String,
    pub scan_id: String,
    pub attempt_number: i64,
    pub title: String,
    pub status: String,
    pub finding_candidates: i64,
    pub coverage_records: i64,
    pub imported_at: String,
}

fn historical_import_runs(
    connection: &rusqlite::Connection,
    before_id: Option<i64>,
) -> Result<Vec<HistoricalImportRun>, String> {
    let mut statement = connection.prepare(
        "SELECT MAX(m.id),b.bundle_id,substr(m.scope_key,6),MAX(m.attempt_number), \
            COALESCE(MAX(CASE WHEN r.record_kind='run_state' AND json_type(r.envelope_json,'$.payload.run_name')='text' \
                THEN json_extract(r.envelope_json,'$.payload.run_name') END),''), \
            COALESCE(MAX(CASE WHEN r.record_kind='run_state' AND json_type(r.envelope_json,'$.payload.status')='text' \
                THEN json_extract(r.envelope_json,'$.payload.status') END),'unknown'), \
            SUM(CASE WHEN r.record_kind='finding_candidate' THEN 1 ELSE 0 END), \
            SUM(CASE WHEN r.record_kind='coverage' THEN 1 ELSE 0 END),b.last_import_at \
         FROM import_bundles b \
         JOIN import_projection_memberships m ON m.bundle_row_id=b.id \
         JOIN import_record_revisions r ON r.id=m.revision_id \
         WHERE b.status IN ('imported','unchanged') \
           AND m.current=1 AND m.tombstone=0 AND m.scope_key LIKE 'scan=%' \
           AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d \
               WHERE d.scan_id=substr(m.scope_key,6)) \
           AND json_extract(r.envelope_json,'$.canonicalSchema')='oviraptor.artifact.v1' \
           AND json_type(r.envelope_json,'$.claim.readOnly')='true' \
           AND json_type(r.envelope_json,'$.claim.executionEligible')='false' \
           AND json_extract(r.envelope_json,'$.claim.authority')='historical_external' \
           AND json_extract(r.envelope_json,'$.claim.reviewState')='unreviewed' \
           AND json_extract(r.envelope_json,'$.claim.readOnly')=1 \
           AND json_extract(r.envelope_json,'$.claim.executionEligible')=0 \
         GROUP BY b.id,m.scope_key HAVING MAX(m.id) < ?1 \
         ORDER BY MAX(m.id) DESC LIMIT 51"
    ).map_err(|error| format!("无法准备历史导入列表：{error}"))?;
    let rows = statement
        .query_map([before_id.unwrap_or(i64::MAX)], |row| {
            let title: String = row.get(4)?;
            Ok(HistoricalImportRun {
                row_id: row.get(0)?,
                bundle_id: row.get(1)?,
                scan_id: crate::agent_runtime::secrets::redact_text_with(
                    &row.get::<_, String>(2)?,
                    None,
                ),
                attempt_number: row.get(3)?,
                title: crate::agent_runtime::secrets::redact_text_with(&title, None),
                status: crate::agent_runtime::secrets::redact_text_with(
                    &row.get::<_, String>(5)?,
                    None,
                ),
                finding_candidates: row.get(6)?,
                coverage_records: row.get(7)?,
                imported_at: row.get(8)?,
            })
        })
        .map_err(|error| format!("无法读取历史导入列表：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析历史导入列表：{error}"))?;
    Ok(rows)
}

/// Bundle identity is the canonical manifest hash, not an arbitrary source
/// path. A caller can only see current, non-revoked, historical claims in it.
fn historical_bundle_previews(
    connection: &rusqlite::Connection,
    bundle_id: &str,
    projection_id: i64,
) -> Result<Vec<HistoricalImportPreview>, String> {
    if bundle_id.len() != 71
        || !bundle_id.starts_with("sha256:")
        || !bundle_id[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("无效的历史导入标识".into());
    }
    let mut statement = connection.prepare(
        "SELECT m.id,substr(m.scope_key,6),m.attempt_number,r.record_kind, \
             CASE WHEN json_type(r.envelope_json,'$.payload.title')='text' THEN json_extract(r.envelope_json,'$.payload.title') ELSE '' END, \
             CASE WHEN json_type(r.envelope_json,'$.payload.severity')='text' THEN json_extract(r.envelope_json,'$.payload.severity') ELSE '' END, \
             CASE WHEN json_type(r.envelope_json,'$.payload.endpoint')='text' THEN json_extract(r.envelope_json,'$.payload.endpoint') \
                  WHEN json_type(r.envelope_json,'$.payload.target')='text' THEN json_extract(r.envelope_json,'$.payload.target') ELSE '' END, \
             CASE WHEN json_type(r.envelope_json,'$.producer.name')='text' THEN json_extract(r.envelope_json,'$.producer.name') ELSE '' END \
         FROM import_bundles b \
         JOIN import_projection_memberships m ON m.bundle_row_id=b.id \
         JOIN import_record_revisions r ON r.id=m.revision_id \
         WHERE b.bundle_id=?1 AND b.status IN ('imported','unchanged') \
           AND m.scope_key=(SELECT selected.scope_key FROM import_projection_memberships selected \
               WHERE selected.id=?2 AND selected.bundle_row_id=b.id AND selected.current=1 AND selected.tombstone=0) \
           AND m.current=1 AND m.tombstone=0 AND m.scope_key LIKE 'scan=%' \
           AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d \
               WHERE d.scan_id=substr(m.scope_key,6)) \
           AND r.record_kind IN ('finding_candidate','coverage','run_state','evidence_note') \
           AND json_extract(r.envelope_json,'$.canonicalSchema')='oviraptor.artifact.v1' \
           AND json_type(r.envelope_json,'$.claim.readOnly')='true' \
           AND json_type(r.envelope_json,'$.claim.executionEligible')='false' \
           AND json_extract(r.envelope_json,'$.claim.authority')='historical_external' \
           AND json_extract(r.envelope_json,'$.claim.reviewState')='unreviewed' \
           AND json_extract(r.envelope_json,'$.claim.readOnly')=1 \
           AND json_extract(r.envelope_json,'$.claim.executionEligible')=0 \
         ORDER BY m.id DESC LIMIT 300"
    ).map_err(|error| format!("无法准备历史产物预览：{error}"))?;
    let rows = statement
        .query_map(rusqlite::params![bundle_id, projection_id], |row| {
            let clean =
                |value: String| crate::agent_runtime::secrets::redact_text_with(&value, None);
            Ok(HistoricalImportPreview {
                membership_id: row.get(0)?,
                scan_id: row.get(1)?,
                attempt_number: row.get(2)?,
                kind: row.get(3)?,
                title: clean(row.get(4)?),
                severity: clean(row.get(5)?),
                target: clean(row.get(6)?),
                producer: clean(row.get(7)?),
                review_state: "unreviewed".into(),
                read_only: true,
                execution_eligible: false,
            })
        })
        .map_err(|error| format!("无法读取历史产物预览：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析历史产物预览：{error}"))?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_historical_import_runs(
    state: State<'_, AppState>,
    before_id: Option<i64>,
) -> Result<Vec<HistoricalImportRun>, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|error| format!("历史导入列表只能读取现有数据库：{error}"))?;
        historical_import_runs(&connection, before_id)
    })
    .await
    .map_err(|error| format!("历史导入列表线程失败：{error}"))?
}

#[tauri::command]
pub async fn list_historical_bundle_previews(
    state: State<'_, AppState>,
    bundle_id: String,
    projection_id: i64,
) -> Result<Vec<HistoricalImportPreview>, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|error| format!("历史产物预览只能读取现有数据库：{error}"))?;
        historical_bundle_previews(&connection, &bundle_id, projection_id)
    })
    .await
    .map_err(|error| format!("历史产物预览线程失败：{error}"))?
}
