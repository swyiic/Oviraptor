//! Source assignments inherit the actual frozen analysis, never a Web execution
//! plan or the current scan-wide importer projection. Initial assessments are
//! tool-free; a separate source_tools assignment grants only scoped source tools.
//! Neither phase grants shell, network or host access.
pub(crate) use super::source_plan_contract::SourcePhaseContract;
use super::{lease::CoordinatorLease, scheduler::ScheduledChild};
use crate::{
    agent_runtime::contract::AgentRole,
    native_pipeline::{
        analysis_view::{AnalysisManifest, SourceAnalysisView},
        results::AnalysisResults,
        snapshot::RepositorySnapshot,
        NativeSourcePlan,
    },
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub fn is_source_role(role: AgentRole) -> bool {
    matches!(role, AgentRole::RepoMapper | AgentRole::SourceAnalyst)
}

pub(crate) fn model_request_limit(plan: &Value) -> Result<i64, String> {
    Ok(SourcePhaseContract::from_plan(plan)?.model_request_limit)
}

/// A Reviewer may serve either surface. Its role alone must never select the
/// Web transport and bypass the frozen source runtime, deadline or budget.
pub(crate) fn uses_source_runtime(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
) -> Result<bool, String> {
    validate_surface_role(connection, lease, role)?;
    let source:bool=connection.query_row("SELECT target_url LIKE 'source:%' OR json_extract(plan_json,'$.surface')='source' FROM agent_runs WHERE id=?1",
        [&lease.root_run_id],|r|r.get::<_,Option<bool>>(0)).map_err(|_|"source_surface_root_unavailable")?.unwrap_or(false);
    Ok(source)
}

/// The root, not the requested child role or caller-supplied slice, selects the
/// execution surface. Version 1 has two assessment and two source-tool phases;
/// a generic Reviewer/Web assignment must not smuggle an extra phase into it.
/// Version 2 adds independent review with its own frozen material, request and
/// completion proofs. Existing v1 roots never inherit that additional phase.
pub(crate) fn validate_surface_role(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
) -> Result<(), String> {
    let (text, target, requests, rooted): (String, String, i64, bool) = connection
        .query_row(
            "SELECT plan_json,target_url,hard_request_budget,root_run_id=id FROM agent_runs
             WHERE id=?1 AND role='coordinator'
             AND (root_run_id=id OR (root_run_id='' AND orchestration_policy='single'
                  AND assignment_id='' AND parent_run_id IS NULL))
             AND scan_id=?2 AND attempt_number=?3",
            params![lease.root_run_id, lease.scan_id, lease.attempt_number],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "source_surface_root_unavailable")?;
    let plan: Value = serde_json::from_str(&text).map_err(|_| "source_surface_plan_invalid")?;
    let source = plan["surface"] == "source"
        || target.starts_with("source:")
        || lease.target_key.starts_with("source:");
    if !source {
        return Ok(());
    }
    let phases = SourcePhaseContract::from_plan(&plan)?;
    // The legacy single-Web root above remains usable for human proposals.
    // Source has never used that identity and cannot inherit its exception.
    if !rooted
        || plan["surface"] != "source"
        || !target.starts_with("source:")
        || target != lease.target_key
        || requests != phases.model_request_limit
        || plan["targetRequestsGranted"] != 0
        || plan["hostActionsGranted"] != 0
    {
        return Err("source_surface_frozen_plan_invalid".into());
    }
    if !is_source_role(role) && !(role == AgentRole::EvidenceReviewer && phases.candidate_review) {
        return Err("source_phase_role_not_in_frozen_plan".into());
    }
    Ok(())
}

pub fn target_key(view: &SourceAnalysisView) -> String {
    format!("source:{}", view.manifest.digest())
}

/// Read sealed source material before a Coordinator exists. This grants no
/// capabilities; registration must separately verify the published runtime.
/// Never repair a missing receipt from mutable importer membership or history.
pub(crate) fn restore_materials(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<(SourceAnalysisView, AnalysisResults, String), String> {
    super::lease::require_active_attempt(connection, scan_id, attempt_number)?;
    let request = connection.query_row(
        "SELECT p.source_path,p.canonical_root,p.scan_type,p.scope_mode,p.diff_base,p.analysis_policy_version
         FROM source_scope_contracts p JOIN sentinel_scans s ON s.id=p.scan_id AND s.attempt_count=p.attempt_number
         JOIN sentinel_scan_attempts a ON a.scan_id=p.scan_id AND a.attempt_number=p.attempt_number
         WHERE p.scan_id=?1 AND p.attempt_number=?2 AND s.status='scanning' AND a.status='scanning'
         AND s.scan_type=p.scan_type AND s.source_path=p.source_path AND p.analysis_policy_version=1",
        params![scan_id,attempt_number], |r| Ok(json!({
            "sourcePath":r.get::<_,String>(0)?,"canonicalRoot":r.get::<_,String>(1)?,
            "scanType":r.get::<_,String>(2)?,"scopeMode":r.get::<_,String>(3)?,
            "diffBase":r.get::<_,String>(4)?,"analysisPolicyVersion":r.get::<_,i64>(5)?
        })),
    ).map_err(|e|format!("source_assignment_scope_unavailable:{e}"))?;
    let root = std::path::Path::new(
        request["sourcePath"]
            .as_str()
            .ok_or("source_assignment_path_missing")?,
    )
    .canonicalize()
    .map_err(|_| "source_assignment_root_unavailable")?;
    if root.to_str() != request["canonicalRoot"].as_str() || !root.is_dir() {
        return Err("source_assignment_root_changed".into());
    }
    restore_sealed_materials(connection, scan_id, attempt_number, &request)
}

/// Read only the specified attempt's sealed inputs. A later scan state or
/// moved original repository must not invalidate retained evidence. Snapshot
/// metadata, selected view bytes, scope, result receipts and plan hashes must
/// verify; this reader never authorizes dispatch, grants or repairs anything.
pub(crate) fn historical_materials(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<(SourceAnalysisView, AnalysisResults, String), String> {
    let request = connection.query_row(
        "SELECT p.source_path,p.canonical_root,p.scan_type,p.scope_mode,p.diff_base,p.analysis_policy_version
         FROM source_scope_contracts p JOIN sentinel_scans s ON s.id=p.scan_id
         JOIN sentinel_scan_attempts a ON a.scan_id=p.scan_id AND a.attempt_number=p.attempt_number
         WHERE p.scan_id=?1 AND p.attempt_number=?2 AND p.analysis_policy_version=1",
        params![scan_id,attempt_number], |r| Ok(json!({
            "sourcePath":r.get::<_,String>(0)?,"canonicalRoot":r.get::<_,String>(1)?,
            "scanType":r.get::<_,String>(2)?,"scopeMode":r.get::<_,String>(3)?,
            "diffBase":r.get::<_,String>(4)?,"analysisPolicyVersion":r.get::<_,i64>(5)?
        })),
    ).map_err(|e|format!("source_history_scope_unavailable:{e}"))?;
    restore_sealed_materials(connection, scan_id, attempt_number, &request)
}

fn restore_sealed_materials(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    request: &Value,
) -> Result<(SourceAnalysisView, AnalysisResults, String), String> {
    let snapshot = RepositorySnapshot::restore(connection, scan_id, attempt_number)?
        .ok_or("source_assignment_snapshot_missing")?;
    let manifest = AnalysisManifest::select(&snapshot, scan_id, attempt_number, request)?;
    let view = SourceAnalysisView::restore(connection, &manifest, &snapshot)?;
    view.verify_source_receipt(connection)?;
    let results = AnalysisResults::load(connection, &view)?;
    let plan = NativeSourcePlan::load(connection, scan_id, attempt_number)?
        .ok_or("source_assignment_plan_missing")?;
    if plan.scan_id != scan_id
        || plan.attempt_number != attempt_number
        || plan.backend != "native"
        || Some(plan.scan_type.as_str()) != request["scanType"].as_str()
        || plan.tree_hash != snapshot.tree_hash
        || plan.commit_sha != snapshot.commit_sha
        || plan.base_sha != snapshot.base_sha
        || plan.diff_base_state != snapshot.diff_base.as_str()
    {
        return Err("source_assignment_plan_mismatch".into());
    }
    Ok((view, results, plan.hash()))
}

fn restore(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<(SourceAnalysisView, AnalysisResults, String), String> {
    super::lease::require_executable_coordinator(connection, lease)?;
    validate_surface_role(connection, lease, AgentRole::SourceAnalyst)?;
    let (view, results, plan_hash) =
        restore_materials(connection, &lease.scan_id, lease.attempt_number)?;
    if lease.target_key != target_key(&view) {
        return Err("source_assignment_target_mismatch".into());
    }
    let root_bound:bool=connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND root_run_id=id AND parent_run_id IS NULL
         AND assignment_id='' AND role='coordinator' AND backend='native' AND orchestration_policy='multi'
         AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND plan_hash=?5 AND evidence_hash=?6
         AND cancel_requested_at='' AND status IN ('prepared','running'))",
        params![lease.root_run_id,lease.scan_id,lease.attempt_number,lease.target_key,plan_hash,results.digest()],|r|r.get(0)
    ).map_err(|e|format!("source_assignment_root_binding:{e}"))?;
    if !root_bound {
        return Err("source_assignment_root_binding_invalid".into());
    }
    Ok((view, results, plan_hash))
}

/// This fixed slice is also the dedup/replay contract. It contains identities,
/// not model-generated grants or a MAX(revision) from an unrelated evidence graph.
pub fn task_slice(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
) -> Result<Value, String> {
    if !is_source_role(role) {
        return Err("source_assignment_role_invalid".into());
    }
    let (view, results, plan_hash) = restore(connection, lease)?;
    Ok(material_task_slice(
        lease, role, &view, &results, &plan_hash,
    ))
}

/// Completion-only reconstruction. It accepts terminal coordinators, but
/// grants no execution and never reads its expected slice from assignments.
/// Uses sealed historical inputs; executable scheduling still uses the live
/// attempt, filesystem and executable-root checks in `task_slice` above.
pub(crate) fn completion_task_slices(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<Vec<(AgentRole, Value)>, String> {
    validate_surface_role(connection, lease, AgentRole::SourceAnalyst)?;
    let (view, results, plan_hash) =
        historical_materials(connection, &lease.scan_id, lease.attempt_number)?;
    let bound: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND root_run_id=id
         AND parent_run_id IS NULL AND assignment_id='' AND role='coordinator'
         AND backend='native' AND orchestration_policy='multi' AND scan_id=?2
         AND attempt_number=?3 AND target_url=?4 AND plan_hash=?5 AND evidence_hash=?6)",
            params![
                lease.root_run_id,
                lease.scan_id,
                lease.attempt_number,
                target_key(&view),
                plan_hash,
                results.digest()
            ],
            |r| r.get(0),
        )
        .map_err(|e| format!("source_completion_root_binding:{e}"))?;
    if !bound || lease.target_key != target_key(&view) {
        return Err("source_completion_root_binding_invalid".into());
    }
    Ok([AgentRole::RepoMapper, AgentRole::SourceAnalyst]
        .into_iter()
        .map(|role| {
            (
                role,
                material_task_slice(lease, role, &view, &results, &plan_hash),
            )
        })
        .collect())
}

fn material_task_slice(
    lease: &CoordinatorLease,
    role: AgentRole,
    view: &SourceAnalysisView,
    results: &AnalysisResults,
    plan_hash: &str,
) -> Value {
    json!({
        "schemaVersion":1,"surface":"source","role":role.as_str(),
        "rootRunId":lease.root_run_id,"scanId":lease.scan_id,"attemptNumber":lease.attempt_number,
        "target":lease.target_key,"sourceBinding":view.manifest.source_binding,
        "analysisDigest":view.manifest.digest(),"analysisResultsDigest":results.digest(),
        "sourcePlanHash":plan_hash,
        "candidateReceipts":results.records.iter().filter(|r|r.record.record_kind=="finding_candidate").collect::<Vec<_>>(),
        "targetRequestsGranted":0,"toolsGranted":[],
    })
}

pub fn validate_task_slice(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    slice: &Value,
    revision: i64,
) -> Result<(), String> {
    if revision != 1 || *slice != task_slice(connection, lease, role)? {
        return Err("source_assignment_contract_mismatch".into());
    }
    Ok(())
}

/// Tool work is a separate assignment contract. An initial assessment's
/// evidence.read grant must never become a grant to invoke repository tools.
pub fn tool_capabilities(role: AgentRole) -> Result<Vec<String>, String> {
    if !is_source_role(role) {
        return Err("source_tool_role_invalid".into());
    }
    Ok(crate::native_pipeline::tools::SOURCE_TOOLS
        .iter()
        .filter(|name| **name != "callgraph.get_slice")
        .filter(|name| role == AgentRole::SourceAnalyst || **name != "evidence.submit_candidate")
        .map(|name| (*name).to_string())
        .collect())
}

pub fn tool_task_slice(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    revision: i64,
) -> Result<Value, String> {
    if revision < 1 {
        return Err("source_tool_revision_invalid".into());
    }
    // Only this real source root's current graph can select a revision. A
    // caller cannot import a revision from a Web root or historical attempt.
    let current: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(revision),1) FROM agent_evidence_revisions WHERE root_run_id=?1",
            [&lease.root_run_id],
            |row| row.get(0),
        )
        .map_err(|_| "source_tool_revision_unavailable")?;
    if revision != current {
        return Err("source_tool_revision_changed".into());
    }
    let mut slice = task_slice(connection, lease, role)?;
    slice["phase"] = json!("source_tools");
    slice["evidenceRevision"] = json!(revision);
    slice["toolsGranted"] = json!(tool_capabilities(role)?);
    Ok(slice)
}

pub fn validate_scheduled_slice(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    slice: &Value,
    revision: i64,
    capabilities: &[String],
) -> Result<(), String> {
    if slice["phase"] == "source_tools" {
        if *slice != tool_task_slice(connection, lease, role, revision)?
            || capabilities != tool_capabilities(role)?
        {
            return Err("source_tool_contract_mismatch".into());
        }
        Ok(())
    } else {
        validate_task_slice(connection, lease, role, slice, revision)?;
        if capabilities.iter().any(|name| {
            !["evidence.read", "mailbox.read", "mailbox.write"].contains(&name.as_str())
        }) {
            return Err("source_assessment_tool_grant_denied".into());
        }
        Ok(())
    }
}

pub struct ToolAuthority {
    pub lease: CoordinatorLease,
    pub revision: i64,
    pub view: SourceAnalysisView,
    pub tools: Vec<String>,
}

/// The source equivalent of Web tool authorization, with no Web execution plan
/// or authentication-session fallback. Call under the tool's write transaction
/// both before and after execution (including reads).
pub fn authorize_tool(
    connection: &Connection,
    scan_id: &str,
    attempt: i64,
    target: &str,
    run_id: &str,
    name: &str,
) -> Result<ToolAuthority, String> {
    super::attempts::require_live_for_run(connection, run_id)?;
    let (lease, role, revision, text) = connection.query_row(
        "SELECT c.root_run_id,c.lease_epoch,c.fencing_token,c.lease_expires_at,r.role,a.evidence_revision,a.task_slice_json
         FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id
         JOIN agent_coordinator_leases c ON c.root_run_id=a.coordinator_run_id AND c.scan_id=r.scan_id
           AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url
         JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id
           AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane
         JOIN agent_capability_leases p ON p.assignment_id=a.id AND p.child_run_id=r.id
           AND p.root_run_id=c.root_run_id AND p.capability=?5
         WHERE r.id=?1 AND r.scan_id=?2 AND r.attempt_number=?3 AND r.target_url=?4
           AND r.root_run_id=c.root_run_id AND r.parent_run_id=c.root_run_id
           AND r.backend='native' AND r.orchestration_policy='multi'
           AND r.role IN ('repo_mapper','source_analyst') AND r.role=a.role
           AND r.status='running' AND r.cancel_requested_at='' AND r.lane='read_only_analysis'
           AND a.lane=r.lane AND a.target_key=r.target_url AND a.state='running'
           AND a.lease_epoch=c.lease_epoch AND a.fencing_token=c.fencing_token
           AND a.lease_expires_at>datetime('now','localtime')
           AND p.lease_epoch=c.lease_epoch AND p.fencing_token=c.fencing_token
           AND p.revoked_at='' AND p.lease_expires_at>datetime('now','localtime')
           AND c.lease_expires_at>datetime('now','localtime')",
        params![run_id,scan_id,attempt,target,name],
        |row| Ok((CoordinatorLease {scan_id:scan_id.into(),attempt_number:attempt,target_key:target.into(),
            root_run_id:row.get(0)?,lease_epoch:row.get(1)?,fencing_token:row.get(2)?,lease_expires_at:row.get(3)?},
            row.get::<_,String>(4)?,row.get::<_,i64>(5)?,row.get::<_,String>(6)?)),
    ).map_err(|_| "source_tool_capability_or_binding_denied")?;
    super::lease::validate_coordinator_lease(connection, &lease)?;
    let role = AgentRole::parse(&role);
    let capabilities = tool_capabilities(role)?;
    if !capabilities.iter().any(|capability| capability == name) {
        return Err("source_tool_role_denied".into());
    }
    let slice: Value = serde_json::from_str(&text).map_err(|_| "source_tool_contract_corrupt")?;
    validate_scheduled_slice(connection, &lease, role, &slice, revision, &capabilities)?;
    let (view, _, _) = restore(connection, &lease)?;
    let mut statement=connection.prepare(
        "SELECT p.capability FROM agent_capability_leases p JOIN agent_runs r ON r.id=p.child_run_id
         WHERE r.id=?1 AND p.assignment_id=r.assignment_id AND p.root_run_id=?2
           AND p.lease_epoch=?3 AND p.fencing_token=?4 AND p.revoked_at=''
           AND p.lease_expires_at>datetime('now','localtime') ORDER BY p.capability")
        .map_err(|_|"source_tool_capabilities_unavailable")?;
    let tools = statement
        .query_map(
            params![
                run_id,
                lease.root_run_id,
                lease.lease_epoch,
                lease.fencing_token
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| "source_tool_capabilities_unavailable")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "source_tool_capabilities_unavailable")?
        .into_iter()
        .filter(|name| capabilities.contains(name))
        .collect();
    Ok(ToolAuthority {
        lease,
        revision,
        view,
        tools,
    })
}

pub fn verify_assignment(
    connection: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Value, String> {
    let (slice,revision):(String,i64) = connection.query_row(
        "SELECT a.task_slice_json,a.evidence_revision FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND a.role=?4 AND a.lane='read_only_analysis'
         AND a.target_key=?5 AND a.lease_epoch=?6 AND a.fencing_token=?7
         AND r.assignment_id=a.id AND r.root_run_id=a.coordinator_run_id AND r.role=a.role AND r.lane=a.lane
         AND r.target_url=a.target_key AND r.scan_id=?8 AND r.attempt_number=?9 AND r.cancel_requested_at=''",
        params![child.assignment_id,child.run_id,lease.root_run_id,child.role.as_str(),lease.target_key,lease.lease_epoch,lease.fencing_token,lease.scan_id,lease.attempt_number],
        |r|Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|e|format!("source_assignment_binding:{e}"))?;
    let slice: Value =
        serde_json::from_str(&slice).map_err(|_| "source_assignment_contract_corrupt")?;
    validate_task_slice(connection, lease, child.role, &slice, revision)?;
    Ok(slice)
}

/// Preliminary, tool-free source assessment. Findings remain candidates and
/// the model is explicitly told that neither code execution nor review has
/// happened. Any deeper source investigation must get its own scoped tools.
pub fn assessment_input(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
) -> Result<Value, String> {
    let slice = task_slice(connection, lease, role)?;
    let (view, results, _) = restore(connection, lease)?;
    let candidates = results
        .candidates(connection)?
        .into_iter()
        .map(|candidate| json!({"receipt":candidate.receipt,"acceptedSources":candidate.sources,"record":candidate.envelope}))
        .collect::<Vec<_>>();
    super::directive::source_guidance::attach(
        connection,
        lease,
        role,
        false,
        json!({"sourceTask":slice,"selectedFiles":view.manifest.files,
        "analyzerRuns":results.runs,"candidateRecords":candidates,"coverageGaps":results.gaps,
        "limitations":["Preliminary read-only assessment of sealed metadata and analyzer candidates.",
            "No source tools, target requests, code execution or independent review have been performed by this role."]}),
    )
}

pub fn validate_request(
    connection: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    request: &Value,
) -> Result<(), String> {
    let input = request["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .filter(|message| message["role"] == "user")
        .and_then(|message| message["content"].as_str())
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    let expected = crate::agent_runtime::secrets::redact_json(&assessment_input(
        connection, lease, child.role,
    )?);
    if input != Some(expected) || request["tools"] != json!([]) {
        return Err("source_specialist_input_mismatch".into());
    }
    Ok(())
}
