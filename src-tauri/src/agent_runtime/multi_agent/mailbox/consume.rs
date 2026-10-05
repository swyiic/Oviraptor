use super::*;

// A Coordinator may consume an exact result locally. Work addressed to a child
// still requires that live worker; saved Gap assessments are receipt-only.
pub(crate) fn consume_coordinator_message(
    db: &Connection,
    lease: &CoordinatorLease,
    recipient: &str,
    id: &str,
    kind: &str,
    expected: &JsonValue,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("mailbox_consume_transaction_required".into());
    }
    let mut receipt = record::Record::load(db, lease, id, recipient)?;
    if receipt.message.kind != kind || receipt.message.payload != *expected {
        return Err("directive_proposal_mailbox_mismatch".into());
    }
    let authority = receipt.consumer_authority(db, lease, recipient)?;
    if receipt.acknowledged.is_empty() {
        let at: String = db
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let changed = db
            .execute(
                "UPDATE agent_messages SET
            delivered_at=CASE WHEN delivered_at='' THEN ?4 ELSE delivered_at END,
            delivery_attempts=delivery_attempts+1,acknowledged_at=?4
            WHERE id=?1 AND to_run_id=?2 AND root_run_id=?3 AND acknowledged_at=''",
                params![id, recipient, lease.root_run_id, at],
            )
            .map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("directive_proposal_mailbox_consumption_conflict".into());
        }
        receipt.expect_delivery(&at)?;
        receipt.expect_ack(&at)?;
    }
    receipt.verify(db, lease, recipient)?;
    if receipt.consumer_authority(db, lease, recipient)? != authority {
        return Err("mailbox_receiver_authority_conflict".into());
    }
    Ok(())
}
