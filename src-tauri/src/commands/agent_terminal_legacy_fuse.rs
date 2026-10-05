// Same original target's protection projection, in the caller's private transaction.
fn terminal_legacy_fuse(
    db: &rusqlite::Connection,
    scan: &str,
    url: &str,
    reason: &str,
) -> Result<(), String> {
    let (project,asset,company,target):(i64,Option<i64>,String,String)=db.query_row("SELECT project_id,asset_id,company,url FROM sentinel_targets WHERE scan_id=?1 AND url=?2",params![scan,url],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
    let normalized = normalized_fuse_url(&target);
    let old_asset: Option<i64> = db
        .query_row(
            "SELECT asset_id FROM sentinel_fuse_zone WHERE project_id=?1 AND normalized_url=?2",
            params![project, normalized],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let asset = asset.or(old_asset);
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_fuse_zone WHERE project_id=?1 AND normalized_url=?2 AND asset_id IS ?3 AND company=?4 AND url=?5 AND source_scan_id=?6 AND reason=?7 AND verdict='pending' AND note='' AND evidence='' AND archived=0)",params![project,normalized,asset,company,target,scan,reason],|r|r.get(0)).map_err(|e|e.to_string())?;
    if exact {
        return Ok(());
    }
    let stamp: String = db
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let changed=db.execute("INSERT INTO sentinel_fuse_zone(project_id,asset_id,company,url,normalized_url,source_scan_id,reason,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(project_id,normalized_url) DO UPDATE SET asset_id=excluded.asset_id,company=excluded.company,url=excluded.url,source_scan_id=excluded.source_scan_id,reason=excluded.reason,verdict='pending',note='',evidence='',archived=0,updated_at=excluded.updated_at",params![project,asset,company,target,normalized,scan,reason,stamp]).map_err(|e|e.to_string())?;
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_fuse_zone WHERE project_id=?1 AND normalized_url=?2 AND asset_id IS ?3 AND company=?4 AND url=?5 AND source_scan_id=?6 AND reason=?7 AND verdict='pending' AND note='' AND evidence='' AND archived=0 AND updated_at=?8)",params![project,normalized,asset,company,target,scan,reason,stamp],|r|r.get(0)).map_err(|e|e.to_string())?;
    if changed != 1 || !exact {
        return Err("terminal_legacy_fuse_write_conflict".into());
    }
    Ok(())
}
