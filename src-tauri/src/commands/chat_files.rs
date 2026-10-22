//! Explicit desktop file actions confined to the selected Host project's workspace.
use crate::AppState;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
use tauri::State;

fn denied() -> String {
    "host_chat_file_path_denied".into()
}
fn safe_parts(value: &str) -> Result<Vec<String>, String> {
    if value.is_empty()
        || value.len() > 2048
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(denied());
    }
    if value
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(denied());
    }
    Path::new(value)
        .components()
        .map(|part| match part {
            Component::Normal(name) => name.to_str().map(str::to_string).ok_or_else(denied),
            _ => Err(denied()),
        })
        .collect()
}

fn checked_child(base: &Path, parts: &[String], create: bool) -> Result<PathBuf, String> {
    let mut current = base.to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        current.push(part);
        let info = match fs::symlink_metadata(&current) {
            Ok(info) => info,
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                if let Err(error) = builder.create(&current) {
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err("host_chat_file_unavailable".into());
                    }
                }
                fs::symlink_metadata(&current)
                    .map_err(|_| "host_chat_file_unavailable".to_string())?
            }
            Err(_) => return Err("host_chat_file_unavailable".into()),
        };
        if info.file_type().is_symlink() || (index + 1 < parts.len() || create) && !info.is_dir() {
            return Err(denied());
        }
        let canonical = fs::canonicalize(&current).map_err(|_| denied())?;
        if !canonical.starts_with(base) {
            return Err(denied());
        }
    }
    Ok(current)
}

fn ordinary_file(path: &Path) -> Result<fs::Metadata, String> {
    let info = fs::symlink_metadata(path).map_err(|_| "host_chat_file_unavailable".to_string())?;
    if !info.is_file() || info.file_type().is_symlink() {
        return Err(denied());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if info.nlink() > 1 {
            return Err(denied());
        }
    }
    Ok(info)
}

fn openable(path: &Path, info: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if info.permissions().mode() & 0o111 != 0 {
            return false;
        }
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "csv"
            | "tsv"
            | "json"
            | "log"
            | "yaml"
            | "yml"
            | "toml"
            | "pdf"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
    )
}

fn action_target(
    data_dir: &Path,
    project_id: i64,
    conversation_id: &str,
    relative_path: Option<&str>,
    action: &str,
) -> Result<PathBuf, String> {
    if project_id < 1
        || conversation_id.is_empty()
        || conversation_id.len() > 128
        || !conversation_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("host_conversation_unavailable".into());
    }
    if !matches!(action, "reveal" | "open") {
        return Err("host_chat_file_action_invalid".into());
    }
    let anchor =
        fs::canonicalize(data_dir).map_err(|_| "host_chat_file_unavailable".to_string())?;
    let database = checked_child(&anchor, &["agent-host".into(), "host.sqlite".into()], false)?;
    ordinary_file(&database)?;
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "host_conversation_unavailable".to_string())?;
    let found: Option<i64> = connection
        .query_row(
            "SELECT project_id FROM conversations WHERE id=?1",
            [conversation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| "host_conversation_unavailable".to_string())?;
    if found != Some(project_id) {
        return Err("host_conversation_unavailable".into());
    }
    let root = checked_child(
        &anchor,
        &[
            "agent-host".into(),
            "workspaces".into(),
            project_id.to_string(),
            "agent".into(),
        ],
        relative_path.is_none() && action == "reveal",
    )?;
    if !root.is_dir() {
        return Err(denied());
    }
    let Some(relative_path) = relative_path else {
        return if action == "reveal" {
            Ok(root)
        } else {
            Err("host_chat_file_action_invalid".into())
        };
    };
    let file = checked_child(&root, &safe_parts(relative_path)?, false)?;
    let info = ordinary_file(&file)?;
    if action == "open" && !openable(&file, &info) {
        return Err("host_chat_file_open_unsupported".into());
    }
    Ok(file)
}

#[tauri::command]
pub async fn agent_host_chat_file_action(
    state: State<'_, AppState>,
    project_id: i64,
    conversation_id: String,
    relative_path: Option<String>,
    action: String,
) -> Result<serde_json::Value, String> {
    let data_dir = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target = action_target(
            &data_dir,
            project_id,
            &conversation_id,
            relative_path.as_deref(),
            &action,
        )?;
        if action == "reveal" {
            tauri_plugin_opener::reveal_item_in_dir(&target)
                .map_err(|_| "host_chat_file_reveal_failed".to_string())?;
        } else {
            tauri_plugin_opener::open_path(&target, None::<&str>)
                .map_err(|_| "host_chat_file_open_failed".to_string())?;
        }
        Ok(serde_json::json!({"path":target.to_string_lossy(),"action":action}))
    })
    .await
    .map_err(|_| "host_chat_file_action_failed".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ovi-chat-files-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("agent-host")).unwrap();
        let db = Connection::open(dir.join("agent-host/host.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE conversations(id TEXT PRIMARY KEY,project_id INTEGER);INSERT INTO conversations VALUES('chat-one',1);INSERT INTO conversations VALUES('chat-two',2);").unwrap();
        dir
    }
    #[test]
    fn finder_directory_is_created_only_for_the_bound_project_on_explicit_reveal() {
        let dir = fixture();
        assert_eq!(
            action_target(&dir, 2, "chat-one", None, "reveal").unwrap_err(),
            "host_conversation_unavailable"
        );
        assert!(!dir.join("agent-host/workspaces").exists());
        let root = action_target(&dir, 1, "chat-one", None, "reveal").unwrap();
        assert!(root.is_dir());
        assert_eq!(
            root,
            fs::canonicalize(&dir)
                .unwrap()
                .join("agent-host/workspaces/1/agent")
        );
        assert!(action_target(&dir, 1, "chat-one", None, "open").is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn clicks_cannot_escape_or_open_scripts_and_html() {
        let dir = fixture();
        let root = action_target(&dir, 1, "chat-one", None, "reveal").unwrap();
        fs::write(root.join("report.md"), "# Report").unwrap();
        fs::write(root.join("preview.html"), "<script>alert(1)</script>").unwrap();
        fs::write(root.join("script.sh"), "echo test").unwrap();
        assert!(action_target(&dir, 1, "chat-one", Some("report.md"), "open").is_ok());
        assert!(action_target(&dir, 1, "chat-one", Some("preview.html"), "reveal").is_ok());
        assert_eq!(
            action_target(&dir, 1, "chat-one", Some("preview.html"), "open").unwrap_err(),
            "host_chat_file_open_unsupported"
        );
        assert_eq!(
            action_target(&dir, 1, "chat-one", Some("script.sh"), "open").unwrap_err(),
            "host_chat_file_open_unsupported"
        );
        for path in [
            "../host.sqlite",
            "/etc/passwd",
            "a/../report.md",
            "a\\report.md",
            "./report.md",
        ] {
            assert!(action_target(&dir, 1, "chat-one", Some(path), "reveal").is_err());
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn symbolic_and_hard_links_and_executable_text_are_rejected() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = fixture();
        let root = action_target(&dir, 1, "chat-one", None, "reveal").unwrap();
        fs::write(dir.join("secret.txt"), "outside").unwrap();
        symlink(dir.join("secret.txt"), root.join("alias.txt")).unwrap();
        assert!(action_target(&dir, 1, "chat-one", Some("alias.txt"), "reveal").is_err());
        fs::hard_link(dir.join("secret.txt"), root.join("hard.txt")).unwrap();
        assert!(action_target(&dir, 1, "chat-one", Some("hard.txt"), "reveal").is_err());
        fs::write(root.join("run.txt"), "#!/bin/sh").unwrap();
        fs::set_permissions(root.join("run.txt"), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(action_target(&dir, 1, "chat-one", Some("run.txt"), "open").is_err());
        fs::remove_dir_all(root).unwrap();
        symlink(&dir, dir.join("agent-host/workspaces/1/agent")).unwrap();
        assert!(action_target(&dir, 1, "chat-one", None, "reveal").is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
