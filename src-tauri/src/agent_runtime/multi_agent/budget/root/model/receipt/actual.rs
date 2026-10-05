//! New frozen Root policy pays a captured invoice inside the original ceiling.
//! Replay reads its original ledger disposition, never today's free capacity.
use super::super::RootModelCall;
use crate::agent_runtime::{model::gateway::ModelResponse, multi_agent::budget};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

fn reported(r: &ModelResponse) -> bool {
    let u = &r.usage;
    r.usage_reported
        && [
            u.input_tokens,
            u.cached_input_tokens,
            u.output_tokens,
            u.total_tokens,
        ]
        .iter()
        .all(|n| *n >= 0)
        && u.model_requests == 1
        && u.cached_input_tokens <= u.input_tokens
        && u.input_tokens.checked_add(u.output_tokens) == Some(u.total_tokens)
}
impl RootModelCall {
    pub(in crate::agent_runtime::multi_agent::budget::root::model) fn actual_cost_policy(
        &self,
        db: &Connection,
    ) -> Result<bool, String> {
        let version: Option<i64> = db.query_row("SELECT json_extract(fact_json,'$.request.version') FROM agent_root_tick_receipts WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND round=?4 AND request_hash=?5 AND phase='request'",params![self.id,self.owner.root,self.owner.id,self.round,self.request],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        match version {
            None | Some(1 | 2) => Ok(false),
            Some(3) => Ok(true),
            _ => Err("root_tick_original_input_conflict".into()),
        }
    }
    pub(in crate::agent_runtime::multi_agent::budget::root::model) fn known_cost_before_settlement(
        &self,
        db: &Connection,
        r: &ModelResponse,
    ) -> Result<bool, String> {
        if !reported(r) {
            return Ok(false);
        }
        let limit: Option<i64> = db.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='model_input_tokens'",[&self.owner.root],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !self.actual_cost_policy(db)? {
            return Ok(limit.is_none() || r.usage.total_tokens <= self.tokens);
        }
        let Ok((remaining, _)) = budget::root::shared::available(db, &self.owner.root) else {
            return Ok(false);
        };
        let extra = r.usage.total_tokens.saturating_sub(self.tokens).max(0);
        if remaining.is_some_and(|n| n < extra) {
            return Ok(false);
        }
        for (dimension, value) in budget::DIMENSIONS[..3].iter().zip([
            r.usage.input_tokens,
            r.usage.cached_input_tokens,
            r.usage.output_tokens,
        ]) {
            let b = budget::balance(db, &self.owner.root, None, dimension)?;
            let projected = b
                .reserved
                .checked_add(b.consumed)
                .and_then(|n| n.checked_add(b.indeterminate))
                .and_then(|n| n.checked_sub(self.tokens))
                .and_then(|n| n.checked_add(value))
                .filter(|n| *n >= 0)
                .ok_or("budget_amount_overflow")?;
            let cap: Option<i64> = db.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",params![self.owner.root,dimension],|r|r.get(0)).map_err(|e|e.to_string())?;
            if cap.is_some_and(|cap| projected > cap) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(in crate::agent_runtime::multi_agent::budget::root::model) fn original_actual_cost_known(
        &self,
        db: &Connection,
        r: &ModelResponse,
    ) -> Result<bool, String> {
        if !self.actual_cost_policy(db)? {
            return Err("root_tick_original_input_conflict".into());
        }
        if !reported(r) {
            return Ok(false);
        }
        let forfeited: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='' AND lease_attempt_id=?2 AND source_id=?3 AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens') AND kind='forfeit')",params![self.owner.root,self.owner.id,self.id],|r|r.get(0)).map_err(|e|e.to_string())?;
        Ok(!forfeited)
    }
    pub(in crate::agent_runtime::multi_agent::budget::root::model) fn fund_original_actual_cost(
        &self,
        tx: &Transaction<'_>,
        r: &ModelResponse,
        dimension: &str,
        amount: i64,
    ) -> Result<(), String> {
        if !self.actual_cost_policy(tx)? || !reported(r) {
            return Err("budget_receipt_transition_invalid".into());
        }
        let value = match dimension {
            "model_input_tokens" => r.usage.input_tokens,
            "model_cached_tokens" => r.usage.cached_input_tokens,
            "model_output_tokens" => r.usage.output_tokens,
            _ => return Err("budget_receipt_transition_invalid".into()),
        };
        if value.checked_sub(self.tokens) != Some(amount) || amount <= 0 {
            return Err("budget_receipt_transition_invalid".into());
        }
        self.owner.verify(tx)?;
        budget::entries::persist_captured_cost(
            tx,
            &self.owner.root,
            self.owner.id.clone(),
            dimension,
            amount,
            &format!("root-model:{}:{dimension}:actual", self.id),
            &self.id,
        )?;
        self.owner.verify(tx)
    }
}
