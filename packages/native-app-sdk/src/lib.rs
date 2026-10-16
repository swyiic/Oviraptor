//! Domain-independent native application SDK. It reads only the declared package,
//! never model settings, credentials, Host databases, or another application's data.
mod driver;
mod manifest;
mod schema;

pub use driver::{run_stdio, EventSink};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

pub const PROTOCOL_VERSION: u64 = 2;
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
pub const MAX_TOOL_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_ACTION_FRAME_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CONTEXT_BYTES: usize = 16 * 1024;
pub const MAX_INPUT_FRAME_BYTES: usize = 1024 * 1024 + MAX_CONTEXT_BYTES;

/// Trusted launch inputs, not a wire DTO. The factory runs only after a validated
/// lifecycle/start. Paths and schema version originate in the installed package.
pub struct Bootstrap {
    pub data_dir: PathBuf,
    pub package_dir: PathBuf,
    pub app_id: String,
    pub data_schema_version: u64,
    pub events: EventSink,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostProject {
    pub id: i64,
    pub name: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeCallContext {
    pub host_bound_project_id: i64,
    pub grant_revision: i64,
    pub operation_id: String,
    pub app_revision: i64,
    pub permissions: Vec<String>,
    pub permission: String,
    pub project: HostProject,
}

impl NativeCallContext {
    /// Domain adapters may repeat this check before their own financial/lease
    /// admission. A context is not a replacement for those domain gates.
    pub fn validate(&self, expected_permission: &str) -> Result<(), String> {
        if !(1..=MAX_SAFE_INTEGER).contains(&self.host_bound_project_id)
            || self.project.id != self.host_bound_project_id
            || !(0..=MAX_SAFE_INTEGER).contains(&self.grant_revision)
            || !(0..=MAX_SAFE_INTEGER).contains(&self.app_revision)
            || self.project.status != "active"
            || !bounded_text(&self.project.name, 256)
            || !bounded_text(&self.operation_id, 128)
            || !manifest::permission_name(&self.permission)
            || self.permission != expected_permission
            || self.permissions.len() > 128
            || self
                .permissions
                .iter()
                .any(|p| !manifest::permission_name(p))
            || self
                .permissions
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.permissions.len()
            || !self.permissions.contains(&self.permission)
        {
            return Err("app_context_invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallSurface {
    Tool,
    ProjectAction,
    ApplicationAction,
}

pub trait NativeApplication {
    /// The schema actually opened/initialized by the domain, not a manifest echo.
    fn data_schema_version(&self) -> u64;
    fn call(
        &mut self,
        surface: CallSurface,
        name: &str,
        args: Value,
        ctx: &NativeCallContext,
    ) -> Result<Value, String>;
    fn status(&mut self) -> Result<Value, String>;
    fn stop(&mut self) -> Result<Value, String>;
}

pub(crate) fn bounded_text(value: &str, bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= bytes && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests;
