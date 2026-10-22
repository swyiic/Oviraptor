//! Read-only application overview across project-owned records.
//! Aggregation belongs to the shell rather than to a specific business module.
use crate::{db, models::DashboardStats, AppState};
use tauri::State;

fn quarantine_count_on(
    connection: &rusqlite::Connection,
    project_id: Option<i64>,
) -> Result<i64, String> {
    // The navigation opens the quarantine list including recycled rows. Reuse
    // that exact owner predicate, rather than add content/page totals that can
    // omit recycled content or count a row twice.
    let query = crate::asset_models::AssetQuery {
        project_id,
        deleted_view: "all".into(),
        probe_view: "blocked".into(),
        ..Default::default()
    };
    let (predicate, values) = super::assets::asset_filter(&query, false);
    connection.query_row(&format!("SELECT COUNT(*) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE {predicate}"),
        rusqlite::params_from_iter(values),|r|r.get(0)).map_err(|e|e.to_string())
}

#[tauri::command]
pub async fn dashboard_stats(
    state: State<'_, AppState>,
    project_id: Option<i64>,
) -> Result<DashboardStats, String> {
    let connection = db::open(&state.db_path)?;
    let project_count = connection
        .query_row(
            "SELECT COUNT(*) FROM projects WHERE status='active'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let running_jobs = connection
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE status IN ('queued','running','cancel_requested')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let (asset_count, alive_count, pending_count): (i64, i64, i64) = connection
        .query_row(
            r#"SELECT COUNT(*),
                      COALESCE(SUM(CASE WHEN a.probe_outcome IN ('web_alive','web_restricted','browser_render_required','virtual_host_required') AND COALESCE(a.content_category,'') NOT IN ('gambling','porn','custom_rule') THEN 1 ELSE 0 END),0),
                      COALESCE(SUM(CASE WHEN pa.decision IN ('pending','uncertain','') THEN 1 ELSE 0 END),0)
               FROM project_assets pa JOIN assets a ON a.id=pa.asset_id
               WHERE pa.is_deleted=0 AND (?1 IS NULL OR pa.project_id=?1)"#,
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| error.to_string())?;
    let blocked_count = quarantine_count_on(&connection, project_id)?;
    let (new_count, changed_count) = connection
        .query_row(
            r#"SELECT COALESCE(SUM(CASE WHEN event_type='new' THEN 1 ELSE 0 END),0),
                      COALESCE(SUM(CASE WHEN event_type='changed' THEN 1 ELSE 0 END),0)
               FROM asset_events
               WHERE created_at>=datetime('now','-30 day') AND (?1 IS NULL OR project_id=?1)"#,
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    Ok(DashboardStats {
        project_count,
        asset_count,
        alive_count,
        pending_count,
        new_count,
        changed_count,
        blocked_count,
        running_jobs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quarantine_navigation_count_matches_full_list_including_recycled_content_and_page_rules() {
        let root =
            std::env::temp_dir().join(format!("ovi-quarantine-count-{}", uuid::Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        let c = db::open(&path).unwrap();
        c.execute(
            "INSERT INTO projects(id,name) VALUES(9101,'Fixture'),(9102,'Other')",
            [],
        )
        .unwrap();
        for (id, deleted, content) in [
            (9101, 0, "blocked_content"),
            (9102, 1, "blocked_content"),
            (9103, 1, "web_alive"),
            (9104, 0, "web_alive"),
        ] {
            c.execute(
                "INSERT INTO assets(id,asset_key,host,probe_outcome) VALUES(?1,?2,?2,?3)",
                rusqlite::params![id, format!("fixture-{id}.test"), content],
            )
            .unwrap();
            c.execute(
                "INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(9101,?1,?2)",
                rusqlite::params![id, deleted],
            )
            .unwrap();
        }
        // A recycled page-rule row is counted once even when its content also
        // becomes blocked. A plain manually recycled clean row is excluded.
        c.execute(
            "UPDATE project_assets SET decision='rejected',note='fixture-rule' WHERE asset_id=9103",
            [],
        )
        .unwrap();
        c.execute("INSERT INTO asset_page_rules(id,project_id,kind,value,enabled) VALUES(9101,9101,'title_contains','fixture',1)",[]).unwrap();
        c.execute("INSERT INTO asset_page_rule_effects(rule_id,project_id,asset_id,active,original_decision,original_note,applied_note) VALUES(9101,9101,9103,1,'pending','','fixture-rule')",[]).unwrap();
        c.execute(
            "INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(9102,9102,1)",
            [],
        )
        .unwrap();
        let query = crate::asset_models::AssetQuery {
            project_id: Some(9101),
            deleted_view: "all".into(),
            probe_view: "blocked".into(),
            page: 1,
            page_size: 50,
            ..Default::default()
        };
        assert_eq!(quarantine_count_on(&c, Some(9101)).unwrap(), 3);
        assert_eq!(
            quarantine_count_on(&c, Some(9101)).unwrap(),
            super::super::assets::list_assets_at(&path, &query)
                .unwrap()
                .total
        );
        assert_eq!(quarantine_count_on(&c, None).unwrap(), 4);
        c.execute(
            "UPDATE assets SET probe_outcome='blocked_content' WHERE id=9103",
            [],
        )
        .unwrap();
        assert_eq!(quarantine_count_on(&c, Some(9101)).unwrap(), 3);
        drop(c);
        let _ = std::fs::remove_dir_all(root);
    }
}
