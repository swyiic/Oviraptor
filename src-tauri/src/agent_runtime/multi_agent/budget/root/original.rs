//! Original Root financial owner; this read never issues a grant or adopts C.
use super::RootOwner;
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::Connection;
use serde_json::Value;

impl RootOwner {
    pub(crate) fn load_original(db: &Connection, root: &str) -> Result<Self, String> {
        let (id, text): (String, String) = db
            .query_row(
                "SELECT id,contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
                [root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "budget_history_requires_reconciliation")?;
        let value: Value =
            serde_json::from_str(&text).map_err(|_| "budget_root_original_owner_conflict")?;
        // Compare the canonical serialized bytes, including JSON whitespace.
        // Value equality alone would accept a changed original control text.
        let canonical =
            serde_json::to_string(&value).map_err(|_| "budget_root_original_owner_conflict")?;
        if canonical != text || value.as_object().is_none_or(|v| v.len() != 2) {
            return Err("budget_root_original_owner_conflict".into());
        }
        if value["root"]["policy"] == "single" {
            return Self::load_single(db, root);
        }
        let c = &value["coordinator"];
        let scope = &value["root"];
        if scope["policy"] != "multi"
            || c.as_object().is_none_or(|v| v.len() != 5)
            || c["scan"] != scope["scan"]
            || c["attempt"] != scope["attempt"]
            || c["target"] != scope["target"]
        {
            return Err("budget_root_original_owner_conflict".into());
        }
        let actor = CoordinatorLease {
            scan_id: c["scan"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or("budget_root_original_owner_conflict")?
                .into(),
            attempt_number: c["attempt"]
                .as_i64()
                .filter(|v| *v > 0)
                .ok_or("budget_root_original_owner_conflict")?,
            target_key: c["target"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or("budget_root_original_owner_conflict")?
                .into(),
            root_run_id: root.into(),
            lease_epoch: c["epoch"]
                .as_i64()
                .filter(|v| *v > 0)
                .ok_or("budget_root_original_owner_conflict")?,
            fencing_token: c["fence"]
                .as_str()
                .filter(|v| uuid::Uuid::parse_str(v).is_ok())
                .ok_or("budget_root_original_owner_conflict")?
                .into(),
            // The frozen financial control never contained a mutable deadline.
            // Any caller requesting publication still validates this exact C.
            lease_expires_at: String::new(),
        };
        let owner = Self {
            root: root.into(),
            id,
            contract: value,
            coordinator: Some(actor),
        };
        owner.verify(db)?;
        super::super::limits::verify_root_contract(db, root)?;
        let origin: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_budget_clock_origins o
            JOIN agent_runs r ON r.id=o.root_run_id WHERE r.id=?1 AND o.started_at=r.created_at)",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !origin {
            return Err("budget_clock_origin_conflict".into());
        }
        Ok(owner)
    }

    pub(crate) fn require_original_coordinator(
        &self,
        db: &Connection,
        lease: &CoordinatorLease,
    ) -> Result<(), String> {
        self.verify(db)?;
        let c = self
            .coordinator
            .as_ref()
            .ok_or("budget_root_original_owner_conflict")?;
        if self.root != lease.root_run_id
            || c.scan_id != lease.scan_id
            || c.attempt_number != lease.attempt_number
            || c.target_key != lease.target_key
            || c.lease_epoch != lease.lease_epoch
            || c.fencing_token != lease.fencing_token
        {
            return Err("budget_root_original_owner_conflict".into());
        }
        // This proves financial identity only, never live/renewable authority.
        Ok(())
    }
}
