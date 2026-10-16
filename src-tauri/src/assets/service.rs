//! Asset application adapter over the real inventory/pipeline owners.
//! Paths come only from a trusted native bootstrap, never RPC arguments.
//! Assets and Scanning own separate compatibility databases; equal IDs do not
//! authorize joining them. Transfer requires an explicit selection receipt.
use crate::{
    asset_models::{
        AssetBulkArchiveInput, AssetBulkDecisionInput, AssetQuery, ContentRuleInput, DecisionInput,
        ExportRequest, StartJobInput, TargetImportInput,
    },
    commands::{asset_export, asset_rule_catalog, assets, workspace_config_profiles},
    configuration_models::ConfigProfileInput,
    db, page_isolation,
};
use rusqlite::OptionalExtension;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[path = "export_artifacts.rs"]
pub(crate) mod export_artifacts;
#[path = "exposure_service.rs"]
pub(crate) mod exposure_service;
#[path = "owned_work.rs"]
pub(crate) mod owned_work;
#[path = "ownership_service.rs"]
pub(crate) mod ownership_service;

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
    jobs: Option<crate::jobs::AssetJobContext>,
}
/// Stable private-store identity shared only by the Assets owner adapters.
/// A project ID, process ID, or an arbitrary legacy row is never an instance.
pub(crate) fn service_instance_on(connection: &rusqlite::Connection) -> Result<String, String> {
    let rows: Vec<(i64, String, i64)> = connection
        .prepare("SELECT singleton,instance_id,schema_version FROM asset_service_store")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect()
        })
        .map_err(|_| "asset_service_instance_unavailable")?;
    if rows.len() != 1
        || rows[0].0 != 1
        || rows[0].2 != 1
        || !uuid::Uuid::parse_str(&rows[0].1)
            .is_ok_and(|id| id.get_version_num() == 4 && id.to_string() == rows[0].1)
    {
        return Err("asset_service_instance_invalid".into());
    }
    Ok(rows[0].1.clone())
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
args!(Artifact {artifact_id:String,offset:Option<u64>,max_bytes:Option<usize>});
args!(StartJob {
    input: StartJobInput
});
args!(ConfigurationImport { content: String });
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
        Self {
            environment,
            jobs: None,
        }
    }
    pub(crate) fn with_jobs(
        mut self,
        context: crate::jobs::AssetJobContext,
    ) -> Result<Self, String> {
        if context.database_path() != self.environment.database_path() {
            return Err("asset_service_job_database_mismatch".into());
        }
        self.jobs = Some(context);
        Ok(self)
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
            "start_job"
                | "resume_job"
                | "import_targets"
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
        let writable =
            writable || secondary_action_permission(command).is_some_and(|(_, mutating)| mutating);
        let mut connection = self.project_on(project, writable)?;
        let path = &self.environment.db_path;
        match command {
            "start_job" => {
                let a: StartJob = input(arguments)?;
                same_project(project, a.input.project_id)?;
                output(crate::jobs::start_job_in(
                    self.jobs
                        .as_ref()
                        .ok_or("asset_service_job_context_unavailable")?,
                    a.input,
                )?)
            }
            "resume_job" | "cancel_job" => {
                let a: Run = input(arguments)?;
                Self::run_in_project(&connection, project, a.run_id)?;
                let context = self
                    .jobs
                    .as_ref()
                    .ok_or("asset_service_job_context_unavailable")?;
                if command == "resume_job" {
                    output(crate::jobs::resume_job_in(context, a.run_id)?)
                } else {
                    output(crate::jobs::cancel_job_in(context, a.run_id)?)
                }
            }
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
                let directory =
                    export_artifacts::project_directory(&self.environment.export_dir, project)?;
                let raw = output(asset_export::export_assets_at(path, &directory, a.request)?)?;
                export_artifacts::wrap_result_on(path, project, &self.environment.export_dir, raw)
            }
            "read_asset_artifact" => {
                let a: Artifact = input(arguments)?;
                export_artifacts::read(
                    path,
                    &self.environment.export_dir,
                    project,
                    &a.artifact_id,
                    a.offset.unwrap_or(0),
                    a.max_bytes.unwrap_or(128 * 1024),
                )
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
            name if ownership_service::project_commands().contains(&name) => {
                ownership_service::dispatch_project(path, project, name, arguments)
            }
            name if exposure_service::project_commands().contains(&name) => {
                exposure_service::dispatch_project(
                    path,
                    &self.environment.export_dir,
                    project,
                    name,
                    arguments,
                )
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
            "hasFofaKey":p.settings.get("fofaKey").and_then(Value::as_str).is_some_and(|v|!v.trim().is_empty())||p.settings.get("configPath").and_then(Value::as_str).filter(|p|!p.is_empty()).is_some_and(|p|self.jobs.as_ref().is_some_and(|c|c.configuration_id(p).is_ok()))
        })).collect::<Vec<_>>()))
    }
    pub(crate) fn dispatch_application_for_project(
        &self,
        project: i64,
        name: &str,
        args: Value,
    ) -> Result<Value, String> {
        if ownership_service::application_commands().contains(&name) {
            let _connection = self.project_on(project, true)?;
            ownership_service::dispatch_application(&self.environment.db_path, project, name, args)
        } else {
            self.dispatch_application(name, args)
        }
    }
    fn configuration_view(&self, profile: Value) -> Value {
        let configured = profile["settings"]["fofaKey"]
            .as_str()
            .is_some_and(|v| !v.trim().is_empty());
        let file_id = profile["settings"]["configPath"]
            .as_str()
            .filter(|v| !v.is_empty())
            .and_then(|p| self.jobs.as_ref().and_then(|c| c.configuration_id(p).ok()));
        let mut view = redacted_configuration_value(profile);
        if let Some(settings) = view.get_mut("settings").and_then(Value::as_object_mut) {
            settings.insert("configPath".into(), json!(""));
            settings.insert(
                "collectionConfigurationId".into(),
                json!(file_id.as_deref().unwrap_or("")),
            );
            settings.insert(
                "collectionConfigurationConfigured".into(),
                json!(file_id.is_some()),
            );
            settings.insert(
                "fofaKeyConfigured".into(),
                json!(configured || file_id.is_some()),
            );
        }
        view
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
            "import_asset_collection_configuration" => {
                let a: ConfigurationImport = input(arguments)?;
                self.jobs
                    .as_ref()
                    .ok_or("asset_service_job_context_unavailable")?
                    .import_collection_configuration(&a.content)
            }
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
                let profiles = workspace_config_profiles::read_config_profiles(&db::open(path)?)?;
                output(
                    profiles
                        .into_iter()
                        .map(|profile| {
                            self.configuration_view(
                                serde_json::to_value(profile).unwrap_or(Value::Null),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            }
            "save_config_profile" => {
                let mut a: Profile = input(arguments)?;
                let mut connection = db::open(path)?;
                let selected_file = a
                    .input
                    .settings
                    .get("collectionConfigurationId")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string);
                let explicit_key = a
                    .input
                    .settings
                    .get("fofaKey")
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.trim().is_empty());
                if let Some(id) = &selected_file {
                    let file = self
                        .jobs
                        .as_ref()
                        .ok_or("asset_service_job_context_unavailable")?
                        .configuration_path(id)?;
                    a.input.settings["configPath"] = json!(file);
                }
                if let Some(id) = a.input.id {
                    let raw: Option<String> = connection
                        .query_row(
                            "SELECT settings_json FROM config_profiles WHERE id=?1",
                            [id],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(|_| "asset_service_configuration_unavailable")?;
                    if let Some(raw) = raw {
                        let stored: Value = serde_json::from_str(&raw)
                            .map_err(|_| "asset_service_configuration_invalid")?;
                        if selected_file.is_none()
                            && !explicit_key
                            && a.input
                                .settings
                                .get("configPath")
                                .and_then(Value::as_str)
                                .is_none_or(|v| v.is_empty())
                        {
                            if let Some(path) = stored.get("configPath") {
                                a.input.settings["configPath"] = path.clone();
                            }
                        }
                        retain_write_only_credentials(&stored, &mut a.input.settings)?;
                    }
                }
                if selected_file.is_some() {
                    a.input.settings["fofaKey"] = json!("");
                } else if explicit_key {
                    a.input.settings["configPath"] = json!("");
                }
                if let Some(path) = a
                    .input
                    .settings
                    .get("configPath")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    self.jobs
                        .as_ref()
                        .ok_or("asset_service_job_context_unavailable")?
                        .configuration_id(path)?;
                }
                if let Some(object) = a.input.settings.as_object_mut() {
                    object.remove("collectionConfigurationId");
                }
                output(workspace_config_profiles::persist_config_profile(
                    &mut connection,
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

pub(super) fn secondary_action_permission(name: &str) -> Option<(&'static str, bool)> {
    match name {
        "get_asset_ownership_profile" => Some(("inventory.read", false)),
        "save_asset_ownership_profile" => Some(("inventory.write", true)),
        "get_asset_ownership_summary" => Some(("inventory.read", false)),
        "assess_asset_ownership" => Some(("inventory.write", true)),
        "update_asset_ownership" => Some(("inventory.authorization.manage", true)),
        "get_project_ownership_bootstrap" => Some(("inventory.read", false)),
        "preview_project_ownership_suggestions" => Some(("inventory.read", false)),
        "start_ownership_analysis" => Some(("inventory.write", true)),
        "cancel_ownership_analysis" => Some(("inventory.write", true)),
        "read_ownership_analysis" => Some(("inventory.read", false)),
        "page_ownership_groups" => Some(("inventory.read", false)),
        "page_ownership_group_members" => Some(("inventory.read", false)),
        "preview_ownership_group_decision" => Some(("inventory.read", false)),
        "apply_ownership_group_decision" => Some(("inventory.write", true)),
        "read_ownership_operation" => Some(("inventory.read", false)),
        "preview_ownership_group_revert" => Some(("inventory.read", false)),
        "apply_ownership_group_revert" => Some(("inventory.write", true)),
        "revoke_ownership_group_authorization" => Some(("inventory.authorization.manage", true)),
        "confirm_asset_ownership" => Some(("inventory.write", true)),
        "authorize_asset_ownership" => Some(("inventory.authorization.manage", true)),
        "page_exposure_findings" => Some(("inventory.read", false)),
        "page_exposure_runs" => Some(("inventory.read", false)),
        "list_exposure_source_results_for_project" => Some(("inventory.read", false)),
        "read_exposure_finding" => Some(("inventory.read", false)),
        "exposure_scope_summary" => Some(("inventory.read", false)),
        "exposure_known_limits" => Some(("inventory.read", false)),
        "exposure_summary" => Some(("inventory.read", false)),
        "list_exposure_findings" => Some(("inventory.read", false)),
        "save_exposure_finding" => Some(("inventory.exposure.manage", true)),
        "list_exposure_runs" => Some(("inventory.read", false)),
        "list_exposure_source_results" => Some(("inventory.read", false)),
        "cancel_exposure_scan" => Some(("inventory.exposure.run", true)),
        "export_exposure_findings" => Some(("inventory.export", true)),
        "page_exposure_incidents" => Some(("inventory.read", false)),
        "read_exposure_incident" => Some(("inventory.read", false)),
        "page_exposure_incident_locations" => Some(("inventory.read", false)),
        "exposure_coverage_drilldown" => Some(("inventory.read", false)),
        "act_on_exposure_incident" => Some(("inventory.exposure.manage", true)),
        "propose_exposure_incidents" => Some(("inventory.exposure.manage", true)),
        "verify_exposure_export" => Some(("inventory.export", true)),
        "review_exposure_finding_scoped" => Some(("inventory.exposure.manage", true)),
        "list_exposure_review_trail" => Some(("inventory.read", false)),
        "save_exposure_remediation" => Some(("inventory.exposure.manage", true)),
        "page_exposure_remediations" => Some(("inventory.read", false)),
        "start_exposure_retest" => Some(("inventory.exposure.run", true)),
        "read_exposure_retest_outcome" => Some(("inventory.read", false)),
        "mark_exposure_remediation" => Some(("inventory.exposure.manage", true)),
        "exposure_review_taxonomy" => Some(("inventory.read", false)),
        "start_exposure_scan" => Some(("inventory.exposure.run", true)),
        "resume_exposure_scan" => Some(("inventory.exposure.run", true)),
        "get_exposure_settings" => Some(("inventory.read", false)),
        "save_exposure_settings" => Some(("inventory.exposure.manage", true)),
        "list_exposure_setting_versions" => Some(("inventory.read", false)),
        "page_exposure_pending_discoveries" => Some(("inventory.read", false)),
        "page_exposure_clusters" => Some(("inventory.read", false)),
        "page_exposure_cluster_members" => Some(("inventory.read", false)),
        "read_exposure_evidence" => Some(("inventory.read", false)),
        "import_exposure_documents" => Some(("inventory.exposure.manage", true)),
        "reclassify_exposure_finding" => Some(("inventory.exposure.manage", true)),
        "grade_exposure_intel" => Some(("inventory.exposure.manage", true)),
        _ => None,
    }
}
pub(super) fn secondary_application_permission(name: &str) -> Option<(&'static str, bool)> {
    match name {
        "save_project_ownership_directory" => Some(("inventory.ownership.directory.manage", true)),
        _ => None,
    }
}

// Credentials are write-only on the application page. Configured booleans are
// derived from the private values; the original persistence owner still applies
// settings_for_update, retired-field protection and its original SQL.
fn redacted_configuration_value(value: Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut view = serde_json::Map::new();
            for (key, value) in fields {
                // Client-supplied display flags never override facts derived
                // from the actual private credential values.
                if key.ends_with("Configured") {
                    continue;
                }
                if crate::agent_runtime::secrets::is_secret_field(&key) {
                    let configured = value.as_str().is_some_and(|v| !v.trim().is_empty());
                    view.insert(format!("{key}Configured"), json!(configured));
                    view.insert(key, Value::String(String::new()));
                } else {
                    view.insert(key, redacted_configuration_value(value));
                }
            }
            Value::Object(view)
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(redacted_configuration_value)
                .collect(),
        ),
        Value::String(value) => Value::String(crate::agent_runtime::secrets::redact_text_with(
            &value, None,
        )),
        other => other,
    }
}
fn model_recipient(value: &Value, flat: bool) -> [String; 3] {
    let keys = if flat {
        ["modelName", "modelApiBase", "modelDeployment"]
    } else {
        ["llm", "apiBase", "deployment"]
    };
    keys.map(|key| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    })
}
fn retain_write_only_credentials(stored: &Value, incoming: &mut Value) -> Result<(), String> {
    fn retain(old: &Value, next: &mut Value, model: bool, flat: bool) -> Result<(), String> {
        let changed = model && model_recipient(old, flat) != model_recipient(next, flat);
        let object = next
            .as_object_mut()
            .ok_or("asset_service_configuration_invalid")?;
        let old = old
            .as_object()
            .ok_or("asset_service_configuration_invalid")?;
        let mut retained = Vec::new();
        for (key, value) in old {
            if crate::agent_runtime::secrets::is_secret_field(key) {
                let blank = object
                    .get(key)
                    .is_none_or(|v| v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty()));
                if blank && value.as_str().is_some_and(|s| !s.trim().is_empty()) {
                    if changed && matches!(key.as_str(), "apiKey" | "localApiKey" | "modelApiKey") {
                        return Err(
                            "asset_service_configuration_recipient_requires_new_credential".into(),
                        );
                    }
                    retained.push((key.clone(), value.clone()));
                }
            }
        }
        object.retain(|key, _| !key.ends_with("Configured"));
        for (key, value) in retained {
            object.insert(key, value);
        }
        Ok(())
    }
    // A profile's own id and recipient own its credentials. Never borrow by
    // array position, and do not treat a local endpoint as the old cloud peer.
    if let Some(next_profiles) = incoming
        .get_mut("modelProfiles")
        .and_then(Value::as_array_mut)
    {
        let old_profiles = stored.get("modelProfiles").and_then(Value::as_array);
        for profile in next_profiles {
            let id = profile
                .get("id")
                .and_then(Value::as_str)
                .ok_or("asset_service_configuration_model_id_invalid")?;
            if let Some(old) = old_profiles.and_then(|list| {
                list.iter()
                    .find(|p| p.get("id").and_then(Value::as_str) == Some(id))
            }) {
                retain(old, profile, true, false)?;
            }
        }
    }
    retain(stored, incoming, true, true)
}

#[path = "service_bootstrap.rs"]
mod bootstrap;
/// Narrow executable facade. Guardian parsing must precede application argv,
/// SDK parser, configuration, store creation and any ordinary service logic.
pub fn run_app_stdio() -> Result<(), String> {
    if let Some(exit_code) = crate::tool_supply::container_parser::guardian_entry() {
        std::process::exit(exit_code)
    }
    crate::native_app_sdk::run_stdio(|bootstrap| {
        Ok(Box::new(bootstrap::Application::open(bootstrap)?))
    })
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
