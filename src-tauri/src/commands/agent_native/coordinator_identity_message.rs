// Reuse the canonical Native emitters; do not adopt a modified consumed ACK.
fn native_coordinator_identity_message_events(
    db:&rusqlite::Connection,actor:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,id:&str,
)->Result<(),String> {
    crate::collaboration_events::client_delivery_schema::verify(db).map_err(|_|"root_identity_delivery_emitter_conflict")?;
    let (created,delivered,acked):(String,String,String)=db.query_row(
        "SELECT created_at,delivered_at,acknowledged_at FROM agent_messages WHERE id=?1 AND root_run_id=?2 AND kind='identity_assessment'",
        params![id,actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"root_identity_output_missing")?;
    let mut q=db.prepare("SELECT scan_id,attempt_number,entity_type,event_type,payload_json,created_at FROM agent_collaboration_events WHERE entity_id=?1 ORDER BY sequence").map_err(|e|e.to_string())?;
    let rows=q.query_map([id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,
        r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?))).map_err(|e|e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let expected=[json!({"kind":"identity_assessment","deliveredAt":"","acknowledgedAt":""}),
        json!({"kind":"identity_assessment","deliveredAt":delivered,"acknowledgedAt":acked})];
    if rows.len()!=2 || delivered.is_empty() || acked.is_empty() || rows.iter().zip(expected).enumerate().any(|(i,(row,payload))|
        row.0!=actor.scan_id || row.1!=actor.attempt_number || row.2!="mailbox_message" || row.3!="mailbox_message"
        || serde_json::from_str::<JsonValue>(&row.4).ok()!=Some(payload) || row.5<created
        || (i==0 && row.5!=created) || (i==1 && row.5<acked)) {
        return Err("root_identity_output_events_invalid".into());
    }
    Ok(())
}
