use super::lease::{require_active_attempt, validate_coordinator_lease, CoordinatorLease};
use crate::agent_runtime::secrets::redact_json;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq)]
pub struct MailboxMessage {
    pub id: String,
    pub kind: String,
    pub payload: JsonValue,
    pub delivery_attempts: i64,
}

mod authority;
mod consume;
mod record;
mod saved_message;
mod send;
pub(crate) use consume::consume_coordinator_message;
pub use send::send;
pub(crate) use send::send_saved_specialist;

#[cfg(test)]
pub fn deliver(
    connection: &Connection,
    lease: &CoordinatorLease,
    run_id: &str,
    limit: i64,
) -> Result<Vec<MailboxMessage>, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定 agent mailbox 投递：{error}"))?;
    validate_coordinator_lease(&transaction, lease)?;
    require_active_attempt(&transaction, &lease.scan_id, lease.attempt_number)?;
    let receiver = authority::receiver(&transaction, lease, run_id)?;
    let mut statement = transaction
        .prepare(
            "SELECT id,kind,payload_json,delivery_attempts FROM agent_messages \
             WHERE to_run_id=?1 AND root_run_id=?2 AND acknowledged_at='' ORDER BY created_at,id LIMIT ?3",
        )
        .map_err(|error| format!("无法打开 agent mailbox：{error}"))?;
    let messages: Vec<MailboxMessage> = statement
        .query_map(
            params![run_id, lease.root_run_id, limit.clamp(1, 100)],
            |row| {
                let raw: String = row.get(2)?;
                let payload = serde_json::from_str(&raw).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(MailboxMessage {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    payload,
                    delivery_attempts: row.get(3)?,
                })
            },
        )
        .map_err(|error| format!("无法读取 agent mailbox：{error}"))?
        .collect::<Result<_, _>>()
        .map_err(|error| format!("无法解码 agent mailbox：{error}"))?;
    drop(statement);
    for message in &messages {
        let mut receipt = record::Record::load(&transaction, lease, &message.id, run_id)?;
        let at: String = transaction
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let changed = transaction
            .execute(
                "UPDATE agent_messages SET delivered_at=CASE WHEN delivered_at='' THEN ?4 ELSE delivered_at END,\
                 delivery_attempts=delivery_attempts+1 WHERE id=?1 AND root_run_id=?2 AND to_run_id=?3 AND acknowledged_at=''",
                params![message.id, lease.root_run_id, run_id,at],
            )
            .map_err(|error| format!("无法标记 agent 消息已投递：{error}"))?;
        if changed != 1 {
            return Err("mailbox_delivery_conflict".into());
        }
        receipt.expect_delivery(&at)?;
        receipt.verify(&transaction, lease, run_id)?;
    }
    if authority::receiver(&transaction, lease, run_id)? != receiver {
        return Err("mailbox_receiver_authority_conflict".into());
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交 agent mailbox 投递：{error}"))?;
    Ok(messages)
}

/// Consume only the message this step created. A child result must never
/// acknowledge an unrelated queued directive or another agent's proposal.
pub fn deliver_expected(
    connection: &Connection,
    lease: &CoordinatorLease,
    run_id: &str,
    message_id: &str,
) -> Result<MailboxMessage, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定指定 agent 消息：{error}"))?;
    validate_coordinator_lease(&transaction, lease)?;
    require_active_attempt(&transaction, &lease.scan_id, lease.attempt_number)?;
    let receiver = authority::receiver(&transaction, lease, run_id)?;
    let message: Option<MailboxMessage> = transaction
        .query_row(
            "SELECT id,kind,payload_json,delivery_attempts FROM agent_messages \
         WHERE id=?1 AND to_run_id=?2 AND root_run_id=?3 AND acknowledged_at=''",
            params![message_id, run_id, lease.root_run_id],
            |row| {
                let raw: String = row.get(2)?;
                let payload = serde_json::from_str(&raw).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(MailboxMessage {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    payload,
                    delivery_attempts: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("无法读取指定 agent 消息：{error}"))?;
    let message =
        message.ok_or_else(|| "mailbox_expected_message_missing_or_acknowledged".to_string())?;
    let mut receipt = record::Record::load(&transaction, lease, message_id, run_id)?;
    let at: String = transaction
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let changed = transaction.execute(
        "UPDATE agent_messages SET delivered_at=CASE WHEN delivered_at='' THEN ?4 ELSE delivered_at END, \
         delivery_attempts=delivery_attempts+1 WHERE id=?1 AND to_run_id=?2 AND root_run_id=?3 AND acknowledged_at=''",
        params![message_id, run_id, lease.root_run_id,at],
    ).map_err(|error| format!("无法投递指定 agent 消息：{error}"))?;
    if changed != 1 {
        return Err("mailbox_delivery_conflict".into());
    }
    receipt.expect_delivery(&at)?;
    receipt.verify(&transaction, lease, run_id)?;
    if authority::receiver(&transaction, lease, run_id)? != receiver {
        return Err("mailbox_receiver_authority_conflict".into());
    }
    transaction
        .commit()
        .map_err(|error| format!("无法提交指定 agent 消息投递：{error}"))?;
    Ok(message)
}

pub fn acknowledge(
    connection: &Connection,
    lease: &CoordinatorLease,
    run_id: &str,
    message_id: &str,
) -> Result<(), String> {
    if !connection.is_autocommit() {
        return acknowledge_in_transaction(connection, lease, run_id, message_id);
    }
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    acknowledge_in_transaction(&tx, lease, run_id, message_id)?;
    tx.commit().map_err(|e| e.to_string())
}

fn acknowledge_in_transaction(
    connection: &Connection,
    lease: &CoordinatorLease,
    run_id: &str,
    message_id: &str,
) -> Result<(), String> {
    let receiver = authority::receiver(connection, lease, run_id)?;
    let mut receipt = record::Record::load(connection, lease, message_id, run_id)?;
    if receipt.delivered.is_empty() || !receipt.acknowledged.is_empty() {
        return Err("mailbox_ack_conflict".into());
    }
    let at: String = connection
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let changed = connection
        .execute(
            "UPDATE agent_messages SET acknowledged_at=?9 \
             WHERE id=?1 AND to_run_id=?2 AND root_run_id=?3 AND delivered_at<>'' AND acknowledged_at='' \
             AND EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE scan_id=?4 AND attempt_number=?5 \
               AND target_key=?6 AND root_run_id=?3 AND lease_epoch=?7 AND fencing_token=?8 \
               AND lease_expires_at>datetime('now','localtime')) \
             AND EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?4 AND attempt_count=?5 AND status='scanning') \
             AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?4)",
            params![message_id, run_id, lease.root_run_id, lease.scan_id, lease.attempt_number,
                lease.target_key, lease.lease_epoch, lease.fencing_token,at],
        )
        .map_err(|error| format!("无法确认 agent 消息：{error}"))?;
    if changed == 1 {
        receipt.expect_ack(&at)?;
        receipt.verify(connection, lease, run_id)?;
        if authority::receiver(connection, lease, run_id)? != receiver {
            return Err("mailbox_receiver_authority_conflict".into());
        }
        Ok(())
    } else {
        validate_coordinator_lease(connection, lease)?;
        require_active_attempt(connection, &lease.scan_id, lease.attempt_number)?;
        Err("mailbox_ack_conflict".into())
    }
}

pub(crate) fn verified_completed_output(db:&Connection,lease:&CoordinatorLease,id:&str)->Result<MailboxMessage,String> {
    let record=record::Record::load(db,lease,id,&lease.root_run_id)?;
    if record.delivered.is_empty() || record.acknowledged.is_empty() || record.message.delivery_attempts!=1 {
        return Err("mailbox_output_not_consumed".into());
    }
    record.verify(db,lease,&lease.root_run_id)?;
    Ok(record.message)
}
