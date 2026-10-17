//! Neutral Scanning↔agent_runtime contract surface (ports + pure signing types).
//!
//! Implementations remain in `crate::scanning` (include units). This module only
//! hosts the port names agent_runtime may depend on without naming
//! `crate::scanning::…` at each call site. Scanning re-exports the same ports.

/// Read-only original root capture / tool-selection policy helpers.
pub(crate) mod scope_contract {
    pub(crate) use crate::scanning::{
        capture_original_tool_selection_policy_on, original_root_capture_mode,
    };
}

/// Financial-continuation birth / authorization port (pure provenance checks + capability type).
pub(crate) mod financial_continuation {
    pub(crate) use crate::scanning::{
        FinancialContinuationAuthorization, native_coordinator_require_original_model_on,
        verify_financial_continuation_birth_on,
    };
}

/// Protection-recovery approval / draft / cancellation type surface (pure types for signing).
pub(crate) mod recovery_approval_signature {
    pub(crate) use crate::scanning::recovery_approval_signature::{
        FreshProtectionRecoveryApproval, FreshProtectionRecoveryCancellation,
        FreshProtectionRecoveryDraft, FreshPricedRecoveryCancellation,
    };
}

/// Fresh / Claimed signing-surface types (pure types for birth-bound issuers).
/// Impl modules stay in `scanning`; agent_runtime names this port only.
pub(crate) mod fresh_signature {
    pub(crate) use crate::scanning::authorization_output_receipt::FreshAuthorizationOutputReceipt;
    pub(crate) use crate::scanning::authorization_side::FreshAuthorizationSideReceipt;
    pub(crate) use crate::scanning::business_logic_dispatch_receipt::FreshBusinessLogicDispatchReceipt;
    pub(crate) use crate::scanning::business_logic_output_receipt::FreshBusinessLogicOutputReceipt;
    pub(crate) use crate::scanning::external_surface_output_receipt::FreshExternalSurfaceOutputReceipt;
    pub(crate) use crate::scanning::external_surface_response_receipt::{
        ClaimedPublicSurfaceWitness, FreshPublicSurfaceResponse,
    };
    pub(crate) use crate::scanning::homogeneous_execution::FreshFact;
    pub(crate) use crate::scanning::http_budget_receipt::FreshHttpBudgetClaim;
    pub(crate) use crate::scanning::http_exchange::FreshProtectionResponse;
}
