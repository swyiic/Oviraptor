//! One bounded local delivery transaction, never model/target I/O authority.
use super::*;
use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    types::Value as SqlValue,
};
pub(crate) struct DeliveryGuard<'a> {
    db: &'a Connection,
    scope: CoordinatorLease,
    job: ProposalJob,
    other: Vec<Rows>,
    floor: i64,
    active: bool,
}
impl<'a> DeliveryGuard<'a> {
    pub(crate) fn install(
        db: &'a Connection,
        scope: &CoordinatorLease,
        job: &ProposalJob,
    ) -> Result<Self, String> {
        let floor = db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let guard = Self {
            db,
            scope: scope.clone(),
            job: ProposalJob {
                directive_id: job.directive_id.clone(),
                child: job.child.clone(),
                input: job.input.clone(),
                revision: job.revision,
                request_message_id: job.request_message_id.clone(),
                state: job.state.clone(),
                response: job.response.clone(),
                usage: job.usage.clone(),
            },
            other: snapshots(db, scope, job, floor)?,
            floor,
            active: true,
        };
        db.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
        Ok(guard)
    }
    pub(crate) fn verify(&self, message: &str) -> Result<(), String> {
        if snapshots(self.db, &self.scope, &self.job, self.floor)? != self.other {
            return Err("proposal_owned_delivery_collateral_write".into());
        }
        let expected = if self.job.response["valid"] == true {
            "completed"
        } else {
            "failed"
        };
        let (source, rejection): (String, String) = self
            .db
            .query_row(
                "SELECT source_draft_id,rejection_code FROM agent_user_directives WHERE id=?1",
                [&self.job.directive_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let delivered: (String, String) = self
            .db
            .query_row(
                "SELECT delivered_at,acknowledged_at FROM agent_messages WHERE id=?1",
                [message],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if delivered.0.is_empty() || delivered.1.is_empty() {
            return Err("proposal_owned_delivery_ack_missing".into());
        }
        let role = self.job.child.role.as_str();
        let mut wanted = vec![
            (
                "assignment",
                self.job.child.assignment_id.as_str(),
                json!({"role":role,"state":expected}),
            ),
            (
                "agent_run",
                self.job.child.run_id.as_str(),
                json!({"role":role,"status":"terminal","terminalState":expected}),
            ),
            (
                "mailbox_message",
                message,
                json!({"kind":"human_assessment_result","deliveredAt":"","acknowledgedAt":""}),
            ),
            (
                "mailbox_message",
                message,
                json!({"kind":"human_assessment_result","deliveredAt":delivered.0,"acknowledgedAt":delivered.1}),
            ),
            (
                "user_directive",
                self.job.directive_id.as_str(),
                json!({"proposalState":expected}),
            ),
            (
                "user_directive",
                self.job.directive_id.as_str(),
                json!({"status":expected,"sourceDraftId":source,"rejectionCode":rejection}),
            ),
        ];
        if expected == "completed" {
            wanted.push((
                "user_directive",
                self.job.directive_id.as_str(),
                json!({"status":"applied","sourceDraftId":source,"rejectionCode":rejection}),
            ));
        }
        let rows=Rows::read(self.db,"SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence",[self.floor])?;
        if rows.values.len() != wanted.len() {
            return Err("proposal_owned_delivery_events_changed".into());
        }
        for row in rows.values {
            let [SqlValue::Text(scan), SqlValue::Integer(attempt), SqlValue::Text(entity), SqlValue::Text(id), SqlValue::Text(event), SqlValue::Text(raw)] =
                row.as_slice()
            else {
                return Err("proposal_owned_delivery_events_changed".into());
            };
            let payload: Value =
                serde_json::from_str(raw).map_err(|_| "proposal_owned_delivery_events_changed")?;
            if scan != &self.scope.scan_id
                || attempt != &self.scope.attempt_number
                || event != entity
            {
                return Err("proposal_owned_delivery_events_changed".into());
            }
            let index = wanted
                .iter()
                .position(|(e, i, p)| *e == entity && *i == id && *p == payload)
                .ok_or("proposal_owned_delivery_events_changed")?;
            wanted.remove(index);
        }
        Ok(())
    }
}
impl Drop for DeliveryGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .db
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}
fn snapshots(
    db: &Connection,
    scope: &CoordinatorLease,
    job: &ProposalJob,
    floor: i64,
) -> Result<Vec<Rows>, String> {
    let worker = attempts::current(db, scope, &job.child.assignment_id)?;
    let mut rows = vec![];
    for(sql,values)in[
        ("SELECT rowid,* FROM agent_runs WHERE id<>?1 ORDER BY rowid",vec![SqlValue::Text(job.child.run_id.clone())]),
        ("SELECT rowid,* FROM agent_assignments WHERE id<>?1 ORDER BY rowid",vec![SqlValue::Text(job.child.assignment_id.clone())]),
        ("SELECT rowid,* FROM agent_assignment_attempts WHERE child_run_id<>?1 ORDER BY rowid",vec![SqlValue::Text(job.child.run_id.clone())]),
        ("SELECT rowid,* FROM agent_user_directives WHERE id<>?1 ORDER BY rowid",vec![SqlValue::Text(job.directive_id.clone())]),
        ("SELECT rowid,* FROM agent_directive_proposals WHERE directive_id<>?1 ORDER BY rowid",vec![SqlValue::Text(job.directive_id.clone())]),
        ("SELECT rowid,* FROM agent_messages WHERE NOT(assignment_id=?1 AND kind='human_assessment_result' AND correlation_id=?2) ORDER BY rowid",vec![SqlValue::Text(job.child.assignment_id.clone()),SqlValue::Text(job.directive_id.clone())]),
        ("SELECT rowid,* FROM agent_capability_leases WHERE child_run_id<>?1 ORDER BY rowid",vec![SqlValue::Text(job.child.run_id.clone())]),
        ("SELECT rowid,* FROM agent_lane_leases WHERE assignment_id<>?1 ORDER BY rowid",vec![SqlValue::Text(job.child.assignment_id.clone())]),
        ("SELECT rowid,* FROM agent_budget_ledger WHERE root_run_id<>?1 ORDER BY rowid",vec![SqlValue::Text(scope.root_run_id.clone())]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE NOT(root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3) ORDER BY rowid",vec![SqlValue::Text(scope.root_run_id.clone()),SqlValue::Text(job.child.assignment_id.clone()),SqlValue::Text(worker.id)]),
        ("SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",vec![SqlValue::Integer(floor)]),
    ]{rows.push(Rows::read(db,sql,rusqlite::params_from_iter(values))?);}
    Ok(rows)
}
fn authorize(context: AuthContext<'_>) -> Authorization {
    let direct = context.database_name == Some("main") && context.accessor.is_none();
    let allowed = match context.action {
        AuthAction::Insert { table_name } if direct => {
            matches!(table_name, "agent_budget_entries" | "agent_messages")
        }
        AuthAction::Delete {
            table_name: "agent_lane_leases",
        } if direct => true,
        AuthAction::Update {
            table_name,
            column_name,
        } if direct => match table_name {
            "agent_budget_ledger" => matches!(
                column_name,
                "reserved_tokens"
                    | "reserved_requests"
                    | "spent_tokens"
                    | "spent_requests"
                    | "updated_at"
            ),
            "agent_runs" => matches!(
                column_name,
                "used_tokens"
                    | "used_cached_tokens"
                    | "used_requests"
                    | "status"
                    | "terminal_state"
                    | "terminal_code"
                    | "terminal_reason"
                    | "finished_at"
                    | "updated_at"
            ),
            "agent_assignments" => matches!(
                column_name,
                "reserved_tokens"
                    | "reserved_requests"
                    | "budget_settled_at"
                    | "state"
                    | "failure_class"
                    | "finished_at"
                    | "updated_at"
            ),
            "agent_assignment_attempts" => {
                matches!(column_name, "state" | "finished_at" | "failure_class")
            }
            "agent_capability_leases" => column_name == "revoked_at",
            "agent_messages" => matches!(
                column_name,
                "delivered_at" | "delivery_attempts" | "acknowledged_at"
            ),
            "agent_directive_proposals" => {
                matches!(column_name, "state" | "result_message_id" | "updated_at")
            }
            "agent_user_directives" => matches!(
                column_name,
                "status" | "applied_at" | "finished_at" | "updated_at"
            ),
            _ => false,
        },
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if context.database_name == Some("main") => matches!(
            context.accessor,
            Some(
                "agent_collaboration_run_update"
                    | "agent_collaboration_assignment_update"
                    | "agent_collaboration_message_insert"
                    | "agent_collaboration_message_update"
                    | "agent_collaboration_directive_update"
                    | "agent_collaboration_directive_proposal"
            )
        ),
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => true,
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
