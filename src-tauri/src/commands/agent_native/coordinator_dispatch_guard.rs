// A single setter covers the whole issuance transaction. No nested owner.
fn native_coordinator_dispatch_protect<T>(
    tx: &rusqlite::Transaction<'_>,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    use rusqlite::hooks::{AuthContext, Authorization};
    tx.authorizer(Some(native_coordinator_dispatch_authorize))
        .map_err(|e| e.to_string())?;
    let result: Result<T, String> = (|| {
        crate::collaboration_events::dispatch_schema::verify(tx)?;
        let value = work()?;
        crate::collaboration_events::dispatch_schema::verify(tx)?;
        Ok(value)
    })();
    let clear = tx
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let value = result?;
    clear?;
    Ok(value)
}

fn native_coordinator_dispatch_authorize(
    c: rusqlite::hooks::AuthContext<'_>,
) -> rusqlite::hooks::Authorization {
    use rusqlite::hooks::{AuthAction, Authorization};
    let direct = c.database_name == Some("main") && c.accessor.is_none();
    let allow = match c.action {
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if c.database_name == Some("main") => c.accessor.is_some_and(|name| {
            crate::collaboration_events::dispatch_schema::EMITTERS.contains(&name)
        }),
        AuthAction::Insert { table_name } if direct => matches!(
            table_name,
            "agent_assignments"
                | "agent_runs"
                | "agent_assignment_attempts"
                | "agent_contract_owners"
                | "agent_lane_leases"
                | "agent_capability_leases"
                | "agent_budget_entries"
                | "agent_budget_limits"
                | "agent_budget_clock_origins"
                | "agent_budget_ledger"
        ),
        AuthAction::Update {
            table_name,
            column_name,
        } if direct => match table_name {
            "agent_runs" => matches!(
                column_name,
                "status" | "plan_hash" | "evidence_hash" | "updated_at"
            ),
            "agent_assignments" => matches!(
                column_name,
                "lease_epoch"
                    | "fencing_token"
                    | "lease_expires_at"
                    | "state"
                    | "leased_at"
                    | "updated_at"
            ),
            "agent_contract_owners" => matches!(
                column_name,
                "assignment_id" | "lease_epoch" | "fencing_token" | "state"
            ),
            "agent_budget_ledger" => matches!(
                column_name,
                "lease_epoch"
                    | "fencing_token"
                    | "reserved_tokens"
                    | "reserved_requests"
                    | "updated_at"
            ),
            _ => false,
        },
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => true,
        _ => false,
    };
    if allow {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
