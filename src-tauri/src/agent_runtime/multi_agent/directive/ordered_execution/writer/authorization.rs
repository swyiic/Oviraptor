use super::Mode;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
pub(super) fn authorize(ctx: AuthContext<'_>, mode: Mode) -> Authorization {
    let direct = ctx.database_name == Some("main") && ctx.accessor.is_none();
    let allowed = match ctx.action {
        AuthAction::Insert { table_name } if direct => match mode {
            Mode::Schedule => matches!(
                table_name,
                "agent_assignments"
                    | "agent_runs"
                    | "agent_assignment_attempts"
                    | "agent_contract_owners"
                    | "agent_lane_leases"
                    | "agent_capability_leases"
                    | "agent_budget_ledger"
                    | "agent_budget_entries"
                    | "agent_budget_limits"
                    | "agent_budget_clock_origins"
                    | "agent_messages"
                    | "agent_directive_ordered_actions"
            ),
            Mode::Defer => false,
            Mode::Finish => matches!(
                table_name,
                "agent_budget_entries" | "agent_messages" | "agent_directive_ordered_receipts"
            ),
            _ => false,
        },
        AuthAction::Delete {
            table_name: "agent_lane_leases",
        } if direct && matches!(mode, Mode::Finish) => true,
        AuthAction::Update {
            table_name,
            column_name,
        } if direct => match (table_name, mode) {
            ("agent_runs", Mode::Schedule) => matches!(
                column_name,
                "status" | "plan_hash" | "evidence_hash" | "updated_at"
            ),
            ("agent_runs", Mode::Start) => {
                matches!(column_name, "status" | "started_at" | "updated_at")
            }
            ("agent_runs", Mode::Finish) => matches!(
                column_name,
                "status"
                    | "used_tokens"
                    | "used_cached_tokens"
                    | "used_requests"
                    | "terminal_state"
                    | "terminal_code"
                    | "terminal_reason"
                    | "finished_at"
                    | "updated_at"
            ),
            ("agent_assignments", Mode::Schedule) => matches!(
                column_name,
                "lease_epoch"
                    | "fencing_token"
                    | "lease_expires_at"
                    | "state"
                    | "leased_at"
                    | "updated_at"
            ),
            ("agent_assignments", Mode::Start) => {
                matches!(column_name, "state" | "started_at" | "updated_at")
            }
            ("agent_assignments", Mode::Finish) => matches!(
                column_name,
                "state"
                    | "failure_class"
                    | "reserved_tokens"
                    | "reserved_requests"
                    | "budget_settled_at"
                    | "finished_at"
                    | "updated_at"
            ),
            ("agent_assignment_attempts", Mode::Start) => {
                matches!(column_name, "state" | "heartbeat_at")
            }
            ("agent_assignment_attempts", Mode::Finish) => {
                matches!(column_name, "state" | "failure_class" | "finished_at")
            }
            ("agent_contract_owners", Mode::Schedule) => matches!(
                column_name,
                "assignment_id" | "lease_epoch" | "fencing_token" | "state"
            ),
            ("agent_budget_ledger", Mode::Schedule) => matches!(
                column_name,
                "lease_epoch"
                    | "fencing_token"
                    | "reserved_tokens"
                    | "reserved_requests"
                    | "updated_at"
            ),
            ("agent_budget_ledger", Mode::Finish) => matches!(
                column_name,
                "reserved_tokens"
                    | "reserved_requests"
                    | "spent_tokens"
                    | "spent_requests"
                    | "updated_at"
            ),
            ("agent_capability_leases", Mode::Finish) => column_name == "revoked_at",
            ("agent_messages", Mode::Consume | Mode::Finish) => matches!(
                column_name,
                "delivered_at" | "delivery_attempts" | "acknowledged_at"
            ),
            ("agent_directive_ordered_actions", Mode::Start) => column_name == "state",
            ("agent_directive_ordered_actions", Mode::Receive) => {
                matches!(column_name, "state" | "response_json" | "usage_json")
            }
            ("agent_directive_ordered_actions", Mode::Finish) => {
                matches!(column_name, "state" | "result_message_id")
            }
            ("agent_user_directives", Mode::Defer) => matches!(column_name,
                "status" | "rejection_code" | "finished_at" | "updated_at"),
            ("agent_user_directives", Mode::Schedule) => {
                matches!(column_name, "status" | "assigned_at" | "updated_at")
            }
            ("agent_user_directives", Mode::Finish) => matches!(
                column_name,
                "status" | "applied_at" | "finished_at" | "updated_at"
            ),
            _ => false,
        },
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if ctx.database_name == Some("main") => match mode {
            Mode::Schedule => matches!(
                ctx.accessor,
                Some(
                    "agent_collaboration_assignment_insert"
                        | "agent_collaboration_assignment_update"
                        | "agent_collaboration_run_insert"
                        | "agent_collaboration_run_update"
                        | "agent_collaboration_message_insert"
                        | "agent_collaboration_directive_update"
                        | "ordered_action_collaboration_insert"
                )
            ),
            Mode::Start => matches!(
                ctx.accessor,
                Some(
                    "agent_collaboration_assignment_update"
                        | "agent_collaboration_run_update"
                        | "ordered_action_collaboration_update"
                )
            ),
            Mode::Defer => ctx.accessor == Some("agent_collaboration_directive_update"),
            Mode::Consume => ctx.accessor == Some("agent_collaboration_message_update"),
            Mode::Receive => ctx.accessor == Some("ordered_action_collaboration_update"),
            Mode::Finish => matches!(
                ctx.accessor,
                Some(
                    "agent_collaboration_assignment_update"
                        | "agent_collaboration_run_update"
                        | "agent_collaboration_message_insert"
                        | "agent_collaboration_message_update"
                        | "agent_collaboration_directive_update"
                        | "ordered_action_collaboration_update"
                        | "ordered_receipt_collaboration_insert"
                )
            ),
        },
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
