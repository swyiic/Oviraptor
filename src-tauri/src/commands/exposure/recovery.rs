// §Q3 — recovery. A restarted application may only continue a run whose frozen plan still
// matches the current authorization and options; otherwise the caller must start a new run.
// Nothing here rewrites history or takes over a claim that is still beating.

use super::adapters::http::RequestMeter;
use super::orchestrator::{
    mark_stage, options_hash, run_plan, scope_hash_of, ItemOutcome, RunReport,
};
use super::scope::ExposureScope;
use super::settings::ExposureSettings;
use super::state::{shift_seconds_back, RunStatus, EXPOSURE_RULE_VERSION};
use super::work_items::{counts, reclaim_expired, DEFAULT_LEASE_SECONDS};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value as JsonValue;

#[derive(Clone, Debug)]
pub struct PlanRow {
    pub run_id: i64,
    /// §E2 the settings version and full value this round was started with; a later edit of
    /// the current configuration must not silently change an in-flight or dead round.
    pub settings_version: i64,
    pub settings_json: String,
    pub project_id: i64,
    pub attempt_number: i64,
    pub scope_version: i64,
    pub scope_hash: String,
    pub scope_receipt_json: Option<String>,
    pub options_hash: String,
    pub planned_items: i64,
    pub requests_used: i64,
    pub bytes_used: i64,
    pub max_requests: i64,
    pub max_bytes: i64,
}

/// Reads the frozen plan and refuses to answer for a run that belongs to another project or
/// that predates this structure.
pub fn load_plan(
    connection: &Connection,
    run_id: i64,
    expected_project_id: i64,
) -> Result<PlanRow, String> {
    let row: Option<PlanRow> = connection
        .query_row(
            "SELECT p.run_id,p.project_id,p.attempt_number,p.scope_version,p.scope_hash,p.options_hash,p.planned_items,p.requests_used,p.bytes_used,p.budget_requests,p.budget_bytes,COALESCE(p.settings_version,1),COALESCE(p.settings_json,'{}'),s.scope_json
             FROM exposure_run_plans p JOIN exposure_runs r ON r.id=p.run_id
             LEFT JOIN exposure_scope_receipts s ON s.run_id=p.run_id AND s.project_id=r.project_id
             WHERE p.run_id=?1 AND r.project_id=?2",
            params![run_id, expected_project_id],
            |row| {
                Ok(PlanRow {
                    run_id: row.get(0)?,
                    project_id: row.get(1)?,
                    attempt_number: row.get(2)?,
                    scope_version: row.get(3)?,
                    scope_hash: row.get(4)?,
                    options_hash: row.get(5)?,
                    planned_items: row.get(6)?,
                    requests_used: row.get(7)?,
                    bytes_used: row.get(8)?,
                    max_requests: row.get(9)?,
                    max_bytes: row.get(10)?,
                    settings_version: row.get(11)?,
                    settings_json: row.get(12)?,
                    scope_receipt_json: row.get(13)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    row.ok_or_else(|| {
        format!("exposure_plan_missing:任务 {run_id} 没有冻结计划（历史任务或跨项目访问），不能作为新合同恢复")
    })
}

/// The settings the round was actually started with, so a resume keeps its own budget.
/// G1-0: the production reader; see `ExposureSettings::from_frozen_strict`.
pub fn frozen_settings_strict(plan_row: &PlanRow) -> Result<ExposureSettings, String> {
    ExposureSettings::from_frozen_strict(
        plan_row.project_id,
        plan_row.settings_version,
        &plan_row.settings_json,
    )
}

/// Lenient legacy reader kept for tests that inspect an already-frozen value.
pub fn frozen_settings(plan_row: &PlanRow) -> ExposureSettings {
    ExposureSettings::from_frozen(
        plan_row.project_id,
        plan_row.settings_version,
        &plan_row.settings_json,
    )
}

impl PlanRow {
    /// §E2 rounds started before the settings contract have no frozen value at all. Their
    /// options hash cannot be re-derived, so resuming them would quietly hand an old round a
    /// budget it never recorded; such a round has to be started again instead.
    pub fn settings_frozen(&self) -> bool {
        serde_json::from_str::<JsonValue>(&self.settings_json)
            .ok()
            .and_then(|row| row.as_object().map(|map| !map.is_empty()))
            .unwrap_or(false)
    }
}

/// Refuses reuse when the authorization scope or the collection options moved, so a resumed
/// round cannot quietly point an old plan at new targets or new budgets.
pub fn verify_frozen(
    plan_row: &PlanRow,
    scope: &ExposureScope,
    settings: &JsonValue,
) -> Result<(), String> {
    if !plan_row.settings_frozen() {
        return Err(
            "exposure_plan_settings_unfrozen：这一轮启动时没有冻结采集配置，无法按当前合同确认它原来的额度；\
             请另起一轮，已花额度与旧结果保留"
                .to_string(),
        );
    }
    let identity = if plan_row
        .scope_hash
        .starts_with(super::orchestrator::SCOPE_HASH_PREFIX)
    {
        Some(plan_row.scope_hash.clone())
    } else if plan_row.scope_hash.starts_with("s2:") {
        plan_row.scope_receipt_json.as_deref().and_then(|raw| {
            super::orchestrator::authenticated_legacy_scope_identity(raw, &plan_row.scope_hash)
        })
    } else {
        None
    };
    let Some(identity) = identity else {
        return Err(
            "exposure_plan_scope_hash_legacy：这一轮的范围指纹含签发时间（G1 之前写入），无法核对是否仍是同一范围；\
             请另起一轮，已花额度与旧结果保留"
                .to_string(),
        );
    };
    let mut moved = Vec::new();
    if identity != scope_hash_of(scope) {
        moved.push("授权范围或归属版本已变化");
    }
    if plan_row.options_hash != options_hash(settings) {
        moved.push("冻结计划与记录的配置不一致");
    }
    if moved.is_empty() {
        return Ok(());
    }
    Err(format!(
        "exposure_plan_frozen_mismatch：{}；请另起新一轮采集，旧结果保留",
        moved.join("，")
    ))
}

pub fn live_claims(
    connection: &Connection,
    run_id: i64,
    lease_seconds: i64,
) -> Result<i64, String> {
    let cutoff = shift_seconds_back(lease_seconds);
    connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_work_items
             WHERE run_id=?1 AND status IN ('claimed','in_progress') AND heartbeat_at<>'' AND heartbeat_at>=?2",
            params![run_id, cutoff],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

/// Read-only recovery hint. Freshly admitted runs without a first claim must not
/// be mistaken for abandoned work; both activity age and claim leases matter.
pub fn stale_without_live_claims(
    connection: &Connection,
    run_id: i64,
    lease_seconds: i64,
) -> Result<bool, String> {
    if live_claims(connection, run_id, lease_seconds)? > 0 {
        return Ok(false);
    }
    let cutoff = shift_seconds_back(lease_seconds);
    connection.query_row("SELECT EXISTS(SELECT 1 FROM exposure_runs r WHERE r.id=?1 AND r.status IN ('queued','running') AND COALESCE(NULLIF(r.started_at,''),r.created_at)<?2 AND NOT EXISTS(SELECT 1 FROM exposure_work_items w WHERE w.run_id=r.id AND w.updated_at>=?2))",params![run_id,cutoff],|r|r.get(0)).map_err(|e|e.to_string())
}

/// Marks a run interrupted only when nothing in it can still be proven to be working. A run
/// with a fresh claim is left exactly as it is, because the owning thread may still be alive.
pub fn mark_interrupted_if_dead(
    connection: &Connection,
    run_id: i64,
    lease_seconds: i64,
) -> Result<bool, String> {
    let active: Option<String> = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if !matches!(active.as_deref(), Some("queued") | Some("running")) {
        return Ok(false);
    }
    if live_claims(connection, run_id, lease_seconds)? > 0 {
        return Ok(false);
    }
    let orphans = reclaim_expired(connection, run_id, lease_seconds)?;
    let open = counts(connection, run_id)?.open;
    connection
        .execute(
            "UPDATE exposure_runs SET status='interrupted',stage='interrupted',error=?2,completed_at=?3 WHERE id=?1 AND status IN ('queued','running')",
            params![
                run_id,
                format!(
                    "{{\"status\":\"interrupted\",\"reasons\":[\"进程已退出，{orphans} 个认领成为孤儿，{open} 个工作项未到终态\"],\"gaps\":[\"restart_orphans:{orphans}\",\"work_items_open:{open}\"],\"ruleVersion\":\"{EXPOSURE_RULE_VERSION}\"}}"
                ),
                super::state::utc_now()
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(true)
}

/// A run without a plan row is a pre-Q3 round. It keeps its own provenance label instead of
/// being back-filled with work items that were never planned.
pub fn legacy_projection(connection: &Connection, run_id: i64) -> Result<Option<String>, String> {
    let planned: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_run_plans WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if planned > 0 {
        return Ok(None);
    }
    let status: Option<String> = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(status.map(|value| {
        format!("legacy_passive_web(状态 {value})：该轮没有工作项台账，覆盖范围以当时的有限采集为准，不能按新合同解释")
    }))
}

/// Continues the frozen plan: already-closed items are not re-fetched, and the round keeps
/// charging against the budget it already spent.
pub fn resume(
    connection: &Connection,
    run_id: i64,
    expected_project_id: i64,
    owner: &str,
    settings: &JsonValue,
    executor: &mut dyn FnMut(&super::work_items::WorkItem) -> Result<ItemOutcome, String>,
) -> Result<RunReport, String> {
    let plan_row = load_plan(connection, run_id, expected_project_id)?;
    if plan_row.project_id != expected_project_id {
        return Err("exposure_scope_mismatch".to_string());
    }
    let scope = ExposureScope::load(connection, expected_project_id, settings)
        .map_err(|denial| format!("{}:{}", denial.code, denial.message))?;
    verify_frozen(&plan_row, &scope, settings)?;
    if !prepare_resume(connection, run_id, expected_project_id)? {
        return Err("exposure_resume_already_admitted:任务已由其他执行器接续".into());
    }
    mark_stage(connection, run_id, "resuming", "");
    run_plan(connection, run_id, owner, executor)
}

/// Shared read-only recovery facts for desktop and Agent callers. A saved cursor is a
/// continuation of the original frozen plan, never permission to reset that plan's budget.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeView {
    pub resumable: bool,
    pub pending_items: i64,
    pub pending_urls: i64,
    pub requests_remaining: i64,
    pub bytes_remaining: i64,
    pub block_reason: String,
    pub paused_reason: String,
    pub cursor_missing_items: i64,
    pub cursor_invalid_items: i64,
    pub usage_is_cumulative: bool,
    pub unknown_urls: i64,
    pub deferred_discovery_count: i64,
    pub unknown_reserved_bytes: i64,
}

pub fn resume_view(
    connection: &Connection,
    run_id: i64,
    project_id: i64,
) -> Result<ResumeView, String> {
    let (status, cancellation): (String, bool) = connection
        .query_row(
            "SELECT status,cancel_requested<>0 FROM exposure_runs WHERE id=?1 AND project_id=?2",
            params![run_id, project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "exposure_run_project_mismatch:任务不属于当前项目或不存在".to_string())?;
    let mut view = ResumeView {
        usage_is_cumulative: true,
        ..Default::default()
    };
    if cancellation || status == "cancelled" {
        view.paused_reason = "用户已暂停；已保存的观察、队列和累计用量保留".into();
    } else if status == "interrupted" {
        view.paused_reason = "执行器退出；从最后落盘的队列继续，未知在途请求不重放".into();
    }
    let plan = match load_plan(connection, run_id, project_id) {
        Ok(plan) => plan,
        Err(_) => {
            view.block_reason = "legacy_plan_missing".into();
            return Ok(view);
        }
    };
    view.requests_remaining = plan.max_requests.saturating_sub(plan.requests_used).max(0);
    view.bytes_remaining = plan.max_bytes.saturating_sub(plan.bytes_used).max(0);
    let frozen = match frozen_settings_strict(&plan) {
        Ok(value) => value.to_settings_json(),
        Err(_) => {
            view.block_reason = "frozen_settings_invalid".into();
            return Ok(view);
        }
    };
    let scope = match ExposureScope::load(connection, project_id, &frozen) {
        Ok(scope) => scope,
        Err(_) => {
            view.block_reason = "authorization_scope_unavailable".into();
            return Ok(view);
        }
    };
    let frozen_valid = verify_frozen(&plan, &scope, &frozen).is_ok();
    for item in super::work_items::read_run_items(connection, run_id, project_id)? {
        let cursor = super::work_items::load_cursor(connection, item.id)?;
        if item.kind == super::orchestrator::KIND_PASSIVE_WEB && !cursor.is_empty() {
            if let Ok((unknown, deferred, unknown_reserved)) =
                super::adapters::passive_web::cursor_progress_counts(&cursor, &item.origin)
            {
                view.unknown_urls = view.unknown_urls.saturating_add(unknown as i64);
                view.deferred_discovery_count = view
                    .deferred_discovery_count
                    .saturating_add(deferred as i64);
                view.unknown_reserved_bytes =
                    view.unknown_reserved_bytes.saturating_add(unknown_reserved);
            }
        }
        if !matches!(
            item.status,
            super::state::WorkItemStatus::Pending
                | super::state::WorkItemStatus::Orphaned
                | super::state::WorkItemStatus::Claimed
                | super::state::WorkItemStatus::InProgress
                | super::state::WorkItemStatus::Cancelled
                | super::state::WorkItemStatus::BlockedQuota
        ) {
            continue;
        }
        if item.kind != super::orchestrator::KIND_PASSIVE_WEB {
            // Keep existing source recovery semantics; terminal quota sources do not
            // become fresh queries merely because another origin has a cursor.
            if item.status != super::state::WorkItemStatus::BlockedQuota {
                view.pending_items += 1;
            }
            continue;
        }
        if cursor.is_empty() {
            // A never-claimed origin is a durable, untouched plan entry. Once a legacy
            // walker sent requests, its missing frontier cannot be reconstructed by guess.
            if item.attempts == 0 && item.requests == 0 && item.observations == 0 {
                view.pending_items += 1;
                view.pending_urls += 1;
            } else {
                view.cursor_missing_items += 1;
            }
            continue;
        }
        match super::adapters::passive_web::validate_cursor_for(&cursor, &item, &scope) {
            Ok(pending) if pending > 0 => {
                view.pending_items += 1;
                view.pending_urls += pending as i64;
            }
            Ok(_) => {}
            Err(_) => view.cursor_invalid_items += 1,
        }
    }
    view.block_reason = if !frozen_valid {
        "frozen_scope_changed"
    } else if live_claims(connection, run_id, DEFAULT_LEASE_SECONDS)? > 0 {
        "live_claim"
    } else if matches!(status.as_str(), "queued" | "running")
        && !stale_without_live_claims(connection, run_id, DEFAULT_LEASE_SECONDS)?
    {
        "executor_active"
    } else if !matches!(
        status.as_str(),
        "interrupted" | "cancelled" | "partial" | "queued" | "running"
    ) {
        "terminal_state"
    } else if view.requests_remaining == 0 {
        "request_budget_exhausted"
    } else if view.bytes_remaining == 0 {
        "byte_budget_exhausted"
    } else if view.cursor_invalid_items > 0 {
        "cursor_invalid"
    } else if view.pending_items == 0 && view.cursor_missing_items > 0 {
        "legacy_checkpoint_missing"
    } else if view.pending_items == 0 {
        "no_pending_work"
    } else {
        ""
    }
    .into();
    view.resumable = view.block_reason.is_empty();
    Ok(view)
}

/// One caller admits a stopped round back to the queue. This only reopens the precise
/// unfinished cursor/fresh items; completed URLs and cumulative usage remain untouched.
pub fn prepare_resume(
    connection: &Connection,
    run_id: i64,
    project_id: i64,
) -> Result<bool, String> {
    let transaction = super::transaction::ImmediateTx::begin(connection)?;
    let c = transaction.connection();
    let view = resume_view(c, run_id, project_id)?;
    if !view.resumable {
        return Err(format!("exposure_resume_{}:不能继续；剩余请求 {}，剩余字节 {}，待处理 URL {}。已保存结果与累计用量保留", view.block_reason, view.requests_remaining, view.bytes_remaining, view.pending_urls));
    }
    // Cursor validation above happened under this same writer transaction, so a second
    // resume cannot race between the decision and admission. Live leases are never stolen.
    let changed = c.execute(
        "UPDATE exposure_runs SET status='queued',stage='resuming',cancel_requested=0,completed_at='',error='',current_source=''
         WHERE id=?1 AND project_id=?2 AND status IN ('interrupted','cancelled','partial')",
        params![run_id, project_id],
    ).map_err(|error| format!("exposure_resume_admission_failed:{error}"))?;
    if changed == 1 {
        let plan = load_plan(c, run_id, project_id)?;
        let settings = frozen_settings_strict(&plan)?.to_settings_json();
        let scope = ExposureScope::load(c, project_id, &settings)
            .map_err(|denial| format!("{}:{}", denial.code, denial.message))?;
        for item in super::work_items::read_run_items(c, run_id, project_id)? {
            if !matches!(
                item.status,
                super::state::WorkItemStatus::Cancelled
                    | super::state::WorkItemStatus::BlockedQuota
            ) {
                continue;
            }
            let can_reopen = if item.kind == super::orchestrator::KIND_PASSIVE_WEB {
                let cursor = super::work_items::load_cursor(c, item.id)?;
                if cursor.is_empty() {
                    item.attempts == 0 && item.requests == 0 && item.observations == 0
                } else {
                    super::adapters::passive_web::validate_cursor_for(&cursor, &item, &scope)? > 0
                }
            } else {
                item.status == super::state::WorkItemStatus::Cancelled
            };
            if can_reopen {
                c.execute("UPDATE exposure_work_items SET status='pending',owner='',heartbeat_at='',finished_at='',updated_at=?2
                    WHERE id=?1 AND run_id=?3 AND project_id=?4 AND status IN ('cancelled','blocked_quota')",
                    params![item.id, super::state::utc_now(), run_id, project_id])
                    .map_err(|error| error.to_string())?;
            }
        }
        // Stale claimed rows can exist after a cold shutdown, but fresh ones were refused.
        reclaim_expired(c, run_id, DEFAULT_LEASE_SECONDS)?;
    }
    transaction.commit()?;
    Ok(changed == 1)
}

/// The meter a resumed round must start from: the plan's own spend, not a fresh budget.
pub fn meter_for(plan_row: &PlanRow) -> RequestMeter {
    RequestMeter {
        requests_used: plan_row.requests_used,
        bytes_used: plan_row.bytes_used,
        max_requests: plan_row.max_requests,
        max_bytes: plan_row.max_bytes,
    }
}

/// True when the run's own status says it stopped without closing its plan.
pub fn is_interrupted(status: RunStatus) -> bool {
    status == RunStatus::Interrupted
}

pub fn default_lease() -> i64 {
    DEFAULT_LEASE_SECONDS
}
