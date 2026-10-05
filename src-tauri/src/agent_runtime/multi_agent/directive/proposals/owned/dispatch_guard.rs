//! Only the original dispatch and shared elapsed-clock entries may be inserted.
//! No trigger, view or nested statement can alter business/other Root rows.
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection,
};
pub(crate) struct DispatchGuard<'a> {
    db: &'a Connection,
    active: bool,
}
impl<'a> DispatchGuard<'a> {
    pub(crate) fn install(db: &'a Connection) -> Result<Self, String> {
        db.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
        Ok(Self { db, active: true })
    }
    pub(crate) fn finish(mut self) -> Result<(), String> {
        self.db
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|e| e.to_string())?;
        self.active = false;
        Ok(())
    }
}
impl Drop for DispatchGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .db
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}
fn authorize(context: AuthContext<'_>) -> Authorization {
    let allowed = match context.action {
        AuthAction::Insert { table_name }
            if context.database_name == Some("main") && context.accessor.is_none() =>
        {
            matches!(
                table_name,
                "agent_specialist_calls" | "agent_budget_entries"
            )
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
}
