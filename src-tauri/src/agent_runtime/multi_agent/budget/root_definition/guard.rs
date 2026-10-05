//! Whole new-declaration transaction guard and exact historical row proof.
use super::NewRootBudgetDeclaration;
use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    params,
    types::Value,
    Transaction,
};

pub(crate) struct NewRootWriter<'tx, 'connection> {
    tx: &'tx Transaction<'connection>,
    declaration: NewRootBudgetDeclaration,
    roots: Rows,
    definitions: Rows,
    projections: Rows,
    events: Rows,
    event_floor: i64,
    writes: u64,
    active: bool,
}

impl<'tx, 'connection> NewRootWriter<'tx, 'connection> {
    pub(crate) fn install(
        tx: &'tx Transaction<'connection>,
        declaration: &NewRootBudgetDeclaration,
    ) -> Result<Self, String> {
        let event_floor = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let guard = Self {
            tx,
            declaration: declaration.clone(),
            roots: Rows::read(tx, "SELECT rowid,* FROM agent_runs ORDER BY rowid", [])?,
            definitions: Rows::read(
                tx,
                "SELECT rowid,* FROM agent_root_budget_definitions ORDER BY rowid",
                [],
            )?,
            projections: Self::other_projections(tx, declaration)?,
            events: Rows::read(
                tx,
                "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",
                [event_floor],
            )?,
            event_floor,
            writes: tx.total_changes(),
            active: true,
        };
        tx.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
        Ok(guard)
    }

    fn other_projections(
        tx: &Transaction<'_>,
        declaration: &NewRootBudgetDeclaration,
    ) -> Result<Rows, String> {
        Rows::read(
            tx,
            "SELECT rowid,* FROM sentinel_checkpoints
            WHERE NOT(scan_id=?1 AND url=?2 AND stage='agent_execution_plan') ORDER BY rowid",
            params![declaration.scan_id, declaration.target_url],
        )
    }

    pub(crate) fn finish(mut self) -> Result<(), String> {
        let root: String = self
            .tx
            .query_row(
                "SELECT root_run_id FROM agent_root_budget_definitions WHERE declaration_id=?1",
                [&self.declaration.declaration_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let frozen = super::read(self.tx, &root)?.ok_or("budget_declaration_root_missing")?;
        let text: String = self
            .tx
            .query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [&root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let projection: String=self.tx.query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage='agent_execution_plan'",
            params![frozen.scan_id,frozen.target_url],|r|r.get(0)).map_err(|e|e.to_string())?;
        let roots = Rows::read(
            self.tx,
            "SELECT rowid,* FROM agent_runs WHERE id<>?1 ORDER BY rowid",
            [&root],
        )?;
        let definitions=Rows::read(self.tx,"SELECT rowid,* FROM agent_root_budget_definitions WHERE root_run_id<>?1 ORDER BY rowid",[&root])?;
        let events = Rows::read(
            self.tx,
            "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",
            [self.event_floor],
        )?;
        let new=Rows::read(self.tx,
            "SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence",
            [self.event_floor])?;
        let event_ok = new.values.len() == 1
            && new.values[0][..5]
                == [
                    Value::Text(frozen.scan_id.clone()),
                    Value::Integer(frozen.attempt_number),
                    Value::Text("agent_run".into()),
                    Value::Text(root),
                    Value::Text("agent_run".into()),
                ]
            && match &new.values[0][5] {
                Value::Text(json) => {
                    serde_json::from_str::<serde_json::Value>(json).ok()
                        == Some(
                            serde_json::json!({"role":"coordinator","status":"prepared","terminalState":""}),
                        )
                }
                _ => false,
            };
        if frozen != self.declaration
            || projection != text
            || roots != self.roots
            || definitions != self.definitions
            || events != self.events
            || !event_ok
            || Self::other_projections(self.tx, &self.declaration)? != self.projections
            || self.tx.total_changes().checked_sub(self.writes) != Some(4)
        {
            return Err("budget_declaration_creation_collateral_write".into());
        }
        self.tx
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|e| e.to_string())?;
        self.active = false;
        Ok(())
    }
}

impl Drop for NewRootWriter<'_, '_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .tx
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}

fn authorize(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Insert { table_name }
            if context.database_name == Some("main")
                && context.accessor.is_none()
                && matches!(
                    table_name,
                    "agent_runs" | "agent_root_budget_definitions" | "sentinel_checkpoints"
                ) =>
        {
            Authorization::Allow
        }
        AuthAction::Update {
            table_name: "sentinel_checkpoints",
            column_name,
        } if context.database_name == Some("main")
            && context.accessor.is_none()
            && matches!(column_name, "raw_json" | "updated_at") =>
        {
            Authorization::Allow
        }
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if context.database_name == Some("main")
            && context.accessor == Some("agent_collaboration_run_insert") =>
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
    }
}
