//! A living original parent renews only its control lease, within its Root clock.
use super::{
    attempts::audit_rows::Rows,
    budget::clock,
    lease::{self, CoordinatorLease},
    supervision_ticket::SupervisionTicket,
};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    params,
    types::Value,
    Connection, TransactionBehavior,
};

pub(super) fn renew_if_due(
    db: &mut Connection,
    actor: &CoordinatorLease,
    parent: &SupervisionTicket,
) -> Result<(), String> {
    parent.check()?;
    lease::validate_coordinator_lease(db, actor)?;
    let current: String = db.query_row("SELECT lease_expires_at FROM agent_coordinator_leases WHERE scan_id=?1
        AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4 AND lease_epoch=?5 AND fencing_token=?6",
        params![actor.scan_id,actor.attempt_number,actor.target_key,actor.root_run_id,actor.lease_epoch,actor.fencing_token], |r|r.get(0))
        .map_err(|e|e.to_string())?;
    let due: bool = db
        .query_row(
            "SELECT datetime(?1)<=datetime('now','+120 seconds','localtime')",
            [&current],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !due || clock::renewal_expiry(db, &actor.root_run_id)? <= current {
        return Ok(());
    }
    let tx = match db.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            return Ok(())
        }
        Err(error) => return Err(format!("coordinator_heartbeat_lock:{error}")),
    };
    // The private supervisor connection has no other authorizer. Deny all
    // unrelated writes, including effects in SQLite triggers and other Roots.
    tx.authorizer(Some(renewal_authorizer))
        .map_err(|e| e.to_string())?;
    let result = renew_on(&tx, actor, parent);
    let clear = tx
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    result?;
    clear?;
    tx.commit()
        .map_err(|e| format!("coordinator_heartbeat_commit:{e}"))
}

fn renewal_authorizer(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Update {
            table_name: "agent_coordinator_leases",
            column_name,
        } if context.database_name == Some("main")
            && matches!(
                column_name,
                "lease_expires_at" | "heartbeat_at" | "updated_at"
            ) =>
        {
            Authorization::Allow
        }
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }
}

fn renew_on(
    db: &Connection,
    actor: &CoordinatorLease,
    parent: &SupervisionTicket,
) -> Result<(), String> {
    parent.check()?;
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    let expiry = clock::renewal_expiry(db, &actor.root_run_id)?;
    let now: String = db
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let sql = "SELECT rowid,* FROM agent_coordinator_leases ORDER BY rowid";
    let mut expected = Rows::read(db, sql, [])?;
    let keys = [
        "scan_id",
        "attempt_number",
        "target_key",
        "root_run_id",
        "lease_epoch",
        "fencing_token",
    ]
    .map(|key| expected.column(key))
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let values = [
        Value::Text(actor.scan_id.clone()),
        Value::Integer(actor.attempt_number),
        Value::Text(actor.target_key.clone()),
        Value::Text(actor.root_run_id.clone()),
        Value::Integer(actor.lease_epoch),
        Value::Text(actor.fencing_token.clone()),
    ];
    let columns = ["lease_expires_at", "heartbeat_at", "updated_at"]
        .map(|name| expected.column(name))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let matches = expected
        .values
        .iter_mut()
        .filter(|row| {
            keys.iter()
                .zip(&values)
                .all(|(index, value)| row[*index] == *value)
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err("coordinator_heartbeat_scope_conflict".into());
    }
    let row = matches
        .into_iter()
        .next()
        .ok_or("coordinator_heartbeat_scope_conflict")?;
    match &row[columns[0]] {
        Value::Text(current) if current >= &expiry => return Ok(()),
        Value::Text(_) => {}
        _ => return Err("coordinator_heartbeat_deadline_invalid".into()),
    }
    for (index, value) in columns.iter().zip([&expiry, &now, &now]) {
        row[*index] = Value::Text(value.clone());
    }
    let changed = db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=?7,heartbeat_at=?8,updated_at=?8
        WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4 AND lease_epoch=?5 AND fencing_token=?6
          AND lease_expires_at>datetime('now','localtime') AND lease_expires_at<?7",
        params![actor.scan_id,actor.attempt_number,actor.target_key,actor.root_run_id,actor.lease_epoch,actor.fencing_token,expiry,now])
        .map_err(|e|format!("coordinator_heartbeat_write:{e}"))?;
    if changed != 1 || Rows::read(db, sql, [])? != expected {
        return Err("coordinator_heartbeat_postcondition".into());
    }
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    clock::supervision_remaining(db, &actor.root_run_id)?;
    parent.check()
}
