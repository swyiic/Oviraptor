//! Upgrade the Native Source parent/receipt pair together. The new receipt
//! owner is derived solely from its existing canonical model round.
use super::*;

const ROUNDS: &str = "agent_source_model_rounds";
const TOOLS: &str = "agent_source_tool_receipts";

fn original(table: &str, sql: &str) -> String {
    if table == ROUNDS {
        sql.replace(
            "PRIMARY KEY(assignment_id,child_run_id,round_number)",
            "PRIMARY KEY(assignment_id,round_number)",
        )
    } else {
        sql.replace("    child_run_id TEXT NOT NULL,\n", "")
            .replace(
                "assignment_id,child_run_id,round_number",
                "assignment_id,round_number",
            )
            .replace(
                "assignment_id,child_run_id,call_id",
                "assignment_id,call_id",
            )
    }
}

struct Copy {
    table: &'static str,
    backup: String,
    columns: String,
    proof: String,
    count: i64,
    objects: Vec<String>,
}

impl Copy {
    fn stage(tx: &Transaction<'_>, table: &'static str) -> Result<Self, String> {
        let incoming: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema m,pragma_foreign_key_list(m.name) f
            WHERE m.type='table' AND f.\"table\"=?1 AND NOT (?1=?2 AND m.name=?3))",
                rusqlite::params![table, ROUNDS, TOOLS],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if incoming {
            return Err(format!("worker_namespace_unexpected_reference:{table}"));
        }
        let copy = Self {
            table,
            backup: format!("native_source_copy_{}", uuid::Uuid::new_v4().simple()),
            columns: columns(tx, table)?,
            proof: projection(tx, table)?,
            count: tx
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .map_err(|e| e.to_string())?,
            objects: schema_objects(tx, table)?,
        };
        tx.execute_batch(&format!(
            "CREATE TEMP TABLE {} AS SELECT rowid AS original_rowid,* FROM {table}",
            copy.backup
        ))
        .map_err(|e| e.to_string())?;
        Ok(copy)
    }

    fn verify(&self, tx: &Transaction<'_>) -> Result<(), String> {
        let saved = self.proof.replacen("rowid", "original_rowid", 1);
        let exact: bool = tx
            .query_row(
                &format!(
                    "SELECT (SELECT count(*) FROM {table})=?1
            AND NOT EXISTS(SELECT {proof} FROM {table} EXCEPT SELECT {saved} FROM {backup})
            AND NOT EXISTS(SELECT {saved} FROM {backup} EXCEPT SELECT {proof} FROM {table})",
                    table = self.table,
                    proof = self.proof,
                    backup = self.backup
                ),
                [self.count],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exact {
            return Err(format!("worker_namespace_copy_changed:{}", self.table));
        }
        let invalid: bool = tx
            .query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check('{}'))",
                    self.table
                ),
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if invalid {
            return Err(format!("worker_namespace_invalid_reference:{}", self.table));
        }
        Ok(())
    }

    fn restore_objects(&self, tx: &Transaction<'_>) -> Result<(), String> {
        for sql in &self.objects {
            let canonical_prefix = "CREATETRIGGERAGENT_SOURCE_TOOL_RECEIPT_IMMUTABLE";
            let sql = if normalized(sql).starts_with(canonical_prefix) {
                let start = super::super::SCHEMA
                    .find("CREATE TRIGGER IF NOT EXISTS agent_source_tool_receipt_immutable")
                    .ok_or("worker_namespace_trigger_missing")?;
                let rest = &super::super::SCHEMA[start..];
                let end = rest
                    .find("END;")
                    .ok_or("worker_namespace_trigger_invalid")?
                    + 4;
                let canonical = &rest[..end];
                let old = canonical.replace(" OR NEW.child_run_id<>OLD.child_run_id\n", "");
                if normalized(sql) != normalized(canonical) && normalized(sql) != normalized(&old) {
                    return Err("worker_namespace_unrecognized_receipt_guard".into());
                }
                canonical
            } else {
                sql
            };
            tx.execute_batch(sql).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), String> {
    let mut old = 0;
    for table in [ROUNDS, TOOLS] {
        let actual: String = tx
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name=?1 AND type='table'",
                [table],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let current = declaration(table)?;
        if normalized(&actual) == normalized(current) {
            continue;
        }
        if normalized(&actual) != normalized(&original(table, current)) {
            return Err(format!(
                "worker_namespace_unrecognized_native_schema:{table}"
            ));
        }
        old += 1;
    }
    if old == 0 {
        return Ok(());
    }
    if old != 2 {
        return Err("worker_namespace_source_pair_inconsistent".into());
    }
    let rounds = Copy::stage(tx, ROUNDS)?;
    let tools = Copy::stage(tx, TOOLS)?;
    tx.execute_batch(&format!(
        "DROP TABLE {TOOLS}; DROP TABLE {ROUNDS}; {} {}",
        declaration(ROUNDS)?,
        declaration(TOOLS)?
    ))
    .map_err(|e| e.to_string())?;
    tx.execute_batch(&format!(
        "INSERT INTO {ROUNDS}(rowid,{cols}) SELECT original_rowid,{cols} FROM {backup}",
        cols = rounds.columns,
        backup = rounds.backup
    ))
    .map_err(|e| e.to_string())?;
    tx.execute_batch(&format!("INSERT INTO {TOOLS}(rowid,{cols},child_run_id) SELECT t.original_rowid,{qualified},m.child_run_id
        FROM {backup} t JOIN {rounds} m ON m.assignment_id=t.assignment_id AND m.round_number=t.round_number",
        cols=tools.columns,qualified=tools.columns.split(',').map(|c|format!("t.{c}")).collect::<Vec<_>>().join(","),backup=tools.backup,rounds=rounds.backup)).map_err(|e|e.to_string())?;
    rounds.verify(tx)?;
    tools.verify(tx)?;
    rounds.restore_objects(tx)?;
    tools.restore_objects(tx)?;
    tx.execute_batch(&format!(
        "DROP TABLE {}; DROP TABLE {}",
        tools.backup, rounds.backup
    ))
    .map_err(|e| e.to_string())?;
    Ok(())
}
