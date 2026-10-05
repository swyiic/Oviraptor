//! Private diagnostic connections can write only their exact production lane.
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection,
};
#[derive(Clone, Copy)]
pub(super) enum Mode {
    Begin,
    Append,
}
pub(super) fn protect<T>(
    db: &Connection,
    mode: Mode,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    db.authorizer(Some(move |context: AuthContext<'_>| {
        let direct = context.database_name == Some("main") && context.accessor.is_none();
        let allowed = match context.action {
            AuthAction::Insert { table_name } if direct => match mode {
                Mode::Begin => table_name == "native_process_log_executions",
                Mode::Append => table_name == "native_process_log_rows",
            },
            AuthAction::Update {
                table_name: "native_process_log_executions",
                column_name,
            } if direct => {
                matches!(mode, Mode::Append)
                    && matches!(column_name, "row_count" | "byte_count" | "state")
            }
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
    .map_err(|_| "native_process_log_authorizer_unavailable")?;
    let value = work();
    let cleared = db
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|_| "native_process_log_authorizer_unavailable");
    // Even failed or ignored writes leave no lingering authority hook.
    let result = value?;
    cleared?;
    Ok(result)
}
