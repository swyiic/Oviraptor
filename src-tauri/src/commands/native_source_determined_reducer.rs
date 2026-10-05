// The shared pure reducer chooses the state; the existing Source atomic
// publisher remains the sole writer and preserves original proof/fees.
fn source_determined_terminal_outcome(proof: &SourceCompletionProof) -> AgentTargetOutcome {
    use crate::agent_runtime::{
        contract::{TerminalSignals, TerminalState},
        reducer,
    };
    // This is reached only after every planned phase/receipt/mailbox and
    // independent candidate/coverage obligation has been audited under the TX.
    let signals = TerminalSignals {
        ledger_closed: true,
        pending_contracts: 0,
        evidence_records: proof.completion.verified_tool_results,
        confirmed_findings: proof.completion.confirmed_findings,
        required_families: proof.completion.uncovered_families.clone(),
        covered_families: Vec::new(),
        detail: proof.completion.summary.clone(),
        ..TerminalSignals::default()
    };
    let reduction = reducer::reduce(&signals);
    let mut completion = proof.completion.clone();
    completion.summary = reduction.reason.clone();
    completion.terminal_code = if reduction.state == TerminalState::Completed {
        AGENT_STOP_FINISH
    } else {
        AGENT_STOP_DERIVED
    };
    match reduction.state {
        TerminalState::Completed => AgentTargetOutcome::Completed(completion),
        TerminalState::BoundedCompleted => AgentTargetOutcome::BoundedCompleted(completion),
        _ => AgentTargetOutcome::Incomplete(AgentStop::new(AGENT_STOP_DERIVED, reduction.reason)),
    }
}
