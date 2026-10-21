//! Public SDK adapter; no public caller selects an engine path or model key.
use super::*;
use crate::native_app_sdk::{Bootstrap, CallSurface, NativeApplication, NativeCallContext};
use crate::scanning as owner;

pub(crate) struct Application {
    service: Service,
    app_revision: Option<i64>,
}

struct NativeScanEventSink(crate::native_app_sdk::EventSink);
impl super::super::service_environment::ScanEventSink for NativeScanEventSink {
    fn emit(&self, event: super::super::service_environment::ScanEvent<'_>) -> Result<(), String> {
        use super::super::service_environment::ScanEvent;
        let payload = match event {
            ScanEvent::RunnerLog(payload) => payload.clone(),
            ScanEvent::SdkLog(payload) => {
                serde_json::to_value(payload).map_err(|_| "scan_event_encoding_failed")?
            }
            ScanEvent::ProcessLog(payload) => {
                serde_json::to_value(payload).map_err(|_| "scan_event_encoding_failed")?
            }
        };
        self.0.notify(event.name(), &payload)
    }
}

fn required_permission(name: &str) -> Option<&'static str> {
    match name {
        "scan_draft_create" => Some("investigation.draft.write"),
        "source_import" => Some("source.import"),
        "scan_workbench_start" => Some("investigation.execute"),
        "scan_draft_read" | "scan_status" | "scan_list" => Some("investigation.read"),
        "scan_start" | "scan_resume" | "scan_retry" => Some("investigation.execute"),
        "scan_pause" | "scan_draft_cancel" => Some("investigation.control"),
        "configure_model" => Some("configuration.write"),
        "configuration_status" => Some("configuration.read"),
        _ => None,
    }
}

fn internal_permission(value: &str) -> Option<&'static str> {
    match value {
        "investigation.draft.write" => Some("draft.write"),
        "investigation.read" => Some("draft.read"),
        "investigation.execute" => Some("scan.execute"),
        "investigation.control" => Some("scan.control"),
        "source.import" => Some("source.import"),
        _ => None,
    }
}

impl Application {
    pub(crate) fn open(input: &Bootstrap) -> Result<Self, String> {
        if input.app_id != "org.oviraptor.scanning" {
            return Err("scan_application_identity_invalid".into());
        }
        if input.data_schema_version != 2 {
            return Err("scan_execution_schema_unsupported".into());
        }
        // The host manages data/package roots; tools and pages cannot choose
        // these values. The original environment checks every root itself.
        let configuration = input.data_dir.join("configuration");
        match fs::create_dir(&configuration) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&configuration, fs::Permissions::from_mode(0o700))
                        .map_err(|_| "scan_configuration_directory_unavailable")?;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("scan_configuration_directory_unavailable".into()),
        }
        let environment = ScanServiceEnvironment::from_roots(
            &input.data_dir,
            &configuration,
            &input.package_dir,
        )?;
        let service = Service::open(environment)?;
        execution::enable(&service)?;
        #[cfg(unix)]
        sources::enable(&service)?;
        #[cfg(unix)]
        workbench::enable(&service)?;
        service.register_process_custody()?;
        super::super::service_environment::install_event_sink(std::sync::Arc::new(
            NativeScanEventSink(input.events.clone()),
        ))?;
        Ok(Self {
            service,
            app_revision: None,
        })
    }

    fn provision(&mut self, name: &str, context: &NativeCallContext) -> Result<Context, String> {
        let permission = required_permission(name).ok_or("scan_method_unavailable")?;
        let input = Context {
            host_bound_project_id: context.host_bound_project_id,
            grant_revision: context.grant_revision,
            operation_id: context.operation_id.clone(),
        };
        input.validate()?;
        context
            .validate(permission)
            .map_err(|_| "scan_host_scope_permission_denied")?;
        if self.app_revision.is_some_and(|r| r != context.app_revision) {
            return Err("scan_application_revision_changed".into());
        }
        self.service.set_execution_project(HostProject {
            project_id: context.host_bound_project_id,
            name: context.project.name.clone(),
            status: context.project.status.clone(),
            grant_revision: context.grant_revision,
            permissions: context
                .permissions
                .iter()
                .filter_map(|p| internal_permission(p).map(str::to_owned))
                .collect(),
        })?;
        self.app_revision = Some(context.app_revision);
        Ok(input)
    }

    fn configure(&self, input: ModelConfiguration, context: &Context) -> Result<Value, String> {
        input.validate()?;
        self.service.require_custody(&self.service.database)?;
        let settings = json!({"modelProfiles":[{"id":"native-service-operator","llm":input.llm,"deployment":input.deployment,"apiBase":input.api_base,"apiKey":if input.deployment=="cloud" {input.api_key.clone()} else {String::new()},"localApiKey":if input.deployment=="local" {input.api_key} else {String::new()}}],"activeModelProfileId":"native-service-operator"});
        // This operator action changes only the independent models default. It
        // never creates/rebinds a historical profile or contacts a provider.
        let desired = crate::model_configuration::model_connection(
            &crate::model_configuration::NormalizedModelSettings::new(&settings),
        )?;
        let mut hash = Sha256::new();
        hash.update(b"oviraptor.scanning.configure-model.v1\0");
        hash.update(self.service.instance_id.as_bytes());
        hash.update(context.host_bound_project_id.to_be_bytes());
        hash.update(context.operation_id.as_bytes());
        let digest = hash.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let request_id = uuid::Uuid::from_bytes(bytes).to_string();
        let mut db = crate::db::open(&self.service.db_path)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| "scan_configuration_write_failed")?;
        let result =
            if let Some(previous) = crate::configuration_catalog::read_receipt(&tx, &request_id)? {
                // Lost-response retries retain their original receipt, even if an
                // operator has since selected another model. Changed payload under
                // the same operation ID is rejected instead of rewriting intent.
                let reference = previous
                    .receipt
                    .reference
                    .as_ref()
                    .filter(|r| r.kind == "models")
                    .ok_or("scan_configuration_operation_conflict")?;
                let stored = crate::configuration_catalog::read_reference(&tx, reference)?;
                let stored = crate::model_configuration::model_connection(
                    &crate::model_configuration::NormalizedModelSettings::new(&stored),
                )?;
                if stored.llm != desired.llm
                    || stored.deployment != desired.deployment
                    || stored.api_base != desired.api_base.trim_end_matches('/')
                    || stored.api_key != desired.api_key
                {
                    return Err("scan_configuration_operation_conflict".into());
                }
                previous
            } else {
                let current = crate::configuration_catalog::read_category(&tx, "models", None)?;
                let input = crate::configuration_catalog::SaveCategoryInput {
                    request_id,
                    section: "models".into(),
                    mode: if current.reference.is_some() {
                        crate::configuration_catalog::SaveMode::Revise
                    } else {
                        crate::configuration_catalog::SaveMode::Create
                    },
                    name: String::new(),
                    settings,
                    expected_settings: current.settings,
                    expected_reference: current.reference,
                    expected_default: current.default_selection,
                    make_default: true,
                };
                crate::configuration_catalog::save_on(&tx, &input)?
            };
        self.service.require_custody(&self.service.database)?;
        tx.commit()
            .map_err(|_| "scan_configuration_commit_unconfirmed_retry_same_operation")?;
        Ok(
            json!({"configured":true,"profileId":0,"configurationSource":"current-defaults",
            "modelReference":result.receipt.reference,"configurationReceipt":result.receipt,
            "currentState":result.current_state,"providerRequestSent":false}),
        )
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelConfiguration {
    llm: String,
    deployment: String,
    api_base: String,
    api_key: String,
}
impl ModelConfiguration {
    fn validate(&self) -> Result<(), String> {
        valid_text(&self.llm, 200, false)?;
        if !matches!(self.deployment.as_str(), "cloud" | "local") {
            return Err("scan_configuration_invalid".into());
        }
        valid_text(&self.api_base, 2048, false)?;
        valid_text(&self.api_key, 8192, self.deployment == "local")?;
        let url = reqwest::Url::parse(&self.api_base).map_err(|_| "scan_configuration_invalid")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err("scan_configuration_invalid".into());
        }
        Ok(())
    }
}

impl NativeApplication for Application {
    fn data_schema_version(&self) -> u64 {
        2
    }
    fn call(
        &mut self,
        surface: CallSurface,
        name: &str,
        args: Value,
        context: &NativeCallContext,
    ) -> Result<Value, String> {
        if self.service.stopped {
            return Err("scan_service_stopped".into());
        }
        if name == "source_import" && !matches!(surface, CallSurface::ProjectAction) {
            return Err("scan_surface_permission_denied".into());
        }
        let application_action = matches!(name, "configure_model" | "configuration_status");
        if application_action != matches!(surface, CallSurface::ApplicationAction) {
            return Err("scan_surface_permission_denied".into());
        }
        match name {
            #[cfg(unix)]
            "scan_workbench_start" => {
                let input: workbench::Start = serde_json::from_value(args.clone())
                    .map_err(|_| "scan_workbench_arguments_invalid")?;
                if matches!(surface, CallSurface::Tool) {
                    input.validate_tool()?;
                } else {
                    input.validate()?;
                }
            }
            #[cfg(unix)]
            "source_import" => {
                let input: sources::Import = serde_json::from_value(args.clone())
                    .map_err(|_| "scan_source_arguments_invalid")?;
                // Validate the entire supplied snapshot before project provisioning.
                input.validate()?;
            }
            "scan_list" => {
                let input: execution::ListTasks =
                    serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
                input.validate(&self.service.instance_id, context.host_bound_project_id)?;
            }
            "scan_draft_create" => {
                let draft: NewDraft =
                    serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
                if matches!(surface, CallSurface::Tool) && draft.targets.len() > 32 {
                    return Err("scan_arguments_invalid".into());
                }
                draft.validate()?;
            }
            "scan_draft_read" | "scan_status" | "scan_start" | "scan_pause" | "scan_resume"
            | "scan_retry" | "scan_draft_cancel" => {
                let input: ReadTask =
                    serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
                valid_text(&input.task_id, 128, false)?;
            }
            "configure_model" => {
                let input: ModelConfiguration = serde_json::from_value(args.clone())
                    .map_err(|_| "scan_configuration_invalid")?;
                input.validate()?;
            }
            "configuration_status" if args == json!({}) => {}
            _ => return Err("scan_method_unavailable".into()),
        }
        let context = self.provision(name, context)?;
        match name {
            #[cfg(unix)]
            "scan_workbench_start" => workbench::start(&self.service,&context,serde_json::from_value(args).map_err(|_| "scan_workbench_arguments_invalid")?),
            #[cfg(unix)]
            "source_import" => sources::import(&self.service,&context,serde_json::from_value(args).map_err(|_| "scan_source_arguments_invalid")?),
            "scan_list"=>execution::list(&self.service,&context,serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?),
            "scan_draft_create" => self.service.create(
                &context,
                serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?,
            ).map(|receipt|json!({"taskId":receipt["taskId"],"creationReceipt":receipt,"executionAvailable":true})),
            "scan_draft_read" | "scan_status" => {
                self.service.authorize(&context, "draft.read")?;
                let args: ReadTask =
                    serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
                execution::read_task(&self.service, &context, &args.task_id)
            }
            "scan_start" | "scan_pause" | "scan_resume" | "scan_retry" | "scan_draft_cancel" => {
                let args: ReadTask =
                    serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
                let action = match name {
                    "scan_start" => execution::Action::Start,
                    "scan_pause" => execution::Action::Pause,
                    "scan_resume" => execution::Action::Resume,
                    "scan_retry" => execution::Action::Retry,
                    _ => execution::Action::CancelDraft,
                };
                execution::perform(&self.service, &context, action, &args.task_id)
            }
            "configure_model" => self
                .configure(serde_json::from_value(args).map_err(|_| "scan_configuration_invalid")?, &context),
            "configuration_status" if args == json!({}) => {
                self.service.require_custody(&self.service.database)?;
                let settings = owner::sentinel_settings(&self.service.database);
                let configured = owner::model_runtime_env(&settings).is_ok();
                self.service.require_custody(&self.service.database)?;
                Ok(json!({"configured":configured,"credentialsDisclosed":false}))
            }
            _ => Err("scan_method_unavailable".into()),
        }
    }

    fn status(&mut self) -> Result<Value, String> {
        self.service.require_custody(&self.service.database)?;
        #[cfg(unix)]
        let unresolved_source = workbench::unresolved_on(&self.service.database)?;
        #[cfg(not(unix))]
        let unresolved_source = false;
        let busy = unresolved_source || execution::busy(&self.service)?;
        self.service.require_custody(&self.service.database)?;
        Ok(
            json!({"busy":busy,"executionAvailable":true,"oldTaskAdoptionAllowed":false,"schemaOwnership":"private_legacy_compatibility_schema"}),
        )
    }

    fn stop(&mut self) -> Result<Value, String> {
        let result = self.status()?;
        if result["busy"] == false {
            self.service.stopped = true;
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "native_service_application_tests.rs"]
mod tests;
