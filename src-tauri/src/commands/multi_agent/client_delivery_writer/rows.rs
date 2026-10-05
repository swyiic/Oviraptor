//! Physical rows writable by this private writer, with original rowid and raw JSON.
use rusqlite::{types::Value, Connection};
use std::collections::BTreeMap;
#[derive(Clone, PartialEq)]
pub(super) struct Table {
    pub(super) columns: Vec<String>,
    pub(super) rows: BTreeMap<i64, Vec<Value>>,
}
pub(super) type Tables = BTreeMap<String, Table>;
impl Table {
    pub(super) fn read(db: &Connection, name: &str) -> Result<Self, String> {
        let escaped = name.replace('"', "\"\"");
        let mut q = db
            .prepare(&format!("SELECT rowid,* FROM \"{escaped}\" ORDER BY rowid"))
            .map_err(|e| format!("client_side_delivery_physical_schema:{e}"))?;
        let columns = q
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let n = columns.len();
        let raw = q
            .query_map([], |r| {
                (0..n)
                    .map(|i| r.get::<_, Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        let mut rows = BTreeMap::new();
        for row in raw {
            let Some(Value::Integer(id)) = row.first() else {
                return Err("client_side_delivery_rowid_missing".into());
            };
            if rows.insert(*id, row).is_some() {
                return Err("client_side_delivery_rowid_conflict".into());
            }
        }
        Ok(Self { columns, rows })
    }
    pub(super) fn index(&self, column: &str) -> Result<usize, String> {
        self.columns
            .iter()
            .position(|n| n == column)
            .ok_or_else(|| "client_side_delivery_column_missing".into())
    }
    pub(super) fn text<'a>(&self, row: &'a [Value], col: &str) -> Result<&'a str, String> {
        match row.get(self.index(col)?) {
            Some(Value::Text(t)) => Ok(t),
            _ => Err("client_side_delivery_column_invalid".into()),
        }
    }
    pub(super) fn number(&self, row: &[Value], col: &str) -> Result<i64, String> {
        match row.get(self.index(col)?) {
            Some(Value::Integer(n)) => Ok(*n),
            _ => Err("client_side_delivery_column_invalid".into()),
        }
    }
    pub(super) fn find(&self, col: &str, key: &str) -> Result<&[Value], String> {
        let mut found = self
            .rows
            .values()
            .filter(|r| self.text(r, col).is_ok_and(|s| s == key));
        let row = found
            .next()
            .ok_or("client_side_delivery_original_row_missing")?;
        if found.next().is_some() {
            return Err("client_side_delivery_original_row_conflict".into());
        }
        Ok(row)
    }
    pub(super) fn set(&self, row: &mut [Value], col: &str, value: Value) -> Result<(), String> {
        row[self.index(col)?] = value;
        Ok(())
    }
    pub(super) fn added<'a>(&'a self, old: &Self) -> Result<Vec<&'a [Value]>, String> {
        if self.columns != old.columns {
            return Err("client_side_delivery_schema_changed".into());
        }
        Ok(self
            .rows
            .iter()
            .filter(|(id, _)| !old.rows.contains_key(id))
            .map(|(_, r)| r.as_slice())
            .collect())
    }
    pub(super) fn replace(&mut self, row: Vec<Value>) -> Result<(), String> {
        let Some(Value::Integer(id)) = row.first() else {
            return Err("client_side_delivery_rowid_missing".into());
        };
        if !self.rows.contains_key(id) {
            return Err("client_side_delivery_original_row_missing".into());
        }
        self.rows.insert(*id, row);
        Ok(())
    }
}
pub(super) fn capture(db: &Connection) -> Result<Tables, String> {
    // The owned hook denies every other table write, including trigger writes.
    // This IMMEDIATE transaction blocks other writers; full schema is checked
    // separately. Do not materialize unrelated business or asset history.
    [
        "agent_budget_entries",
        "agent_messages",
        "agent_collaboration_events",
        "agent_budget_ledger",
        "agent_runs",
        "agent_assignments",
        "agent_assignment_attempts",
        "agent_capability_leases",
        "agent_lane_leases",
        "sqlite_sequence",
    ]
    .into_iter()
    .map(|name| Ok((name.to_string(), Table::read(db, name)?)))
    .collect()
}
pub(super) fn get<'a>(tables: &'a Tables, name: &str) -> Result<&'a Table, String> {
    tables
        .get(name)
        .ok_or_else(|| "client_side_delivery_table_missing".into())
}
