//! Preserve every old allocator row; exactly five canonical events advance one.
use super::rows::Table;
use rusqlite::{types::Value, Connection};
pub(super) struct Sequence {
    old: Table,
}
impl Sequence {
    pub(super) fn capture(db: &Connection) -> Result<Self, String> {
        Ok(Self {
            old: Table::read(db, "sqlite_sequence")?,
        })
    }
    pub(super) fn verify(&self, db: &Connection, delta: i64) -> Result<(), String> {
        let now = Table::read(db, "sqlite_sequence")?;
        if now.columns != self.old.columns {
            return Err("client_side_grant_allocator_conflict".into());
        }
        if delta == 0 {
            return if now == self.old {
                Ok(())
            } else {
                Err("client_side_grant_allocator_conflict".into())
            };
        }
        let mut found = false;
        for row in &self.old.rows {
            let mut expected = row.clone();
            if self.old.text(row, "name")? == "agent_collaboration_events" {
                found = true;
                let index = self
                    .old
                    .columns
                    .iter()
                    .position(|c| c == "seq")
                    .ok_or("client_side_grant_allocator_conflict")?;
                expected[index] = Value::Integer(
                    self.old
                        .number(row, "seq")?
                        .checked_add(delta)
                        .ok_or("client_side_grant_allocator_conflict")?,
                );
            }
            if now.physical(row)? != expected.as_slice() {
                return Err("client_side_grant_allocator_conflict".into());
            }
        }
        let added = now.new_rows(&self.old)?;
        if found {
            if !added.is_empty() {
                return Err("client_side_grant_allocator_conflict".into());
            }
        } else {
            if added.len() != 1
                || now.text(added[0], "name")? != "agent_collaboration_events"
                || now.number(added[0], "seq")? != delta
            {
                return Err("client_side_grant_allocator_conflict".into());
            }
        }
        Ok(())
    }
}
