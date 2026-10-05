use super::*;
pub(super) fn verify(
    db: &Connection,
    scope: &CoordinatorLease,
    id: &str,
    child: &scheduler::ScheduledChild,
    mode: Mode,
    floor: i64,
) -> Result<(), String> {
    if matches!(mode, Mode::Defer) {
        let (draft, original, state) = source(db, id)?;
        let reason: String = db.query_row("SELECT rejection_code FROM agent_user_directives WHERE id=?1",
            [id], |r| r.get(0)).map_err(|e| e.to_string())?;
        let rows = Rows::read(db, "SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence", [floor])?;
        let [row] = rows.values.as_slice() else { return Err("ordered_defer_events_changed".into()); };
        let [SqlValue::Text(scan), SqlValue::Integer(attempt), SqlValue::Text(entity), SqlValue::Text(key), SqlValue::Text(event), SqlValue::Text(raw)] = row.as_slice()
            else { return Err("ordered_defer_events_changed".into()); };
        let payload: Value = serde_json::from_str(raw).map_err(|_| "ordered_defer_events_changed")?;
        if original.root_run_id != scope.root_run_id || original.lease_epoch != scope.lease_epoch
            || original.fencing_token != scope.fencing_token || state != "deferred"
            || reason != "ordered_budget_unavailable" || scan != &scope.scan_id || attempt != &scope.attempt_number
            || entity != "user_directive" || key != id || event != "user_directive"
            || payload != json!({"status":"deferred","sourceDraftId":draft.id,"rejectionCode":reason}) {
            return Err("ordered_defer_events_changed".into());
        }
        return Ok(());
    }
    let job = load_for_child(db, id, child)?;
    let role = child.role.as_str();
    let action = job.action.action_id.as_str();
    let action_event = |state: &str| json!({"orderedActionId":action,"orderedOrder":job.action.order,"orderedState":state});
    let (source, rejection): (String, String) = db
        .query_row(
            "SELECT source_draft_id,rejection_code FROM agent_user_directives WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let directive =
        |state: &str| json!({"status":state,"sourceDraftId":source,"rejectionCode":rejection});
    let mut wanted: Vec<(&str, String, Value)> = vec![];
    let mut push = |entity: &'static str, key: &str, payload: Value| {
        wanted.push((entity, key.into(), payload))
    };
    match mode {
        Mode::Defer => return Err("ordered_defer_events_changed".into()),
        Mode::Schedule => {
            push(
                "assignment",
                &child.assignment_id,
                json!({"role":role,"state":"prepared"}),
            );
            push(
                "assignment",
                &child.assignment_id,
                json!({"role":role,"state":"leased"}),
            );
            push(
                "agent_run",
                &child.run_id,
                json!({"role":role,"status":"prepared","terminalState":""}),
            );
            push(
                "mailbox_message",
                &job.request_message_id,
                json!({"kind":"human_ordered_assessment_request","deliveredAt":"","acknowledgedAt":""}),
            );
            push("user_directive", id, action_event("prepared"));
            if job.action.order == 1 {
                push("user_directive", id, directive("assigned"));
            }
        }
        Mode::Consume => {
            let (d, a): (String, String) = db
                .query_row(
                    "SELECT delivered_at,acknowledged_at FROM agent_messages WHERE id=?1",
                    [&job.request_message_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| e.to_string())?;
            if d.is_empty() || a.is_empty() {
                return Err("ordered_request_ack_missing".into());
            }
            push(
                "mailbox_message",
                &job.request_message_id,
                json!({"kind":"human_ordered_assessment_request","deliveredAt":d,"acknowledgedAt":a}),
            );
        }
        Mode::Start => {
            push(
                "assignment",
                &child.assignment_id,
                json!({"role":role,"state":"running"}),
            );
            push(
                "agent_run",
                &child.run_id,
                json!({"role":role,"status":"running","terminalState":""}),
            );
            push("user_directive", id, action_event("executing"));
        }
        Mode::Receive => push("user_directive", id, action_event("received")),
        Mode::Finish => {
            let state = if job.response["valid"] == true {
                "completed"
            } else {
                "failed"
            };
            let (d, a): (String, String) = db
                .query_row(
                    "SELECT delivered_at,acknowledged_at FROM agent_messages WHERE id=?1",
                    [&job.result_message_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| e.to_string())?;
            let receipt =
                receipts::verified(db, &job)?.ok_or("ordered_terminal_receipt_missing")?;
            push(
                "assignment",
                &child.assignment_id,
                json!({"role":role,"state":state}),
            );
            push(
                "agent_run",
                &child.run_id,
                json!({"role":role,"status":"terminal","terminalState":state}),
            );
            push(
                "mailbox_message",
                &job.result_message_id,
                json!({"kind":"human_ordered_assessment_result","deliveredAt":"","acknowledgedAt":""}),
            );
            push(
                "mailbox_message",
                &job.result_message_id,
                json!({"kind":"human_ordered_assessment_result","deliveredAt":d,"acknowledgedAt":a}),
            );
            push("user_directive", id, action_event(state));
            push(
                "user_directive",
                id,
                json!({"orderedActionId":action,"orderedOrder":job.action.order,"orderedReceiptId":receipt["receiptId"]}),
            );
            if state == "failed" {
                push("user_directive", id, directive("failed"));
            } else if job.action.order == 2 {
                push("user_directive", id, directive("applied"));
                push("user_directive", id, directive("completed"));
            }
        }
    }
    let rows=Rows::read(db,"SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence",[floor])?;
    if rows.values.len() != wanted.len() {
        return Err("ordered_collaboration_events_changed".into());
    }
    for row in rows.values {
        let [SqlValue::Text(scan), SqlValue::Integer(attempt), SqlValue::Text(entity), SqlValue::Text(key), SqlValue::Text(event), SqlValue::Text(raw)] =
            row.as_slice()
        else {
            return Err("ordered_collaboration_events_changed".into());
        };
        let payload: Value =
            serde_json::from_str(raw).map_err(|_| "ordered_collaboration_events_changed")?;
        if scan != &scope.scan_id || attempt != &scope.attempt_number || event != entity {
            return Err("ordered_collaboration_events_changed".into());
        }
        let position = wanted
            .iter()
            .position(|(e, k, p)| *e == entity && k == key && p == &payload)
            .ok_or("ordered_collaboration_events_changed")?;
        wanted.remove(position);
    }
    Ok(())
}
fn load_for_child(
    db: &Connection,
    id: &str,
    child: &scheduler::ScheduledChild,
) -> Result<ActionJob, String> {
    let order:i64=db.query_row("SELECT action_order FROM agent_directive_ordered_actions WHERE directive_id=?1 AND child_run_id=?2 AND assignment_id=?3",
       params![id,child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    load(db, id, order)?.ok_or("ordered_guard_job_missing".into())
}
