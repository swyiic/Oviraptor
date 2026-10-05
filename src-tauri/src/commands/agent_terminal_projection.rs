// The tally and target projection consume the canonical reduced obligations.
// This transforms a completion claim only; it never creates execution evidence.
fn agent_outcome_from_reduction(
    outcome: AgentTargetOutcome,
    reduction: Option<&crate::agent_runtime::reducer::Reduction>,
) -> AgentTargetOutcome {
    use crate::agent_runtime::contract::{terminal_code, TerminalState};
    let Some(reduced) = reduction else {
        return outcome;
    };
    let Some(completion) = outcome.completion().cloned() else {
        return outcome;
    };
    let stop = AgentStop {
        code: match reduced.code.as_str() {
            terminal_code::HARD_WALL_TIME_BUDGET => terminal_code::HARD_WALL_TIME_BUDGET,
            terminal_code::PERSISTENCE_FAILURE => AGENT_STOP_PERSISTENCE,
            terminal_code::EXECUTION_AUTHORIZATION_DENIED => {
                terminal_code::EXECUTION_AUTHORIZATION_DENIED
            }
            terminal_code::REQUEST_RECONCILIATION_REQUIRED => {
                terminal_code::REQUEST_RECONCILIATION_REQUIRED
            }
            terminal_code::LEDGER_COMPLETE => AGENT_STOP_DERIVED,
            _ => outcome.terminal_code(),
        },
        reason: reduced.reason.clone(),
    };
    match reduced.state {
        TerminalState::Completed => AgentTargetOutcome::Completed(completion),
        TerminalState::BoundedCompleted => AgentTargetOutcome::BoundedCompleted(completion),
        TerminalState::Incomplete => AgentTargetOutcome::Incomplete(stop),
        TerminalState::Limited => AgentTargetOutcome::Limited(stop),
        TerminalState::Failed | TerminalState::PersistenceFailure => {
            AgentTargetOutcome::Failed(stop)
        }
        TerminalState::ResumeIncompatible => AgentTargetOutcome::ResumeIncompatible(stop),
        TerminalState::Cancelled => AgentTargetOutcome::Cancelled,
    }
}
