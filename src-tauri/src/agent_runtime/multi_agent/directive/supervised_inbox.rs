// One original-parent transaction for claim and rehydration. No lease acquisition.
use super::{delivery, inbox_claim, CoordinatorLease, UserDirective};
use rusqlite::{Connection, Transaction, TransactionBehavior};

pub(crate) fn collect_for_original_parent(
    db: &Connection,
    actor: &CoordinatorLease,
    check_original_parent: &dyn Fn(&Connection) -> Result<(), String>,
) -> Result<Vec<UserDirective>, String> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| format!("directive_original_parent_lock:{e}"))?;
    check_original_parent(&tx)?;
    inbox_claim::claim_pending_in_transaction(&tx, actor, 50)?;
    let items = delivery::prepare_model_context_in_transaction(&tx, actor)?;
    // Recheck cancellation, original C/worker grants and the weak parent after
    // all trigger-backed writes; a revoked collection rolls back both phases.
    check_original_parent(&tx)?;
    tx.commit()
        .map_err(|e| format!("directive_original_parent_commit:{e}"))?;
    Ok(items)
}
// A capacity refusal can defer human work, never spend or widen the original task.
pub(crate) fn defer_for_original_parent(
    db: &Connection,
    actor: &CoordinatorLease,
    check_original_parent: &dyn Fn(&Connection) -> Result<(), String>,
) -> Result<usize, String> {
    use rusqlite::{
        hooks::{AuthAction, AuthContext, Authorization},
        params,
    };
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    check_original_parent(&tx)?;
    // A paid overrun or unknown invoice cannot be treated as a pre-send capacity refusal.
    crate::agent_runtime::multi_agent::budget::admission::require_determinate(
        &tx,
        &actor.root_run_id,
    )?;
    crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
        &tx,
        &actor.root_run_id,
    )?;
    crate::collaboration_events::closure_schema::verify(&tx)?;
    let floor: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    tx.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Update {
            table_name,
            column_name,
        } if c.database_name == Some("main")
            && c.accessor.is_none()
            && table_name == "agent_user_directives"
            && matches!(column_name, "status" | "rejection_code" | "updated_at") =>
        {
            Authorization::Allow
        }
        AuthAction::Insert { table_name }
            if c.database_name == Some("main")
                && c.accessor == Some("agent_collaboration_directive_update")
                && table_name == "agent_collaboration_events" =>
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
    }))
    .map_err(|e| e.to_string())?;
    let result: Result<usize, String> = (|| {
        let ids = {
            let mut q=tx.prepare("SELECT id FROM agent_user_directives WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4 AND claim_run_id=?4 AND claim_lease_epoch=?5 AND claim_fencing_token=?6 AND status='accepted' ORDER BY rowid").map_err(|e|e.to_string())?;
            let rows = q
                .query_map(
                    params![
                        actor.scan_id,
                        actor.attempt_number,
                        actor.target_key,
                        actor.root_run_id,
                        actor.lease_epoch,
                        actor.fencing_token
                    ],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };
        let mut expected = Vec::new();
        for id in &ids {
            let fact = super::confirmed_fact_for_root(&tx, actor, id)?;
            let changed = tx.execute("UPDATE agent_user_directives SET status='deferred',rejection_code='root_human_assessment_budget_unavailable',updated_at=datetime('now','localtime') WHERE id=?1 AND status='accepted' AND scan_id=?2 AND attempt_number=?3 AND target_key=?4 AND root_run_id=?5 AND claim_run_id=?5 AND claim_lease_epoch=?6 AND claim_fencing_token=?7",params![id,actor.scan_id,actor.attempt_number,actor.target_key,actor.root_run_id,actor.lease_epoch,actor.fencing_token]).map_err(|e|e.to_string())?;
            if changed != 1 {
                return Err("root_human_deferral_not_persisted".into());
            }
            if super::confirmed_fact_for_root(&tx, actor, id)? != fact {
                return Err("root_human_original_confirmation_changed".into());
            }
            expected.push((actor.scan_id.clone(), actor.attempt_number, "user_directive".to_owned(),id.clone(),"user_directive".to_owned(),serde_json::json!({"status":"deferred","sourceDraftId":fact["draftId"],"rejectionCode":"root_human_assessment_budget_unavailable"})));
        }
        let mut q = tx.prepare("SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence").map_err(|e|e.to_string())?;
        let emitted = q
            .query_map([floor], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let actual = emitted
            .into_iter()
            .map(|(scan, attempt, entity, id, event, payload)| {
                serde_json::from_str::<serde_json::Value>(&payload)
                    .map(|payload| (scan, attempt, entity, id, event, payload))
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if actual != expected {
            return Err("root_human_deferral_events_changed".into());
        }
        check_original_parent(&tx)?;
        Ok(ids.len())
    })();
    tx.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string())?;
    let count = result?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(count)
}
