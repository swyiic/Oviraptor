// Audit original paid publications and committed cursor bindings in the caller's
// single WAL snapshot. A saved record never becomes current execution authority.
fn native_root_decision_timeline(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> Result<Vec<JsonValue>, String> {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    if db.is_autocommit() {
        return Err("root_tick_projection_requires_snapshot".into());
    }
    let mut statement = db
        .prepare(
            "SELECT p.root_run_id,json_extract(p.fact_json,'$.eventSequence')
        FROM agent_root_tick_receipts p JOIN agent_runs r ON r.id=p.root_run_id
        JOIN agent_root_budget_attempts x ON x.id=p.lease_attempt_id AND x.root_run_id=p.root_run_id
        WHERE p.phase='publication' AND ((r.scan_id=?1 AND r.attempt_number=?2)
          OR (json_extract(x.contract_json,'$.root.scan')=?1
            AND json_extract(x.contract_json,'$.root.attempt')=?2)) ORDER BY p.root_run_id,p.round",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![scan, attempt], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut expected = std::collections::BTreeMap::<String, i64>::new();
    let mut timeline = Vec::new();
    for row in rows {
        let (root, model_event) = row.map_err(|e| e.to_string())?;
        let p = Tick::project_publication(db, &root, model_event)?;
        if p.scan != scan
            || p.attempt != attempt
            || expected.insert(p.entity.clone(), p.sequence).is_some()
        {
            return Err("root_tick_timeline_scope_conflict".into());
        }
        timeline.push(
            json!({"id":p.entity,"sequence":p.sequence,"timestamp":p.created,
            "eventType":"root_decision","fromRole":"coordinator","fromRunId":p.root,
            "toRole":"operator","toRunId":"","messageKind":"root_decision_summary",
            "correlationId":p.record["callId"],"assignmentId":"","evidenceRevision":0,
            "threadKey":p.thread,"targetKey":p.target,
            "deliveryState":"persisted","ackState":"n/a","status":"recorded",
            "summary":"Root 已保存结构化决策摘要；建议不代表派发、采纳或验证完成。",
            "decisionRecord":p.record}),
        );
    }
    let mut bindings = expected.clone();
    let mut mapping = db
        .prepare(
            "SELECT m.entity_id,m.collaboration_sequence
        FROM agent_root_tick_timeline_receipts m JOIN agent_runs r ON r.id=m.root_run_id
        JOIN agent_root_budget_attempts x ON x.id=m.lease_attempt_id
        WHERE (m.scan_id=?1 AND m.attempt_number=?2) OR (r.scan_id=?1 AND r.attempt_number=?2)
          OR (json_extract(x.contract_json,'$.root.scan')=?1
            AND json_extract(x.contract_json,'$.root.attempt')=?2)",
        )
        .map_err(|e| e.to_string())?;
    for row in mapping
        .query_map(params![scan, attempt], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?
    {
        let (entity, sequence) = row.map_err(|e| e.to_string())?;
        if bindings.remove(&entity) != Some(sequence) {
            return Err("root_tick_timeline_orphan_or_alias".into());
        }
    }
    if !bindings.is_empty() {
        return Err("root_tick_timeline_receipt_unverified".into());
    }
    // No page can hide a forged or missing channel. Alias audit in Tick also
    // includes other scans/attempts using this original call/entity/cursor.
    let mut statement = db
        .prepare(
            "SELECT entity_id,sequence FROM agent_collaboration_events
        WHERE scan_id=?1 AND attempt_number=?2
          AND (event_type='root_decision' OR entity_type='root_decision') ORDER BY sequence",
        )
        .map_err(|e| e.to_string())?;
    for row in statement
        .query_map(params![scan, attempt], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?
    {
        let (entity, sequence) = row.map_err(|e| e.to_string())?;
        if expected.remove(&entity) != Some(sequence) {
            return Err("root_tick_timeline_orphan_or_alias".into());
        }
    }
    if !expected.is_empty() {
        return Err("root_tick_timeline_receipt_unverified".into());
    }
    Ok(timeline)
}
