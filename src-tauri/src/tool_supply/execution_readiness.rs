//! Read-only supply/platform admission. This does not grant a role, claim a
//! process, execute a package, or manufacture an execution receipt.
use super::{approval, frozen_plan, ToolAdministratorConfiguration, ToolCapabilityManifest};
use rusqlite::Transaction;
use serde_json::json;

/// No JSON constructor, caller-selected runner, or support boolean exists.
/// A Broker must still prove its original worker capability and input scope.
pub(crate) struct VerifiedToolExecutionReadiness {
    current: frozen_plan::VerifiedCurrentToolPlan,
    approval: approval::VerifiedToolApproval,
    container: Option<super::container_parser::Runtime>,
}
impl VerifiedToolExecutionReadiness {
    pub(crate) fn original_plan(&self) -> &frozen_plan::VerifiedCurrentToolPlan {
        &self.current
    }
    pub(crate) fn original_approval(&self) -> &approval::VerifiedToolApproval {
        &self.approval
    }
    pub(crate) fn container(&self) -> Option<&super::container_parser::Runtime> {
        self.container.as_ref()
    }
}

fn platform() -> String {
    let os = std::env::consts::OS;
    format!("{os}-{}", std::env::consts::ARCH)
}

fn require_fixed_parser_contract(
    manifest: &ToolCapabilityManifest,
    runtime: &str,
) -> Result<(), String> {
    if manifest.platform != platform() {
        return Err("tool_execution_platform_mismatch".into());
    }
    // This names the sole prospective fixed parser adapter, not arbitrary
    // package entrypoints or an assertion that this adapter already executes.
    if runtime != "offline-parser-v1" && runtime != super::container_parser::ADAPTER {
        return Err("tool_execution_runtime_unsupported".into());
    }
    if manifest.network_policy != "none"
        || manifest.filesystem_policy != "read-only-input-and-private-temp-output"
        || manifest.process_limit != 1
        || manifest.cpu_limit != 1
        || manifest.memory_mib == 0
        || manifest.timeout_seconds == 0
        || manifest.output_bytes == 0
        || manifest.input_schema_version != 1
        || manifest.output_schema_version != 1
    {
        return Err("tool_execution_parser_policy_unsupported".into());
    }
    Ok(())
}

fn require_actual_supported_parser() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        super::macos_static_parser::MacosStaticParserReadiness::inspect()
            .require_supported()
            .map_err(str::to_owned)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("unsupported_sandbox:fixed_parser_platform_adapter_unavailable".into())
    }
}

pub(crate) fn require_current_on(
    tx: &Transaction<'_>,
    configuration: &ToolAdministratorConfiguration,
    root: &str,
    approval_id: &str,
) -> Result<VerifiedToolExecutionReadiness, String> {
    let current = frozen_plan::require_current_on(tx, configuration, root)?;
    if !uuid::Uuid::parse_str(approval_id).is_ok_and(|id| id.to_string() == approval_id) {
        return Err("tool_execution_approval_id_invalid".into());
    }
    let tools = current.fact()["tools"]
        .as_array()
        .ok_or("tool_execution_original_inventory_changed")?;
    let selected: Vec<_> = tools
        .iter()
        .filter(|item| item["approvalId"].as_str() == Some(approval_id))
        .collect();
    if selected.len() != 1 {
        return Err("tool_execution_approval_not_frozen".into());
    }
    let approved = approval::read_original_on(tx, configuration, approval_id)?
        .ok_or("tool_execution_original_approval_missing")?;
    let expected = json!({
        "approvalId": approved.id(), "approvalHash": approved.envelope_hash(),
        "manifest": approved.manifest(), "runtimeAdapter": approved.runtime_adapter(),
        "runtimeDigest": approved.runtime_digest(),
    });
    if selected[0] != &expected {
        return Err("tool_execution_original_inventory_changed".into());
    }
    require_fixed_parser_contract(approved.manifest(), approved.runtime_adapter())?;
    // The original host parser remains unsupported. The explicit container
    // adapter binds its complete fixed mechanism to the original administrator
    // digest; each execution additionally verifies its actual cgroup policy.
    // Neither this inventory proof nor container availability grants a Worker.
    let container = if approved.runtime_adapter() == super::container_parser::ADAPTER {
        let runtime = super::container_parser::Runtime::inspect()?;
        if runtime.digest() != approved.runtime_digest() {
            return Err("tool_execution_runtime_digest_mismatch".into());
        }
        Some(runtime)
    } else {
        require_actual_supported_parser()?;
        None
    };
    Ok(VerifiedToolExecutionReadiness {
        current,
        approval: approved,
        container,
    })
}

