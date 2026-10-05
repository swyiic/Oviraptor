//! Lossless SQLite values, column order and original physical rowids.
use rusqlite::{types::Value, Connection};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Table {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub(super) enum Cell {
    Null,
    Integer(i64),
    Real(u64),
    Text(String),
    Blob(Vec<u8>),
}
impl Cell {
    fn from(v: Value) -> Self {
        match v {
            Value::Null => Self::Null,
            Value::Integer(v) => Self::Integer(v),
            Value::Real(v) => Self::Real(v.to_bits()),
            Value::Text(v) => Self::Text(v),
            Value::Blob(v) => Self::Blob(v),
        }
    }
    fn value(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Integer(v) => Value::Integer(*v),
            Self::Real(v) => Value::Real(f64::from_bits(*v)),
            Self::Text(v) => Value::Text(v.clone()),
            Self::Blob(v) => Value::Blob(v.clone()),
        }
    }
}
pub(super) fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
impl Table {
    pub fn read(db: &Connection, name: &str, filter: &str, scan: &str) -> Result<Self, String> {
        let mut q = db
            .prepare(&format!(
                "SELECT rowid,* FROM {} WHERE {filter} ORDER BY rowid",
                quote(name)
            ))
            .map_err(|e| e.to_string())?;
        let mut columns = q
            .column_names()
            .iter()
            .map(|v| (*v).to_owned())
            .collect::<Vec<_>>();
        columns[0] = "rowid".into(); // INTEGER PRIMARY KEY aliases otherwise rename this column.
        let rows = q
            .query_map([scan], |r| {
                (0..columns.len())
                    .map(|i| r.get::<_, Value>(i).map(Cell::from))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            name: name.into(),
            columns,
            rows,
        })
    }
    pub fn restore(&self, db: &Connection) -> Result<(), String> {
        if self.columns.len() < 2
            || self.columns[0] != "rowid"
            || self.columns[1..]
                .iter()
                .any(|c| c.eq_ignore_ascii_case("rowid"))
        {
            return Err("deleted_audit_columns_invalid".into());
        }
        // Never execute archived SQL. The original Multi exit verifier also
        // checks its trusted schema and guards in this private audit arena.
        if self.name == "agent_multi_exit_receipts" {
            db.execute_batch(include_str!(
                "../multi_agent/budget/clock/finalization/exit_receipt/schema.sql"
            ))
            .map_err(|e| e.to_string())?;
            let mut columns = db
                .prepare("SELECT rowid,* FROM agent_multi_exit_receipts LIMIT 0")
                .map_err(|e| e.to_string())?
                .column_names()
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>();
            columns[0] = "rowid".into();
            if columns != self.columns {
                return Err("deleted_audit_columns_invalid".into());
            }
        } else {
            db.execute_batch(&format!(
                "CREATE TABLE {}({})",
                quote(&self.name),
                self.columns[1..]
                    .iter()
                    .map(|c| quote(c))
                    .collect::<Vec<_>>()
                    .join(",")
            ))
            .map_err(|e| e.to_string())?;
        }
        let sql = format!(
            "INSERT INTO {}({}) VALUES({})",
            quote(&self.name),
            self.columns
                .iter()
                .map(|c| quote(c))
                .collect::<Vec<_>>()
                .join(","),
            vec!["?"; self.columns.len()].join(",")
        );
        for r in &self.rows {
            if r.len() != self.columns.len() || !matches!(r[0],Cell::Integer(v) if v>0) {
                return Err("deleted_audit_row_invalid".into());
            }
            db.execute(&sql, rusqlite::params_from_iter(r.iter().map(Cell::value)))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn original_filter(&self) -> Result<String, String> {
        let ids = self
            .rows
            .iter()
            .map(|r| match r.first() {
                Some(Cell::Integer(v)) if *v > 0 => Ok(v.to_string()),
                _ => Err("deleted_audit_row_invalid".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!("rowid IN({}) AND ?1 IS NOT NULL", ids.join(",")))
    }
    pub fn outside_filter(&self) -> Result<String, String> {
        if self.name == "sentinel_deleted_scans" {
            return Ok("scan_id<>?1".into());
        }
        let ids = self
            .rows
            .iter()
            .map(|r| match r.first() {
                Some(Cell::Integer(v)) => Ok(v.to_string()),
                _ => Err("deleted_audit_row_invalid".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!(
            "rowid NOT IN({}) AND ?1 IS NOT NULL",
            ids.join(",")
        ))
    }
    pub fn canonical(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}
