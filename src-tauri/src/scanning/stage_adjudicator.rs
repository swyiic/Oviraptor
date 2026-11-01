//! Stage adjudicator skeleton (§9.1 of AUTONOMOUS_OSS_SOURCE_AUDIT_DESIGN).
//!
//! PURE and deterministic: no LLM call, no I/O, no spend. It classifies a
//! finished stage's stop-reason/terminal signal into a gate tier (§9.0) and a
//! default decision, so the pipeline can — in a later, supervised wiring step —
//! auto-absorb tier-1 brittle/false-positive gates instead of hard-failing the
//! whole scan.
//!
//! What this is deliberately NOT, yet:
//!   * It does not touch `reducer.rs` or any live control flow. Flipping a real
//!     "门禁→放行" has to be watched against a real scan's budget, so wiring is a
//!     separate step the user asked to do together.
//!   * It makes no bounded-LLM adjudication (the spending half of §9.1). This
//!     skeleton only encodes the deterministic tier rules that need no model.
//!
//! Safety invariant: `adjudicate_stage` returns `Some(..)` with
//! `auto_absorbed = true` ONLY for tier-1 signals on a known allowlist. Every
//! tier-3 safety signal (and every unknown signal) is never auto-absorbed, so
//! this skeleton can never widen what a scan proceeds past. Tier-3 is matched
//! first so a safety signal can never be shadowed by a looser tier-1/2 match.
#![allow(dead_code)] // Inert until the supervised wiring step reads these verdicts.

use super::agent_stage_decisions::{
    record_stage_decision, StageDecision, StageOutcome,
};
use rusqlite::Connection;

/// A deterministic verdict for a finished stage. `auto_absorbed` is the only
/// field that may (later) change behavior: it is true solely for tier-1
/// brittle/false-positive gates that are safe to continue past without a model
/// call. Tier-2/3 verdicts carry a decision for the record and the UI, but
/// `auto_absorbed = false` means "defer to the existing reducer".
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StageVerdict {
    pub outcome: StageOutcome,
    pub decision: StageDecision,
    pub gate_tier: i64,
    pub auto_absorbed: bool,
    pub rationale: String,
}

// The tokens below are matched as **substrings**, case-sensitive, so one list
// covers both vocabularies this classifier sees: the stage-level raw stop
// reasons (e.g. `root_frame_mapper_schema_invalid`) and the stable terminal
// codes committed per attempt (`src/agent_runtime/contract.rs::terminal_code`,
// e.g. `hard_token_budget`, `confirmed_waf_or_challenge`). A token therefore
// has to be specific enough not to hit a benign code like `finish_target` or
// `coverage_ledger_complete`.

/// Brittle/false-positive signals (§9.0 tier 1), safe to auto-absorb and keep
/// going: schema-shape wobble (`*_schema_invalid`, the exact family §1.x already
/// salvages case-by-case) and a confirmed WAF/challenge false positive
/// (terminal code `confirmed_waf_or_challenge`).
const TIER1_ABSORB_SIGNALS: &[&str] = &[
    "_schema_invalid",
    "confirmed_waf_or_challenge",
];

/// Resource/budget signals (§9.0 tier 2): not a correctness failure, a limit.
/// Degrade to partial rather than hard-fail — but never silently continue past
/// the limit (that would spend), so the decision is `degrade`, not `continue`,
/// and `auto_absorbed` stays false. `budget` catches every hard/soft budget
/// terminal code and the `*_budget_exhausted` stage reasons; `no_progress` the
/// no-progress window/streak; `rate_limit` the persistent-rate-limit stop.
const TIER2_RESOURCE_SIGNALS: &[&str] = &[
    "budget",
    "no_progress",
    "rate_limit",
];

/// Safety signals (§9.0 tier 3) that must ALWAYS escalate to the user and never
/// be auto-absorbed: outbound fetch / network authorization, execution
/// authorization, human reconciliation of request effects, financial
/// authorization, irreversible writes.
const TIER3_SAFETY_SIGNALS: &[&str] = &[
    "authorization_denied",
    "not_authorized",
    "reconciliation_required",
    "network_authorization",
    "financial",
    "irreversible",
];

fn matches_any(signal: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| signal.contains(needle))
}

/// Classify a finished stage from its stop-reason/terminal signal. Returns
/// `None` when the signal is not on any tier list — meaning "no opinion, use the
/// existing reducer behavior" — rather than forcing a misleading label on an
/// unclassified stop.
pub(crate) fn adjudicate_stage(stage: &str, signal: &str) -> Option<StageVerdict> {
    // Tier 3 first: a safety signal must never be shadowed by a looser match.
    if matches_any(signal, TIER3_SAFETY_SIGNALS) {
        return Some(StageVerdict {
            outcome: StageOutcome::Blocked,
            decision: StageDecision::NeedsUser,
            gate_tier: 3,
            auto_absorbed: false,
            rationale: format!("{stage}:真·安全门禁,升级用户决策(§9.0 第3档)"),
        });
    }
    if matches_any(signal, TIER1_ABSORB_SIGNALS) {
        return Some(StageVerdict {
            outcome: StageOutcome::Degraded,
            decision: StageDecision::RetryAdjusted,
            gate_tier: 1,
            auto_absorbed: true,
            rationale: format!("{stage}:脆门禁/误报,自动吸收并调整重试(§9.0 第1档)"),
        });
    }
    if matches_any(signal, TIER2_RESOURCE_SIGNALS) {
        return Some(StageVerdict {
            outcome: StageOutcome::Blocked,
            decision: StageDecision::Degrade,
            gate_tier: 2,
            auto_absorbed: false,
            rationale: format!("{stage}:资源/预算到顶,降级为 partial(§9.0 第2档)"),
        });
    }
    None
}

/// Adjudicate a stage and, when it classifies, append the verdict to the
/// §9.3 timeline. Returns the verdict (if any) so a later wiring step can act on
/// `auto_absorbed`. Writing the record never moves control flow — see
/// `agent_stage_decisions`. A failed record write is swallowed to a log-less
/// `None`-of-error so adjudication can never itself break a scan; the verdict is
/// still returned.
pub(crate) fn adjudicate_and_record(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    stage: &str,
    signal: &str,
) -> Option<StageVerdict> {
    let verdict = adjudicate_stage(stage, signal)?;
    let _ = record_stage_decision(
        connection,
        scan_id,
        attempt_number,
        stage,
        verdict.outcome,
        verdict.decision,
        verdict.gate_tier,
        &verdict.rationale,
        "{}",
    );
    Some(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier1_schema_wobble_is_auto_absorbed() {
        let verdict = adjudicate_stage("root_frame_mapper", "root_frame_mapper_schema_invalid")
            .expect("classified");
        assert_eq!(verdict.gate_tier, 1);
        assert!(verdict.auto_absorbed);
        assert_eq!(verdict.decision, StageDecision::RetryAdjusted);
    }

    #[test]
    fn tier1_waf_false_positive_is_auto_absorbed() {
        let verdict = adjudicate_stage("probe", "confirmed_waf_or_challenge").expect("classified");
        assert_eq!(verdict.gate_tier, 1);
        assert!(verdict.auto_absorbed);
    }

    #[test]
    fn tier2_budget_degrades_but_is_not_absorbed() {
        let verdict = adjudicate_stage("exec", "budget_model_requests_exhausted").expect("classified");
        assert_eq!(verdict.gate_tier, 2);
        assert!(!verdict.auto_absorbed);
        assert_eq!(verdict.decision, StageDecision::Degrade);
    }

    #[test]
    fn tier3_safety_always_escalates_never_absorbs() {
        let verdict = adjudicate_stage("fetch", "source_fetch_not_authorized").expect("classified");
        assert_eq!(verdict.gate_tier, 3);
        assert!(!verdict.auto_absorbed);
        assert_eq!(verdict.decision, StageDecision::NeedsUser);
    }

    #[test]
    fn tier3_beats_tier1_when_both_substrings_present() {
        // A contrived signal carrying both a safety token and a tier-1 token must
        // resolve to tier 3 — a safety gate can never be auto-absorbed.
        let verdict = adjudicate_stage("fetch", "not_authorized_schema_invalid").expect("classified");
        assert_eq!(verdict.gate_tier, 3);
        assert!(!verdict.auto_absorbed);
    }

    #[test]
    fn unknown_signal_has_no_opinion() {
        assert!(adjudicate_stage("exec", "some_unclassified_reason").is_none());
    }

    #[test]
    fn terminal_budget_codes_classify_as_tier2() {
        // The stable terminal codes from contract.rs::terminal_code must land in
        // tier 2 through the broadened `budget`/`no_progress`/`rate_limit` tokens.
        for code in [
            "hard_token_budget",
            "hard_request_budget",
            "hard_wall_time_budget",
            "soft_budget_without_progress",
            "no_progress_window",
            "persistent_rate_limit",
        ] {
            let verdict = adjudicate_stage("attempt_terminal", code)
                .unwrap_or_else(|| panic!("{code} should classify"));
            assert_eq!(verdict.gate_tier, 2, "{code}");
            assert!(!verdict.auto_absorbed, "{code}");
        }
    }

    #[test]
    fn benign_terminal_codes_have_no_opinion() {
        // A clean finish or a plain configuration error must not be claimed by the
        // agent timeline (configuration is a real error, not a tier-1 absorb).
        for code in [
            "finish_target",
            "coverage_ledger_complete",
            "configuration",
            "model_provider_failure",
        ] {
            assert!(
                adjudicate_stage("attempt_terminal", code).is_none(),
                "{code} should stay unclassified"
            );
        }
    }

    #[test]
    fn reconciliation_required_escalates_to_tier3() {
        let verdict = adjudicate_stage("attempt_terminal", "request_reconciliation_required")
            .expect("classified");
        assert_eq!(verdict.gate_tier, 3);
        assert_eq!(verdict.decision, StageDecision::NeedsUser);
    }
}
