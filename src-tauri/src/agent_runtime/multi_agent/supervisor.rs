//! Parent-owned expiry supervision, without model, target or budget authority.
use super::{
    attempts,
    lease::{self, CoordinatorLease},
    scheduler::ScheduledChild,
    supervision_ticket::{SupervisionState, SupervisionTicket, ThreadLife},
};
use crate::agent_runtime::contract::AgentRole;
use super::parent_invocation_owner::ParentInvocationOwner;
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use std::{
    path::Path,
    sync::{mpsc, Arc},
    thread::JoinHandle,
    time::Duration,
};

pub(crate) struct WorkerSupervisor {
    stop: mpsc::SyncSender<()>,
    state: Arc<SupervisionState>,
    worker: Option<JoinHandle<()>>,
    _owner: ParentInvocationOwner,
}

impl WorkerSupervisor {
    pub(crate) fn start(path: &Path, actor: &CoordinatorLease) -> Result<Self, String> {
        let owner=ParentInvocationOwner::claim(path,&actor.scan_id,actor.attempt_number,&actor.root_run_id)?;
        Self::start_owned(path,actor,owner)
    }

    pub(crate) fn start_owned(path:&Path,actor:&CoordinatorLease,owner:ParentInvocationOwner)->Result<Self,String> {
        owner.validate(path,actor)?;
        // Open only an existing database. A mistyped path must never create a
        // second business database or run schema initialization on a worker.
        let mut db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|e| format!("worker_supervisor_open:{e}"))?;
        db.busy_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        db.pragma_update(None, "foreign_keys", true)
            .map_err(|e| e.to_string())?;
        crate::collaboration_events::install_commit_notifications(&db);
        validate_parent(&db, actor)?;
        // Scope by immutable Root, not epoch/fence: a replacement Coordinator
        // must not overlap the original service that has not finished joining.
        // Claim before the first renewal/withdrawal; a duplicate has no cleanup.
        // Already held by this exact original scope, never claimed twice.
        let state = SupervisionState::new(path, actor)?;
        sweep(&mut db, actor, &state.ticket())?;
        let (stop, receiver) = mpsc::sync_channel(1);
        let thread_state = state.clone();
        let actor = actor.clone();
        let worker = std::thread::Builder::new()
            .name("native-worker-supervisor".into())
            .spawn(move || {
                let life = ThreadLife(thread_state);
                loop {
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    if let Err(error) = sweep(&mut db, &actor, &life.0.ticket()) {
                        life.0.fail(error);
                        break;
                    }
                }
            })
            .map_err(|e| format!("worker_supervisor_thread:{e}"))?;
        Ok(Self {
            stop,
            state,
            worker: Some(worker),
            _owner: owner,
        })
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        self.ticket().check()?;
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Err("worker_supervisor_stopped".into());
        }
        Ok(())
    }

    pub(crate) fn ticket(&self) -> SupervisionTicket {
        self.state.ticket()
    }
}

impl Drop for WorkerSupervisor {
    fn drop(&mut self) {
        self.state.stop();
        let _ = self.stop.try_send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn sweep(
    db: &mut Connection,
    actor: &CoordinatorLease,
    parent: &SupervisionTicket,
) -> Result<(), String> {
    validate_parent(db, actor)?;
    super::coordinator_heartbeat::renew_if_due(db, actor, parent)?;
    let missing: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a
        WHERE a.coordinator_run_id=?1 AND a.state IN ('leased','running','waiting_review','paused')
          AND a.budget_settled_at='' AND (SELECT count(*) FROM agent_assignment_attempts x
            WHERE x.assignment_id=a.id AND x.child_run_id=a.child_run_id AND x.root_run_id=?1)<>1)",
            [&actor.root_run_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("worker_supervisor_workers:{e}"))?;
    if missing {
        return Err("worker_supervisor_worker_missing_or_conflicting".into());
    }
    let due: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a
        JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id
        WHERE a.coordinator_run_id=?1 AND x.state IN ('leased','running','paused')
          AND COALESCE(datetime(x.expires_at)<=datetime('now','localtime'),1))",
            [&actor.root_run_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("worker_supervisor_deadlines:{e}"))?;
    if !due {
        return Ok(());
    }
    // A healthy foreground transaction may own the SQLite writer. Workers'
    // existing deadline gates already prohibit expired execution while the
    // supervisor waits to withdraw. Retry contention, never partial writes.
    let tx = match db.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            return Ok(())
        }
        Err(error) => return Err(format!("worker_supervisor_lock:{error}")),
    };
    validate_parent(&tx, actor)?;
    let children = {
        let mut stmt = tx.prepare("SELECT a.id,a.child_run_id,a.role FROM agent_assignments a
            JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id
            WHERE a.coordinator_run_id=?1 AND x.state IN ('leased','running','paused')
              AND COALESCE(datetime(x.expires_at)<=datetime('now','localtime'),1)
            ORDER BY a.id").map_err(|e| format!("worker_supervisor_read:{e}"))?;
        let rows = stmt
            .query_map([&actor.root_run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut children = Vec::new();
        for row in rows {
            let (assignment_id, run_id, role) = row.map_err(|e| e.to_string())?;
            let parsed = AgentRole::try_parse(&role)
                .filter(|parsed| parsed.as_str() == role && *parsed != AgentRole::Coordinator)
                .ok_or("worker_supervisor_role_invalid")?;
            children.push(ScheduledChild {
                assignment_id,
                run_id,
                role: parsed,
            });
        }
        children
    };
    for child in children {
        if !attempts::try_expire_original_in_transaction(&tx, actor, &child)? {
            return Err("worker_supervisor_expiry_unconfirmed".into());
        }
    }
    // Recheck after trigger-backed withdrawals, before committing any worker.
    validate_parent(&tx, actor)?;
    tx.commit()
        .map_err(|e| format!("worker_supervisor_commit:{e}"))
}

fn validate_parent(db: &Connection, actor: &CoordinatorLease) -> Result<(), String> {
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    let native: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs
        WHERE id=?1 AND root_run_id=id AND parent_run_id IS NULL AND assignment_id=''
          AND role='coordinator' AND backend='native' AND orchestration_policy='multi'
          AND scan_id=?2 AND attempt_number=?3 AND target_url=?4)",
            params![
                actor.root_run_id,
                actor.scan_id,
                actor.attempt_number,
                actor.target_key
            ],
            |row| row.get(0),
        )
        .map_err(|e| format!("worker_supervisor_parent:{e}"))?;
    if !native {
        return Err("worker_supervisor_parent_binding_conflict".into());
    }
    super::budget::clock::supervision_remaining(db, &actor.root_run_id)?;
    Ok(())
}
