//! Execution rights belong to one parent instance, never a replacement service.
use super::lease::{self, CoordinatorLease};
use rusqlite::{params, Connection};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
};

pub(super) struct SupervisionState {
    alive: AtomicBool,
    failure: Mutex<Option<String>>,
    database: PathBuf,
    actor: CoordinatorLease,
}

impl SupervisionState {
    pub(super) fn new(path: &Path, actor: &CoordinatorLease) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            alive: AtomicBool::new(true),
            failure: Mutex::new(None),
            database: path
                .canonicalize()
                .map_err(|e| format!("worker_supervision_path:{e}"))?,
            actor: actor.clone(),
        }))
    }

    pub(super) fn stop(&self) {
        self.alive.store(false, Ordering::Release);
    }

    pub(super) fn fail(&self, error: String) {
        if let Ok(mut failure) = self.failure.lock() {
            *failure = Some(error);
        }
        self.stop();
    }

    pub(super) fn ticket(self: &Arc<Self>) -> SupervisionTicket {
        SupervisionTicket {
            state: Arc::downgrade(self),
        }
    }

    fn check(&self) -> Result<(), String> {
        if let Some(error) = self
            .failure
            .lock()
            .map_err(|_| "worker_supervisor_state_poisoned")?
            .as_ref()
        {
            return Err(error.clone());
        }
        if !self.alive.load(Ordering::Acquire) {
            return Err("worker_supervisor_stopped".into());
        }
        Ok(())
    }
}

// Also revoke on thread panic. A cloned ticket never keeps its parent alive.
pub(super) struct ThreadLife(pub(super) Arc<SupervisionState>);
impl Drop for ThreadLife {
    fn drop(&mut self) {
        self.0.stop();
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SupervisionTicket {
    state: Weak<SupervisionState>,
}

impl SupervisionTicket {
    fn state(&self, path: &Path) -> Result<Arc<SupervisionState>, String> {
        let state = self.state.upgrade().ok_or("worker_supervisor_stopped")?;
        state.check()?;
        if path
            .canonicalize()
            .map_err(|_| "worker_supervision_path_unavailable")?
            != state.database
        {
            return Err("worker_supervision_database_conflict".into());
        }
        Ok(state)
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        self.state
            .upgrade()
            .ok_or("worker_supervisor_stopped")?
            .check()
    }

    pub(crate) fn check_actor(&self, path: &Path, actor: &CoordinatorLease) -> Result<(), String> {
        let state = self.state(path)?;
        let original = &state.actor;
        if original.scan_id != actor.scan_id
            || original.attempt_number != actor.attempt_number
            || original.target_key != actor.target_key
            || original.root_run_id != actor.root_run_id
            || original.lease_epoch != actor.lease_epoch
            || original.fencing_token != actor.fencing_token
        {
            return Err("worker_supervision_actor_conflict".into());
        }
        state.check()
    }

    pub(crate) fn original_actor_for_run(
        &self,
        db: &Connection,
        path: &Path,
        scan: &str,
        attempt: i64,
        target: &str,
        run: &str,
    ) -> Result<CoordinatorLease, String> {
        self.check_run(db, path, scan, attempt, target, run)?;
        let state = self.state(path)?;
        state.check()?;
        Ok(state.actor.clone())
    }

    pub(crate) fn check_run(
        &self,
        db: &Connection,
        path: &Path,
        scan: &str,
        attempt: i64,
        target: &str,
        run: &str,
    ) -> Result<(), String> {
        let state = self.state(path)?;
        let actor = &state.actor;
        if actor.scan_id != scan || actor.attempt_number != attempt || actor.target_key != target {
            return Err("worker_supervision_run_conflict".into());
        }
        let bound: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1
            AND (id=?2 OR root_run_id=?2) AND scan_id=?3 AND attempt_number=?4 AND target_url=?5)",
                params![run, actor.root_run_id, scan, attempt, target],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !bound {
            return Err("worker_supervision_run_conflict".into());
        }
        lease::validate_coordinator_lease(db, actor)?;
        lease::require_executable_coordinator(db, actor)?;
        state.check()
    }
}
