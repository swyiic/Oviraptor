//! Provider billing algorithm shared by current and original receipt owners.
use super::{verify_for_attempt, ReceiptOwner};
use crate::agent_runtime::{
    multi_agent::{
        budget::{Kind, DIMENSIONS},
        lease::CoordinatorLease,
    },
    store::UsageDelta,
};
use rusqlite::{params, Transaction};

#[allow(clippy::too_many_arguments)]
pub(super) fn record_with_provenance(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    call_source: &str,
    receipt_hash: &str,
    usage: &UsageDelta,
    estimate: Option<i64>,
    reported: bool,
    owner: &ReceiptOwner,
) -> Result<(), String> {
    owner.verify(tx)?;
    let values = [
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.model_requests,
    ];
    if usage.model_requests != 1
        || usage.total_tokens < 0
        || values.iter().any(|v| *v < 0)
        || usage.cached_input_tokens > usage.input_tokens
        || usage
            .input_tokens
            .checked_add(usage.output_tokens)
            .is_none_or(|n| n > usage.total_tokens)
    {
        return Err("budget_usage_invalid".into());
    }
    let source = format!("{call_source}:receipt:{receipt_hash}");
    let prefix = super::super::scope::stored_key(
        tx,
        &lease.root_run_id,
        assignment,
        &owner.attempt,
        &format!("received:{call_source}:"),
    )?;
    let mut statement=tx.prepare("SELECT dimension,kind,amount,idempotency_key,source_id FROM agent_budget_entries
        WHERE root_run_id=?1 AND assignment_id=?2 AND substr(idempotency_key,1,length(?3))=?3 AND lease_attempt_id=?4 ORDER BY rowid").map_err(|e|e.to_string())?;
    let existing = statement
        .query_map(
            params![lease.root_run_id, assignment, prefix, owner.attempt],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if !existing.is_empty() {
        for (dimension, kind, amount, key, saved_source) in existing {
            if saved_source != source {
                return Err("budget_entry_replay_conflict".into());
            }
            owner.append(tx, &dimension, Kind::parse(&kind)?, amount, &key, &source)?;
        }
        owner.verify(tx)?;
        return verify_for_attempt(
            tx,
            lease,
            assignment,
            call_source,
            receipt_hash,
            usage,
            &owner.attempt,
        );
    }
    // A provider-reported, internally consistent cost is KNOWN, and is billed as
    // the actual cost even when it exceeds this child's default per-call token
    // grant — in total OR in a single class. That grant is a reservation hint for
    // admission, not the spending authority: the real ceiling is the Root hard
    // limit, enforced on every ledger append below (`budget_hard_limit_exceeded`),
    // and a class overrun is funded by reserving the extra from the Root budget
    // just before it is consumed. Children such as external_surface hold a small
    // grant (e.g. 2000 tokens) yet legitimately spend more on a real page, so
    // forfeiting the overrun to an indeterminate balance wedged the whole branch
    // on `budget_indeterminate_requires_reconciliation`. Only an unreported or
    // self-inconsistent usage now stays unknown (and is forfeited against the
    // held estimate); a reported overrun is settled elastically within the Root
    // budget, mirroring the Root tick's own v3 actual-cost path.
    let known = reported
        && usage.input_tokens.checked_add(usage.output_tokens) == Some(usage.total_tokens);
    let mut reserved = [0; 4];
    for (index, dimension) in DIMENSIONS[..4].iter().enumerate() {
        reserved[index] = owner.balance(tx, dimension)?.reserved;
    }
    for (index, dimension) in DIMENSIONS[..4].iter().enumerate() {
        // Fund a reported class overrun from the Root budget before consuming it.
        // The reserve append is itself bounded by the Root hard limit, so this
        // cannot spend past the frozen ceiling; beyond it the append fails
        // cleanly as `budget_hard_limit_exceeded` rather than wedging as unknown.
        if known && index < 3 && values[index] > reserved[index] {
            owner.append(
                tx,
                dimension,
                Kind::Reserve,
                values[index] - reserved[index],
                &format!("receipt-reserve:{call_source}:{dimension}"),
                &source,
            )?;
        }
        let kind = if index == 3 || known {
            Kind::Consume
        } else {
            Kind::Forfeit
        };
        let amount = if kind == Kind::Consume {
            values[index]
        } else {
            reserved[index].min(estimate.unwrap_or(reserved[index]))
        };
        if amount > 0 {
            owner.append(
                tx,
                dimension,
                kind,
                amount,
                &format!("{prefix}{dimension}"),
                &source,
            )?;
        }
    }
    owner.verify(tx)?;
    verify_for_attempt(
        tx,
        lease,
        assignment,
        call_source,
        receipt_hash,
        usage,
        &owner.attempt,
    )
}
