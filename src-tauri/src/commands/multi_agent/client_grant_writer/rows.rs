//! Physical rows retained in memory only, including rowid and original raw JSON.
use rusqlite::{types::Value, Connection};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, PartialEq)]
pub(super) struct Table {
    pub(super) name: &'static str,
    pub(super) columns: Vec<String>,
    pub(super) rows: Vec<Vec<Value>>,
}
impl Table {
    pub(super) fn read(db: &Connection, name: &'static str) -> Result<Self, String> {
        let mut q = db
            .prepare(&format!("SELECT rowid,* FROM {name} ORDER BY rowid"))
            .map_err(|e| e.to_string())?;
        let columns = q
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let n = columns.len();
        let rows = q
            .query_map([], |r| {
                (0..n)
                    .map(|i| r.get::<_, Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            name,
            columns,
            rows,
        })
    }
    pub(super) fn value<'a>(&self, row: &'a [Value], column: &str) -> Result<&'a Value, String> {
        let n = self
            .columns
            .iter()
            .position(|c| c == column)
            .ok_or("client_side_grant_schema_conflict")?;
        row.get(n)
            .ok_or_else(|| "client_side_grant_schema_conflict".into())
    }
    pub(super) fn text<'a>(&self, row: &'a [Value], column: &str) -> Result<&'a str, String> {
        match self.value(row, column)? {
            Value::Text(v) => Ok(v),
            _ => Err("client_side_grant_schema_conflict".into()),
        }
    }
    pub(super) fn number(&self, row: &[Value], column: &str) -> Result<i64, String> {
        match self.value(row, column)? {
            Value::Integer(v) => Ok(*v),
            _ => Err("client_side_grant_schema_conflict".into()),
        }
    }
    pub(super) fn only(&self, row: &[Value], column: &str, expected: &str) -> Result<(), String> {
        if self.text(row, column)? == expected {
            Ok(())
        } else {
            Err("client_side_grant_row_scope_conflict".into())
        }
    }
    fn indexed(&self) -> Result<BTreeMap<i64, &[Value]>, String> {
        self.rows
            .iter()
            .map(|row| match row.first() {
                Some(Value::Integer(id)) => Ok((*id, row.as_slice())),
                _ => Err("client_side_grant_schema_conflict".into()),
            })
            .collect()
    }
    pub(super) fn physical<'a>(&'a self, old: &[Value]) -> Result<&'a [Value], String> {
        self.rows
            .iter()
            .find(|r| r.first() == old.first())
            .map(Vec::as_slice)
            .ok_or_else(|| "client_side_grant_original_row_changed".into())
    }
    pub(super) fn new_rows<'a>(&'a self, old: &Self) -> Result<Vec<&'a [Value]>, String> {
        if self.columns != old.columns {
            return Err("client_side_grant_schema_conflict".into());
        }
        let ids = old.indexed()?.into_keys().collect::<BTreeSet<_>>();
        Ok(self
            .indexed()?
            .into_iter()
            .filter_map(|(id, row)| (!ids.contains(&id)).then_some(row))
            .collect())
    }
    pub(super) fn require_old(&self, old: &Self) -> Result<(), String> {
        if self.columns != old.columns {
            return Err("client_side_grant_schema_conflict".into());
        }
        let current = self.indexed()?;
        for (id, row) in old.indexed()? {
            if current.get(&id).copied() != Some(row) {
                return Err("client_side_grant_original_row_changed".into());
            }
        }
        Ok(())
    }
}
