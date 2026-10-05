use super::*;
use crate::agent_runtime::multi_agent::attempts::AssignmentAttempt;
use rusqlite::types::Value;

struct StoredRecord {
    values: Vec<Value>,
    from: String,
    to: String,
    from_role: String,
    to_role: String,
    assignment: String,
    revision: i64,
    kind: String,
    payload: String,
    delivered: String,
    acknowledged: String,
    attempts: i64,
    run: String,
}

// Capture all persisted columns so a write trigger cannot silently change an
// unrelated route, payload, timestamp or receipt while reporting success.
pub(super) struct Record {
    columns: Vec<String>,
    values: Vec<Value>,
    pub message: MailboxMessage,
    pub delivered: String,
    pub acknowledged: String,
    worker: AssignmentAttempt,
}

#[derive(PartialEq, Eq)]
pub(super) enum ConsumerAuthority {
    Live(Option<AssignmentAttempt>),
    SavedGap(AssignmentAttempt),
}

impl Record {
    fn text(&self, name: &str) -> Result<&str, String> {
        let index = self
            .columns
            .iter()
            .position(|column| column == name)
            .ok_or("mailbox_receipt_column_missing")?;
        match self.values.get(index) {
            Some(Value::Text(value)) => Ok(value),
            _ => Err("mailbox_receipt_column_invalid".into()),
        }
    }

    pub fn consumer_authority(
        &self,
        db: &Connection,
        lease: &CoordinatorLease,
        recipient: &str,
    ) -> Result<ConsumerAuthority, String> {
        match authority::receiver(db, lease, recipient) {
            Ok(live) => Ok(ConsumerAuthority::Live(live)),
            Err(error) => {
                // This is a Coordinator's local assessment receipt, never an
                // instruction delivered to reactivate a paused/expired worker.
                if self.message.kind != "proposal_assessed" || recipient != self.worker.child_run_id
                {
                    return Err(error);
                }
                saved_message::verify(
                    db,
                    lease,
                    &self.worker,
                    self.text("from_run_id")?,
                    self.text("to_run_id")?,
                    self.text("from_agent")?,
                    self.text("to_agent")?,
                    &self.message.kind,
                    self.text("correlation_id")?,
                    &self.message.payload,
                )?;
                Ok(ConsumerAuthority::SavedGap(self.worker.clone()))
            }
        }
    }

    pub fn load(
        db: &Connection,
        lease: &CoordinatorLease,
        id: &str,
        recipient: &str,
    ) -> Result<Self, String> {
        let mut query = db
            .prepare("SELECT * FROM agent_messages WHERE id=?1 AND to_run_id=?2 AND root_run_id=?3")
            .map_err(|e| e.to_string())?;
        let columns = query
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let count = columns.len();
        let StoredRecord {
            values,
            from,
            to,
            from_role,
            to_role,
            assignment,
            revision,
            kind,
            payload,
            delivered,
            acknowledged,
            attempts,
            run,
        } = query
            .query_row(params![id, recipient, lease.root_run_id], |r| {
                Ok(StoredRecord {
                    values: (0..count)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                    from: r.get("from_run_id")?,
                    to: r.get("to_run_id")?,
                    from_role: r.get("from_agent")?,
                    to_role: r.get("to_agent")?,
                    assignment: r.get("assignment_id")?,
                    revision: r.get("evidence_revision")?,
                    kind: r.get("kind")?,
                    payload: r.get("payload_json")?,
                    delivered: r.get("delivered_at")?,
                    acknowledged: r.get("acknowledged_at")?,
                    attempts: r.get("delivery_attempts")?,
                    run: r.get("run_id")?,
                })
            })
            .map_err(|e| format!("mailbox_record_missing_or_invalid:{e}"))?;
        if run != lease.root_run_id || attempts < 0 {
            return Err("mailbox_record_binding_conflict".into());
        }
        let worker = authority::route(
            db,
            lease,
            &from,
            &to,
            &from_role,
            &to_role,
            &assignment,
            revision,
        )?;
        let payload =
            serde_json::from_str(&payload).map_err(|e| format!("无法解码 agent mailbox：{e}"))?;
        Ok(Self {
            columns,
            values,
            message: MailboxMessage {
                id: id.into(),
                kind,
                payload,
                delivery_attempts: attempts,
            },
            delivered,
            acknowledged,
            worker,
        })
    }

    fn set(&mut self, column: &str, value: Value) -> Result<(), String> {
        let index = self
            .columns
            .iter()
            .position(|name| name == column)
            .ok_or("mailbox_receipt_column_missing")?;
        self.values[index] = value;
        Ok(())
    }

    pub fn expect_delivery(&mut self, at: &str) -> Result<(), String> {
        if self.delivered.is_empty() {
            self.set("delivered_at", Value::Text(at.into()))?;
        }
        let count = self
            .message
            .delivery_attempts
            .checked_add(1)
            .ok_or("mailbox_delivery_count_overflow")?;
        self.set("delivery_attempts", Value::Integer(count))
    }

    pub fn expect_ack(&mut self, at: &str) -> Result<(), String> {
        self.set("acknowledged_at", Value::Text(at.into()))
    }

    pub fn verify(
        &self,
        db: &Connection,
        lease: &CoordinatorLease,
        recipient: &str,
    ) -> Result<(), String> {
        let actual = Self::load(db, lease, &self.message.id, recipient)?;
        if actual.columns != self.columns
            || actual.values != self.values
            || actual.worker != self.worker
        {
            return Err("mailbox_receipt_postcondition_failed".into());
        }
        Ok(())
    }
}
