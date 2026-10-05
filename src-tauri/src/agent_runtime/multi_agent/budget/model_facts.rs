//! Fee-only original call facts. These never publish a response or renew work.
use super::receipts;
use crate::agent_runtime::{
    multi_agent::{lease::CoordinatorLease, scheduler::ScheduledChild},
    store::{self, UsageDelta},
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
mod dispatch;
use dispatch::Dispatch;

// Only this exact original pristine claim proves no send. A restored phase
// label, foreign assignment/worker or an already published result cannot refund.
pub(crate) const SOURCE_UNSENT_BINDING: &str = "f.family='source-round'
    AND f.root_run_id=c.root_run_id AND f.assignment_id=c.assignment_id
    AND f.child_run_id=c.child_run_id AND f.lease_attempt_id=x.id
    AND f.round_number=c.round_number AND f.request_hash=c.request_hash
    AND f.lease_epoch=c.lease_epoch AND f.fencing_token=c.fencing_token
    AND f.phase='unsent' AND f.fact_json=json_object('code','model_cancelled_before_transport')
    AND c.state='executing' AND c.response_json='{}' AND c.usage_json='{}'
    AND c.response_hash='' AND c.event_sequence=0 AND c.failure_code='' AND c.finished_at=''
    AND NOT EXISTS(SELECT 1 FROM agent_source_tool_receipts t
      WHERE t.child_run_id=c.child_run_id AND t.round_number=c.round_number)";

pub(crate) enum Fact<'a> {
    Received {
        hash: &'a str,
        usage: &'a UsageDelta,
        reported: bool,
    },
    Uncertain,
    Unsent,
}

impl Fact<'_> {
    fn content(&self) -> (&'static str, Value) {
        match self {
            Self::Received {
                hash,
                usage,
                reported,
            } => (
                "received",
                json!({"responseHash":hash,"usage":usage.as_json(),"usageReported":reported}),
            ),
            Self::Uncertain => ("uncertain", json!({"code":"model_outcome_unknown"})),
            Self::Unsent => ("unsent", json!({"code":"model_cancelled_before_transport"})),
        }
    }
}

pub(crate) fn specialist(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    request_hash: &str,
    fact: Fact<'_>,
) -> Result<(), String> {
    record(
        tx,
        Dispatch {
            family: "specialist",
            lease,
            child,
            round: 1,
            request_hash,
            estimate: None,
        },
        fact,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn source_round(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    round: i64,
    request_hash: &str,
    estimate: i64,
    fact: Fact<'_>,
) -> Result<(), String> {
    record(
        tx,
        Dispatch {
            family: "source-round",
            lease,
            child,
            round,
            request_hash,
            estimate: Some(estimate),
        },
        fact,
    )
}

fn record(tx: &Transaction<'_>, call: Dispatch<'_>, fact: Fact<'_>) -> Result<(), String> {
    let owner = receipts::original_owner(
        tx,
        &call.child.run_id,
        call.lease,
        &call.child.assignment_id,
    )?;
    let dispatch = call.snapshot(tx)?;
    let (phase, detail) = fact.content();
    if let Fact::Received { hash, .. } = &fact {
        if hash.len() != 64 {
            return Err("model_cost_receipt_hash_invalid".into());
        }
    }
    let expected = json!([
        call.family,
        call.lease.root_run_id,
        call.child.assignment_id,
        owner.attempt_id(),
        call.child.run_id,
        call.lease.lease_epoch,
        call.lease.fencing_token,
        call.round,
        call.request_hash,
        phase,
        detail.to_string()
    ])
    .to_string();
    let read = |db: &Connection| -> Result<Option<String>, String> {
        db.query_row("SELECT json_array(family,root_run_id,assignment_id,lease_attempt_id,child_run_id,
        lease_epoch,fencing_token,round_number,request_hash,phase,fact_json) FROM agent_model_cost_facts WHERE family=?1 AND child_run_id=?2 AND round_number=?3",
        params![call.family,call.child.run_id,call.round],|r|r.get(0)).optional().map_err(|e|e.to_string())
    };
    if let Some(saved) = read(tx)? {
        if saved != expected {
            return Err("model_cost_fact_replay_conflict".into());
        }
        // Unknown replay has no further financial writes. Its immutable fact
        // and matching original dispatch cannot be changed into a known bill.
        owner.verify(tx)?;
        return Ok(());
    }
    let n=tx.execute("INSERT INTO agent_model_cost_facts(family,root_run_id,assignment_id,lease_attempt_id,child_run_id,
        lease_epoch,fencing_token,round_number,request_hash,phase,fact_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![call.family,call.lease.root_run_id,call.child.assignment_id,owner.attempt_id(),call.child.run_id,
            call.lease.lease_epoch,call.lease.fencing_token,call.round,call.request_hash,phase,detail.to_string()]).map_err(|e|e.to_string())?;
    if n != 1 || read(tx)?.as_deref() != Some(expected.as_str()) {
        return Err("model_cost_fact_write_missing".into());
    }
    match fact {
        Fact::Received {
            hash,
            usage,
            reported,
        } => receipts::record_original_estimated(
            tx,
            &owner,
            &call.source(),
            hash,
            usage,
            call.estimate,
            reported,
        )?,
        Fact::Uncertain => {
            receipts::forfeit_original_call_estimated(tx, &owner, &call.source(), call.estimate)?
        }
        Fact::Unsent => {}
    }
    if call.snapshot(tx)? != dispatch || read(tx)?.as_deref() != Some(expected.as_str()) {
        return Err("model_cost_original_proof_changed".into());
    }
    owner.verify(tx)
}
