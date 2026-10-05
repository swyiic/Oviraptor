//! One original route and exact semantic receipt; no additional mailbox claims.
use super::{
    proof::Proof,
    rows::{get, Table, Tables},
};
use rusqlite::types::Value;
pub(super) fn dedup(p: &Proof) -> String {
    format!(
        "{}:evidence_summary:client-side:{}:1",
        p.assignment, p.assignment
    )
}
pub(super) fn previous(p: &Proof) -> Result<Option<&[Value]>, String> {
    let t = get(&p.tables, "agent_messages")?;
    let key = dedup(p);
    let mut found = t.rows.values().filter(|r| {
        t.text(r, "dedup_key").is_ok_and(|v| v == key)
            && t.text(r, "run_id").is_ok_and(|v| v == p.root)
    });
    let row = found.next().map(Vec::as_slice);
    if found.next().is_some() {
        return Err("client_side_delivery_mailbox_duplicate".into());
    }
    Ok(row)
}
pub(super) fn expected(
    p: &Proof,
    actual: &Tables,
    expected: &mut Tables,
    end: &str,
) -> Result<(String, bool, bool), String> {
    let old = get(&p.tables, "agent_messages")?;
    let t = get(actual, "agent_messages")?;
    let key = dedup(p);
    let mut own = t.rows.values().filter(|r| {
        t.text(r, "dedup_key").is_ok_and(|v| v == key)
            && t.text(r, "run_id").is_ok_and(|v| v == p.root)
    });
    let row = own.next().ok_or("client_side_delivery_mailbox_missing")?;
    if own.next().is_some() {
        return Err("client_side_delivery_mailbox_duplicate".into());
    }
    let id = t.text(row, "id")?.to_string();
    let previous = previous(p)?;
    let insert = previous.is_none();
    let ack = previous
        .map(|r| old.text(r, "acknowledged_at"))
        .transpose()?
        .unwrap_or("")
        .is_empty();
    let delivered = t.text(row, "delivered_at")?;
    let acknowledged = t.text(row, "acknowledged_at")?;
    if delivered.is_empty() || acknowledged.is_empty() || t.number(row, "delivery_attempts")? != 1 {
        return Err("client_side_delivery_mailbox_receipt_conflict".into());
    }
    if ack && (acknowledged < p.now.as_str() || acknowledged > end) {
        return Err("client_side_delivery_mailbox_timestamp_conflict".into());
    }
    if insert {
        if uuid::Uuid::parse_str(&id).is_err()
            || delivered != acknowledged
            || t.added(old)?.len() != 1
        {
            return Err("client_side_delivery_mailbox_insert_conflict".into());
        }
        let correlation = format!("client-side:{}", p.assignment);
        for col in &t.columns {
            match col.as_str() {
                "rowid" | "id" | "delivered_at" | "acknowledged_at" => {}
                "run_id" | "root_run_id" | "to_run_id" => exact(t, row, col, &p.root)?,
                "from_run_id" => exact(t, row, col, &p.run)?,
                "assignment_id" => exact(t, row, col, &p.assignment)?,
                "from_agent" => exact(t, row, col, "client_side")?,
                "to_agent" => exact(t, row, col, "coordinator")?,
                "kind" => exact(t, row, col, "evidence_summary")?,
                "correlation_id" => exact(t, row, col, &correlation)?,
                "dedup_key" => exact(t, row, col, &key)?,
                "payload_json" => exact(t, row, col, &p.payload)?,
                "artifact_refs_json" => exact(t, row, col, "[]")?,
                "evidence_revision" | "delivery_attempts" if t.number(row, col)? == 1 => {}
                "created_at" if t.text(row, col)? >= p.now.as_str() && t.text(row, col)? <= end => {
                }
                _ => return Err("client_side_delivery_mailbox_schema_conflict".into()),
            }
        }
    } else {
        if !t.added(old)?.is_empty() {
            return Err("client_side_delivery_extra_mailbox_row".into());
        }
        let previous = previous.unwrap();
        let mut allowed = previous.to_vec();
        if ack {
            if old.number(previous, "delivery_attempts")? != 0 {
                return Err("client_side_delivery_mailbox_attempt_conflict".into());
            }
            if old.text(previous, "delivered_at")?.is_empty() {
                if delivered != acknowledged {
                    return Err("client_side_delivery_mailbox_timestamp_conflict".into());
                }
                old.set(&mut allowed, "delivered_at", Value::Text(delivered.into()))?;
            }
            old.set(
                &mut allowed,
                "acknowledged_at",
                Value::Text(acknowledged.into()),
            )?;
            old.set(&mut allowed, "delivery_attempts", Value::Integer(1))?;
        }
        expected
            .get_mut("agent_messages")
            .unwrap()
            .replace(allowed)?;
    }
    Ok((id, insert, ack))
}
fn exact(t: &Table, row: &[Value], col: &str, value: &str) -> Result<(), String> {
    if t.text(row, col)? == value {
        Ok(())
    } else {
        Err("client_side_delivery_mailbox_scope_conflict".into())
    }
}
