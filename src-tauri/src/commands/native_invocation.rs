use crate::agent_runtime::execution_owner::{claim_native_invocation, NativeInvocationOwner};

/// Admission failures are deliberately outside AgentTargetOutcome: a caller
/// that never owned execution cannot publish terminal facts for another caller.
#[derive(Debug)]
struct OwnedAgentTargetOutcome {
    outcome: AgentTargetOutcome,
    original_terminal: OriginalAgentTerminalIdentity,
    log_path: PathBuf,
    _invocation: NativeInvocationOwner,
}
