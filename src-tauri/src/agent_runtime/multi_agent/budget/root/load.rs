//! Load the original Single financial owner. This path performs no writes.
use super::RootOwner;
use rusqlite::Connection;
use serde_json::Value;

impl RootOwner {
    #[cfg(test)]
    pub(crate) fn initialize_control(
        tx: &rusqlite::Transaction<'_>,
        root: &str,
    ) -> Result<Self, String> {
        super::transaction::protect(tx, || Self::initialize(tx, root))
    }

    pub(crate) fn load_single(db: &Connection, root: &str) -> Result<Self, String> {
        let (id, text): (String, String) = db
            .query_row(
                "SELECT id,contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "budget_history_requires_reconciliation")?;
        let contract: Value =
            serde_json::from_str(&text).map_err(|_| "budget_root_original_owner_conflict")?;
        let canonical_text = contract.to_string();
        if contract["root"]["policy"] != "single"
            || !contract["coordinator"].is_null()
            || canonical_text != text
        {
            return Err("budget_root_original_owner_conflict".into());
        }
        let owner = Self {
            root: root.into(),
            id,
            contract,
            coordinator: None,
        };
        owner.verify(db)?;
        super::super::limits::verify_root_contract(db, root)?;
        owner.verify_http_history(db)?;
        let origin: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_budget_clock_origins o JOIN agent_runs r
             ON r.id=o.root_run_id WHERE r.id=?1 AND o.started_at=r.created_at)",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !origin {
            return Err("budget_clock_origin_conflict".into());
        }
        // No active-attempt, deadline, determinate-cost or current-C check here:
        // a captured original response may still record its original fees.
        Ok(owner)
    }

    fn verify_http_history(&self, db: &Connection) -> Result<(), String> {
        let bad: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_http_request_claims c WHERE c.run_id=?1
             AND (NOT EXISTS(SELECT 1 FROM agent_budget_entries e
               WHERE e.root_run_id=?1 AND e.assignment_id='' AND e.lease_attempt_id=?2
               AND e.dimension='target_requests' AND e.kind='reserve' AND e.amount=1
               AND e.source_id=printf('http:%s:%s:%d:%s',c.run_id,c.invocation_id,c.request_index,c.request_hash)
               AND e.idempotency_key=printf('root:%s:target:%s:reserve',?2,e.source_id))
             OR NOT EXISTS(SELECT 1 FROM agent_budget_entries e
               WHERE e.root_run_id=?1 AND e.assignment_id='' AND e.lease_attempt_id=?2
               AND e.dimension='target_requests' AND e.kind='forfeit' AND e.amount=1
               AND e.source_id=printf('http:%s:%s:%d:%s',c.run_id,c.invocation_id,c.request_index,c.request_hash)
               AND e.idempotency_key=printf('root:%s:target:%s:dispatch',?2,e.source_id))
             OR (c.response_status>0 AND NOT EXISTS(SELECT 1 FROM agent_budget_entries e
               WHERE e.root_run_id=?1 AND e.assignment_id='' AND e.lease_attempt_id=?2
               AND e.dimension='target_requests' AND e.kind='reconcile' AND e.amount=1
               AND e.source_id=printf('http:%s:%s:%d:%s',c.run_id,c.invocation_id,c.request_index,c.request_hash)
               AND e.idempotency_key=printf('root:%s:target:%s:receipt',?2,e.source_id)))))",
            rusqlite::params![self.root, self.id], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if bad {
            return Err("budget_history_requires_reconciliation".into());
        }
        // Each original target entry must also have its exact HTTP source.
        // An unreceived synthetic receipt must never make pending work live.
        let extra: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_budget_entries e
             WHERE e.root_run_id=?1 AND e.assignment_id='' AND e.dimension='target_requests'
             AND (e.lease_attempt_id<>?2 OR e.amount<>1 OR NOT EXISTS(
               SELECT 1 FROM agent_http_request_claims c WHERE c.run_id=?1
               AND e.source_id=printf('http:%s:%s:%d:%s',c.run_id,c.invocation_id,c.request_index,c.request_hash)
               AND ((e.kind='reserve' AND e.idempotency_key=printf('root:%s:target:%s:reserve',?2,e.source_id))
                 OR (e.kind='forfeit' AND e.idempotency_key=printf('root:%s:target:%s:dispatch',?2,e.source_id))
                 OR (e.kind='reconcile' AND c.response_status BETWEEN 100 AND 599 AND c.received_at<>''
                   AND e.idempotency_key=printf('root:%s:target:%s:receipt',?2,e.source_id))))))",
            rusqlite::params![self.root, self.id], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if extra {
            return Err("budget_history_requires_reconciliation".into());
        }
        Ok(())
    }
}
