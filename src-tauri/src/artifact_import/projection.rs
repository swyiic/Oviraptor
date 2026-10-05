//! §12 Stage 2 — the canonical importer runs *beside* the legacy one, so every run
//! ends with a comparison rather than a takeover. The legacy writer keeps its rows;
//! nothing here mutates `sentinel_*`.

use rusqlite::Connection;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShadowReport {
    pub canonical_candidates: usize,
    pub legacy_findings: usize,
    pub matched: usize,
    pub only_canonical: usize,
    pub only_legacy: usize,
}

fn count(connection: &Connection, sql: &str) -> Result<usize, String> {
    connection
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .map(|value| value.max(0) as usize)
        .map_err(|error| format!("无法完成 shadow compare：{error}"))
}

pub fn shadow_compare(connection: &Connection) -> Result<ShadowReport, String> {
    let canonical = count(
        connection,
        "SELECT COUNT(DISTINCT r.logical_key)
         FROM import_projection_memberships m
         JOIN import_record_revisions r ON r.id=m.revision_id
         WHERE m.current=1 AND r.record_kind='finding_candidate'",
    )?;
    let legacy = count(
        connection,
        "SELECT COUNT(*) FROM sentinel_findings WHERE kind='vulnerability'",
    )?;
    let matched = count(
        connection,
        "SELECT COUNT(DISTINCT r.logical_key)
         FROM import_projection_memberships m
         JOIN import_record_revisions r ON r.id=m.revision_id
         JOIN sentinel_findings f ON f.record_key=r.logical_key
         WHERE m.current=1 AND r.record_kind='finding_candidate'",
    )?;
    Ok(ShadowReport {
        canonical_candidates: canonical,
        legacy_findings: legacy,
        matched,
        only_canonical: canonical.saturating_sub(matched),
        only_legacy: legacy.saturating_sub(matched),
    })
}
