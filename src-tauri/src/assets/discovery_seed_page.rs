//! Native presentation endpoints. Reads never initialize or rewrite history;
//! the separate local preparation action builds only derived seed indexes.
use super::*;

pub(crate) fn read_on(c: &Connection, query: &seed_ledger::Query) -> Result<Value, String> {
    let mut value = seed_ledger::read(c, query)?;
    let p = value["projectId"]
        .as_i64()
        .ok_or("项目种子响应缺少项目范围")?;
    let history = rounds::history(c, p, 1, 10)?;
    value["rounds"] = history["items"].clone();
    value["roundsTotal"] = history["total"].clone();
    Ok(value)
}
#[tauri::command]
pub async fn list_project_seeds(
    state: tauri::State<'_, AppState>,
    query: seed_ledger::Query,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || read_on(&db::open(&path)?, &query))
        .await
        .map_err(err)?
}
#[tauri::command]
pub async fn read_project_seed(
    state: tauri::State<'_, AppState>,
    project_id: i64,
    seed_id: i64,
    page: Option<i64>,
    page_size: Option<i64>,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        seed_ledger::detail(
            &db::open(&path)?,
            project_id,
            seed_id,
            page.unwrap_or(1),
            page_size.unwrap_or(20),
        )
    })
    .await
    .map_err(err)?
}
pub(crate) fn prepare_on(path: &Path, p: i64) -> Result<Value, String> {
    let c = db::open(path)?;
    rounds::read_project(&c, p)?;
    // Every batch owns a short transaction. No source table, model request,
    // configuration, authorization, network or paid receipt is changed.
    for _ in 0..2000 {
        let state = seed_ledger::initialize(&c, p)?;
        if state["initialized"] == true || state["hasMore"] == false {
            return seed_ledger::read(
                &c,
                &serde_json::from_value(json!({"projectId":p,"page":1,"pageSize":50}))
                    .map_err(err)?,
            );
        }
    }
    Err("历史种子仍在本地整理；已保存断点，下次可继续".into())
}
#[tauri::command]
pub async fn prepare_project_seeds(
    state: tauri::State<'_, AppState>,
    project_id: i64,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || prepare_on(&path, project_id))
        .await
        .map_err(err)?
}
#[tauri::command]
pub async fn list_project_seed_rounds(
    state: tauri::State<'_, AppState>,
    project_id: i64,
    page: Option<i64>,
    page_size: Option<i64>,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut value = rounds::history(
            &db::open(&path)?,
            project_id,
            page.unwrap_or(1),
            page_size.unwrap_or(10),
        )?;
        value["projectId"] = json!(project_id);
        Ok(value)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
#[path="discovery_seed_page_tests.rs"]
mod tests;
