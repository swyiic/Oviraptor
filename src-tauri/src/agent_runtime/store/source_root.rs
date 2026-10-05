//! Source's absence-before-INSERT token; no historical row can construct it.
use crate::agent_runtime::multi_agent::{attempts::audit_rows::Rows, source::SourcePhaseContract};
use rusqlite::{params, Transaction};

pub(crate) struct SourceRootInsertion<'tx, 'db> {
    tx: &'tx Transaction<'db>,
    root: String,
    scan: String,
    attempt: i64,
    target: String,
}
impl<'tx, 'db> SourceRootInsertion<'tx, 'db> {
    pub(crate) fn capture_absent(
        tx: &'tx Transaction<'db>,
        root: &str,
        scan: &str,
        attempt: i64,
        target: &str,
    ) -> Result<Self, String> {
        if root.is_empty() || scan.is_empty() || attempt <= 0 || !target.starts_with("source:") {
            return Err("source_finance_birth_scope_invalid".into());
        }
        let occupied:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 OR
            (scan_id=?2 AND attempt_number=?3 AND role='coordinator' AND target_url LIKE 'source:%'))
            OR EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE root_run_id=?1 OR
            (scan_id=?2 AND attempt_number=?3 AND target_key LIKE 'source:%'))",
            params![root,scan,attempt],|r|r.get(0)).map_err(|e|e.to_string())?;
        if occupied {
            return Err("source_finance_birth_requires_absent_root_and_scope".into());
        }
        Ok(Self {
            tx,
            root: root.into(),
            scan: scan.into(),
            attempt,
            target: target.into(),
        })
    }
    pub(crate) fn inserted(&self) -> Result<NewlyInsertedSourceRoot<'_, 'db>, String> {
        let root_row = Rows::read(
            self.tx,
            "SELECT rowid,* FROM agent_runs WHERE id=?1",
            [&self.root],
        )?;
        if root_row.values.len() != 1 {
            return Err("source_finance_birth_root_missing".into());
        }
        let pristine:bool=self.tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND scan_id=?2
            AND attempt_number=?3 AND target_url=?4 AND backend='native' AND role='coordinator' AND status='prepared'
            AND root_run_id=id AND parent_run_id IS NULL AND assignment_id='' AND orchestration_policy='multi'
            AND started_at='' AND finished_at='' AND used_tokens=0 AND used_cached_tokens=0 AND used_requests=0
            AND reserved_tokens=0 AND reserved_requests=0 AND heartbeat_at='' AND lease_expires_at='' AND cancel_requested_at=''
            AND lane='' AND capability_lease_json='[]' AND terminal_state='' AND terminal_code='' AND terminal_reason='')",
            params![self.root,self.scan,self.attempt,self.target],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !pristine {
            return Err("source_finance_birth_root_changed".into());
        }
        let plan: String = self
            .tx
            .query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [&self.root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value =
            serde_json::from_str(&plan).map_err(|_| "source_finance_birth_plan_invalid")?;
        SourcePhaseContract::from_plan(&value)?;
        let canonical = value.to_string();
        if value["surface"] != "source" || canonical != plan {
            return Err("source_finance_birth_plan_invalid".into());
        }
        Ok(NewlyInsertedSourceRoot {
            tx: self.tx,
            root: &self.root,
            root_row,
        })
    }
}

// Fields are private. Neither Copy nor Clone: the captured Root stays identical
// before and after control publication in the caller's original transaction.
pub(crate) struct NewlyInsertedSourceRoot<'tx, 'db> {
    tx: &'tx Transaction<'db>,
    root: &'tx str,
    root_row: Rows,
}
impl NewlyInsertedSourceRoot<'_, '_> {
    pub(crate) fn transaction(&self) -> &Transaction<'_> {
        self.tx
    }
    pub(crate) fn id(&self) -> &str {
        self.root
    }
    pub(crate) fn verify(&self) -> Result<(), String> {
        if Rows::read(
            self.tx,
            "SELECT rowid,* FROM agent_runs WHERE id=?1",
            [self.root],
        )? != self.root_row
        {
            return Err("source_finance_birth_root_changed".into());
        }
        Ok(())
    }
}
