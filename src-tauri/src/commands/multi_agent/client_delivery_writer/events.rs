//! Normal new receipt produces four exact events; completed replay produces zero.
use super::{
    proof::Proof,
    rows::{get, Tables},
};
use rusqlite::types::Value;
use serde_json::json;
pub(super) fn verify(
    p: &Proof,
    actual: &Tables,
    expected: &mut Tables,
    id: &str,
    insert: bool,
    ack: bool,
    end: &str,
) -> Result<(), String> {
    let old = get(&p.tables, "agent_collaboration_events")?;
    let t = get(actual, "agent_collaboration_events")?;
    let rows = t.added(old)?;
    let mut events = Vec::new();
    if !p.replay {
        events.push((
            "assignment",
            p.assignment.as_str(),
            json!({"role":"client_side","state":"completed"}),
        ));
        events.push((
            "agent_run",
            p.run.as_str(),
            json!({"role":"client_side","status":"terminal","terminalState":"completed"}),
        ));
    }
    if insert {
        events.push((
            "mailbox_message",
            id,
            json!({"kind":"evidence_summary","deliveredAt":"","acknowledgedAt":""}),
        ));
    }
    if ack {
        let messages = get(actual, "agent_messages")?;
        let message = messages.find("id", id)?;
        events.push(("mailbox_message",id,json!({"kind":"evidence_summary",
            "deliveredAt":messages.text(message,"delivered_at")?,"acknowledgedAt":messages.text(message,"acknowledged_at")?})));
    }
    if events.len() != rows.len() {
        return Err("client_side_delivery_event_count_conflict".into());
    }
    let oldseq = get(&p.tables, "sqlite_sequence")?;
    let own = oldseq.rows.values().find(|r| {
        oldseq
            .text(r, "name")
            .is_ok_and(|n| n == "agent_collaboration_events")
    });
    let floor = own
        .map(|r| oldseq.number(r, "seq"))
        .transpose()?
        .unwrap_or(0);
    for (i, (row, (entity, entity_id, payload))) in rows.iter().zip(events.iter()).enumerate() {
        let seq = floor
            .checked_add(i as i64 + 1)
            .ok_or("client_side_delivery_event_sequence_conflict")?;
        let parsed: serde_json::Value = serde_json::from_str(t.text(row, "payload_json")?)
            .map_err(|_| "client_side_delivery_event_payload_conflict")?;
        if t.text(row, "scan_id")? != p.scan
            || t.number(row, "attempt_number")? != p.attempt
            || t.text(row, "entity_type")? != *entity
            || t.text(row, "entity_id")? != *entity_id
            || t.text(row, "event_type")? != *entity
            || parsed != *payload
            || t.number(row, "sequence")? != seq
            || row.first() != Some(&Value::Integer(seq))
            || t.text(row, "created_at")? < p.now.as_str()
            || t.text(row, "created_at")? > end
        {
            return Err("client_side_delivery_event_scope_conflict".into());
        }
    }
    let seq = get(actual, "sqlite_sequence")?;
    let delta =
        i64::try_from(events.len()).map_err(|_| "client_side_delivery_event_count_conflict")?;
    if delta == 0 {
        return if seq == oldseq {
            Ok(())
        } else {
            Err("client_side_delivery_allocator_conflict".into())
        };
    }
    let n = floor
        .checked_add(delta)
        .ok_or("client_side_delivery_allocator_conflict")?;
    if let Some(row) = own {
        let mut row = row.clone();
        oldseq.set(&mut row, "seq", Value::Integer(n))?;
        expected.get_mut("sqlite_sequence").unwrap().replace(row)?;
        if !seq.added(oldseq)?.is_empty() {
            return Err("client_side_delivery_allocator_conflict".into());
        }
    } else {
        let added = seq.added(oldseq)?;
        if added.len() != 1
            || seq.text(added[0], "name")? != "agent_collaboration_events"
            || seq.number(added[0], "seq")? != n
        {
            return Err("client_side_delivery_allocator_conflict".into());
        }
    }
    Ok(())
}
