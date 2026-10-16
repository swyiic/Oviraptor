//! Trusted process inputs and display observations, separate from scan authority.
//! These types are not RPC inputs. Original startup, lease, finance and receipt
//! checks remain inside the Scanning owner for every execution.
use crate::{
    agent_runtime::model::diagnostics::Hint as SdkHint,
    native_pipeline::process::log::Hint as ProcessHint,
    tool_supply::ToolAdministratorConfiguration,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tauri::{AppHandle, Emitter};

pub(crate) trait ScanEnvironment {
    fn frontend_worker(&self) -> Result<PathBuf, String>;
    fn tool_configuration(&self) -> Result<ToolAdministratorConfiguration, String>;
}

/// Preserve desktop resolution timing: source-only work never resolves a
/// browser resource or tool-administrator configuration merely by constructing
/// this adapter.
pub(crate) struct DesktopScanEnvironment<'a> {
    app: &'a AppHandle,
}

impl<'a> DesktopScanEnvironment<'a> {
    pub(crate) fn new(app: &'a AppHandle) -> Self {
        Self { app }
    }
}

impl ScanEnvironment for DesktopScanEnvironment<'_> {
    fn frontend_worker(&self) -> Result<PathBuf, String> {
        crate::worker_resources::resolve_frontend_recon_worker(self.app)
    }

    fn tool_configuration(&self) -> Result<ToolAdministratorConfiguration, String> {
        ToolAdministratorConfiguration::from_app(self.app)
    }
}

/// A future native service bootstrap supplies its own existing canonical roots.
/// It cannot obtain them from an Agent argument, inherit a desktop DB path, or
/// infer that their existence grants execution. No service is launched here.
#[allow(dead_code)]
pub(crate) struct ScanServiceEnvironment {
    data_root: PathBuf,
    configuration_root: PathBuf,
    resource_root: PathBuf,
}

#[allow(dead_code)]
impl ScanServiceEnvironment {
    pub(crate) fn from_roots(
        data_root: &Path,
        configuration_root: &Path,
        resource_root: &Path,
    ) -> Result<Self, String> {
        Ok(Self {
            data_root: checked_root(data_root, true)?,
            configuration_root: checked_root(configuration_root, true)?,
            resource_root: checked_root(resource_root, false)?,
        })
    }

    pub(crate) fn data_root(&self) -> &Path {
        &self.data_root
    }
}

impl ScanEnvironment for ScanServiceEnvironment {
    fn frontend_worker(&self) -> Result<PathBuf, String> {
        checked_root(&self.resource_root, false)?;
        crate::worker_resources::resolve_frontend_recon_worker_in(&self.resource_root)
    }

    fn tool_configuration(&self) -> Result<ToolAdministratorConfiguration, String> {
        checked_root(&self.configuration_root, true)?;
        ToolAdministratorConfiguration::from_service_root(&self.configuration_root)
    }
}

fn checked_root(path: &Path, private: bool) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "scan_service_root_unavailable")?;
    if !path.is_absolute() || !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("scan_service_root_invalid".into());
    }
    let canonical = fs::canonicalize(path).map_err(|_| "scan_service_root_unavailable")?;
    if canonical != path {
        return Err("scan_service_root_not_canonical".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let forbidden = if private { 0o077 } else { 0o022 };
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & forbidden != 0 {
            return Err("scan_service_root_permissions".into());
        }
    }
    #[cfg(not(unix))]
    let _ = private;
    Ok(canonical)
}

/// Only the three existing display-notification kinds cross this boundary.
/// Payloads are observations, never task admission, model output or invoices.
#[derive(Clone, Copy)]
pub(crate) enum ScanEvent<'a> {
    RunnerLog(&'a serde_json::Value),
    SdkLog(&'a SdkHint),
    ProcessLog(&'a ProcessHint),
}

impl ScanEvent<'_> {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::RunnerLog(_) => "nest-runner-log",
            Self::SdkLog(_) => "nest-native-sdk-log",
            Self::ProcessLog(_) => "nest-native-process-log",
        }
    }

    fn failure_code(self) -> &'static str {
        match self {
            Self::RunnerLog(_) => "runner_log_notification_unavailable",
            Self::SdkLog(_) => "native_sdk_log_notification_unavailable",
            Self::ProcessLog(_) => "native_process_log_notification_unavailable",
        }
    }
}

pub(crate) trait ScanEventSink: Send + Sync {
    fn emit(&self, event: ScanEvent<'_>) -> Result<(), String>;
}

struct DesktopScanEventSink(AppHandle);

impl ScanEventSink for DesktopScanEventSink {
    fn emit(&self, event: ScanEvent<'_>) -> Result<(), String> {
        let result = match event {
            ScanEvent::RunnerLog(payload) => self.0.emit(event.name(), payload),
            ScanEvent::SdkLog(hint) => self.0.emit(event.name(), hint),
            ScanEvent::ProcessLog(hint) => self.0.emit(event.name(), hint),
        };
        result.map_err(|_| event.failure_code().into())
    }
}

static EVENT_SINK: OnceLock<Arc<dyn ScanEventSink>> = OnceLock::new();

pub(crate) fn install_event_sink(sink: Arc<dyn ScanEventSink>) -> Result<(), String> {
    EVENT_SINK
        .set(sink)
        .map_err(|_| "scan_event_sink_already_installed".into())
}

pub(crate) fn install_desktop_event_sink(app: AppHandle) {
    // The old desktop OnceLock kept the first installed handle as well.
    let _ = install_event_sink(Arc::new(DesktopScanEventSink(app)));
}

fn emit_to(sink: Option<&dyn ScanEventSink>, event: ScanEvent<'_>) -> Result<(), String> {
    match sink {
        Some(sink) => sink.emit(event).map_err(|_| event.failure_code().into()),
        None => Ok(()),
    }
}

pub(crate) fn publish_event(event: ScanEvent<'_>) -> Result<(), String> {
    emit_to(EVENT_SINK.get().map(Arc::as_ref), event)
}

#[cfg(test)]
#[path = "service_environment_tests.rs"]
mod tests;
