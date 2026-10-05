use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordinatorLease {
    pub scan_id: String,
    pub attempt_number: i64,
    pub target_key: String,
    pub root_run_id: String,
    pub lease_epoch: i64,
    pub fencing_token: String,
    pub lease_expires_at: String,
}

/// A fencing token can outlive a paused, replaced, or soft-deleted attempt.
/// Callers that create or advance executable work must check this under their
/// own write transaction; cleanup and historical reads deliberately do not.
pub fn require_active_attempt(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<(), String> {
    let active: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2 AND status='scanning') \
             AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            params![scan_id, attempt_number],
            |row| row.get(0),
        )
        .map_err(|error| format!("agent_attempt_status_unavailable:{error}"))?;
    if active {
        Ok(())
    } else {
        Err("agent_attempt_not_active".into())
    }
}

/// A scan may still be active while this particular target's Coordinator has
/// finished. Its valid lease is retained for bookkeeping, never for new work.
pub fn require_open_coordinator(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_key: &str,
    root_run_id: &str,
) -> Result<(), String> {
    let open: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND role='coordinator' \
         AND (root_run_id=id OR (root_run_id='' AND orchestration_policy='single' AND assignment_id='' AND parent_run_id IS NULL)) \
         AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND status IN ('prepared','running') AND cancel_requested_at='')",
        params![root_run_id,scan_id,attempt_number,target_key], |row|row.get(0),
    ).map_err(|error|format!("coordinator_status_unavailable:{error}"))?;
    if open {
        Ok(())
    } else {
        Err("coordinator_not_executable".into())
    }
}

pub fn require_executable_coordinator(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<(), String> {
    require_active_attempt(connection, &lease.scan_id, lease.attempt_number)?;
    require_open_coordinator(
        connection,
        &lease.scan_id,
        lease.attempt_number,
        &lease.target_key,
        &lease.root_run_id,
    )
}

pub fn acquire_coordinator_lease(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_key: &str,
    root_run_id: &str,
    ttl_seconds: i64,
) -> Result<CoordinatorLease, String> {
    if scan_id.trim().is_empty() || root_run_id.trim().is_empty() || attempt_number < 1 {
        return Err("coordinator_lease_scope_invalid".into());
    }
    let ttl = ttl_seconds.clamp(15, 600);
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定 Coordinator lease：{error}"))?;
    let current: Option<(String, i64, String, bool)> = transaction
        .query_row(
            "SELECT root_run_id,lease_epoch,fencing_token,lease_expires_at>datetime('now','localtime') \
             FROM agent_coordinator_leases WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3",
            params![scan_id, attempt_number, target_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| format!("无法读取 Coordinator lease：{error}"))?;
    let (epoch, token) = match current {
        Some((owner, epoch, token, true)) if owner == root_run_id => (epoch, token),
        Some((owner, _, _, true)) => {
            return Err(format!("coordinator_lease_held:{owner}"));
        }
        Some((_, epoch, _, false)) => (epoch.saturating_add(1), uuid::Uuid::new_v4().to_string()),
        None => (1, uuid::Uuid::new_v4().to_string()),
    };
    let modifier = format!("+{ttl} seconds");
    transaction
        .execute(
            "INSERT INTO agent_coordinator_leases(scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at,heartbeat_at) \
             VALUES(?1,?2,?3,?4,?5,?6,datetime('now',?7,'localtime'),datetime('now','localtime')) \
             ON CONFLICT(scan_id,attempt_number,target_key) DO UPDATE SET root_run_id=excluded.root_run_id,lease_epoch=excluded.lease_epoch,\
             fencing_token=excluded.fencing_token,lease_expires_at=excluded.lease_expires_at,heartbeat_at=datetime('now','localtime'),updated_at=datetime('now','localtime')",
            params![scan_id, attempt_number, target_key, root_run_id, epoch, token, modifier],
        )
        .map_err(|error| format!("无法写入 Coordinator lease：{error}"))?;
    let lease = transaction
        .query_row(
            "SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at \
             FROM agent_coordinator_leases WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3",
            params![scan_id, attempt_number, target_key],
            |row| {
                Ok(CoordinatorLease {
                    scan_id: row.get(0)?,
                    attempt_number: row.get(1)?,
                    target_key: row.get(2)?,
                    root_run_id: row.get(3)?,
                    lease_epoch: row.get(4)?,
                    fencing_token: row.get(5)?,
                    lease_expires_at: row.get(6)?,
                })
            },
        )
        .map_err(|error| format!("无法确认 Coordinator lease：{error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交 Coordinator lease：{error}"))?;
    Ok(lease)
}

pub fn validate_coordinator_lease(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<(), String> {
    let valid: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases \
             WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4 \
             AND lease_epoch=?5 AND fencing_token=?6 AND lease_expires_at>datetime('now','localtime'))",
            params![
                lease.scan_id,
                lease.attempt_number,
                lease.target_key,
                lease.root_run_id,
                lease.lease_epoch,
                lease.fencing_token
            ],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法验证 Coordinator lease：{error}"))?;
    if valid {
        Ok(())
    } else {
        Err("stale_coordinator_fencing_token".into())
    }
}
