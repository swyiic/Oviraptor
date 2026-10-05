//! Actual SDK diagnostics writer/reader against the full application schema.
use super::{owner::Owner, persist, replay};
use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;
mod reads;
mod writes;
pub(crate) use reads::read_contract;
pub(crate) use writes::write_contract;
fn original(db: &Connection, root: &str) -> Owner {
    Owner::root(db, root, 1).unwrap()
}
fn snapshot(db: &Connection) -> Vec<(String, Vec<String>)> {
    let names=db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
  .query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut q = db
                .prepare(&format!(
                    "SELECT * FROM \"{}\" ORDER BY rowid",
                    name.replace('"', "\"\"")
                ))
                .unwrap();
            let n = q.column_count();
            let rows = q
                .query_map([], |r| {
                    Ok(format!(
                        "{:?}",
                        (0..n)
                            .map(|i| r.get::<_, rusqlite::types::Value>(i))
                            .collect::<Result<Vec<_>, _>>()?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}
fn stages(db: &Connection, id: &str) -> Vec<String> {
    db.prepare("SELECT stage FROM native_sdk_log_rows WHERE owner_id=?1 ORDER BY ordinal")
        .unwrap()
        .query_map([id], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}
fn start(path: &Path, root: &str) -> (Connection, Owner) {
    let mut db = persist::connection(path).unwrap();
    let owner = original(&db, root);
    persist::begin(&mut db, &owner).unwrap();
    // SDK 02 had a non-atomic first stage. This setup preserves its normal prefix;
    // the dedicated atomic-begin negative never performs this fallback.
    if stages(&db, &owner.owner_id).is_empty() {
        persist::append(&mut db, &owner, "prepared", "", "").unwrap();
    }
    (db, owner)
}
fn read(path: &Path, scan: &str, attempt: i64, cursor: Option<i64>, after: i64) -> Value {
    serde_json::to_value(replay::read(path, scan, attempt, cursor, None, after, 300).unwrap())
        .unwrap()
}
