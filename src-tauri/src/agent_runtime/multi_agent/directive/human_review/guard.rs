//! Restrict actual human-decision writes; scoped fingerprints are not authority.
use super::{draft_store, HumanDirectiveDraft};
use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    params, Connection,
};
use serde_json::{json, Value};
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Review,
    Approval,
}
pub(crate) struct Writer<'a> {
    db: &'a Connection,
    draft: HumanDirectiveDraft,
    mode: Mode,
    floor: i64,
    events: Rows,
    drafts: Rows,
    queue: Rows,
    receipts: Rows,
    host: Rows,
    active: bool,
}
impl<'a> Writer<'a> {
    pub(crate) fn install(
        db: &'a Connection,
        mode: Mode,
        draft: &HumanDirectiveDraft,
    ) -> Result<Self, String> {
        let floor = db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
                [],
                |r| r.get(0),
            )
            .map_err(|_| "directive_human_review_write_guard_read")?;
        let writer = Self {
            db,
            draft: draft.clone(),
            mode,
            floor,
            events: Rows::read(
                db,
                "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",
                [floor],
            )?,
            drafts: Rows::read(
                db,
                "SELECT rowid,* FROM agent_directive_drafts WHERE id<>?1 ORDER BY rowid",
                [&draft.id],
            )?,
            queue: Rows::read(
                db,
                "SELECT rowid,* FROM agent_user_directives ORDER BY rowid",
                [],
            )?,
            receipts: Rows::read(
                db,
                "SELECT rowid,* FROM agent_directive_human_reviews ORDER BY rowid",
                [],
            )?,
            host: Rows::read(
                db,
                "SELECT rowid,* FROM agent_host_boundary_candidates ORDER BY rowid",
                [],
            )?,
            active: true,
        };
        db.authorizer(Some(move |context: AuthContext<'_>| {
            authorize(context, mode)
        }))
        .map_err(|_| "directive_human_review_write_guard_unavailable")?;
        Ok(writer)
    }
    pub(crate) fn finish(mut self) -> Result<(), String> {
        self.verify()?;
        self.db
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|_| "directive_human_review_write_guard_unavailable")?;
        self.active = false;
        Ok(())
    }
    fn verify(&self) -> Result<(), String> {
        let raw: String = self
            .db
            .query_row(
                "SELECT receipt_json FROM agent_directive_human_reviews WHERE draft_id=?1",
                [&self.draft.id],
                |r| r.get(0),
            )
            .map_err(|_| "directive_human_review_write_guard_receipt_missing")?;
        let receipt: Value = serde_json::from_str(&raw)
            .map_err(|_| "directive_human_review_write_guard_receipt_invalid")?;
        let next = receipt["successorDraftId"].as_str().unwrap_or("");
        let directive = receipt["directiveId"].as_str().unwrap_or("");
        let receipt_id = receipt["receiptId"]
            .as_str()
            .ok_or("directive_human_review_write_guard_receipt_invalid")?;
        if (matches!(self.mode, Mode::Approval)
            && (receipt["kind"] != "approve" || directive.is_empty() || !next.is_empty()))
            || (matches!(self.mode, Mode::Review)
                && (!directive.is_empty()
                    || !matches!(receipt["kind"].as_str(), Some("revise" | "reject"))))
        {
            return Err("directive_human_review_write_guard_receipt_invalid".into());
        }
        if Rows::read(self.db,"SELECT rowid,* FROM agent_directive_drafts WHERE id<>?1 AND id<>?2 ORDER BY rowid",params![self.draft.id,next])?!=self.drafts
   || Rows::read(self.db,"SELECT rowid,* FROM agent_user_directives WHERE id<>?1 ORDER BY rowid",[directive])?!=self.queue
   || Rows::read(self.db,"SELECT rowid,* FROM agent_directive_human_reviews WHERE receipt_id<>?1 ORDER BY rowid",[receipt_id])?!=self.receipts
   || Rows::read(self.db,"SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=?1 ORDER BY rowid",[self.floor])?!=self.events {
   return Err("directive_human_review_unrelated_rows_changed".into());
  }
        let mut expected: Vec<(String, String, Value)> = vec![];
        let after = draft_store::load_draft(self.db, &self.draft.id)?
            .ok_or("directive_human_review_write_guard_draft_missing")?;
        expected.push(("directive_draft".into(),after.id.clone(),json!({"status":after.status,"revision":after.revision,"validationResult":after.validation_result})));
        if !next.is_empty() {
            let child = draft_store::load_draft(self.db, next)?
                .ok_or("directive_human_review_write_guard_draft_missing")?;
            expected.push(("directive_draft".into(),child.id.clone(),json!({"status":child.status,"revision":child.revision,"validationResult":child.validation_result})));
        }
        if !directive.is_empty() {
            expected.push((
                "user_directive".into(),
                directive.into(),
                json!({"status":"pending","sourceDraftId":self.draft.id}),
            ));
        }
        let hosts: Vec<(String, String, String)> = if next.is_empty() {
            vec![]
        } else {
            self.db.prepare("SELECT id,status,source_kind FROM agent_host_boundary_candidates WHERE source_draft_id=?1")
   .map_err(|_|"directive_human_review_write_guard_host_invalid")?.query_map([next],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
   .map_err(|_|"directive_human_review_write_guard_host_invalid")?.collect::<Result<_,_>>().map_err(|_|"directive_human_review_write_guard_host_invalid")?
        };
        if hosts.len() > 1 {
            return Err("directive_human_review_write_guard_host_invalid".into());
        }
        let host_id = hosts.first().map_or("", |h| h.0.as_str());
        if Rows::read(
            self.db,
            "SELECT rowid,* FROM agent_host_boundary_candidates WHERE id<>?1 ORDER BY rowid",
            [host_id],
        )? != self.host
        {
            return Err("directive_human_review_unrelated_rows_changed".into());
        }
        for (id, status, kind) in hosts {
            expected.push((
                "host_boundary_candidate".into(),
                id,
                json!({"status":status,"sourceKind":kind}),
            ));
        }
        let actual:Vec<(String,i64,String,String,String,String,String)>=self.db.prepare("SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json,created_at FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence")
   .map_err(|_|"directive_human_review_write_guard_events_invalid")?.query_map([self.floor],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))
   .map_err(|_|"directive_human_review_write_guard_events_invalid")?.collect::<Result<_,_>>().map_err(|_|"directive_human_review_write_guard_events_invalid")?;
        if actual.len() != expected.len() {
            return Err("directive_human_review_write_guard_events_invalid".into());
        }
        for (scan, attempt, entity, id, event, raw, time) in actual {
            let payload: Value = serde_json::from_str(&raw)
                .map_err(|_| "directive_human_review_write_guard_events_invalid")?;
            let Some(index) = expected
                .iter()
                .position(|(e, i, p)| e == &entity && i == &id && p == &payload)
            else {
                return Err("directive_human_review_write_guard_events_invalid".into());
            };
            if scan != self.draft.scan_id
                || attempt != self.draft.attempt_number
                || event != entity
                || time.is_empty()
            {
                return Err("directive_human_review_write_guard_events_invalid".into());
            }
            expected.remove(index);
        }
        Ok(())
    }
}
impl Drop for Writer<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .db
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}
fn authorize(context: AuthContext<'_>, mode: Mode) -> Authorization {
    let direct = context.database_name == Some("main") && context.accessor.is_none();
    let allowed = match context.action {
        AuthAction::Insert { table_name } if direct => match mode {
            Mode::Review => matches!(
                table_name,
                "agent_directive_drafts"
                    | "agent_directive_human_reviews"
                    | "agent_host_boundary_candidates"
            ),
            Mode::Approval => matches!(
                table_name,
                "agent_user_directives" | "agent_directive_human_reviews"
            ),
        },
        AuthAction::Update {
            table_name: "agent_directive_drafts",
            column_name,
        } if direct => match mode {
            Mode::Review => matches!(column_name, "status" | "updated_at"),
            Mode::Approval => matches!(
                column_name,
                "status" | "confirmed_directive_id" | "confirmed_at" | "updated_at"
            ),
        },
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if context.database_name == Some("main") => matches!(
            context.accessor,
            Some(
                "agent_collaboration_draft_insert"
                    | "agent_collaboration_draft_update"
                    | "agent_collaboration_directive_insert"
                    | "agent_collaboration_host_boundary_insert"
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
