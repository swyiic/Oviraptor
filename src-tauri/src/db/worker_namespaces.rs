//! Native worker storage upgrade. No retired data, worker identity, grants or
//! cost facts are created. The two existing tables keep every original value.
use rusqlite::{Connection, Transaction, TransactionBehavior};

const TABLES: [&str; 2] = ["agent_specialist_calls", "agent_capability_leases"];
#[path = "worker_namespaces/source.rs"]
mod source;

fn declaration(table: &str) -> Result<&'static str, String> {
    let start = super::SCHEMA
        .find(&format!("CREATE TABLE IF NOT EXISTS {table} ("))
        .ok_or("worker_namespace_schema_missing")?;
    let rest = &super::SCHEMA[start..];
    let end = rest.find("\n);").ok_or("worker_namespace_schema_invalid")? + 3;
    Ok(&rest[..end])
}

fn normalized(sql: &str) -> String {
    sql.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase()
        .replace("IFNOTEXISTS", "")
        .trim_end_matches(';')
        .into()
}

fn original_declaration(table: &str, current: &str) -> String {
    match table {
        "agent_specialist_calls" => current
            .replace(
                "assignment_id TEXT NOT NULL REFERENCES",
                "assignment_id TEXT PRIMARY KEY REFERENCES",
            )
            .replace(
                "child_run_id TEXT NOT NULL PRIMARY KEY REFERENCES",
                "child_run_id TEXT NOT NULL UNIQUE REFERENCES",
            ),
        "agent_capability_leases" => current.replace(
            "UNIQUE(assignment_id,child_run_id,capability)",
            "UNIQUE(assignment_id,capability)",
        ),
        _ => unreachable!(),
    }
}

fn schema_objects(tx: &Transaction<'_>, table: &str) -> Result<Vec<String>, String> {
    let mut query = tx.prepare("SELECT sql FROM sqlite_schema WHERE tbl_name=?1 AND type IN ('index','trigger') AND sql IS NOT NULL ORDER BY type,name")
        .map_err(|e|e.to_string())?;
    let rows = query
        .query_map([table], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string());
    rows
}

fn projection(tx: &Transaction<'_>, table: &str) -> Result<String, String> {
    let mut query = tx
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let names = query
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    // Canonical columns are fixed identifiers from this binary's schema.
    Ok(std::iter::once("rowid".into())
        .chain(names.iter().map(|name| {
            let quoted = format!("\"{}\"", name.replace('"', "\"\""));
            format!("typeof({quoted}),quote({quoted})")
        }))
        .collect::<Vec<_>>()
        .join(","))
}

fn rebuild(tx: &Transaction<'_>, table: &str) -> Result<(), String> {
    let current = declaration(table)?;
    let actual: String = tx
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if normalized(&actual) == normalized(current) {
        return Ok(());
    }
    if normalized(&actual) != normalized(&original_declaration(table, current)) {
        return Err(format!(
            "worker_namespace_unrecognized_native_schema:{table}"
        ));
    }
    // These tables have no incoming FKs in the canonical schema. Refuse an
    // unexpected extension rather than dropping its referenced parent.
    let incoming: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema m,pragma_foreign_key_list(m.name) f WHERE m.type='table' AND f.\"table\"=?1)",[table],|r|r.get(0)).map_err(|e|e.to_string())?;
    if incoming {
        return Err(format!("worker_namespace_unexpected_reference:{table}"));
    }
    let objects = schema_objects(tx, table)?;
    let proof = projection(tx, table)?;
    let names = columns(tx, table)?;
    let backup = format!("native_worker_copy_{}", uuid::Uuid::new_v4().simple());
    let old_count: i64 = tx
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    tx.execute_batch(&format!("CREATE TEMP TABLE {backup} AS SELECT rowid AS original_rowid,* FROM {table}; DROP TABLE {table}; {current}
        INSERT INTO {table}(rowid,{names}) SELECT original_rowid,{names} FROM {backup};")).map_err(|e|format!("worker_namespace_copy:{table}:{e}"))?;
    let saved_proof = proof.replacen("rowid", "original_rowid", 1);
    let exact: bool = tx
        .query_row(
            &format!(
                "SELECT (SELECT count(*) FROM {table})=?1
        AND NOT EXISTS(SELECT {proof} FROM {table} EXCEPT SELECT {saved_proof} FROM {backup})
        AND NOT EXISTS(SELECT {saved_proof} FROM {backup} EXCEPT SELECT {proof} FROM {table})"
            ),
            [old_count],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exact {
        return Err(format!("worker_namespace_copy_changed:{table}"));
    }
    for sql in objects {
        tx.execute_batch(&sql).map_err(|e| e.to_string())?;
    }
    let invalid: bool = tx
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check('{table}'))"),
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if invalid {
        return Err(format!("worker_namespace_invalid_reference:{table}"));
    }
    tx.execute_batch(&format!("DROP TABLE {backup}"))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn columns(tx: &Transaction<'_>, table: &str) -> Result<String, String> {
    let mut query = tx
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let names = query
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(names
        .into_iter()
        .map(|n| format!("\"{}\"", n.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(","))
}

pub(super) fn migrate(db: &mut Connection) -> Result<(), String> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    for table in TABLES {
        rebuild(&tx, table)?;
    }
    source::migrate(&tx)?;
    tx.commit().map_err(|e| e.to_string())
}
