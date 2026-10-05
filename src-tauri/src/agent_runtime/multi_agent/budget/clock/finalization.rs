//! Frozen terminal cutoff and financial-only elapsed transitions.
//! Admission uses the existing live clock; this module creates no identity,
//! limits, origin, grant, lease, assignment, slot or execution capability.
use super::super::{append_inner, balance, limits, root::RootOwner, Authority, Kind};
use super::final_receipt;
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::{params, Connection, Transaction};
mod exit_receipt;

#[derive(Debug)]
pub(crate) struct FinalClock {
    lease: CoordinatorLease,
    cutoff: String,
    was_terminal: bool,
    binding: String,
    sources: String,
    wall_receipt: Option<String>,
    receipt: Option<String>,
    event_floor: i64,
    exceptional: Option<super::elapsed_fact::ElapsedFact>,
}

impl FinalClock {
    pub(crate) fn capture(db: &Connection, lease: &CoordinatorLease) -> Result<Self, String> {
        Self::capture_observed(db, lease, None)
    }

    pub(super) fn capture_observed(
        db: &Connection,
        lease: &CoordinatorLease,
        observed_cutoff: Option<&str>,
    ) -> Result<Self, String> {
        let (status,finished):(String,String)=db.query_row(
            "SELECT status,finished_at FROM agent_runs WHERE id=?1 AND root_run_id=id
              AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND backend='native'
              AND role='coordinator' AND orchestration_policy='multi' AND assignment_id='' AND parent_run_id IS NULL",
            params![lease.root_run_id,lease.scan_id,lease.attempt_number,lease.target_key],
            |r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"budget_root_binding_conflict")?;
        if !matches!(
            status.as_str(),
            "prepared" | "running" | "paused" | "terminal"
        ) {
            return Err("budget_root_binding_conflict".into());
        }
        let was_terminal = status == "terminal";
        let exceptional = super::elapsed_fact::verify_existing(db, lease)?;
        let cutoff = if was_terminal {
            if finished.trim().is_empty() {
                return Err("budget_history_requires_reconciliation".into());
            }
            finished
        } else {
            if !finished.is_empty() {
                return Err("budget_root_binding_conflict".into());
            }
            if let Some(saved) = &exceptional {
                saved.cutoff.clone()
            } else if let Some(cutoff) = observed_cutoff {
                cutoff.to_string()
            } else {
                db.query_row(
                    "SELECT strftime('%Y-%m-%d %H:%M:%f','now','localtime')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?
            }
        };
        Ok(Self {
            lease: lease.clone(),
            cutoff,
            was_terminal,
            binding: binding(db, &lease.root_run_id)?,
            sources: final_receipt::sources(db, &lease.root_run_id)?,
            exceptional,
            wall_receipt: None,
            receipt: None,
            event_floor: db
                .query_row(
                    "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?,
        })
    }

    pub(crate) fn was_terminal(&self) -> bool {
        self.was_terminal
    }

    pub(crate) fn cutoff(&self) -> &str {
        &self.cutoff
    }

    pub(crate) fn sample(&mut self, tx: &Transaction<'_>) -> Result<(), String> {
        self.sample_inner(tx)?;
        self.wall_receipt = Some(final_receipt::wall(tx, &self.lease.root_run_id)?);
        Ok(())
    }

    pub(crate) fn seal(
        &mut self,
        db: &Connection,
        state: &str,
        code: &str,
        reason: &str,
    ) -> Result<(), String> {
        if self.wall_receipt.as_ref() != Some(&final_receipt::wall(db, &self.lease.root_run_id)?)
            || self.sources != final_receipt::sources(db, &self.lease.root_run_id)?
        {
            return Err("budget_clock_final_persistence_conflict".into());
        }
        // Directive cancellation may legitimately release its own unused
        // allocations. Pin all physical costs only after that closure, while
        // the original wall sample, control/limits/origin/C remain unchanged.
        self.receipt = Some(final_receipt::all_costs(db, &self.lease.root_run_id)?);
        self.verify_closed(db, state, code, reason)?;
        exit_receipt::persist_or_verify(db, self, state, code, reason)
    }

    fn sample_inner(&self, tx: &Transaction<'_>) -> Result<(), String> {
        // Original overrun/expired financial facts stop all fresh work. This
        // private terminal consumer verifies them without clamping or changing
        // the original wall ledger, hard contract, owner or clock.
        if self.verify_exceptional(tx)? {
            return Ok(());
        }
        let configured: i64 = tx
            .query_row(
                "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
                [&self.lease.root_run_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if configured == 0 {
            let history: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=?1)
                OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1)
                OR EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)",
                    [&self.lease.root_run_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            return if history {
                Err("budget_history_requires_reconciliation".into())
            } else {
                Ok(())
            };
        }
        if configured != 10 {
            return Err("budget_dimension_contract_incomplete".into());
        }
        let (origin,current,elapsed,not_future):(String,String,Option<i64>,bool)=tx.query_row(
            "SELECT o.started_at,r.created_at,CAST((julianday(?2)-julianday(o.started_at))*86400000 AS INTEGER),
                COALESCE(julianday(?2)<=julianday('now','localtime'),0)
             FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id WHERE r.id=?1",
            params![self.lease.root_run_id,self.cutoff],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"budget_clock_origin_missing")?;
        if origin != current {
            return Err("budget_clock_origin_conflict".into());
        }
        let elapsed = elapsed
            .filter(|v| *v >= 0 && not_future)
            .ok_or("budget_clock_origin_invalid")?;
        let previous = balance(tx, &self.lease.root_run_id, None, "wall_time_ms")?;
        if previous.reserved != 0 || previous.indeterminate != 0 {
            return Err("budget_clock_unsettled_sample".into());
        }
        let delta = elapsed
            .checked_sub(previous.consumed)
            .filter(|v| *v >= 0)
            .ok_or("budget_clock_moved_backwards")?;
        let limit:i64=tx.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='wall_time_ms'",[&self.lease.root_run_id],|r|r.get(0)).map_err(|_|"budget_clock_limit_missing")?;
        // Without a saved original exceptional fact, this ordinary ledger
        // path cannot close an overrun. Never clamp elapsed or increase hard.
        if elapsed >= limit {
            return Err("budget_wall_time_exhausted".into());
        }
        if self.was_terminal && delta != 0 {
            return Err("budget_history_requires_reconciliation".into());
        }
        limits::verify_root_contract(tx, &self.lease.root_run_id)?;
        let root_owner: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)",
                [&self.lease.root_run_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if root_owner {
            let owner = RootOwner::load_original(tx, &self.lease.root_run_id)?;
            owner.require_original_coordinator(tx, &self.lease)?;
            if delta > 0 {
                let source = format!(
                    "root-clock:{elapsed}:{}",
                    crate::agent_runtime::store::stable_hash(&origin)
                );
                owner.append(
                    tx,
                    "wall_time_ms",
                    Kind::Reserve,
                    delta,
                    &format!("{source}:reserve"),
                    &source,
                )?;
                owner.append(
                    tx,
                    "wall_time_ms",
                    Kind::Consume,
                    delta,
                    &format!("{source}:consume"),
                    &source,
                )?;
            }
            owner.require_original_coordinator(tx, &self.lease)?;
        } else {
            // Preserve the existing Native worker owner and key namespace.
            let assignment: String = tx
                .query_row(
                    "SELECT id FROM agent_assignments WHERE coordinator_run_id=?1
                AND lease_epoch=?2 AND fencing_token=?3 ORDER BY rowid DESC LIMIT 1",
                    params![
                        self.lease.root_run_id,
                        self.lease.lease_epoch,
                        self.lease.fencing_token
                    ],
                    |r| r.get(0),
                )
                .map_err(|_| "budget_history_requires_reconciliation")?;
            crate::agent_runtime::multi_agent::attempts::current(tx, &self.lease, &assignment)?;
            if delta > 0 {
                let source = format!(
                    "clock:{}:{elapsed}",
                    crate::agent_runtime::store::stable_hash(&origin)
                );
                for kind in [Kind::Reserve, Kind::Consume] {
                    append_inner(
                        tx,
                        &self.lease,
                        &assignment,
                        "wall_time_ms",
                        kind,
                        delta,
                        &format!("{source}:{}", kind.as_str()),
                        &source,
                        Authority::ClockSample,
                    )?;
                }
            }
        }
        let actual = balance(tx, &self.lease.root_run_id, None, "wall_time_ms")?;
        if actual.consumed != elapsed || actual.reserved != 0 || actual.indeterminate != 0 {
            return Err("budget_clock_final_persistence_conflict".into());
        }
        let unchanged:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_budget_clock_origins o ON o.root_run_id=r.id
            WHERE r.id=?1 AND r.created_at=?2 AND o.started_at=?2 AND (?3=0 OR (r.status='terminal' AND r.finished_at=?4)))",
            params![self.lease.root_run_id,origin,self.was_terminal,self.cutoff],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !unchanged {
            return Err("budget_clock_origin_conflict".into());
        }
        limits::verify_root_contract(tx, &self.lease.root_run_id)
    }

    fn verify_exceptional(&self, db: &Connection) -> Result<bool, String> {
        let Some(saved) = &self.exceptional else {
            // A normal sample cannot silently adopt a newly introduced fact.
            if super::elapsed_fact::read(db, &self.lease.root_run_id)?.is_some() {
                return Err("budget_elapsed_fact_original_conflict".into());
            }
            return Ok(false);
        };
        if super::elapsed_fact::verify_existing(db, &self.lease)?.as_ref() != Some(saved)
            || saved.cutoff != self.cutoff
        {
            return Err("budget_elapsed_fact_original_conflict".into());
        }
        Ok(true)
    }

    // Financial observation may precede a later recovery. It never marks a
    // Root terminal, releases reservations, ACKs messages or requires live C.
    pub(super) fn verify_observation(&self, db: &Connection) -> Result<(), String> {
        if self.wall_receipt.as_ref() != Some(&final_receipt::wall(db, &self.lease.root_run_id)?)
            || self.sources != final_receipt::sources(db, &self.lease.root_run_id)?
            || self.binding != binding(db, &self.lease.root_run_id)?
        {
            return Err("budget_clock_final_persistence_conflict".into());
        }
        self.verify_exceptional(db)?;
        limits::verify_root_contract(db, &self.lease.root_run_id)
    }

    /// Original financial observation only; no live C or capability escapes.
    pub(crate) fn verify_original_exit(db: &Connection, root: &str) -> Result<(), String> {
        exit_receipt::verify_original(db, root)
    }

    pub(crate) fn verify_closed(
        &self,
        db: &Connection,
        state: &str,
        code: &str,
        reason: &str,
    ) -> Result<(), String> {
        self.verify_financial_closed(db, state, code, reason)?;
        crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(db, &self.lease)
    }

    fn verify_financial_closed(
        &self,
        db: &Connection,
        state: &str,
        code: &str,
        reason: &str,
    ) -> Result<(), String> {
        let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND root_run_id=id
            AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND backend='native' AND role='coordinator'
            AND orchestration_policy='multi' AND assignment_id='' AND parent_run_id IS NULL
            AND status='terminal' AND finished_at=?5 AND terminal_state=?6 AND terminal_code=?7 AND terminal_reason=?8)",params![self.lease.root_run_id,self.lease.scan_id,
                self.lease.attempt_number,self.lease.target_key,self.cutoff,state,code,reason],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !exact {
            return Err("budget_clock_final_persistence_conflict".into());
        }
        let event_count:i64=db.query_row("SELECT count(*) FROM agent_collaboration_events
            WHERE entity_type='agent_run' AND entity_id=?1 AND event_type='agent_run' AND sequence>?2
            AND scan_id=?3 AND attempt_number=?4 AND json_extract(payload_json,'$.role')='coordinator'
            AND json_extract(payload_json,'$.status')='terminal' AND json_extract(payload_json,'$.terminalState')=?5
            AND (SELECT count(*) FROM json_each(payload_json))=3",
            params![self.lease.root_run_id,self.event_floor,self.lease.scan_id,self.lease.attempt_number,state],|r|r.get(0)).map_err(|e|e.to_string())?;
        if event_count != i64::from(!self.was_terminal) {
            return Err("budget_closure_terminal_event_conflict".into());
        }
        if self.was_terminal {
            let original_event:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_collaboration_events
                WHERE entity_type='agent_run' AND entity_id=?1 AND event_type='agent_run' AND sequence<=?2
                AND scan_id=?3 AND attempt_number=?4 AND json_extract(payload_json,'$.role')='coordinator'
                AND json_extract(payload_json,'$.status')='terminal' AND json_extract(payload_json,'$.terminalState')=?5
                AND (SELECT count(*) FROM json_each(payload_json))=3)",
                params![self.lease.root_run_id,self.event_floor,self.lease.scan_id,self.lease.attempt_number,state],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !original_event {
                return Err("budget_closure_terminal_event_conflict".into());
            }
        }
        if self.receipt.as_ref() != Some(&final_receipt::all_costs(db, &self.lease.root_run_id)?)
            || self.wall_receipt.as_ref()
                != Some(&final_receipt::wall(db, &self.lease.root_run_id)?)
            || self.sources != final_receipt::sources(db, &self.lease.root_run_id)?
        {
            return Err("budget_clock_final_persistence_conflict".into());
        }
        if binding(db, &self.lease.root_run_id)? != self.binding {
            return Err("budget_root_binding_conflict".into());
        }
        let configured: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
                [&self.lease.root_run_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if configured != 0 {
            limits::verify_root_contract(db, &self.lease.root_run_id)?;
            let elapsed: i64 = db
                .query_row(
                    "SELECT CAST((julianday(?2)-julianday(o.started_at))*86400000 AS INTEGER)
                FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id
                WHERE r.id=?1 AND o.started_at=r.created_at",
                    params![self.lease.root_run_id, self.cutoff],
                    |r| r.get(0),
                )
                .map_err(|_| "budget_clock_origin_conflict")?;
            let actual = balance(db, &self.lease.root_run_id, None, "wall_time_ms")?;
            let expected = if let Some(fact) = &self.exceptional {
                self.verify_exceptional(db)?;
                if elapsed != fact.elapsed {
                    return Err("budget_elapsed_fact_cutoff_conflict".into());
                }
                fact.journaled
            } else {
                elapsed
            };
            if actual.consumed != expected || actual.reserved != 0 || actual.indeterminate != 0 {
                return Err("budget_clock_final_persistence_conflict".into());
            }
        }
        // Pure original financial proof. Live publication checks C separately.
        Ok(())
    }
}

fn binding(db: &Connection, root: &str) -> Result<String, String> {
    db.query_row("SELECT scan_id,attempt_number,target_url,plan_hash,plan_json,created_at,
        hard_token_budget,hard_request_budget,orchestration_policy,root_run_id,backend,role,assignment_id,parent_run_id
        FROM agent_runs WHERE id=?1",[root],|r|Ok(serde_json::json!({
            "scan":r.get::<_,String>(0)?,"attempt":r.get::<_,i64>(1)?,"target":r.get::<_,String>(2)?,
            "planHash":r.get::<_,String>(3)?,"plan":r.get::<_,String>(4)?,"origin":r.get::<_,String>(5)?,
            "tokens":r.get::<_,i64>(6)?,"requests":r.get::<_,i64>(7)?,"policy":r.get::<_,String>(8)?,
            "declaredRoot":r.get::<_,String>(9)?,"backend":r.get::<_,String>(10)?,"role":r.get::<_,String>(11)?,
            "assignment":r.get::<_,String>(12)?,"parent":r.get::<_,Option<String>>(13)?,
        }).to_string())).map_err(|_|"budget_root_binding_conflict".into())
}
