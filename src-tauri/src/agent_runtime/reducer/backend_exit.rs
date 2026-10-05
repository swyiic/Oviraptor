//! Typed Rust executor exits; model summaries cannot supply this disposition.
use super::{reduce, Reduction};
use crate::agent_runtime::contract::{terminal_code, TerminalSignals, TerminalState};
#[derive(Clone, Copy, Debug)]
pub(crate) enum BackendExit {
    Completed,
    Bounded,
    Incomplete,
    Limited,
    Failed,
    ResumeIncompatible,
    Cancelled,
}
/// Preserve an explicit non-success cause even when no contracts are pending.
/// Successful reports still pass through the ordinary obligation reducer.
pub(crate) fn reduce_backend_exit(
    signals: &TerminalSignals,
    exit: BackendExit,
    code: &str,
    reason: &str,
) -> Reduction {
    let state = match exit {
        BackendExit::Completed => return reduce(signals),
        BackendExit::Bounded => TerminalState::BoundedCompleted,
        BackendExit::Incomplete => TerminalState::Incomplete,
        BackendExit::Limited => TerminalState::Limited,
        BackendExit::Failed if code == terminal_code::PERSISTENCE_FAILURE => {
            TerminalState::PersistenceFailure
        }
        BackendExit::Failed => TerminalState::Failed,
        BackendExit::ResumeIncompatible => TerminalState::ResumeIncompatible,
        BackendExit::Cancelled => TerminalState::Cancelled,
    };
    Reduction {
        state,
        code: code.into(),
        reason: reason.into(),
    }
}
