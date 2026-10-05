//! Read-only typed row snapshots for original worker audit postconditions.
use rusqlite::{types::Value, Connection, Params};

#[derive(Clone, PartialEq)]
pub(crate) struct Rows {
    pub(crate) columns: Vec<String>,
    pub(crate) values: Vec<Vec<Value>>,
}

impl Rows {
    pub(crate) fn read(db: &Connection, sql: &str, params: impl Params) -> Result<Self, String> {
        let mut query = db
            .prepare(sql)
            .map_err(|e| format!("expiry_proof_read:{e}"))?;
        let columns = query
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let values = query
            .query_map(params, |row| {
                (0..columns.len()).map(|i| row.get(i)).collect()
            })
            .map_err(|e| format!("expiry_proof_read:{e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("expiry_proof_read:{e}"))?;
        Ok(Self { columns, values })
    }

    pub(crate) fn column(&self, name: &str) -> Result<usize, String> {
        self.columns
            .iter()
            .position(|c| c == name)
            .ok_or_else(|| "expiry_proof_column_missing".into())
    }

    pub(crate) fn row(&self, id: &str) -> Result<&Vec<Value>, String> {
        let key = self.column("id")?;
        self.values
            .iter()
            .find(|r| r[key] == Value::Text(id.into()))
            .ok_or_else(|| "expiry_proof_row_missing".into())
    }

    pub(crate) fn text(&self, id: &str, column: &str) -> Result<String, String> {
        match &self.row(id)?[self.column(column)?] {
            Value::Text(text) => Ok(text.clone()),
            _ => Err("expiry_proof_text_invalid".into()),
        }
    }

    pub(crate) fn set(&mut self, id: &str, column: &str, value: &str) -> Result<(), String> {
        let key = self.column("id")?;
        let index = self.column(column)?;
        let row = self
            .values
            .iter_mut()
            .find(|r| r[key] == Value::Text(id.into()))
            .ok_or("expiry_proof_row_missing")?;
        row[index] = Value::Text(value.into());
        Ok(())
    }
}
