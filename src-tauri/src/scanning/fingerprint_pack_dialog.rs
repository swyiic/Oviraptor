//! Native save dialog for identification-only exports. No database access.
use std::io::Write;
use std::path::Path;
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub async fn scan_save_fingerprint_pack(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    scope: String,
) -> Result<serde_json::Value, String> {
    let data_dir = state.app_data_dir.clone();
    let export = super::fingerprint_pack::scan_export_fingerprint_pack(state, scope).await?;
    tauri::async_runtime::spawn_blocking(move || {
        let file_name = export["fileName"].as_str().ok_or("缺少导出文件名")?;
        let Some(file) = app
            .dialog()
            .file()
            .add_filter("Oviraptor fingerprint JSON", &["json"])
            .set_file_name(file_name)
            .blocking_save_file()
        else {
            return Ok(serde_json::json!({"cancelled": true}));
        };
        let path = file.into_path().map_err(|error| error.to_string())?;
        let content = export["content"].as_str().ok_or("缺少导出内容")?;
        save_export(&path, &data_dir, content)?;
        Ok(
            serde_json::json!({"cancelled": false, "path": path.to_string_lossy(),
            "count": export["count"], "sha256": export["sha256"]}),
        )
    })
    .await
    .map_err(|error| format!("导出任务失败：{error}"))?
}

fn save_export(path: &Path, data_dir: &Path, content: &str) -> Result<(), String> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Err("指纹包必须保存为 .json 文件".into());
    }
    let parent = path
        .parent()
        .ok_or("无效的导出目录")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let live_dir = data_dir.canonicalize().map_err(|e| e.to_string())?;
    if parent.starts_with(&live_dir) {
        return Err("请导出到应用数据目录之外；修改用户指纹请使用导入功能".into());
    }
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("导出目的地必须是普通 JSON 文件".into());
        }
    }
    let temporary = parent.join(format!(
        ".oviraptor-fingerprint-export-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(content.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|e| format!("无法保存指纹包：{e}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_refuses_live_data_and_non_json_and_preserves_existing_files() {
        let root = std::env::temp_dir().join(format!("ovi-export-test-{}", uuid::Uuid::new_v4()));
        let live = root.join("live");
        let out = root.join("out");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let database = live.join("oviraptor.sqlite3");
        std::fs::write(&database, b"synthetic database sentinel").unwrap();
        assert!(save_export(&database, &live, "{}").is_err());
        assert!(save_export(&live.join("fingerprints.user.json"), &live, "{}").is_err());
        let destination = out.join("share.json");
        std::fs::write(&destination, b"old").unwrap();
        save_export(&destination, &live, "{\"fingerprints\":[]}").unwrap();
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "{\"fingerprints\":[]}"
        );
        assert_eq!(
            std::fs::read(&database).unwrap(),
            b"synthetic database sentinel"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
