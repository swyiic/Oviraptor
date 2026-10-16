//! Explicit read-only compatibility adapter for an app-private legacy snapshot.
//! No desktop startup, model, scanning launch, schema migration or grants run here.
//! The new Agent host never imports this crate or reads this snapshot itself.
use crate::commands::assets::statistics::{self, ProjectAssetFilter};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const ASSETS: &str = "org.oviraptor.assets";
const SCANNING: &str = "org.oviraptor.scanning";
const MAX_FRAME: usize = 128 * 1024;
const MAX_RESULT: usize = 64 * 1024;
const MAX_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    jsonrpc: String,
    id: Value,
    method: String,
    params: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Context {
    host_bound_project_id: i64,
    grant_revision: i64,
    operation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    name: String,
    arguments: Value,
    context: Context,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Search {
    #[serde(default)]
    filter: ProjectAssetFilter,
    #[serde(default = "default_limit")]
    limit: i64,
    cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScanList {
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    20
}

fn open_snapshot(data_dir: &Path) -> Result<Connection, &'static str> {
    if !data_dir.is_absolute()
        || fs::canonicalize(data_dir).map_err(|_| "legacy_snapshot_unavailable")? != data_dir
    {
        return Err("legacy_snapshot_unavailable");
    }
    let path = data_dir.join("legacy-input.sqlite3");
    let metadata = fs::symlink_metadata(&path).map_err(|_| "legacy_snapshot_unavailable")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("legacy_snapshot_unavailable");
    }
    let connection = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|_| "legacy_snapshot_unavailable")?;
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(|_| "legacy_snapshot_unavailable")?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(|_| "legacy_snapshot_unavailable")?;
    Ok(connection)
}

fn context_valid(context: &Context) -> bool {
    (1..=MAX_INTEGER).contains(&context.host_bound_project_id)
        && (0..=MAX_INTEGER).contains(&context.grant_revision)
        && !context.operation_id.is_empty()
        && context.operation_id.chars().count() <= 128
        && !context.operation_id.chars().any(char::is_control)
}

fn safe_text(text: &str, bytes: usize) -> String {
    let clean = crate::agent_runtime::secrets::redact_text_with(text, None);
    let mut end = clean.len().min(bytes);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    clean[..end].to_string()
}

fn invoke(data_dir: &Path, app_id: &str, call: Call) -> Result<Value, &'static str> {
    if !context_valid(&call.context) {
        return Err("app_context_invalid");
    }
    let supported = match app_id {
        ASSETS => matches!(
            call.name.as_str(),
            "saved_assets_stats" | "saved_assets_search"
        ),
        SCANNING => call.name == "saved_scans_list",
        _ => false,
    };
    if !supported {
        return Err("legacy_tool_unavailable");
    }
    let connection = open_snapshot(data_dir)?;
    let project = call.context.host_bound_project_id;
    let exists = connection
        .query_row("SELECT id FROM projects WHERE id=?1", [project], |row| {
            row.get::<_, i64>(0)
        })
        .optional()
        .map_err(|_| "legacy_read_failed")?;
    if exists.is_none() {
        return Err("legacy_project_unavailable");
    }
    let data = match call.name.as_str() {
        "saved_assets_stats" => {
            let filter: ProjectAssetFilter =
                serde_json::from_value(call.arguments).map_err(|_| "app_arguments_invalid")?;
            serde_json::to_value(
                statistics::stats_on(&connection, project, &filter)
                    .map_err(|_| "legacy_read_failed")?,
            )
            .map_err(|_| "legacy_read_failed")?
        }
        "saved_assets_search" => {
            let input: Search =
                serde_json::from_value(call.arguments).map_err(|_| "app_arguments_invalid")?;
            if input.filter.deleted_view != "active" {
                return Err("legacy_deleted_content_unavailable");
            }
            serde_json::to_value(
                statistics::search_on(
                    &connection,
                    project,
                    &input.filter,
                    input.limit,
                    input.cursor.as_deref(),
                )
                .map_err(|_| "legacy_read_failed")?,
            )
            .map_err(|_| "legacy_read_failed")?
        }
        "saved_scans_list" => {
            let input: ScanList =
                serde_json::from_value(call.arguments).map_err(|_| "app_arguments_invalid")?;
            if !(1..=20).contains(&input.limit) {
                return Err("app_arguments_invalid");
            }
            let scans = crate::scanning::list_sentinel_scans_on(
                &connection,
                Some(project),
                Some(input.limit),
                None,
                None,
            )
            .map_err(|_| "legacy_read_failed")?;
            let mut items = Vec::new();
            for scan in scans {
                let deleted: bool = connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
                        [&scan.id],
                        |row| row.get(0),
                    )
                    .map_err(|_| "legacy_read_failed")?;
                if deleted {
                    continue;
                }
                items.push(json!({"id":safe_text(&scan.id,200),"projectId":project,"name":safe_text(&scan.task_name,256),"status":safe_text(&scan.status,40),"scanType":safe_text(&scan.scan_type,40),"updatedAt":safe_text(&scan.updated_at,80)}));
            }
            json!({"items":items,"limit":input.limit,"coverage":"bounded_saved_snapshot_projection","executionAvailable":false})
        }
        _ => return Err("legacy_tool_unavailable"),
    };
    Ok(
        json!({"source":"explicit_app_private_legacy_snapshot","readOnly":true,"liveExecution":false,"hostBoundProjectId":project,"grantRevision":call.context.grant_revision,"data":data}),
    )
}

fn response(data_dir: &Path, app_id: &str, frame: &[u8]) -> Value {
    let request: Request = match serde_json::from_slice(frame) {
        Ok(value) => value,
        Err(_) => return failure(Value::Null, "app_protocol_invalid"),
    };
    let id = request.id.clone();
    if request.jsonrpc != "2.0" || !matches!(id, Value::String(_) | Value::Number(_)) {
        return failure(id, "app_protocol_invalid");
    }
    let result = match request.method.as_str() {
        "describe" => {
            if request.params != json!({"hostApiVersion":"1.0.0"}) {
                Err("app_protocol_invalid")
            } else {
                Ok(
                    json!({"protocolVersion":1,"appId":app_id,"adapter":"legacy_read_only","tools":if app_id==ASSETS{vec!["saved_assets_stats","saved_assets_search"]}else{vec!["saved_scans_list"]},"executionAvailable":false}),
                )
            }
        }
        "tool/call" => serde_json::from_value::<Call>(request.params)
            .map_err(|_| "app_protocol_invalid")
            .and_then(|call| invoke(data_dir, app_id, call)),
        _ => Err("legacy_method_unavailable"),
    };
    match result {
        Ok(result) if serde_json::to_vec(&result).is_ok_and(|bytes| bytes.len() <= MAX_RESULT) => {
            json!({"jsonrpc":"2.0","id":id,"result":result})
        }
        Ok(_) => failure(id, "app_result_too_large"),
        Err(code) => failure(id, code),
    }
}
fn failure(id: Value, code: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":code,"data":{"code":code}}})
}

/// The app supplies its own data directory and fixed app identity at spawn.
/// Only legacy-input.sqlite3 beneath that directory can be read; never an
/// arbitrary database path received from a model tool argument or environment.
pub fn run_stdio() -> Result<(), &'static str> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 || args[0] != "--app" || args[2] != "--data-dir" {
        return Err("legacy_bridge_arguments_invalid");
    }
    let app_id = args[1].to_str().ok_or("legacy_bridge_arguments_invalid")?;
    if !matches!(app_id, ASSETS | SCANNING) {
        return Err("legacy_bridge_arguments_invalid");
    }
    let data_dir = PathBuf::from(&args[3]);
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let mut frame = Vec::new();
        let count = (&mut input)
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut frame)
            .map_err(|_| "legacy_protocol_read_failed")?;
        if count == 0 {
            break;
        }
        if frame.len() > MAX_FRAME {
            return Err("legacy_protocol_frame_too_large");
        }
        let result = response(&data_dir, app_id, &frame);
        serde_json::to_writer(&mut output, &result).map_err(|_| "legacy_protocol_write_failed")?;
        output
            .write_all(b"\n")
            .map_err(|_| "legacy_protocol_write_failed")?;
        output.flush().map_err(|_| "legacy_protocol_write_failed")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let raw =
                std::env::temp_dir().join(format!("oviraptor-legacy-app-{}", uuid::Uuid::new_v4()));
            let path = crate::db::initialize(&raw).unwrap();
            fs::rename(&path, raw.join("legacy-input.sqlite3")).unwrap();
            let root = fs::canonicalize(raw).unwrap();
            let connection = Connection::open(root.join("legacy-input.sqlite3")).unwrap();
            connection.execute_batch("INSERT INTO projects(id,name) VALUES(91,'Owned'),(92,'Other');INSERT INTO assets(id,asset_key,host,title) VALUES(1,'one','owned.example','Owned'),(2,'two','other.example','Private');INSERT INTO project_assets(project_id,asset_id) VALUES(91,1),(92,2);INSERT INTO sentinel_scans(id,project_id,project_name,status,task_name,task_path,source_path) VALUES('owned',91,'Owned','draft','Saved draft','/private/evidence','/private/source'),('other',92,'Other','scanning','Other scan','/private/other','/private/other');").unwrap();
            drop(connection);
            Self { root }
        }
        fn call(&self, app: &str, name: &str, args: Value, project: i64) -> Value {
            response(&self.root,app,&serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"tool/call","params":{"name":name,"arguments":args,"context":{"hostBoundProjectId":project,"grantRevision":3,"operationId":"read-1"}}})).unwrap())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn existing_assets_owner_reads_only_app_snapshot_and_preserves_file_bytes() {
        let f = Fixture::new();
        let path = f.root.join("legacy-input.sqlite3");
        let before = fs::read(&path).unwrap();
        let stats = f.call(ASSETS, "saved_assets_stats", json!({}), 91);
        assert_eq!(stats["result"]["data"]["activeTotal"], 1);
        let page = f.call(ASSETS, "saved_assets_search", json!({}), 91);
        assert_eq!(page["result"]["data"]["items"][0]["host"], "owned.example");
        assert!(!page.to_string().contains("other.example"));
        assert_eq!(fs::read(path).unwrap(), before);
    }
    #[test]
    fn real_scanning_projection_is_project_bound_and_never_exports_private_paths_or_execution() {
        let f = Fixture::new();
        let result = f.call(SCANNING, "saved_scans_list", json!({}), 91);
        assert_eq!(
            result["result"]["data"]["items"].as_array().unwrap().len(),
            1
        );
        assert_eq!(result["result"]["data"]["items"][0]["id"], "owned");
        assert_eq!(result["result"]["data"]["executionAvailable"], false);
        assert!(!result.to_string().contains("/private/"));
        assert_eq!(
            f.call(SCANNING, "start_scan", json!({}), 91)["error"]["data"]["code"],
            "legacy_tool_unavailable"
        );
    }
    #[test]
    fn unknown_arguments_cross_app_and_deleted_content_fail_closed() {
        let f = Fixture::new();
        assert!(
            f.call(ASSETS, "saved_assets_stats", json!({"projectId":92}), 91)["error"].is_object()
        );
        assert!(f.call(
            ASSETS,
            "saved_assets_search",
            json!({"filter":{"deletedView":"trash"}}),
            91
        )["error"]
            .is_object());
        assert!(f.call(ASSETS, "saved_scans_list", json!({}), 91)["error"].is_object());
        assert!(f.call(ASSETS, "saved_assets_stats", json!({}), 0)["error"].is_object());
        let connection = open_snapshot(&f.root).unwrap();
        assert!(connection.execute("DELETE FROM assets", []).is_err());
    }
    #[test]
    fn missing_snapshot_never_creates_a_database_or_reports_zero_inventory() {
        let f = Fixture::new();
        fs::remove_file(f.root.join("legacy-input.sqlite3")).unwrap();
        assert_eq!(
            f.call(ASSETS, "saved_assets_stats", json!({}), 91)["error"]["data"]["code"],
            "legacy_snapshot_unavailable"
        );
        assert!(!f.root.join("legacy-input.sqlite3").exists());
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_snapshot_is_not_a_legacy_read_capability() {
        let f = Fixture::new();
        let path = f.root.join("legacy-input.sqlite3");
        fs::rename(&path, f.root.join("original.sqlite3")).unwrap();
        std::os::unix::fs::symlink("original.sqlite3", &path).unwrap();
        assert_eq!(
            f.call(ASSETS, "saved_assets_stats", json!({}), 91)["error"]["data"]["code"],
            "legacy_snapshot_unavailable"
        );
    }
}
