//! No new fact reuses the original receipt. A new fact gets one next ordinal.
use super::Tick;
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::Value;
impl Tick {
    pub(crate) fn next_round(
        tx: &Transaction<'_>,
        root: &str,
        basis: &Value,
    ) -> Result<i64, String> {
        let hash = Self::basis_hash(basis);
        let saved: Option<i64> = tx
            .query_row(
                "SELECT round FROM agent_root_tick_receipts WHERE root_run_id=?1
            AND phase='request' AND basis_hash=?2",
                params![root, hash],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(round) = saved {
            return Ok(round);
        }
        let (last, count): (i64, i64) = tx
            .query_row(
                "SELECT COALESCE(MAX(round),0),COUNT(DISTINCT round)
            FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'",
                [root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if last != count {
            return Err("root_tick_round_history_conflict".into());
        }
        last.checked_add(1)
            .filter(|n| *n > 0)
            .ok_or_else(|| "root_tick_round_overflow".into())
    }
}
