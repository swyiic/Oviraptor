//! Asset application adapter over the real inventory/pipeline owners.
//! Paths come only from a trusted native bootstrap, never RPC arguments.
//! Assets and Scanning own separate compatibility databases; equal IDs do not
//! authorize joining them. Transfer requires an explicit selection receipt.
use crate::{
    commands::{asset_export, asset_rule_catalog, assets, workspace_config_profiles},
    db,
    models::{
        AssetBulkArchiveInput, AssetBulkDecisionInput, AssetQuery, ConfigProfileInput,
        ContentRuleInput, DecisionInput, ExportRequest, TargetImportInput,
    },
    page_isolation,
};
use rusqlite::OptionalExtension;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_INTEGER: i64 = 9_007_199_254_740_991;
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
fn identity(metadata: &fs::Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        FileIdentity {}
    }
}
fn checked(path: &Path, directory: bool, private: bool) -> Result<FileIdentity, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "asset_service_path_unavailable")?;
    if !path.is_absolute()
        || metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
        || fs::canonicalize(path).map_err(|_| "asset_service_path_unavailable")? != path
    {
        return Err("asset_service_path_invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let forbidden = if private { 0o077 } else { 0o022 };
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & forbidden != 0
            || (!directory && metadata.nlink() != 1)
        {
            return Err("asset_service_path_permissions".into());
        }
    }
    #[cfg(not(unix))]
    let _ = private;
    Ok(identity(&metadata))
}

/// Trusted Assets-owned existing paths, not deserializable RPC inputs.
/// Construction opens no DB, migrates/adopts nothing, and never claims old jobs.
#[derive(Clone)]
pub(crate) struct AssetServiceEnvironment {
    data_root: PathBuf,
    db_path: PathBuf,
    export_dir: PathBuf,
    identities: [FileIdentity; 3],
}
impl AssetServiceEnvironment {
    pub(crate) fn from_paths(
        data_root: &Path,
        db_path: &Path,
        export_dir: &Path,
    ) -> Result<Self, String> {
        if db_path == data_root
            || export_dir == data_root
            || !db_path.starts_with(data_root)
            || !export_dir.starts_with(data_root)
        {
            return Err("asset_service_path_scope_invalid".into());
        }
        Ok(Self {
            data_root: data_root.into(),
            db_path: db_path.into(),
            export_dir: export_dir.into(),
            identities: [
                checked(data_root, true, true)?,
                checked(db_path, false, false)?,
                checked(export_dir, true, true)?,
            ],
        })
    }
    fn revalidate(&self) -> Result<(), String> {
        let current = [
            checked(&self.data_root, true, true)?,
            checked(&self.db_path, false, false)?,
            checked(&self.export_dir, true, true)?,
        ];
        if current != self.identities {
            return Err("asset_service_path_identity_changed".into());
        }
        Ok(())
    }
    pub(crate) fn database_path(&self) -> &Path {
        &self.db_path
    }
    pub(crate) fn data_root(&self) -> &Path {
        &self.data_root
    }
    pub(crate) fn export_dir(&self) -> &Path {
        &self.export_dir
    }
}

pub(crate) struct AssetService {
    environment: AssetServiceEnvironment,
}
fn input<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "asset_service_arguments_invalid".into())
}
fn output(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|_| "asset_service_result_invalid".into())
}
fn same_project(expected: i64, provided: i64) -> Result<(), String> {
    if provided != expected {
        return Err("asset_service_project_scope_mismatch".into());
    }
    Ok(())
}
fn optional_project(expected: i64, provided: Option<i64>) -> Result<(), String> {
    if let Some(provided) = provided {
        same_project(expected, provided)?;
    }
    Ok(())
}
fn query_for(project: i64, mut query: AssetQuery) -> Result<AssetQuery, String> {
    optional_project(project, query.project_id)?;
    query.project_id = Some(project);
    Ok(query)
}
macro_rules! args {
    ($name:ident {$($field:ident : $kind:ty),* $(,)?}) => {
        #[derive(Deserialize)]
        #[serde(rename_all="camelCase",deny_unknown_fields)]
        struct $name { $($field:$kind),* }
    };
}
args!(Empty {});
args!(Project { project_id: Option<i64> });
args!(TargetImport {
    input: TargetImportInput
});
args!(TargetId { target_id: i64 });
args!(Query { query: AssetQuery });
args!(Entry { project_id: Option<i64>, asset_id: i64 });
args!(Decision {
    input: DecisionInput
});
args!(BulkDecision {
    input: AssetBulkDecisionInput
});
args!(Archive { project_id: Option<i64>, asset_ids: Vec<i64>, deleted: bool });
args!(BulkArchive {
    input: AssetBulkArchiveInput
});
args!(Runs { project_id: Option<i64>, limit: Option<i64> });
args!(Logs { run_id: Option<i64>, project_id: Option<i64>, limit: Option<i64> });
args!(Events { project_id: Option<i64>, event_type: Option<String>, limit: Option<i64> });
args!(Run { run_id: i64 });
args!(PreviewRule {
    input: page_isolation::RuleInput
});
args!(ApplyRule {
    input: page_isolation::ApplyInput
});
args!(DisableRule { project_id: Option<i64>, rule_id: i64 });
args!(Export {
    request: ExportRequest
});
args!(ContentRule {
    input: ContentRuleInput
});
args!(Profile {
    input: ConfigProfileInput
});
args!(ProfileId { profile_id: i64 });
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Search {
    #[serde(default)]
    filter: assets::statistics::ProjectAssetFilter,
    limit: Option<i64>,
    cursor: Option<String>,
}

impl AssetService {
    pub(crate) fn new(environment: AssetServiceEnvironment) -> Self {
        Self { environment }
    }
    pub(crate) fn environment(&self) -> &AssetServiceEnvironment {
        &self.environment
    }
    fn project_on(&self, project: i64, writable: bool) -> Result<rusqlite::Connection, String> {
        if !(1..=MAX_INTEGER).contains(&project) {
            return Err("asset_service_project_invalid".into());
        }
        self.environment.revalidate()?;
        let connection = db::open(&self.environment.db_path)?;
        let status: Option<String> = connection
            .query_row("SELECT status FROM projects WHERE id=?1", [project], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|_| "asset_service_project_unavailable")?;
        match status.as_deref() {
            Some("active") => Ok(connection),
            Some(_) if !writable => Ok(connection),
            Some(_) => Err("asset_service_project_archived".into()),
            None => Err("asset_service_project_unavailable".into()),
        }
    }
    fn run_in_project(
        connection: &rusqlite::Connection,
        project: i64,
        run: i64,
    ) -> Result<(), String> {
        let belongs: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND project_id=?2)",
                [run, project],
                |r| r.get(0),
            )
            .map_err(|_| "asset_service_run_unavailable")?;
        if !belongs {
            return Err("asset_service_run_scope_mismatch".into());
        }
        Ok(())
    }
    /// Project is from trusted host mapping, not tool arguments. Full legacy UI
    /// DTOs stay intact; this is separate from bounded Agent read projections.
    pub(crate) fn dispatch_project(
        &self,
        project: i64,
        command: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let writable = matches!(
            command,
            "import_targets"
                | "remove_target"
                | "mark_asset_entry_opened"
                | "update_decision"
                | "update_asset_decisions"
                | "soft_delete_assets"
                | "soft_delete_asset_selections"
                | "apply_page_rule"
                | "disable_page_rule"
                | "acknowledge_interrupted_run"
        );
        let mut connection = self.project_on(project, writable)?;
        let path = &self.environment.db_path;
        match command {
            "import_targets" => {
                let a: TargetImport = input(arguments)?;
                same_project(project, a.input.project_id)?;
                output(assets::import_targets_at(path, a.input)?)
            }
            "list_targets" => {
                let a: Project = input(arguments)?;
                optional_project(project, a.project_id)?;
                output(assets::list_targets_at(path, project)?)
            }
            "remove_target" => {
                let a: TargetId = input(arguments)?;
                let exists: bool = connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM targets WHERE id=?1 AND project_id=?2)",
                        [a.target_id, project],
                        |r| r.get(0),
                    )
                    .map_err(|_| "asset_service_target_unavailable")?;
                if !exists {
                    return Err("asset_service_target_scope_mismatch".into());
                }
                output(assets::remove_target_at(path, a.target_id)?)
            }
            "list_assets" => {
                let a: Query = input(arguments)?;
                output(assets::list_assets_at(path, &query_for(project, a.query)?)?)
            }
            "mark_asset_entry_opened" => {
                let a: Entry = input(arguments)?;
                optional_project(project, a.project_id)?;
                output(assets::mark_asset_entry_opened_at(
                    path, project, a.asset_id,
                )?)
            }
            "update_decision" => {
                let a: Decision = input(arguments)?;
                same_project(project, a.input.project_id)?;
                output(assets::update_decision_at(path, a.input)?)
            }
            "update_asset_decisions" => {
                let a: BulkDecision = input(arguments)?;
                for selection in &a.input.selections {
                    same_project(project, selection.project_id)?;
                }
                output(assets::update_asset_decisions_at(path, a.input)?)
            }
            "soft_delete_assets" => {
                let a: Archive = input(arguments)?;
                optional_project(project, a.project_id)?;
                output(assets::soft_delete_assets_at(
                    path,
                    project,
                    a.asset_ids,
                    a.deleted,
                )?)
            }
            "soft_delete_asset_selections" => {
                let a: BulkArchive = input(arguments)?;
                for selection in &a.input.selections {
                    same_project(project, selection.project_id)?;
                }
                output(assets::soft_delete_asset_selections_at(path, a.input)?)
            }
            "list_runs" => {
                let a: Runs = input(arguments)?;
                optional_project(project, a.project_id)?;
                output(assets::list_runs_at(path, Some(project), a.limit)?)
            }
            "list_logs" => {
                let a: Logs = input(arguments)?;
                optional_project(project, a.project_id)?;
                if let Some(run) = a.run_id {
                    Self::run_in_project(&connection, project, run)?;
                }
                output(assets::list_logs_at(
                    path,
                    a.run_id,
                    Some(project),
                    a.limit,
                )?)
            }
            "list_asset_events" => {
                let a: Events = input(arguments)?;
                optional_project(project, a.project_id)?;
                output(assets::list_asset_events_at(
                    path,
                    Some(project),
                    a.event_type,
                    a.limit,
                )?)
            }
            "acknowledge_interrupted_run" => {
                let a: Run = input(arguments)?;
                Self::run_in_project(&connection, project, a.run_id)?;
                output(assets::acknowledge_interrupted_run_at(path, a.run_id)?)
            }
            "preview_page_rule" => {
                let a: PreviewRule = input(arguments)?;
                same_project(project, a.input.project_id)?;
                output(page_isolation::preview_on(&connection, a.input)?)
            }
            "apply_page_rule" => {
                let a: ApplyRule = input(arguments)?;
                same_project(project, a.input.rule.project_id)?;
                output(page_isolation::apply_on(&mut connection, path, a.input)?)
            }
            "list_page_rules" => {
                let a: Project = input(arguments)?;
                optional_project(project, a.project_id)?;
                output(page_isolation::list_on(&connection, project)?)
            }
            "disable_page_rule" => {
                let a: DisableRule = input(arguments)?;
                optional_project(project, a.project_id)?;
                output(page_isolation::disable_on(
                    &mut connection,
                    path,
                    project,
                    a.rule_id,
                )?)
            }
            "export_assets" => {
                let mut a: Export = input(arguments)?;
                a.request.query = query_for(project, a.request.query)?;
                output(asset_export::export_assets_at(
                    path,
                    &self.environment.export_dir,
                    a.request,
                )?)
            }
            "saved_assets_stats" => {
                let filter = input(arguments)?;
                output(assets::statistics::stats_on(&connection, project, &filter)?)
            }
            "saved_assets_search" => {
                let a: Search = input(arguments)?;
                if a.filter.deleted_view != "active" {
                    return Err("asset_service_deleted_tool_content_denied".into());
                }
                output(assets::statistics::search_on(
                    &connection,
                    project,
                    &a.filter,
                    a.limit.unwrap_or(20),
                    a.cursor.as_deref(),
                )?)
            }
            "list_asset_job_profiles" => {
                let _: Empty = input(arguments)?;
                self.job_profiles()
            }
            _ => Err("asset_service_project_method_unavailable".into()),
        }
    }
    fn job_profiles(&self) -> Result<Value, String> {
        self.environment.revalidate()?;
        let profiles =
            workspace_config_profiles::read_config_profiles(&db::open(&self.environment.db_path)?)?;
        Ok(json!(profiles.into_iter().map(|p|json!({
            "id":p.id,"name":p.name,"isDefault":p.is_default,
            "runProbe":p.settings.get("runProbe").and_then(Value::as_bool).unwrap_or(true),
            "runRefine":p.settings.get("runRefine").and_then(Value::as_bool).unwrap_or(true),
            "hasFofaKey":p.settings.get("fofaKey").and_then(Value::as_str).is_some_and(|v|!v.trim().is_empty())
        })).collect::<Vec<_>>()))
    }
    /// Explicit user-only application administration, NOT project tools.
    /// Legacy content rules affect all projects in this Assets-owned DB;
    /// profile configuration can include private credentials. The outer
    /// authorization must never expose this entry as a model capability.
    pub(crate) fn dispatch_application(
        &self,
        command: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        self.environment.revalidate()?;
        let path = &self.environment.db_path;
        match command {
            "get_asset_rule_catalog" => {
                let _: Empty = input(arguments)?;
                output(asset_rule_catalog::asset_rule_catalog_on(&db::open(path)?)?)
            }
            "add_content_rule" => {
                let a: ContentRule = input(arguments)?;
                output(assets::add_content_rule_at(path, a.input)?)
            }
            "list_config_profiles" => {
                let _: Empty = input(arguments)?;
                output(workspace_config_profiles::read_config_profiles(&db::open(
                    path,
                )?)?)
            }
            "save_config_profile" => {
                let a: Profile = input(arguments)?;
                output(workspace_config_profiles::persist_config_profile(
                    &mut db::open(path)?,
                    a.input,
                )?)
            }
            "delete_config_profile" => {
                let a: ProfileId = input(arguments)?;
                output(workspace_config_profiles::delete_config_profile_inner(
                    &db::open(path)?,
                    a.profile_id,
                )?)
            }
            _ => Err("asset_service_application_method_unavailable".into()),
        }
    }
}
#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
