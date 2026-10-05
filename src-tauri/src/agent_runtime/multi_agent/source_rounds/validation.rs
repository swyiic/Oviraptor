fn verify_checkpoint(db: &Connection, call: &PendingRound) -> Result<(), String> {
    let (last, sequence): (i64, i64) = db.query_row(
        "SELECT COALESCE(MAX(round_number),0),COALESCE(MAX(event_sequence),0) FROM agent_source_model_rounds WHERE assignment_id=?1 AND child_run_id=?2 AND state='received'",
        params![call.child.assignment_id,call.child.run_id], |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|e|e.to_string())?;
    if last == 0 {
        return Ok(());
    }
    let sum = cumulative(db, call, last)?;
    let snapshot =
        store::read_snapshot(db, &call.child.run_id)?.ok_or("source_round_checkpoint_missing")?;
    let state = RunState::from_json(&snapshot.snapshot);
    if snapshot.last_sequence < sequence
        || state.run_id != call.child.run_id
        || state.turns != last
        || state.model_requests != sum.model_requests
        || state.input_tokens != sum.input_tokens
        || state.cached_input_tokens != sum.cached_input_tokens
        || state.output_tokens != sum.output_tokens
        || state.used_tokens != sum.total_tokens
        || state.target_requests != 0
    {
        return Err("source_round_checkpoint_invalid".into());
    }
    Ok(())
}

fn verify_budget(db: &Connection, call: &PendingRound) -> Result<(), String> {
    let sum = cumulative(db, call, call.number - 1)?;
    let (tokens, requests): (i64, i64) = db.query_row(
        "SELECT a.reserved_tokens,a.reserved_requests FROM agent_assignments a JOIN agent_budget_ledger b
         ON b.root_run_id=a.coordinator_run_id AND b.lease_epoch=a.lease_epoch AND b.fencing_token=a.fencing_token
         WHERE a.id=?1 AND a.budget_settled_at='' AND b.reserved_tokens>=a.reserved_tokens AND b.reserved_requests>=a.reserved_requests",
        [&call.child.assignment_id], |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|_|"source_round_budget_binding_invalid")?;
    if sum
        .total_tokens
        .checked_add(call.reserved_tokens)
        .is_none_or(|n| n > tokens)
        || call.number > requests
    {
        return Err("source_round_budget_exhausted".into());
    }
    Ok(())
}

fn verify_request(db: &Connection, call: &PendingRound) -> Result<(), String> {
    if call.number == 1 {
        let slice: String = db
            .query_row(
                "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
                [&call.child.assignment_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let messages = call.request["messages"]
            .as_array()
            .ok_or("source_round_messages_invalid")?;
        let input = messages
            .last()
            .filter(|m| m["role"] == "user")
            .and_then(|m| m["content"].as_str())
            .ok_or("source_round_input_missing")?;
        let input = parse(input.into())?;
        if input["sourceTask"] != redact_json(&parse(slice)?) {
            return Err("source_round_source_task_changed".into());
        }
        super::directive::source_guidance::validate_input(
            db,
            &call.lease,
            call.child.role,
            true,
            &input,
        )?;
        if messages
            .iter()
            .any(|m| !matches!(m["role"].as_str(), Some("user" | "system")))
        {
            return Err("source_round_initial_history_invalid".into());
        }
    } else {
        let previous = historical(db, call, call.number - 1)?;
        let expected = super::directive::source_guidance::attach_round(
            db,
            &call.lease,
            call.child.role,
            call.number,
            continuation(db, &previous)?,
        )?;
        if call.request != expected {
            return Err("source_round_transcript_changed".into());
        }
    }
    Ok(())
}
