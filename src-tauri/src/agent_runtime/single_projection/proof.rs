//! Full physical history and exact event scope. No repair of historical facts.
use rusqlite::{params, types::Value, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
pub(super) fn hash(
    db: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
) -> Result<String, String> {
    let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
    let n = q.column_count();
    let rows = q
        .query_map(args, |r| {
            (0..n)
                .map(|i| r.get::<_, Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(crate::agent_runtime::store::stable_hash(&format!(
        "{rows:?}"
    )))
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Proof {
    pub version: u8,
    pub root: String,
    pub control: String,
    pub financial: String,
    pub cutoff: String,
    pub source: String,
    pub paused: bool,
    pub terminal: String,
    pub code: String,
    pub reason: String,
    pub row: String,
    pub snapshot: String,
    pub event_id: i64,
    pub event: String,
    pub collaboration_sequence: i64,
    pub collaboration: String,
}
impl Proof {
    pub(super) fn verify(
        &self,
        db: &Connection,
        root: &str,
        control: &str,
        financial: &str,
        cutoff: &str,
    ) -> Result<(), String> {
        let (status,terminal,code,reason,finished):(String,String,String,String,String)=db.query_row(
            "SELECT status,terminal_state,terminal_code,terminal_reason,finished_at FROM agent_runs WHERE id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|e.to_string())?;
        if self.version != 1
            || self.root != root
            || self.control != control
            || self.financial != financial
            || self.cutoff != cutoff
            || status != if self.paused { "paused" } else { "terminal" }
            || terminal != self.terminal
            || code != self.code
            || reason != self.reason
            || finished != if self.paused { "" } else { cutoff }
            || self.row != hash(db, "SELECT rowid,* FROM agent_runs WHERE id=?1", [root])?
            || self.snapshot
                != hash(
                    db,
                    "SELECT rowid,* FROM agent_snapshots WHERE run_id=?1",
                    [root],
                )?
            || self.event
                != hash(
                    db,
                    "SELECT rowid,* FROM agent_events WHERE id=?1",
                    [self.event_id],
                )?
            || self.collaboration
                != hash(
                    db,
                    "SELECT rowid,* FROM agent_collaboration_events WHERE sequence=?1",
                    [self.collaboration_sequence],
                )?
        {
            return Err("single_projection_saved_proof_conflict".into());
        }
        Ok(())
    }
}
pub(super) struct Before {
    native: i64,
    collaboration: i64,
    old_native: String,
    old_collaboration: String,
    other_roots: String,
    other_snapshots: String,
}
impl Before {
    pub(super) fn capture(db: &Connection, root: &str) -> Result<Self, String> {
        let native = db
            .query_row("SELECT COALESCE(max(id),0) FROM agent_events", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        let collaboration = db
            .query_row(
                "SELECT COALESCE(max(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            native,
            collaboration,
            old_native: hash(
                db,
                "SELECT rowid,* FROM agent_events WHERE id<=?1 ORDER BY rowid",
                [native],
            )?,
            old_collaboration: hash(
                db,
                "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",
                [collaboration],
            )?,
            other_roots: hash(
                db,
                "SELECT rowid,* FROM agent_runs WHERE id<>?1 ORDER BY rowid",
                [root],
            )?,
            other_snapshots: hash(
                db,
                "SELECT rowid,* FROM agent_snapshots WHERE run_id<>?1 ORDER BY rowid",
                [root],
            )?,
        })
    }
    pub(super) fn verify(
        &self,
        db: &Connection,
        root: &str,
        report: &super::BackendReport,
        event: (&str, &Json),
        paused: bool,
        terminal: &str,
    ) -> Result<(i64, i64), String> {
        if self.old_native!=hash(db,"SELECT rowid,* FROM agent_events WHERE id<=?1 ORDER BY rowid",[self.native])?
            || self.old_collaboration!=hash(db,"SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",[self.collaboration])?
            || self.other_roots!=hash(db,"SELECT rowid,* FROM agent_runs WHERE id<>?1 ORDER BY rowid",[root])?
            || self.other_snapshots!=hash(db,"SELECT rowid,* FROM agent_snapshots WHERE run_id<>?1 ORDER BY rowid",[root])?
        {return Err("single_projection_old_rows_conflict".into());}
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_events WHERE id>?1",
                [self.native],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let collab_count: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_collaboration_events WHERE sequence>?1",
                [self.collaboration],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if count != 1 || collab_count != 1 {
            return Err("single_projection_event_count_conflict".into());
        }
        let (kind, payload) = event;
        let event:i64=db.query_row("SELECT id FROM agent_events WHERE id>?1 AND run_id=?2 AND event_type=?3 AND payload_json=?4 AND artifact_refs_json='[]'",
            params![self.native,root,kind,payload.to_string()],|r|r.get(0)).map_err(|_|"single_projection_event_scope_conflict")?;
        let expected = json!({"role":"coordinator","status":if paused {"paused"} else {"terminal"},"terminalState":terminal});
        let collab:(i64,String)=db.query_row("SELECT sequence,payload_json FROM agent_collaboration_events WHERE sequence>?1 AND scan_id=?2 AND attempt_number=?3 AND entity_type='agent_run' AND entity_id=?4 AND event_type='agent_run'",
            params![self.collaboration,report.scan_id,report.attempt_number,root],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"single_projection_collaboration_scope_conflict")?;
        if serde_json::from_str::<Json>(&collab.1)
            .map_err(|_| "single_projection_collaboration_payload_conflict")?
            != expected
        {
            return Err("single_projection_collaboration_payload_conflict".into());
        }
        Ok((event, collab.0))
    }
}
