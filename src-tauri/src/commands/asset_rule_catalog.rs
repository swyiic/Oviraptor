//! Read-only inventory of project page rules and legacy content rules.
use crate::{db, AppState};
use tauri::State;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedContentRule {
    id: i64,
    keyword: String,
    enabled: bool,
    created_at: String,
    example_title: String,
    example_url: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetRuleCatalog {
    project_rules: Vec<crate::page_isolation::Rule>,
    legacy_rules: Vec<SavedContentRule>,
}

pub(crate) fn asset_rule_catalog_on(connection: &rusqlite::Connection) -> Result<AssetRuleCatalog, String> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let projects = {
        let mut s = transaction
            .prepare("SELECT DISTINCT project_id FROM asset_page_rules ORDER BY project_id")
            .map_err(|e| e.to_string())?;
        let rows = s
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    let mut project_rules = Vec::new();
    for project in projects {
        project_rules.extend(crate::page_isolation::list_on(&transaction, project)?);
    }
    let legacy_rules = {
        let mut s=transaction.prepare("SELECT r.id,r.keyword,r.enabled,r.created_at,COALESCE(a.title,''),COALESCE(NULLIF(a.link,''),a.host,'') FROM content_rules r LEFT JOIN assets a ON a.id=r.source_asset_id ORDER BY r.id DESC").map_err(|e|e.to_string())?;
        let rows = s
            .query_map([], |r| {
                Ok(SavedContentRule {
                    id: r.get(0)?,
                    keyword: r.get(1)?,
                    enabled: r.get(2)?,
                    created_at: r.get(3)?,
                    example_title: r.get(4)?,
                    example_url: r.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(AssetRuleCatalog {
        project_rules,
        legacy_rules,
    })
}

#[tauri::command]
pub async fn get_asset_rule_catalog(
    state: State<'_, AppState>,
) -> Result<AssetRuleCatalog, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || asset_rule_catalog_on(&db::open(&path)?))
        .await
        .map_err(|e| e.to_string())?
}

