//! Full old event rows and exact newly published closure scopes.
//! Reads only event history; no business table, identity or grant is written.
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::{params, types::Value, Connection};
use serde_json::{json, Value as Json};

pub(super) struct Events {
    native_floor: i64,
    collaboration_floor: i64,
    old_native: String,
    old_collaboration: String,
}

impl Events {
    pub(super) fn capture(db: &Connection) -> Result<Self, String> {
        let native_floor = floor(db, "agent_events", "id")?;
        let collaboration_floor = floor(db, "agent_collaboration_events", "sequence")?;
        Ok(Self {
            native_floor,
            collaboration_floor,
            old_native: history(db, "agent_events", "id", native_floor)?,
            old_collaboration: history(
                db,
                "agent_collaboration_events",
                "sequence",
                collaboration_floor,
            )?,
        })
    }

    pub(super) fn verify(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
        source: bool,
    ) -> Result<(), String> {
        if self.old_native != history(db, "agent_events", "id", self.native_floor)?
            || self.old_collaboration
                != history(
                    db,
                    "agent_collaboration_events",
                    "sequence",
                    self.collaboration_floor,
                )?
        {
            return Err("budget_closure_old_event_conflict".into());
        }
        let native: Vec<(String, String, String, String)> = {
            let mut q = db.prepare("SELECT run_id,event_type,payload_json,artifact_refs_json FROM agent_events WHERE id>?1 ORDER BY id").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([self.native_floor], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<rusqlite::Result<_>>()
                .map_err(|e| e.to_string())?
        };
        if native.len() != usize::from(source) {
            return Err("budget_closure_source_event_count_conflict".into());
        }
        for (run, kind, text, refs) in native {
            let payload: Json =
                serde_json::from_str(&text).map_err(|_| "budget_closure_source_event_conflict")?;
            if run != actor.root_run_id
                || kind != "terminal_reduced"
                || !payload.is_object()
                || refs != "[]"
            {
                return Err("budget_closure_source_event_conflict".into());
            }
            // Source production separately re-audits the complete exact frozen
            // payload, phase assignments, runtime, gate and actual sequence.
        }
        let collaboration: Vec<(String, i64, String, String, String, String)> = {
            let mut q = db.prepare("SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([self.collaboration_floor], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<rusqlite::Result<_>>()
                .map_err(|e| e.to_string())?
        };
        for (scan, attempt, entity, id, kind, text) in collaboration {
            if scan != actor.scan_id || attempt != actor.attempt_number || entity != kind {
                return Err("budget_closure_event_scope_conflict".into());
            }
            let payload: Json =
                serde_json::from_str(&text).map_err(|_| "budget_closure_event_payload_conflict")?;
            let expected = match entity.as_str() {
                "agent_run" => db.query_row("SELECT role,status,terminal_state FROM agent_runs
                    WHERE id=?1 AND root_run_id=?2 AND scan_id=?3 AND attempt_number=?4 AND target_url=?5",
                    params![id,actor.root_run_id,actor.scan_id,actor.attempt_number,actor.target_key],|r|Ok(json!({
                        "role":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"terminalState":r.get::<_,String>(2)?})))
                    .map_err(|_|"budget_closure_event_scope_conflict")?,
                "assignment" => db.query_row("SELECT role,state FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2
                    AND target_key=?3 AND lease_epoch=?4 AND fencing_token=?5",
                    params![id,actor.root_run_id,actor.target_key,actor.lease_epoch,actor.fencing_token],|r|Ok(json!({
                        "role":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?})))
                    .map_err(|_|"budget_closure_event_scope_conflict")?,
                "user_directive" => directive_payload(db, actor, &id, &payload)?,
                _ => return Err("budget_closure_event_scope_conflict".into()),
            };
            if payload != expected {
                return Err("budget_closure_event_payload_conflict".into());
            }
        }
        Ok(())
    }
}

fn directive_payload(
    db: &Connection,
    c: &CoordinatorLease,
    id: &str,
    payload: &Json,
) -> Result<Json, String> {
    let (status,draft,code,body):(String,String,String,String)=db.query_row("SELECT status,source_draft_id,rejection_code,payload_json
        FROM agent_user_directives WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 AND root_run_id=?4 AND target_key=?5",
        params![id,c.scan_id,c.attempt_number,c.root_run_id,c.target_key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
        .map_err(|_|"budget_closure_event_scope_conflict")?;
    if payload.get("proposalState").is_some() {
        let state:String=db.query_row("SELECT p.state FROM agent_directive_proposals p JOIN agent_assignments a ON a.id=p.assignment_id
            WHERE p.directive_id=?1 AND a.coordinator_run_id=?2 AND a.child_run_id=p.child_run_id
            AND a.target_key=?3 AND a.lease_epoch=?4 AND a.fencing_token=?5",
            params![id,c.root_run_id,c.target_key,c.lease_epoch,c.fencing_token],|r|r.get(0))
            .map_err(|_|"budget_closure_event_scope_conflict")?;
        Ok(json!({"proposalState":state}))
    } else if payload.get("deliveryState").is_some() {
        let body: Json =
            serde_json::from_str(&body).map_err(|_| "budget_closure_event_payload_conflict")?;
        Ok(json!({"status":status,"deliveryState":body["modelDelivery"]["state"]}))
    } else {
        Ok(json!({"status":status,"sourceDraftId":draft,"rejectionCode":code}))
    }
}

fn floor(db: &Connection, table: &str, column: &str) -> Result<i64, String> {
    db.query_row(
        &format!("SELECT COALESCE(MAX({column}),0) FROM {table}"),
        [],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

fn history(db: &Connection, table: &str, column: &str, floor: i64) -> Result<String, String> {
    let mut q = db
        .prepare(&format!(
            "SELECT rowid,* FROM {table} WHERE {column}<=?1 ORDER BY rowid"
        ))
        .map_err(|e| e.to_string())?;
    let count = q.column_count();
    let rows = q
        .query_map([floor], |r| {
            (0..count)
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
