//! §5.3 Loop5: one owner per contract across all child runs of a root.
//!
//! A contract key binds attempt + canonical target + method/action +
//! normalized parameter shape + identity pair + business object + purpose +
//! side-effect class. The first assignment to acquire a key owns it; a
//! different assignment must never steal it, and stale fencing fails closed.
//! Acquire is wired into the scheduler (Loop6); release wiring is deferred.

//! The scheduler acquire path is production-wired (Loop6 schedule path); release
//! has no production caller yet and keeps a staging-only allow until terminal
//! reclamation calls it.

use rusqlite::{params, Connection};

/// Join normalized key parts with a unit separator so `["a:b", "c"]` and
/// `["a", "b:c"]` never collide.
pub fn contract_key(parts: &[&str]) -> String {
    parts.join("\u{1f}")
}

/// The §5.3 v1 key the scheduler binds per child: attempt, canonical target,
/// role, trigger and evidence revision. It lives beside the primitive (not in
/// the scheduler) so tests and future key versions share one definition.
pub fn schedule_contract_key(
    attempt_number: i64,
    target_key: &str,
    role: &str,
    trigger_code: &str,
    evidence_revision: i64,
) -> String {
    contract_key(&[
        &attempt_number.to_string(),
        target_key,
        role,
        trigger_code,
        &evidence_revision.to_string(),
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractAcquisition {
    pub owner_assignment_id: String,
    pub created: bool,
}

fn current_owner(
    connection: &Connection,
    root_run_id: &str,
    contract_key: &str,
) -> Result<Option<(String, i64, String, String)>, String> {
    connection
        .query_row(
            "SELECT assignment_id,lease_epoch,fencing_token,state \
             FROM agent_contract_owners WHERE root_run_id=?1 AND contract_key=?2",
            params![root_run_id, contract_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("无法读取合同 owner：{other}")),
        })
}

/// Read-only verification of the original held owner and fencing.
/// Missing or released authority is never reacquired by this check.
pub(crate) fn verify_held_owner(
    connection: &Connection,
    root: &str,
    key: &str,
    assignment: &str,
    epoch: i64,
    fence: &str,
) -> Result<(), String> {
    let Some((owner, current_epoch, current_fence, state)) = current_owner(connection, root, key)?
    else {
        return Err("contract_owner_missing_after_acquire".into());
    };
    if state != "held" {
        return Err("contract_not_held".into());
    }
    if owner != assignment {
        return Err("contract_owned_by_other".into());
    }
    if current_epoch != epoch || current_fence != fence {
        return Err("stale_contract_fencing".into());
    }
    Ok(())
}

/// Explicit acquisition may rotate a held contract. Read-only replay uses the
/// verifier above so an existing child never gains a new token implicitly.
pub fn acquire_contract_owner(
    connection: &Connection,
    root_run_id: &str,
    key: &str,
    assignment_id: &str,
    lease_epoch: i64,
    fencing_token: &str,
) -> Result<ContractAcquisition, String> {
    if key.trim().is_empty() || assignment_id.trim().is_empty() {
        return Err("contract_key_and_assignment_required".into());
    }
    let inserted = connection
        .execute(
            "INSERT INTO agent_contract_owners(root_run_id,contract_key,assignment_id,lease_epoch,fencing_token) \
             VALUES(?1,?2,?3,?4,?5) ON CONFLICT(root_run_id,contract_key) DO NOTHING",
            params![root_run_id, key, assignment_id, lease_epoch, fencing_token],
        )
        .map_err(|error| format!("无法获取合同 owner：{error}"))?;
    let Some((owner, epoch, fence, state)) = current_owner(connection, root_run_id, key)? else {
        return Err("contract_owner_missing_after_acquire".into());
    };
    if state == "released" {
        // A released contract may be acquired anew; the row keeps the last
        // release timestamp for audit, ownership moves to the new assignment.
        connection
            .execute(
                "UPDATE agent_contract_owners SET assignment_id=?1,lease_epoch=?2,fencing_token=?3,state='held' \
                 WHERE root_run_id=?4 AND contract_key=?5 AND state='released'",
                params![assignment_id, lease_epoch, fencing_token, root_run_id, key],
            )
            .map_err(|error| format!("无法重新获取已释放合同：{error}"))?;
        return Ok(ContractAcquisition {
            owner_assignment_id: assignment_id.to_string(),
            created: true,
        });
    }
    if state != "held" {
        return Err("contract_not_held".into());
    }
    if owner != assignment_id {
        return Err("contract_owned_by_other".into());
    }
    // Same owner, newer fencing rotates the epoch forward; older fencing fails.
    if epoch > lease_epoch {
        return Err("stale_contract_fencing".into());
    }
    if epoch < lease_epoch || fence != fencing_token {
        connection
            .execute(
                "UPDATE agent_contract_owners SET lease_epoch=?1,fencing_token=?2 \
                 WHERE root_run_id=?3 AND contract_key=?4 AND assignment_id=?5 AND state='held'",
                params![lease_epoch, fencing_token, root_run_id, key, assignment_id],
            )
            .map_err(|error| format!("无法轮换合同 fencing：{error}"))?;
    }
    Ok(ContractAcquisition {
        owner_assignment_id: owner,
        created: inserted == 1,
    })
}

/// Release a held contract. Only the owning assignment with matching fencing
/// may release; the row is kept for audit, never deleted.
#[allow(dead_code)] // Loop6: no production release caller yet; covered by tests.
pub fn release_contract_owner(
    connection: &Connection,
    root_run_id: &str,
    key: &str,
    assignment_id: &str,
    lease_epoch: i64,
    fencing_token: &str,
) -> Result<(), String> {
    let Some((owner, epoch, fence, state)) = current_owner(connection, root_run_id, key)? else {
        return Err("contract_not_found".into());
    };
    if state != "held" {
        return Err("contract_not_held".into());
    }
    if owner != assignment_id {
        return Err("contract_owned_by_other".into());
    }
    if epoch != lease_epoch || fence != fencing_token {
        return Err("stale_contract_fencing".into());
    }
    connection
        .execute(
            "UPDATE agent_contract_owners SET state='released',released_at=datetime('now','localtime') \
             WHERE root_run_id=?1 AND contract_key=?2 AND assignment_id=?3 AND state='held'",
            params![root_run_id, key, assignment_id],
        )
        .map_err(|error| format!("无法释放合同 owner：{error}"))?;
    Ok(())
}
