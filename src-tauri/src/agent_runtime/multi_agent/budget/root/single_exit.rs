//! An original Single owner's final financial interval. Never a resume grant.
use super::RootOwner;
use crate::agent_runtime::multi_agent::budget::{balance, Kind};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

mod proof;
mod writer;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SingleExitFact {
    version: u8,
    receipt_id: String,
    root_run_id: String,
    control_id: String,
    origin: String,
    cutoff: String,
    hard_ms: i64,
    pub(crate) elapsed_ms: i64,
    pub(crate) journaled_ms: i64,
    pub(crate) unsettled_ms: i64,
    pub(crate) exhausted: bool,
    binding: String,
    sources: String,
    wall: String,
}

impl RootOwner {
    pub(crate) fn read_single_exit(&self, db:&Connection)->Result<SingleExitFact,String> {
        if self.coordinator.is_some() || self.contract["root"]["policy"]!="single" {return Err("budget_single_exit_owner_conflict".into());}
        let (id,control,text):(String,String,String)=db.query_row("SELECT receipt_id,control_id,fact_json FROM agent_single_exit_receipts WHERE root_run_id=?1",[&self.root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"budget_single_exit_receipt_missing")?;
        let fact:SingleExitFact=serde_json::from_str(&text).map_err(|_|"budget_single_exit_receipt_invalid")?;
        if fact.receipt_id!=id || fact.control_id!=control || serde_json::to_string(&fact).map_err(|e|e.to_string())?!=text {return Err("budget_single_exit_receipt_conflict".into());}
        fact.verify(db,self)?;Ok(fact)
    }
    pub(crate) fn require_single_deletion_idle(&self, db:&Connection)->Result<Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>,String> {
        let mut owners=super::target::lifetime::require_idle_original(db,self)?;
        if let Some(sdk)=super::model::single_lifetime::require_idle_original(db,self)? {owners.push(sdk);}
        Ok(owners)
    }
    pub(crate) fn close_single_finance(&self, db: &Connection) -> Result<SingleExitFact, String> {
        if self.coordinator.is_some() || self.contract["root"]["policy"] != "single" {
            return Err("budget_single_exit_owner_conflict".into());
        }
        writer::write(db, |tx| self.close_single_on(tx))
    }

    fn close_single_on(&self, tx: &Transaction<'_>) -> Result<SingleExitFact, String> {
        let original = RootOwner::load_single(tx, &self.root)?;
        if original.id != self.id || original.contract != self.contract {
            return Err("budget_root_original_owner_conflict".into());
        }
        let _idle_single_sdk=super::model::single_lifetime::require_idle_original(tx,&original)?;
        let _idle_single_http=super::target::lifetime::require_idle_original(tx,&original)?;
        let saved: Option<(String, String, String)> = tx.query_row(
            "SELECT receipt_id,control_id,fact_json FROM agent_single_exit_receipts WHERE root_run_id=?1",
            [&self.root], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().map_err(|e| e.to_string())?;
        if let Some((id, control, text)) = saved {
            let fact: SingleExitFact =
                serde_json::from_str(&text).map_err(|_| "budget_single_exit_receipt_invalid")?;
            if fact.receipt_id != id
                || fact.control_id != control
                || serde_json::to_string(&fact).map_err(|e| e.to_string())? != text
            {
                return Err("budget_single_exit_receipt_conflict".into());
            }
            fact.verify(tx, self)?;
            return Ok(fact);
        }
        let (status, finished): (String, String) = tx
            .query_row(
                "SELECT status,finished_at FROM agent_runs WHERE id=?1",
                [&self.root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if !matches!(
            status.as_str(),
            "prepared" | "running" | "paused" | "terminal"
        ) || (status == "terminal") != !finished.is_empty()
        {
            return Err("budget_single_exit_scope_conflict".into());
        }
        let cutoff = if status == "terminal" {
            finished
        } else {
            tx.query_row(
                "SELECT strftime('%Y-%m-%d %H:%M:%f','now','localtime')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?
        };
        let (origin, elapsed, hard) = proof::clock(tx, &self.root, &cutoff)?;
        let binding = proof::binding(tx, &self.root)?;
        let sources = proof::sources(tx, &self.root)?;
        let before = balance(tx, &self.root, None, "wall_time_ms")?;
        if before.reserved != 0 || before.indeterminate != 0 {
            return Err("budget_clock_unsettled_sample".into());
        }
        let delta = elapsed
            .checked_sub(before.consumed)
            .filter(|v| *v >= 0)
            .ok_or("budget_clock_moved_backwards")?;
        // Accounting records all elapsed facts, but a finite box never grows.
        // An overrun is retained as unsettled original cost, not a clamped sample.
        let journaled = if elapsed <= hard {
            if delta > 0 {
                let source = format!(
                    "root-clock:{elapsed}:{}",
                    crate::agent_runtime::store::stable_hash(&origin)
                );
                self.append(
                    tx,
                    "wall_time_ms",
                    Kind::Reserve,
                    delta,
                    &format!("{source}:reserve"),
                    &source,
                )?;
                self.append(
                    tx,
                    "wall_time_ms",
                    Kind::Consume,
                    delta,
                    &format!("{source}:consume"),
                    &source,
                )?;
            }
            elapsed
        } else {
            before.consumed
        };
        let fact = SingleExitFact {
            version: 1,
            receipt_id: uuid::Uuid::new_v4().to_string(),
            root_run_id: self.root.clone(),
            control_id: self.id.clone(),
            origin,
            cutoff,
            hard_ms: hard,
            elapsed_ms: elapsed,
            journaled_ms: journaled,
            unsettled_ms: elapsed - journaled,
            exhausted: elapsed >= hard,
            binding,
            sources,
            wall: proof::wall(tx, &self.root)?,
        };
        fact.verify(tx, self)?;
        let text = serde_json::to_string(&fact).map_err(|e| e.to_string())?;
        let changed=tx.execute("INSERT INTO agent_single_exit_receipts(receipt_id,root_run_id,control_id,fact_json) VALUES(?1,?2,?3,?4)",
            params![fact.receipt_id,self.root,self.id,text]).map_err(|e|e.to_string())?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_single_exit_receipts WHERE receipt_id=?1 AND root_run_id=?2 AND control_id=?3 AND fact_json=?4)",
            params![fact.receipt_id,self.root,self.id,text],|r|r.get(0)).map_err(|e|e.to_string())?;
        if changed != 1 || !exact {
            return Err("budget_single_exit_persistence_conflict".into());
        }
        fact.verify(tx, self)?;
        Ok(fact)
    }
}

impl SingleExitFact {
    pub(crate) fn reference(&self) -> (&str, &str, &str) {
        (&self.receipt_id, &self.control_id, &self.cutoff)
    }
    pub(crate) fn verify_original(&self, db: &Connection, owner: &RootOwner) -> Result<(), String> {
        self.verify(db, owner)
    }
    fn verify(&self, db: &Connection, owner: &RootOwner) -> Result<(), String> {
        owner.verify(db)?;
        let (origin, elapsed, hard) = proof::clock(db, &owner.root, &self.cutoff)?;
        let b = balance(db, &owner.root, None, "wall_time_ms")?;
        if self.version != 1
            || uuid::Uuid::parse_str(&self.receipt_id).is_err()
            || self.root_run_id != owner.root
            || self.control_id != owner.id
            || self.origin != origin
            || self.elapsed_ms != elapsed
            || self.hard_ms != hard
            || self.exhausted != (elapsed >= hard)
            || self.journaled_ms < 0
            || self.journaled_ms > hard
            || self.journaled_ms > elapsed
            || self.unsettled_ms != elapsed - self.journaled_ms
            || (elapsed <= hard && self.journaled_ms != elapsed)
            || b.consumed != self.journaled_ms
            || b.reserved != 0
            || b.indeterminate != 0
            || self.binding != proof::binding(db, &owner.root)?
            || self.sources != proof::sources(db, &owner.root)?
            || self.wall != proof::wall(db, &owner.root)?
        {
            return Err("budget_single_exit_receipt_conflict".into());
        }
        Ok(())
    }
}

pub(super) fn require_no_exit(db: &Connection, root: &str) -> Result<(), String> {
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_single_exit_receipts WHERE root_run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        Err("budget_single_continuation_requires_explicit_contract".into())
    } else {
        Ok(())
    }
}
