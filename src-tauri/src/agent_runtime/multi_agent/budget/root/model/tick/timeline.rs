//! One committed chat cursor for one original paid Root publication.
//! No authority, mailbox ack, dispatch claim, assistant body or repair writes.
use super::{SavedDecision, Tick};
use crate::agent_runtime::store;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};

pub(crate) struct TimelineReceipt {
    pub(crate) entity: String,
    pub(crate) sequence: i64,
    pub(crate) created: String,
}
impl Tick {
    fn timeline_entity(&self, event: i64) -> String {
        json!([self.call.owner.root, event]).to_string()
    }
    fn timeline_payload(&self, saved: &SavedDecision, event: i64) -> Value {
        json!({"schemaVersion":1,"advisoryOnly":true,"rootRunId":self.call.owner.root,
            "rootControl":self.call.owner.id,"callId":self.call.id,"round":self.call.round,
            "modelEventSequence":event,"requestHash":self.call.request,"basisHash":self.basis,
            "decisionHash":store::stable_hash(&saved.summary.as_json().to_string())})
    }
    pub(super) fn append_timeline(
        &self,
        tx: &Transaction<'_>,
        saved: &SavedDecision,
        event: i64,
    ) -> Result<(), String> {
        let scope = &self.call.owner.contract["root"];
        let entity = self.timeline_entity(event);
        let payload = self.timeline_payload(saved, event).to_string();
        let scan = scope["scan"]
            .as_str()
            .ok_or("root_tick_timeline_scope_invalid")?;
        let attempt = scope["attempt"]
            .as_i64()
            .ok_or("root_tick_timeline_scope_invalid")?;
        let inserted = tx
            .execute(
                "INSERT INTO agent_collaboration_events
            (scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
            VALUES(?1,?2,'root_decision',?3,'root_decision',?4)",
                params![scan, attempt, entity, payload],
            )
            .map_err(|e| e.to_string())?;
        if inserted != 1 {
            return Err("root_tick_timeline_insert_unconfirmed".into());
        }
        let sequence = tx.last_insert_rowid();
        let created: String = tx
            .query_row(
                "SELECT created_at FROM agent_collaboration_events
            WHERE sequence=?1",
                [sequence],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let inserted = tx
            .execute(
                "INSERT INTO agent_root_tick_timeline_receipts
            (call_id,root_run_id,lease_attempt_id,model_event_sequence,collaboration_sequence,
            scan_id,attempt_number,entity_id,payload_json,event_created_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    self.call.id,
                    self.call.owner.root,
                    self.call.owner.id,
                    event,
                    sequence,
                    scan,
                    attempt,
                    entity,
                    payload,
                    created
                ],
            )
            .map_err(|e| e.to_string())?;
        if inserted != 1 || self.verify_timeline(tx, saved, event)?.sequence != sequence {
            return Err("root_tick_timeline_receipt_unconfirmed".into());
        }
        Ok(())
    }
    pub(super) fn verify_timeline(
        &self,
        db: &Connection,
        saved: &SavedDecision,
        event: i64,
    ) -> Result<TimelineReceipt, String> {
        let entity = self.timeline_entity(event);
        let payload = self.timeline_payload(saved, event).to_string();
        let scope = &self.call.owner.contract["root"];
        let row: Option<(i64, String)> = db
            .query_row(
                "SELECT m.collaboration_sequence,m.event_created_at
            FROM agent_root_tick_timeline_receipts m JOIN agent_collaboration_events e
              ON e.sequence=m.collaboration_sequence AND e.scan_id=m.scan_id
              AND e.attempt_number=m.attempt_number AND e.entity_type='root_decision'
              AND e.event_type='root_decision' AND e.entity_id=m.entity_id
              AND e.payload_json=m.payload_json AND e.created_at=m.event_created_at
            WHERE m.call_id=?1 AND m.root_run_id=?2 AND m.lease_attempt_id=?3
              AND m.model_event_sequence=?4 AND m.scan_id=?5 AND m.attempt_number=?6
              AND m.entity_id=?7 AND m.payload_json=?8",
                params![
                    self.call.id,
                    self.call.owner.root,
                    self.call.owner.id,
                    event,
                    scope["scan"]
                        .as_str()
                        .ok_or("root_tick_timeline_scope_invalid")?,
                    scope["attempt"]
                        .as_i64()
                        .ok_or("root_tick_timeline_scope_invalid")?,
                    entity,
                    payload
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (sequence, created) = row.ok_or("root_tick_timeline_receipt_unverified")?;
        // Audit aliases across all scopes before any view/page filtering. A
        // second row cannot replace this original cursor by becoming newest.
        let count:i64=db.query_row("SELECT count(*) FROM agent_collaboration_events
            WHERE sequence=?1 OR ((event_type='root_decision' OR entity_type='root_decision')
              AND (entity_id=?2 OR (json_valid(payload_json) AND json_extract(payload_json,'$.callId')=?3)))",
            params![sequence,entity,self.call.id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if sequence <= 0 || created.is_empty() || count != 1 || saved.unknown {
            return Err("root_tick_timeline_receipt_unverified".into());
        }
        Ok(TimelineReceipt {
            entity,
            sequence,
            created,
        })
    }
}
