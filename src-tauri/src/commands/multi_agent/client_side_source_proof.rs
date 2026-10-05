// Complete old source rows, including rowid, pinned to the original HTTP worker.
fn client_side_source_rows(
    db: &rusqlite::Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<String, String> {
    let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
    let n = q.column_count();
    let rows = q
        .query_map(p, |r| {
            (0..n)
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(format!("{rows:?}"))
}
fn client_side_source_proof(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    node: &crate::agent_runtime::evidence_graph::contract::EvidenceNode,
    identity: &str,
    status: i64,
) -> Result<String, String> {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    RootOwner::load_original(db, &lease.root_run_id)?.require_original_coordinator(db, lease)?;
    let req = node.payload["requestId"]
        .as_str()
        .ok_or("client_side_request_ref_missing")?;
    let ordinal = req
        .strip_prefix("req-")
        .and_then(|n| n.parse::<i64>().ok())
        .filter(|n| *n > 0)
        .ok_or("client_side_request_ref_invalid")?;
    if req != format!("req-{ordinal:04}") {
        return Err("client_side_request_ref_invalid".into());
    }
    let (assignment,worker,worker_epoch,invocation,index,wire):(String,String,i64,String,i64,String)=db.query_row(
        "SELECT a.id,x.id,x.lease_epoch,c.invocation_id,c.request_index,c.request_hash
         FROM agent_http_request_claims c JOIN agent_runs r ON r.id=c.run_id
         JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id
         JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=r.id
         WHERE c.run_id=?1 AND c.ordinal=?2 AND c.scan_id=?3 AND c.attempt_number=?4 AND c.target_url=?5
           AND c.identity_handle=?6 AND c.response_status=?7 AND c.received_at<>''
           AND a.coordinator_run_id=?8 AND a.target_key=?5 AND a.lease_epoch=?9 AND a.fencing_token=?10
           AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0
           AND r.backend='native' AND r.role='web_executor' AND r.lane='target_touching' AND r.root_run_id=?8
           AND r.scan_id=?3 AND r.attempt_number=?4 AND r.target_url=?5 AND r.parent_run_id=?8
           AND r.status='terminal' AND r.terminal_state='completed'
           AND x.root_run_id=?8 AND x.coordinator_epoch=?9 AND x.coordinator_fencing_token=?10
           AND x.state='completed' AND x.finished_at<>''
           AND EXISTS(SELECT 1 FROM agent_budget_entries b WHERE b.root_run_id=?8 AND b.assignment_id=a.id
             AND b.lease_attempt_id=x.id AND b.dimension='target_requests' AND b.kind='forfeit' AND b.amount=1
             AND b.source_id=printf('http:%s:%s:%d:%s',c.run_id,c.invocation_id,c.request_index,c.request_hash))
           AND EXISTS(SELECT 1 FROM tool_invocations i WHERE i.run_id=r.id AND i.invocation_id=c.invocation_id
             AND i.tool_name=c.tool_name AND i.policy_decision='allow' AND i.status='completed' AND i.finished_at<>'')",
        params![node.created_by_run_id,ordinal,lease.scan_id,lease.attempt_number,lease.target_key,identity,status,
            lease.root_run_id,lease.lease_epoch,lease.fencing_token],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|_|"client_side_original_request_owner_missing")?;
    if uuid::Uuid::parse_str(&worker).is_err() || worker_epoch <= 0 {
        return Err("client_side_original_worker_invalid".into());
    }
    let source = format!(
        "http:{}:{invocation}:{index}:{wire}",
        node.created_by_run_id
    );
    let entries:Vec<(String,String,i64)>=db.prepare("SELECT kind,idempotency_key,amount FROM agent_budget_entries
        WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4 ORDER BY rowid")
        .map_err(|e|e.to_string())?.query_map(params![lease.root_run_id,assignment,worker,source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
        .map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let prefix = if worker_epoch == 1 {
        format!("target:{source}")
    } else {
        format!("worker:{worker}:target:{source}")
    };
    let expected = vec![
        ("reserve".to_string(), format!("{prefix}:reserve"), 1),
        ("forfeit".to_string(), format!("{prefix}:dispatch"), 1),
        ("reconcile".to_string(), format!("{prefix}:receipt"), 1),
    ];
    if entries != expected {
        return Err("client_side_original_target_cost_proof_missing".into());
    }
    let mut raw = String::new();
    for (sql, key) in [
        (
            "SELECT rowid,* FROM agent_evidence_nodes WHERE id=?1",
            node.id.as_str(),
        ),
        (
            "SELECT rowid,* FROM agent_runs WHERE id=?1",
            node.created_by_run_id.as_str(),
        ),
        (
            "SELECT rowid,* FROM agent_assignments WHERE id=?1",
            assignment.as_str(),
        ),
        (
            "SELECT rowid,* FROM agent_assignment_attempts WHERE id=?1",
            worker.as_str(),
        ),
        (
            "SELECT rowid,* FROM agent_root_budget_attempts WHERE root_run_id=?1",
            lease.root_run_id.as_str(),
        ),
    ] {
        raw.push_str(&client_side_source_rows(db, sql, [key])?);
    }
    raw.push_str(&client_side_source_rows(
        db,
        "SELECT rowid,* FROM agent_http_request_claims WHERE run_id=?1 AND ordinal=?2",
        params![node.created_by_run_id, ordinal],
    )?);
    raw.push_str(&client_side_source_rows(
        db,
        "SELECT rowid,* FROM tool_invocations WHERE run_id=?1 AND invocation_id=?2",
        params![node.created_by_run_id, invocation],
    )?);
    raw.push_str(&client_side_source_rows(db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id=?2 ORDER BY rowid",
        params![lease.root_run_id,source])?);
    Ok(crate::agent_runtime::store::stable_hash(&raw))
}
