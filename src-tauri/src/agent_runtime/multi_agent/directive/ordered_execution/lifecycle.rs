use super::*;
pub(crate) fn start(
    db: &Connection,
    scope: &CoordinatorLease,
    id: &str,
    order: i64,
    check: impl Fn(&Connection) -> Result<(), String>,
) -> Result<bool, String> {
    let private = writer::private_connection(db, Mode::Start)?;
    let db = &private;
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    current(&tx, scope)?;
    check(&tx)?;
    let job = load(&tx, id, order)?.ok_or("ordered_job_missing")?;
    if job.state != "prepared" {
        return Ok(false);
    }
    binding::verify_request(&tx, &job, true)?;
    if order == 2 {
        let first = load(&tx, id, 1)?.ok_or("ordered_predecessor_missing")?;
        if receipts::verified(&tx, &first)?.is_none_or(|r| r["outcome"] != "valid_advisory") {
            return Err("ordered_predecessor_not_valid".into());
        }
    }
    let guard = Writer::install(&private, scope, id, &job.child, Mode::Start)?;
    scheduler::mark_child_running_in_transaction(&tx, scope, &job.child)?;
    let changed=tx.execute("UPDATE agent_directive_ordered_actions SET state='executing' WHERE action_id=?1 AND state='prepared'",[&job.action.action_id]).map_err(|e|e.to_string())?;
    if changed != 1 || load(&tx, id, order)?.is_none_or(|j| j.state != "executing") {
        return Err("ordered_start_not_persisted".into());
    }
    event_exact(&tx, &job, "orderedState", &json!("executing"))?;
    guard.verify()?;
    check(&tx)?;
    current(&tx, scope)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(true)
}
pub(crate) fn finish_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    scope: &CoordinatorLease,
    job: &ActionJob,
    result: &str,
) -> Result<Value, String> {
    let db: &Connection = tx;
    current(db, scope)?;
    financial::verify_projection(db, job)?;
    let success = job.response["valid"] == true;
    let expected = if success { "completed" } else { "failed" };
    let changed=db.execute("UPDATE agent_directive_ordered_actions SET state=?1,result_message_id=?2 WHERE action_id=?3 AND state='received'",
       params![expected,result,job.action.action_id]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("ordered_finish_not_persisted".into());
    }
    if !success {
        transition_directive_in_transaction(tx, scope, &job.directive_id, "assigned", "failed")?;
    } else if job.action.order == 2 {
        transition_directive_in_transaction(tx, scope, &job.directive_id, "assigned", "applied")?;
        transition_directive_in_transaction(tx, scope, &job.directive_id, "applied", "completed")?;
    }
    let stored =
        load(db, &job.directive_id, job.action.order)?.ok_or("ordered_finish_not_persisted")?;
    if stored.state != expected || stored.result_message_id != result {
        return Err("ordered_finish_not_persisted".into());
    }
    receipts::record(db, &stored)
}
pub(crate) fn consume_request(
    db: &Connection,
    scope: &CoordinatorLease,
    job: &ActionJob,
    check: impl Fn(&Connection) -> Result<(), String>,
) -> Result<(), String> {
    let private = writer::private_connection(db, Mode::Consume)?;
    let db = &private;
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    current(&tx, scope)?;
    check(&tx)?;
    let current_job =
        load(&tx, &job.directive_id, job.action.order)?.ok_or("ordered_job_missing")?;
    if current_job.state != "prepared"
        || current_job.input != job.input
        || current_job.child != job.child
    {
        return Err("ordered_request_state_conflict".into());
    }
    let ack: String = tx
        .query_row(
            "SELECT acknowledged_at FROM agent_messages WHERE id=?1",
            [&job.request_message_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if ack.is_empty() {
        let guard = Writer::install(&private, scope, &job.directive_id, &job.child, Mode::Consume)?;
        mailbox::consume_coordinator_message(
            &tx,
            scope,
            &job.child.run_id,
            &job.request_message_id,
            "human_ordered_assessment_request",
            &job.input,
        )?;
        binding::verify_request(&tx, job, true)?;
        guard.verify()?;
    } else {
        binding::verify_request(&tx, job, true)?;
    }
    check(&tx)?;
    current(&tx, scope)?;
    tx.commit().map_err(|e| e.to_string())
}
