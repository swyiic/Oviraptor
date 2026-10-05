// Single fresh work is separate from an already claimed model's in-flight check.
fn native_single_policy(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<bool, String> {
    let Some(run) = &context.run else {
        #[cfg(test)]
        {
            return Ok(false);
        }
        #[cfg(not(test))]
        {
            return Err("tool_run_not_found".into());
        }
    };
    if run.db_path != context.db_path {
        return Err("tool_authorization_unavailable".into());
    }
    let policy: String = db
        .query_row(
            "SELECT orchestration_policy FROM agent_runs WHERE id=?1 AND scan_id=?2
         AND attempt_number=?3 AND target_url=?4 AND backend='native'",
            params![
                run.run_id,
                context.scan_id,
                context.attempt_number,
                context.target_url
            ],
            |r| r.get(0),
        )
        .map_err(|_| "tool_run_not_found")?;
    match policy.as_str() {
        "single" => Ok(true),
        "multi" => Ok(false),
        _ => Err("budget_execution_policy_invalid".into()),
    }
}

fn native_prepare_single_budget(context: &AgentRunContext) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let db = db::open(&context.db_path)?;
    if !native_single_policy(&db, context)? {
        return Ok(());
    }
    agent_require_frozen_web_plan(&db, context).map_err(str::to_string)?;
    let run = context.run.as_ref().ok_or("tool_run_not_found")?;
    RootOwner::load_single(&db, &run.run_id)?.require_live(&db)
}

fn agent_single_require_fresh_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    if native_single_policy(db, context)? {
        RootOwner::load_single(
            db,
            &context.run.as_ref().ok_or("tool_run_not_found")?.run_id,
        )?
        .require_live(db)?;
    }
    Ok(())
}

fn agent_single_require_fresh(context: &AgentRunContext) -> Result<(), String> {
    agent_single_require_fresh_on(&db::open(&context.db_path)?, context)
}

// Private writers have no schema, trigger, or unrelated business write authority.
#[derive(Clone, Copy)]
enum SingleWrite {
    HttpClaim,
    HttpReceipt,
    Finding,
}

fn single_http_transaction<T>(
    tx: &rusqlite::Transaction<'_>,
    receipt: bool,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    single_scoped_write(
        tx,
        if receipt {
            SingleWrite::HttpReceipt
        } else {
            SingleWrite::HttpClaim
        },
        work,
    )
}

fn single_finding_write<T>(
    db: &rusqlite::Connection,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    single_scoped_write(db, SingleWrite::Finding, work)
}

fn single_scoped_write<T>(
    db: &rusqlite::Connection,
    mode: SingleWrite,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    db.authorizer(Some(move |context: AuthContext<'_>| {
        let main = context.database_name == Some("main") && context.accessor.is_none();
        let allowed = match context.action {
            AuthAction::Insert { table_name } if main => match mode {
                SingleWrite::HttpClaim => matches!(
                    table_name,
                    "agent_budget_entries"
                        | "agent_http_budget_origins"
                        | "agent_http_request_claims"
                ),
                SingleWrite::HttpReceipt => table_name == "agent_budget_entries",
                SingleWrite::Finding => {
                    matches!(table_name, "sentinel_findings" | "agent_finding_candidates")
                }
            },
            AuthAction::Update {
                table_name,
                column_name,
            } if main => match mode {
                SingleWrite::HttpReceipt => {
                    table_name == "agent_http_request_claims"
                        && matches!(column_name, "response_status" | "received_at")
                }
                SingleWrite::Finding => match table_name {
                    "sentinel_findings" => matches!(
                        column_name,
                        "title" | "severity" | "record_json" | "updated_at"
                    ),
                    "agent_finding_candidates" => matches!(
                        column_name,
                        "title"
                            | "severity"
                            | "record_json"
                            | "candidate_revision"
                            | "status"
                            | "reviewer_run_id"
                            | "published_at"
                            | "updated_at"
                    ),
                    _ => false,
                },
                SingleWrite::HttpClaim => false,
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
    }))
    .map_err(|e| e.to_string())?;
    let result = work();
    let clear = db
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let value = result?;
    clear?;
    Ok(value)
}
