//! Neutral Scanning↔agent_runtime contract surface for root-capture reads.
//!
//! Implementations remain in `crate::scanning` (include units). This module only
//! hosts the port names agent_runtime may depend on without naming
//! `crate::scanning::…` at each call site. Scanning re-exports the same port.

/// Read-only original root capture / tool-selection policy helpers.
pub(crate) mod scope_contract {
    pub(crate) use crate::scanning::{
        capture_original_tool_selection_policy_on, original_root_capture_mode,
    };
}
