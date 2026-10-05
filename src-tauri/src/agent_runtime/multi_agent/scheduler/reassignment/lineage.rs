//! Immutable provenance authorizes read-only lookup of an explicit replacement.
use super::*;
use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;

pub(super) fn original_hash(db: &Connection, id: &str) -> Result<String, String> {
    let mut values = Vec::new();
    for sql in [
        "SELECT * FROM agent_assignment_attempts WHERE id=?1 ORDER BY rowid",
        "SELECT * FROM agent_runs WHERE id=(SELECT child_run_id FROM agent_assignment_attempts WHERE id=?1) ORDER BY rowid",
        "SELECT * FROM agent_capability_leases WHERE child_run_id=(SELECT child_run_id FROM agent_assignment_attempts WHERE id=?1) ORDER BY rowid",
        "SELECT * FROM agent_budget_entries WHERE lease_attempt_id=?1 ORDER BY rowid",
    ] {values.push(Rows::read(db,sql,[id])?.values);}
    Ok(store::stable_hash(&format!("{values:?}")))
}

pub(super) fn verify(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &AgentAssignment,
) -> Result<(), String> {
    let current = super::super::attempts::current(db, lease, &assignment.id)?;
    let mut statement=db.prepare("SELECT x.id,x.child_run_id,x.lease_epoch,x.state,p.original_attempt_id,p.original_hash,p.assignment_hash
        FROM agent_assignment_attempts x LEFT JOIN agent_assignment_replacements p ON p.replacement_attempt_id=x.id
        AND p.root_run_id=x.root_run_id AND p.assignment_id=x.assignment_id AND p.coordinator_epoch=x.coordinator_epoch
        AND p.coordinator_fencing_token=x.coordinator_fencing_token
        WHERE x.root_run_id=?1 AND x.assignment_id=?2 AND x.coordinator_epoch=?3 AND x.coordinator_fencing_token=?4 ORDER BY x.lease_epoch")
        .map_err(|e|e.to_string())?;
    let rows = statement
        .query_map(
            params![
                lease.root_run_id,
                assignment.id,
                lease.lease_epoch,
                lease.fencing_token
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let hash = store::stable_hash(&assignment.content_key().to_string());
    if rows.len() as i64 != current.lease_epoch {
        return Err("assignment_replacement_lineage_incomplete".into());
    }
    for (index, row) in rows.iter().enumerate() {
        if row.2 != index as i64 + 1
            || (index == 0 && (row.1 != format!("run-{}", assignment.id) || row.4.is_some()))
        {
            return Err("assignment_replacement_lineage_conflict".into());
        }
        if index > 0 {
            let previous = &rows[index - 1];
            let original = ScheduledChild {
                assignment_id: assignment.id.clone(),
                run_id: previous.1.clone(),
                role: assignment.role,
            };
            if previous.3 != "expired"
                || row.4.as_deref() != Some(previous.0.as_str())
                || row.6.as_deref() != Some(hash.as_str())
                || row.5.as_deref() != Some(original_hash(db, &previous.0)?.as_str())
            {
                return Err("assignment_replacement_lineage_conflict".into());
            }
            super::undispatched::verify(db, lease, &original, true)?;
        }
    }
    if rows
        .last()
        .is_none_or(|r| r.0 != current.id || r.1 != assignment.child_run_id)
    {
        return Err("assignment_replacement_lineage_conflict".into());
    }
    Ok(())
}

pub(super) fn replay(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    original: &ScheduledChild,
    assignment: &AgentAssignment,
) -> Result<ScheduledChild, String> {
    verify(tx, lease, assignment)?;
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignment_replacements p
        JOIN agent_assignment_attempts old ON old.id=p.original_attempt_id JOIN agent_assignment_attempts new ON new.id=p.replacement_attempt_id
        WHERE p.root_run_id=?1 AND p.assignment_id=?2 AND old.child_run_id=?3 AND new.child_run_id=?4)",
        params![lease.root_run_id,assignment.id,original.run_id,assignment.child_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact {
        return Err("assignment_replacement_replay_not_current".into());
    }
    verify_scheduled_authority(tx, lease, assignment)
}
