//! Original Root request/semantic decision/publication; no SDK retry of a bill.
use super::super::RootOwner;
use super::RootModelCall;
use crate::agent_runtime::{model::gateway::ModelResponse, store};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::time::Duration;

mod cost;
mod lifetime;
pub(crate) use lifetime::{require_idle_original, verify_closed_original, INVOCATION_KIND};
mod history;
pub(crate) mod local;
pub(crate) mod decision;
mod publication;
mod public_projection;
mod human_projection;
mod unpublished_projection;
mod terminal_proof;
use terminal_proof::TerminalCapture;
mod timeline;
mod storage;
mod round;
mod observation;
use decision::DecisionSummary;

#[derive(Clone)]
pub(crate) struct Tick {
    call: RootModelCall,
    basis: String,
    request_fact: Value,
    dispatch_proof: Value,
}
#[derive(Clone)]
pub(crate) struct SavedDecision {
    pub(crate) summary: DecisionSummary,
    pub(crate) local_step: Option<local::LocalStep>,
    invoice: Value,
    cost_proof: Value,
    request_proof: Value,
    pub(crate) unknown: bool,
}
pub(crate) enum Begin {
    Dispatch(Tick),
    Saved(Tick, Box<SavedDecision>),
}
pub(crate) struct Received {
    pub(crate) terminal: TerminalCapture,
    pub(crate) decision: Result<SavedDecision, String>,
    pub(crate) unknown: bool,
}
impl Tick {
    pub(crate) fn begin(
        tx: &Transaction<'_>,
        root: &str,
        round: i64,
        basis: &Value,
        request: &Value,
        estimate: i64,
    ) -> Result<Begin, String> {
        super::super::transaction::protect(tx, || {
            Self::begin_on(tx, root, round, basis, request, estimate)
        })
    }
    fn begin_on(
        tx: &Transaction<'_>,
        root: &str,
        round: i64,
        basis: &Value,
        request: &Value,
        estimate: i64,
    ) -> Result<Begin, String> {
        if round < 1 || estimate <= 0 || basis.is_null() || request.is_null() {
            return Err("root_tick_input_invalid".into());
        }
        let saved: Option<(String, String, i64, String, String, String)> = tx
            .query_row(
                "SELECT call_id,lease_attempt_id,round,request_hash,basis_hash,fact_json
             FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'
             AND (round=?2 OR basis_hash=?3)",
                params![root, round, Self::basis_hash(basis)],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let owner = RootOwner::load_original(tx, root)?;
        if owner.contract["root"]["policy"] != "multi" {
            return Err("root_tick_requires_multi_coordinator".into());
        }
        // New calls require an independent original terminal proof. Existing
        // paid Native frames retain their frozen version and exact identity.
        let version = if let Some((_, _, _, _, _, text)) = &saved {
            let value: Value = serde_json::from_str(text).map_err(|_| "root_tick_json_invalid")?;
            match value["request"]["version"].as_i64() {
                Some(v @ 1..=3) => v,
                _ => return Err("root_tick_original_input_conflict".into()),
            }
        } else { 3 };
        let fact = json!({"version":version,"owner":owner.contract,"basis":basis,"request":request});
        let hash = store::stable_hash(&fact.to_string());
        if let Some((id, owner_id, saved_round, saved_hash, basis_hash, text)) = saved {
            let value: Value = serde_json::from_str(&text).map_err(|_| "root_tick_json_invalid")?;
            let canonical = value.to_string();
            if canonical != text
                || value["version"] != 1
                || value["request"] != fact
                || owner_id != owner.id
                || saved_round != round
                || saved_hash != hash
                || basis_hash != Self::basis_hash(basis)
            {
                return Err("root_tick_original_input_conflict".into());
            }
            let tokens: i64 = tx
                .query_row(
                    "SELECT json_extract(receipt_json,'$.reservedTokens')
                FROM agent_root_model_journal WHERE call_id=?1 AND root_run_id=?2
                AND lease_attempt_id=?3 AND round=?4 AND request_hash=?5 AND phase='dispatch'",
                    params![id, root, owner_id, round, hash],
                    |r| r.get(0),
                )
                .map_err(|_| "root_tick_original_dispatch_missing")?;
            let call = RootModelCall {
                owner,
                id,
                round,
                request: hash,
                tokens,
                remaining: Duration::ZERO,
            };
            if tokens != estimate
                || call.id
                    != store::stable_hash(
                        &json!({"owner":call.owner.id,
                "root":root,"round":round,"request":call.request})
                        .to_string(),
                    )
            {
                return Err("root_tick_original_dispatch_conflict".into());
            }
            let tick = Self {
                call,
                basis: basis_hash,
                request_fact: fact,
                dispatch_proof: value["dispatchProof"].clone(),
            };
            tick.verify_request(tx)?;
            let decision = tick
                .load_decision(tx)?
                .ok_or("root_tick_dispatch_unknown_no_retry")?;
            tick.verify_paid(tx, &decision)?;
            tick.verify_original_terminal_proof(tx)?;
            return Ok(Begin::Saved(tick, Box::new(decision)));
        }
        let legacy: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_root_model_journal j
            WHERE j.root_run_id=?1 AND ((j.phase='dispatch' AND NOT EXISTS(SELECT 1
              FROM agent_root_tick_receipts t WHERE t.call_id=j.call_id AND t.phase='request'))
              OR (j.phase='received' AND NOT EXISTS(SELECT 1 FROM agent_root_tick_receipts t
                WHERE t.call_id=j.call_id AND t.phase='publication'))))",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if legacy {
            return Err("root_tick_history_not_recoverable".into());
        }
        let call = RootModelCall::claim_on(tx, root, round, &hash, estimate)?;
        let mut tick = Self {
            call,
            basis: Self::basis_hash(basis),
            request_fact: fact,
            dispatch_proof: Value::Null,
        };
        tick.dispatch_proof = tick.cost_rows(tx, None)?;
        tick.insert(tx, "request", &tick.request_receipt())?;
        tick.verify_request(tx)?;
        tick.call.require_executable(tx)?;
        Ok(Begin::Dispatch(tick))
    }
    // Copy only this already claimed original scope for financial unit tests.
    // This verifier creates no owner, dispatch, budget or executable permission.
    #[cfg(test)]
    pub(crate) fn original_model_call_for_test(&self, db: &Connection) -> Result<RootModelCall,String> {
        self.verify_request(db)?;
        Ok(self.call.clone())
    }
    pub(crate) fn basis(&self) -> &Value {
        &self.request_fact["basis"]
    }
    pub(crate) fn remaining(&self) -> Duration {
        self.call.remaining
    }
    pub(crate) fn require_executable(&self, db: &Connection) -> Result<(), String> {
        self.verify_request(db)?;
        self.verify_local_parents(db)?;
        self.call.require_executable(db)
    }
    /// New frozen financial policy retains the physical invoice before any
    /// additional cost writer. Old Native transactions retain their old shape.
    pub(crate) fn persist_received_invoice(&self, db: &Connection, response: &ModelResponse) -> Result<(), String> {
        if self.request_fact["version"] != 3 { return Ok(()); }
        if !db.is_autocommit() { return Err("root_tick_invoice_requires_own_transaction".into()); }
        let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
        super::super::transaction::protect(&tx, || {
            self.verify_request(&tx)?;
            self.call.insert(&tx, "received", &RootModelCall::received_fact(response))?;
            self.verify_request(&tx)
        })?;
        tx.commit().map_err(|e|e.to_string())
    }
    pub(crate) fn received(
        &self,
        tx: &Transaction<'_>,
        response: &ModelResponse,
    ) -> Result<Received, String> {
        // Parse and sanitize first. Invalid/private/raw response text is never
        // persisted; its original fee/hash still must be retained afterwards.
        let local = self.call.owner.contract["root"].get("localDeliberation").is_some();
        let semantic = if local {
            local::LocalStep::parse(response, &self.request_fact["basis"]).and_then(|step| {
                let summary = match &step { Some(step)=>step.summary()?, None=>DecisionSummary::parse_response(response)? };
                Ok((summary, step))
            })
        } else { DecisionSummary::parse_response(response).map(|s|(s,None)) };
        super::super::transaction::protect(tx, || {
            self.verify_request(tx)?;
            let unknown = self.call.terminal_on(tx, "received", Some(response), "")?;
            // Capture failure remains separate from the already recorded bill.
            let terminal = self.capture_terminal_proof(tx, "received", Some(response));
            let decision = match semantic {
                Ok((summary, local_step)) => {
                    let invoice: Value = tx
                        .query_row(
                            "SELECT receipt_json FROM agent_root_model_journal
                        WHERE call_id=?1 AND phase='received'",
                            [&self.call.id],
                            |r| r.get::<_, String>(0),
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))?;
                    let saved = SavedDecision {
                        summary,
                        local_step,
                        invoice,
                        unknown,
                        cost_proof: self.cost_rows(tx, Some(response))?,
                        request_proof: self.request_row(tx)?,
                    };
                    match self
                        .insert(tx, "decision", &saved.fact())
                        .and_then(|_| self.verify_paid(tx, &saved))
                    {
                        Ok(()) => Ok(saved),
                        Err(_) => {
                            // A failed local semantic publication cannot erase
                            // the already retained original bill. No replay or
                            // dispatch follows a missing/unverified decision.
                            self.call.verify(tx, "received", &saved.invoice)?;
                            Err("root_tick_decision_receipt_unconfirmed".into())
                        }
                    }
                }
                Err(code) => Err(code),
            };
            self.verify_request(tx)?;
            Ok(Received { decision, unknown, terminal })
        })
    }
    pub(crate) fn stopped(
        &self,
        tx: &Transaction<'_>,
        phase: &str,
        code: &str,
    ) -> Result<TerminalCapture, String> {
        super::super::transaction::protect(tx, || {
            self.verify_request(tx)?;
            self.call.terminal_on(tx, phase, None, code)?;
            let terminal = self.capture_terminal_proof(tx, phase, None);
            self.verify_request(tx)?;
            Ok(terminal)
        })
    }
    fn verify_request(&self, db: &Connection) -> Result<(), String> {
        self.call.owner.verify(db)?;
        self.call.verify(db, "dispatch", &self.call.dispatch())?;
        if self.read(db, "request")? != Some(self.request_receipt())
            || self.cost_rows(db, None)? != self.dispatch_proof
        {
            return Err("root_tick_request_changed".into());
        }
        Ok(())
    }
    fn verify_paid(&self, db: &Connection, saved: &SavedDecision) -> Result<(), String> {
        self.verify_request(db)?;
        saved.summary.validate()?;
        if let Some(step) = &saved.local_step {
            if self.call.owner.contract["root"]["localDeliberation"].is_null() { return Err("root_local_history_not_authorized".into()); }
            step.verify(&self.request_fact["basis"])?;
            if step.summary()? != saved.summary { return Err("root_local_summary_changed".into()); }
        }
        if saved.unknown != self.cost_unknown(db, &saved.financial_response()?)? {
            return Err("root_tick_cost_status_changed".into());
        }
        if self.read(db, "decision")? != Some(saved.fact())
            || self.request_row(db)? != saved.request_proof
            || self.cost_rows(db, Some(&saved.financial_response()?))? != saved.cost_proof
        {
            return Err("root_tick_decision_changed".into());
        }
        self.call.verify(db, "received", &saved.invoice)
    }
    fn load_decision(&self, db: &Connection) -> Result<Option<SavedDecision>, String> {
        self.read(db, "decision")?
            .map(|value| {
                let result = SavedDecision {
                    summary: DecisionSummary::from_json(&value["summary"])?,
                    local_step: value.get("localStep").map(|v|local::LocalStep::from_json(v,&self.request_fact["basis"])).transpose()?,
                    invoice: value["invoice"].clone(),
                    cost_proof: value["costProof"].clone(),
                    request_proof: value["requestProof"].clone(),
                    unknown: value["unknown"]
                        .as_bool()
                        .ok_or("root_tick_decision_invalid")?,
                };
                if result.fact() != value {
                    return Err("root_tick_decision_invalid".into());
                }
                Ok(result)
            })
            .transpose()
    }
    fn request_receipt(&self) -> Value {
        json!({"version":1,"request":self.request_fact,"dispatchProof":self.dispatch_proof})
    }
}
