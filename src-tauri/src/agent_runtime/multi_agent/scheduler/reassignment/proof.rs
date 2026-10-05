//! Root-scoped before/after proof. Replacement cannot mutate old audit rows,
//! shared authority, siblings, business effects, or the clock origin.
use super::*;
use crate::agent_runtime::multi_agent::attempts::{audit_rows::Rows, expiry_proof::FROZEN};
use rusqlite::types::Value;

pub(super) struct ReplacementProof {
    frozen: Vec<(String, Rows)>,
    assignments: Rows,
    entry_floor: i64,
    event_floor: i64,
    new_run: String,
    new_id: String,
    issued_at: String,
}

impl ReplacementProof {
    pub(super) fn capture(
        db: &Connection,
        lease: &CoordinatorLease,
        old: &ScheduledChild,
        new_run: &str,
        new_id: &str,
        now: &str,
    ) -> Result<Self, String> {
        let mut sql = FROZEN
            .iter()
            .filter(|s| !s.contains("FROM agent_budget_entries"))
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        // UUIDs generated inside the transaction never come from SQL input.
        sql.extend([
            format!("SELECT * FROM agent_runs WHERE (id=?1 OR root_run_id=?1) AND id<>'{new_run}' ORDER BY rowid"),
            format!("SELECT * FROM agent_assignment_attempts WHERE root_run_id=?1 AND id<>'{new_id}' ORDER BY rowid"),
            format!("SELECT * FROM agent_capability_leases WHERE root_run_id=?1 AND child_run_id<>'{new_run}' ORDER BY rowid"),
            format!("SELECT * FROM agent_assignment_replacements WHERE root_run_id=?1 AND replacement_attempt_id<>'{new_id}' ORDER BY rowid"),
        ]);
        let entry_floor: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(rowid),0) FROM agent_budget_entries",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let event_floor: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        sql.push(format!("SELECT * FROM agent_budget_entries WHERE root_run_id=?1 AND rowid<={entry_floor} ORDER BY rowid"));
        sql.push(format!("SELECT * FROM agent_collaboration_events WHERE scan_id=(SELECT scan_id FROM agent_runs WHERE id=?1) AND attempt_number=(SELECT attempt_number FROM agent_runs WHERE id=?1) AND sequence<={event_floor} ORDER BY sequence"));
        let frozen = sql
            .into_iter()
            .map(|s| Ok((s.clone(), Rows::read(db, &s, [&lease.root_run_id])?)))
            .collect::<Result<_, String>>()?;
        let mut assignments = Rows::read(
            db,
            "SELECT * FROM agent_assignments WHERE coordinator_run_id=?1 ORDER BY rowid",
            [&lease.root_run_id],
        )?;
        for (column, value) in [
            ("child_run_id", new_run),
            ("state", "leased"),
            ("failure_class", ""),
            ("lease_expires_at", lease.lease_expires_at.as_str()),
            ("updated_at", now),
        ] {
            assignments.set(&old.assignment_id, column, value)?;
        }
        Ok(Self {
            frozen,
            assignments,
            entry_floor,
            event_floor,
            new_run: new_run.into(),
            new_id: new_id.into(),
            issued_at: now.into(),
        })
    }

    pub(super) fn verify(
        &self,
        db: &Connection,
        lease: &CoordinatorLease,
        old: &ScheduledChild,
        new: &ScheduledChild,
        old_id: &str,
        new_id: &str,
    ) -> Result<(), String> {
        for (sql, expected) in &self.frozen {
            if Rows::read(db, sql, [&lease.root_run_id])? != *expected {
                return Err("assignment_replacement_collateral_write".into());
            }
        }
        if new.run_id != self.new_run
            || new_id != self.new_id
            || Rows::read(
                db,
                "SELECT * FROM agent_assignments WHERE coordinator_run_id=?1 ORDER BY rowid",
                [&lease.root_run_id],
            )? != self.assignments
        {
            return Err("assignment_replacement_assignment_changed".into());
        }
        let assignment = super::super::assignment::load_assignment(db, &new.assignment_id)?
            .ok_or("assignment_replacement_missing")?;
        if store::load_run(db, &new.run_id)? != Some(child_grant_row(lease, &assignment)) {
            return Err("assignment_replacement_new_run_changed".into());
        }
        let pristine:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND plan_json='{}'
            AND started_at='' AND finished_at='' AND datetime(created_at)<=datetime('now','localtime') AND updated_at=created_at)",
            [&new.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !pristine {
            return Err("assignment_replacement_new_run_changed".into());
        }
        let heartbeat: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts WHERE id=?1
            AND leased_at=?2 AND heartbeat_at=?2)",
                params![new_id, self.issued_at],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !heartbeat {
            return Err("assignment_replacement_new_worker_changed".into());
        }
        let entries=Rows::read(db,"SELECT assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id FROM agent_budget_entries WHERE root_run_id=?1 AND rowid>?2 ORDER BY rowid",params![lease.root_run_id,self.entry_floor])?;
        let mut expected = Vec::new();
        let legacy_slot =
            super::super::budget::root_definition::read(db, &lease.root_run_id)?.is_none();
        for (worker, kind, source) in [(old_id, "release", false), (new_id, "reserve", true)] {
            let ordinal: i64 = db
                .query_row(
                    "SELECT lease_epoch FROM agent_assignment_attempts WHERE id=?1",
                    [worker],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            for (dimension, amount) in super::super::budget::DIMENSIONS[..4]
                .iter()
                .zip([
                    assignment.reserved_tokens,
                    assignment.reserved_tokens,
                    assignment.reserved_tokens,
                    assignment.reserved_requests,
                ])
                .chain(std::iter::once((&"concurrency_batches", 1)))
            {
                if (*dimension == "concurrency_batches" && !legacy_slot) || amount <= 0 {
                    continue;
                }
                let slot = *dimension == "concurrency_batches";
                let key = if source {
                    if slot {
                        format!("slot:{}", assignment.id)
                    } else {
                        format!("reserve:{}:{dimension}", assignment.id)
                    }
                } else if slot {
                    format!("slot-finish:{}", assignment.id)
                } else {
                    format!("finish:{}:{dimension}", assignment.id)
                };
                let key = if ordinal == 1 {
                    key
                } else {
                    format!("worker:{worker}:{key}")
                };
                let source = if source {
                    format!("assignment:{}", assignment.id)
                } else if slot {
                    format!("terminal:{}", assignment.id)
                } else {
                    format!("unsent:{}", assignment.id)
                };
                expected.push(vec![
                    Value::Text(assignment.id.clone()),
                    Value::Text(worker.into()),
                    Value::Text((*dimension).into()),
                    Value::Text(kind.into()),
                    Value::Integer(amount),
                    Value::Text(key),
                    Value::Text(source),
                ]);
            }
        }
        if entries.values != expected {
            return Err("assignment_replacement_budget_changed".into());
        }
        let events=Rows::read(db,"SELECT entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2 AND sequence>?3 ORDER BY sequence",params![lease.scan_id,lease.attempt_number,self.event_floor])?;
        let expected_events=vec![
            vec![Value::Text("assignment".into()),Value::Text(old.assignment_id.clone()),Value::Text("assignment".into()),Value::Text(serde_json::json!({"role":old.role.as_str(),"state":"leased"}).to_string())],
            vec![Value::Text("agent_run".into()),Value::Text(new.run_id.clone()),Value::Text("agent_run".into()),Value::Text(serde_json::json!({"role":new.role.as_str(),"status":"prepared","terminalState":""}).to_string())],
        ];
        if events.values.len() != expected_events.len()
            || events
                .values
                .iter()
                .zip(expected_events)
                .any(|(actual, expected)| {
                    actual[..3] != expected[..3]
                        || match (&actual[3], &expected[3]) {
                            (Value::Text(a), Value::Text(b)) => {
                                serde_json::from_str::<JsonValue>(a).ok()
                                    != serde_json::from_str::<JsonValue>(b).ok()
                            }
                            _ => true,
                        }
                })
        {
            return Err("assignment_replacement_event_missing_or_changed".into());
        }
        validate_coordinator_lease(db, lease)?;
        require_executable_coordinator(db, lease)
    }
}
