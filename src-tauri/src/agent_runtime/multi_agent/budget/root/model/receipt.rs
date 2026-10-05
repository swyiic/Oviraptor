//! Settle only this call's reservation, while preserving its original invoice.
use super::RootModelCall;
use crate::agent_runtime::{
    model::gateway::ModelResponse,
    multi_agent::budget::{self, Kind, DIMENSIONS},
};
use rusqlite::{params, Transaction};

mod actual;

pub(super) fn settle(
    tx: &Transaction<'_>,
    call: &RootModelCall,
    phase: &str,
    response: Option<&ModelResponse>,
) -> Result<bool, String> {
    let mut known = false;
    let mut values = [0i64; 4];
    let limit: Option<i64> = tx.query_row(
        "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='model_input_tokens'",
        [&call.owner.root], |r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if let Some(response) = response {
        let u = &response.usage;
        values = [
            u.input_tokens,
            u.cached_input_tokens,
            u.output_tokens,
            u.model_requests,
        ];
        known = call.known_cost_before_settlement(tx, response)?;
    }
    for (index, dimension) in DIMENSIONS[..4].iter().enumerate() {
        let mut held = if index == 3 { 1 } else { call.tokens };
        let key = format!("root-model:{}:{dimension}:reserve", call.id);
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=''
            AND lease_attempt_id=?2 AND dimension=?3 AND kind='reserve' AND amount=?4 AND source_id=?5 AND idempotency_key=?6)",
            params![call.owner.root,call.owner.id,dimension,held,call.id,budget::root::stored_key(tx,&call.owner.root,&call.owner.id,&key)?],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !exact {
            return Err("budget_root_original_reservation_conflict".into());
        }
        if phase == "received" && known && index < 3 && values[index] > held {
            if limit.is_some() {
                call.fund_original_actual_cost(tx, response.ok_or("budget_root_terminal_invalid")?, dimension, values[index] - held)?;
            } else {
            call.owner.append_known_cost(
                tx,
                dimension,
                values[index] - held,
                &format!("root-model:{}:{dimension}:unlimited", call.id),
                &call.id,
            )?;
            }
            held = values[index];
        }
        let (kind, amount) = match phase {
            "unsent" => (Kind::Release, held),
            "uncertain" => (Kind::Forfeit, held),
            "received" if index == 3 => (Kind::Consume, 1),
            "received" if known => (Kind::Consume, values[index]),
            "received" => (Kind::Forfeit, held),
            _ => return Err("budget_root_terminal_invalid".into()),
        };
        if amount > 0 {
            call.owner.append(
                tx,
                dimension,
                kind,
                amount,
                &format!("root-model:{}:{dimension}:terminal", call.id),
                &call.id,
            )?;
        }
        if phase == "received" && (index == 3 || known) && held > amount {
            call.owner.append(
                tx,
                dimension,
                Kind::Release,
                held - amount,
                &format!("root-model:{}:{dimension}:release", call.id),
                &call.id,
            )?;
        }
    }
    call.owner.verify(tx)?;
    Ok(phase == "uncertain" || phase == "received" && !known)
}
