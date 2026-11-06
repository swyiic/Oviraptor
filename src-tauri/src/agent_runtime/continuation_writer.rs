//! Which continuation holder may append budget rows for a scan attempt.
//! The begin thread binds this around its own hold. A later worker must
//! present the same holder or the append is refused.
use rusqlite::{params, OptionalExtension};

std::thread_local! {
    static CONTINUATION_WRITE_HOLDER: std::cell::RefCell<Option<(String, i64, String)>> = std::cell::RefCell::new(None);
}

pub(crate) fn bind(scan_id: &str, attempt: i64, holder: &str) {
    CONTINUATION_WRITE_HOLDER.with(|slot| *slot.borrow_mut() = Some((scan_id.to_string(), attempt, holder.to_string())));
}

pub(crate) fn clear() {
    CONTINUATION_WRITE_HOLDER.with(|slot| *slot.borrow_mut() = None);
}

pub(crate) fn bound() -> Option<(String, i64, String)> {
    CONTINUATION_WRITE_HOLDER.with(|slot| slot.borrow().clone())
}

/// A claiming continuation holder owns new budget rows for that root.
/// No holder row means the existing worker path is unchanged.
pub(crate) fn guard_budget_append(db: &rusqlite::Connection, root: &str) -> Result<(), String> {
    let mapped = db.query_row(
        "SELECT scan_id, attempt_number FROM agent_runs WHERE id=?1 AND backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id=''",
        [root],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    ).optional().unwrap_or(None);
    let Some((scan_id, attempt)) = mapped else { return Ok(()); };
    let holder = db.query_row(
        "SELECT holder_id FROM scan_continuation_holders WHERE scan_id=?1 AND attempt_number=?2 AND state='claiming'",
        params![scan_id, attempt],
        |row| row.get::<_, String>(0),
    ).optional().unwrap_or(None);
    let Some(holder_id) = holder else { return Ok(()); };
    match bound() {
        Some((id, bound_attempt, bound_holder)) if id == scan_id && bound_attempt == attempt && bound_holder == holder_id => Ok(()),
        _ => Err("continuation_holder_stale".into()),
    }
}
