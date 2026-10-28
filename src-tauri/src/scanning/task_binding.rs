//! Persistent session-to-task binding and idempotent commands.
//! A binding records where results return. It does not create a scan,
//! grant execution, or move an event onto another conversation.
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS nest_session_task_bindings(
    binding_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    project_id INTEGER NOT NULL,
    task_id TEXT NOT NULL,
    binding_version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(conversation_id, project_id, task_id)
);
CREATE TABLE IF NOT EXISTS nest_task_commands(
    command_id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL,
    binding_id TEXT NOT NULL,
    project_id INTEGER NOT NULL,
    task_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    state TEXT NOT NULL,
    failure_reason TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(project_id, idempotency_key)
);
CREATE TABLE IF NOT EXISTS nest_result_deliveries(
    delivery_id TEXT PRIMARY KEY,
    command_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(command_id, conversation_id)
);
CREATE TABLE IF NOT EXISTS nest_host_outbox(
    event_id TEXT PRIMARY KEY,
    project_id INTEGER NOT NULL,
    host_conversation_id TEXT NOT NULL,
    task_id TEXT NOT NULL DEFAULT '',
    command_id TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    evidence_json TEXT NOT NULL DEFAULT '{}',
    state TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    acked_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_nest_host_outbox_pending ON nest_host_outbox(project_id, state, created_at);";

fn ensure_binding_store(c: &rusqlite::Connection) -> Result<(), String> {
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    // Tables created before binding_version existed keep their rows. The default
    // is the first version; a missing column is not a reason to reject callers
    // that never sent a version.
    let _ = c.execute(
        "ALTER TABLE nest_session_task_bindings ADD COLUMN binding_version INTEGER NOT NULL DEFAULT 1",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE nest_task_commands ADD COLUMN execution_actor TEXT NOT NULL DEFAULT ''",
        [],
    );
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS nest_execution_controls(
            control_id TEXT PRIMARY KEY,
            command_id TEXT NOT NULL UNIQUE,
            project_id INTEGER NOT NULL,
            task_id TEXT NOT NULL,
            attempt_number INTEGER NOT NULL,
            kind TEXT NOT NULL,
            state TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// Conversations owned by the Agent Host are bound as `host:<id>`. Only the
/// Host writes that form, from the conversation of the verified run that made
/// the bridge call; a model-supplied conversation id never reaches it.
pub(crate) const HOST_CONVERSATION_PREFIX: &str = "host:";

pub(crate) fn host_conversation(conversation: &str) -> Option<&str> {
    conversation.strip_prefix(HOST_CONVERSATION_PREFIX)
}

/// The Host checks its own conversation when it takes the event from the
/// outbox; a desktop conversation must still exist in this project.
fn bound_conversation_available(c: &rusqlite::Connection, project: i64, conversation: &str) -> i64 {
    if host_conversation(conversation).is_some() {
        return 1;
    }
    c.query_row(
        "SELECT COUNT(*) FROM conversations WHERE id=?1 AND project_id=?2 AND archived=0",
        params![conversation, project], |row| row.get(0),
    ).unwrap_or(0)
}

fn delivery_event_id(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

/// Posts one note to the bound conversation under an event id derived from the
/// delivery slot. A retry after a crash between this write and the delivery
/// state update finds the same id and does not post a second message.
fn post_bound_message(
    c: &rusqlite::Connection,
    project: i64,
    conversation: &str,
    slot: &str,
    command: &str,
    task: &str,
    note: &str,
) -> Result<(), String> {
    let event = delivery_event_id(&[slot, command, conversation]);
    let written = if let Some(host) = host_conversation(conversation) {
        c.execute(
            "INSERT OR IGNORE INTO nest_host_outbox(event_id,project_id,host_conversation_id,task_id,command_id,kind,text) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![event, project, host, task, command, slot, note],
        )
    } else {
        c.execute(
            "INSERT OR IGNORE INTO conversation_messages(id,conversation_id,sequence,role,content,created_at) VALUES(?1,?2,COALESCE((SELECT MAX(sequence) FROM conversation_messages WHERE conversation_id=?2),0)+1,'assistant',?3,datetime('now'))",
            params![event, conversation, note],
        )
    };
    written.map(|_| ()).map_err(|_| "result_delivery_not_posted".to_string())
}

/// One result event per Host binding and settled attempt state. The event id
/// is derived from that state, so it is written once however often the
/// outbox is reconciled, including after a restart.
fn enqueue_bound_task_results(c: &rusqlite::Connection, project: i64) -> Result<(), String> {
    let bindings: Vec<(String, String, String)> = {
        let mut statement = c.prepare(
            "SELECT binding_id, conversation_id, task_id FROM nest_session_task_bindings WHERE project_id=?1 AND conversation_id LIKE 'host:%'",
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([project], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for (binding, conversation, task) in bindings {
        let Some(host) = host_conversation(&conversation) else { continue };
        let Some((status, attempt)) = c.query_row(
            "SELECT status, attempt_count FROM sentinel_scans WHERE id=?1 AND project_id=?2",
            params![task, project], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        ).optional().map_err(|error| error.to_string())? else { continue };
        if attempt < 1 || !matches!(status.as_str(), "completed" | "partial" | "recon_only" | "failed" | "cancelled" | "paused") {
            continue;
        }
        let event = delivery_event_id(&["task_result", &binding, &attempt.to_string(), &status]);
        let exists: bool = c.query_row("SELECT 1 FROM nest_host_outbox WHERE event_id=?1", [&event], |_| Ok(true))
            .optional().map_err(|error| error.to_string())?.unwrap_or(false);
        if exists { continue; }
        let mut counts = serde_json::Map::new();
        {
            let mut statement = c.prepare("SELECT verdict, COUNT(*) FROM sentinel_validations WHERE scan_id=?1 GROUP BY verdict")
                .map_err(|error| error.to_string())?;
            let rows = statement.query_map([&task], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
                .map_err(|error| error.to_string())?;
            for row in rows {
                let (verdict, count) = row.map_err(|error| error.to_string())?;
                counts.insert(verdict, json!(count));
            }
        }
        let validation_ids: Vec<i64> = {
            let mut statement = c.prepare(
                "SELECT id FROM sentinel_validations WHERE scan_id=?1 AND verdict NOT IN ('pending','rejected','') ORDER BY id LIMIT 5",
            ).map_err(|error| error.to_string())?;
            let rows = statement.query_map([&task], |row| row.get(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            rows
        };
        let evidence = json!({
            "scanId": task, "attemptNumber": attempt, "status": status,
            "validationCounts": counts, "validationIds": validation_ids,
            "readWith": "scan_results",
        });
        let refs = if validation_ids.is_empty() {
            "暂无已判定的验证记录".to_string()
        } else {
            validation_ids.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join("、")
        };
        let text = crate::agent_runtime::secrets::redact_text_with(&format!(
            "扫描任务 {task} 第 {attempt} 次尝试已到达状态 {status}。{progress}验证记录分布：{counts}。证据引用：{refs}（用 scan_results 按 taskId 读取）。这是执行事件，不代表已经独立复核。",
            progress = target_progress_phrase(c, &task),
            counts = Value::Object(counts.clone()),
        ), None);
        c.execute(
            "INSERT OR IGNORE INTO nest_host_outbox(event_id,project_id,host_conversation_id,task_id,kind,text,evidence_json) VALUES(?1,?2,?3,?4,'task_result',?5,?6)",
            params![event, project, host, task, text, evidence.to_string()],
        ).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Pending Host deliveries for one project, after reconciling result events.
/// Reading does not change any state: the Host acknowledges what it stored.
pub(crate) fn host_outbox_pull(c: &rusqlite::Connection, project: i64, limit: i64) -> Result<Value, String> {
    ensure_binding_store(c)?;
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    enqueue_bound_task_results(c, project)?;
    let mut statement = c.prepare(
        "SELECT event_id, host_conversation_id, task_id, command_id, kind, text, evidence_json FROM nest_host_outbox WHERE project_id=?1 AND state='pending' ORDER BY created_at, rowid LIMIT ?2",
    ).map_err(|error| error.to_string())?;
    let deliveries = statement.query_map(params![project, limit.clamp(1, 64)], |row| {
        let evidence: String = row.get(6)?;
        Ok(json!({
            "eventId": row.get::<_, String>(0)?, "conversationId": row.get::<_, String>(1)?,
            "taskId": row.get::<_, String>(2)?, "commandId": row.get::<_, String>(3)?,
            "kind": row.get::<_, String>(4)?, "text": row.get::<_, String>(5)?,
            "evidence": serde_json::from_str::<Value>(&evidence).unwrap_or(Value::Null),
        }))
    }).map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(json!({"projectId": project, "deliveries": deliveries}))
}

/// The Host reports each event it stored (`delivered`) or refused because the
/// conversation no longer exists in that project (`rejected`). Unknown or
/// already settled ids are reported back unchanged.
pub(crate) fn host_outbox_ack(c: &rusqlite::Connection, project: i64, acks: &[(String, String)]) -> Result<Value, String> {
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    let tx = c.unchecked_transaction().map_err(|error| error.to_string())?;
    let mut settled = Vec::new();
    for (event, outcome) in acks {
        if !matches!(outcome.as_str(), "delivered" | "rejected") {
            return Err("scan_arguments_invalid".into());
        }
        let changed = tx.execute(
            "UPDATE nest_host_outbox SET state=?3, acked_at=datetime('now') WHERE event_id=?1 AND project_id=?2 AND state='pending'",
            params![event, project, outcome],
        ).map_err(|error| error.to_string())?;
        settled.push(json!({"eventId": event, "changed": changed == 1}));
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(json!({"projectId": project, "acks": settled}))
}

/// A page write may name the binding version and attempt it read. Absent fields
/// stay compatible with callers that never had them. A carried value must match
/// the stored row; an older value refuses the write.
pub(crate) fn refuse_stale_page_write(
    c: &rusqlite::Connection,
    project: i64,
    task: &str,
    binding_version: Option<i64>,
    attempt: Option<i64>,
) -> Result<(), String> {
    if binding_version.is_none() && attempt.is_none() {
        return Ok(());
    }
    if let Some(expected) = attempt {
        let stored: i64 = c.query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1 AND project_id=?2",
            params![task, project], |row| row.get(0),
        ).map_err(|_| "scan_task_unavailable".to_string())?;
        if expected != stored {
            return Err("stale_page_write".into());
        }
    }
    if let Some(expected) = binding_version {
        let stored = match c.query_row(
            "SELECT binding_version FROM nest_session_task_bindings WHERE project_id=?1 AND task_id=?2 ORDER BY binding_version DESC LIMIT 1",
            params![project, task], |row| row.get::<_, i64>(0),
        ) {
            Ok(stored) => stored,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Err("stale_page_write".into()),
            Err(error) => {
                let message = error.to_string();
                if message.contains("no such table") || message.contains("no such column") {
                    return Err("stale_page_write".into());
                }
                return Err(message);
            }
        };
        if expected != stored {
            return Err("stale_page_write".into());
        }
    }
    Ok(())
}

pub(crate) fn current_binding_version(c: &rusqlite::Connection, project: i64, task: &str) -> Option<i64> {
    c.query_row(
        "SELECT binding_version FROM nest_session_task_bindings WHERE project_id=?1 AND task_id=?2 ORDER BY binding_version DESC LIMIT 1",
        params![project, task], |row| row.get(0),
    ).ok()
}

pub(crate) fn require_existing_binding(c: &rusqlite::Connection, project: i64, conversation: &str, task: &str) -> Result<(), String> {
    ensure_binding_store(c)?;
    let found: i64 = c.query_row(
        "SELECT COUNT(*) FROM nest_session_task_bindings WHERE conversation_id=?1 AND project_id=?2 AND task_id=?3",
        params![conversation, project, task], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if found != 1 { return Err("task_binding_missing".into()); }
    Ok(())
}

pub(crate) fn bind(c: &rusqlite::Connection, project: i64, conversation: &str, task: &str) -> Result<Value, String> {
    ensure_binding_store(c)?;
    // A Host conversation was checked against this project by the Host before
    // it injected the id; desktop conversations are checked here.
    if host_conversation(conversation).is_none() {
        let conversation_project: i64 = c.query_row(
            "SELECT project_id FROM conversations WHERE id=?1 AND archived=0",
            [conversation], |row| row.get(0),
        ).map_err(|_| "conversation_unavailable".to_string())?;
        if conversation_project != project { return Err("conversation_project_mismatch".into()); }
    }
    let task_project: i64 = c.query_row(
        "SELECT project_id FROM sentinel_scans WHERE id=?1",
        [task], |row| row.get(0),
    ).map_err(|_| "scan_task_unavailable".to_string())?;
    if task_project != project { return Err("scan_project_mismatch".into()); }
    let scans_before: i64 = c.query_row("SELECT COUNT(*) FROM sentinel_scans", [], |row| row.get(0)).map_err(|error| error.to_string())?;
    if let Some(existing) = c.query_row(
        "SELECT binding_id FROM nest_session_task_bindings WHERE conversation_id=?1 AND project_id=?2 AND task_id=?3",
        params![conversation, project, task], |row| row.get::<_, String>(0),
    ).optional().map_err(|error| error.to_string())? {
        return Ok(json!({"bindingId": existing, "created": false, "taskId": task, "conversationId": conversation}));
    }
    let id = uuid::Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO nest_session_task_bindings(binding_id,conversation_id,project_id,task_id) VALUES(?1,?2,?3,?4)",
        params![id, conversation, project, task],
    ).map_err(|error| error.to_string())?;
    let scans_after: i64 = c.query_row("SELECT COUNT(*) FROM sentinel_scans", [], |row| row.get(0)).map_err(|error| error.to_string())?;
    if scans_after != scans_before { return Err("binding_created_a_scan".into()); }
    Ok(json!({"bindingId": id, "created": true, "taskId": task, "conversationId": conversation}))
}

fn ensure_command_scope(c: &rusqlite::Connection) -> Result<(), String> {
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    let _ = c.execute("ALTER TABLE nest_task_commands ADD COLUMN payload_text TEXT NOT NULL DEFAULT ''", []);
    let _ = c.execute("ALTER TABLE nest_task_commands ADD COLUMN plan_boundary TEXT NOT NULL DEFAULT ''", []);
    let _ = c.execute("ALTER TABLE nest_task_commands ADD COLUMN command_version INTEGER NOT NULL DEFAULT 0", []);
    let _ = c.execute("ALTER TABLE nest_task_commands ADD COLUMN decision_reason TEXT NOT NULL DEFAULT ''", []);
    Ok(())
}

fn reanalyze_fields(value: &mut Value, kind: &str, scope: &str) {
    if kind != "reanalyze" { return; }
    value["automaticVerification"] = json!("unavailable");
    value["reviewClaimed"] = json!(false);
    value["executionClaimed"] = json!(false);
    value["analysisScope"] = json!(scope);
}

const SUPPLEMENT_CLOSED_WORK_UNCHANGED: &str = "supplement_closed_work_unchanged";
/// The quota gate allowed a cap change, and no ledger function can raise the
/// original allowance without minting a new one. The cap stays where it was.
const GATE_ALLOWED_CAP_NOT_RAISED: &str = "gate_allowed_cap_not_raised";

fn supplement_fields(value: &mut Value, kind: &str, boundary: &str, command_attempt: i64, current_attempt: i64) {
    if kind != "supplement" { return; }
    value["planBoundary"] = json!(boundary);
    value["effectiveFromAttempt"] = json!(command_attempt);
    value["appliesToCurrentAttempt"] = json!(command_attempt == current_attempt);
    value["rewritesCompletedWork"] = json!(false);
    value["scopeExpanded"] = json!(false);
    value["budgetChanged"] = json!(false);
}

/// The boundary is a plan version something else already wrote, or the schedule
/// mode already stored for this attempt plus that attempt number. Missing or
/// any other mode stays on the legacy pipeline. This does not mint a version.
fn supplement_boundary(c: &rusqlite::Connection, task: &str, attempt: i64) -> String {
    let version = c.query_row(
        "SELECT plan_hash FROM native_scan_plans WHERE scan_id=?1 AND attempt_number=?2",
        params![task, attempt], |row| row.get::<_, String>(0),
    ).optional().ok().flatten().filter(|hash| !hash.is_empty());
    if let Some(version) = version {
        return version;
    }
    let mode = stored_schedule_mode(c, task, attempt);
    format!("{mode}:{attempt}")
}

fn stored_schedule_mode(c: &rusqlite::Connection, task: &str, attempt: i64) -> &'static str {
    // A stored plan row decides. Only an explicit homogeneous_workers value
    // leaves the legacy pipeline. A missing field on that row is still legacy.
    match c.query_row(
        "SELECT json_extract(plan_json,'$.scheduling_model') FROM native_scan_plans WHERE scan_id=?1 AND attempt_number=?2",
        params![task, attempt], |row| row.get::<_, Option<String>>(0),
    ).optional() {
        Ok(Some(Some(model))) if model == "homogeneous_workers" => return "homogeneous_workers",
        Ok(Some(_)) => return "legacy_role_pipeline",
        _ => {}
    }
    let policy = c.query_row(
        "SELECT json_extract(policy_json,'$.scheduling_model') FROM sentinel_scan_contexts WHERE scan_id=?1",
        [task], |row| row.get::<_, Option<String>>(0),
    ).optional().ok().flatten().flatten();
    if policy.as_deref() == Some("homogeneous_workers") {
        "homogeneous_workers"
    } else {
        "legacy_role_pipeline"
    }
}

fn target_status_is_closed(status: &str) -> bool {
    matches!(status, "completed" | "recon_only" | "manual_review" | "fuse_excluded" | "cancelled" | "failed" | "limited")
}

/// True only when every stored target is already closed. No rows is not closed work.
fn supplement_would_only_rewrite_closed_work(c: &rusqlite::Connection, task: &str) -> Result<bool, String> {
    let mut statement = match c.prepare("SELECT status FROM sentinel_targets WHERE scan_id=?1") {
        Ok(statement) => statement,
        Err(error) => {
            let message = error.to_string();
            if message.contains("no such table") { return Ok(false); }
            return Err(message);
        }
    };
    let rows = statement.query_map([task], |row| row.get::<_, String>(0)).map_err(|error| error.to_string())?;
    let mut saw_target = false;
    for row in rows {
        saw_target = true;
        if !target_status_is_closed(&row.map_err(|error| error.to_string())?) {
            return Ok(false);
        }
    }
    Ok(saw_target)
}

fn supplement_target_key(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// A free-text note does not name a target set or a model cap. A JSON object
/// does, and only then can the supplement ask for a wider set or a new cap.
fn supplement_scope_budget_request(c: &rusqlite::Connection, task: &str, payload: &str) -> Result<Option<(Vec<String>, bool)>, String> {
    let Ok(value) = serde_json::from_str::<Value>(payload) else { return Ok(None); };
    let Some(object) = value.as_object() else { return Ok(None); };
    if !object.contains_key("targets") && !object.contains_key("maxModelRequests") && !object.contains_key("maxModelTokens") {
        return Ok(None);
    }
    let current = match c.prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1") {
        Ok(mut statement) => {
            let rows = statement.query_map([task], |row| row.get::<_, String>(0)).map_err(|error| error.to_string())?;
            let mut keys = std::collections::BTreeSet::new();
            for row in rows {
                keys.insert(supplement_target_key(&row.map_err(|error| error.to_string())?));
            }
            keys
        }
        Err(error) => {
            let message = error.to_string();
            if message.contains("no such table") {
                std::collections::BTreeSet::new()
            } else {
                return Err(message);
            }
        }
    };
    let mut outside = Vec::new();
    if let Some(targets) = object.get("targets").and_then(Value::as_array) {
        for target in targets {
            let Some(url) = target.as_str().map(str::trim).filter(|url| !url.is_empty()) else { continue; };
            let key = supplement_target_key(url);
            if !current.contains(&key) && !outside.iter().any(|saved: &String| saved == &key) {
                outside.push(url.to_string());
            }
        }
    }
    let budget_changed = ["maxModelRequests", "maxModelTokens"].into_iter().any(|field| {
        let Some(requested) = object.get(field).and_then(Value::as_i64) else { return false; };
        stored_model_limit(c, task, field).unwrap_or(None) != Some(requested)
    });
    if outside.is_empty() && !budget_changed { return Ok(None); }
    Ok(Some((outside, budget_changed)))
}

fn stored_model_limit(c: &rusqlite::Connection, task: &str, field: &str) -> Result<Option<i64>, String> {
    let sql = match field {
        "maxModelRequests" => "SELECT CAST(json_extract(policy_json,'$.maxModelRequests') AS INTEGER) FROM sentinel_scan_contexts WHERE scan_id=?1 LIMIT 1",
        "maxModelTokens" => "SELECT CAST(json_extract(policy_json,'$.maxModelTokens') AS INTEGER) FROM sentinel_scan_contexts WHERE scan_id=?1 LIMIT 1",
        _ => return Ok(None),
    };
    match c.query_row(sql, [task], |row| row.get::<_, Option<i64>>(0)) {
        Ok(value) => Ok(value),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => {
            let message = error.to_string();
            if message.contains("no such table") { return Ok(None); }
            Err(message)
        }
    }
}

fn supplement_url_in_project(c: &rusqlite::Connection, project: i64, url: &str) -> Result<bool, String> {
    let key = supplement_target_key(url);
    let asset: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.is_deleted=0 AND (lower(rtrim(trim(COALESCE(NULLIF(a.link,''), a.host)), '/'))=?2 OR lower(rtrim(trim(a.host), '/'))=?2))",
        params![project, key], |row| row.get(0),
    ).or_else(|error| {
        if error.to_string().contains("no such table") { Ok(false) } else { Err(error.to_string()) }
    })?;
    if asset { return Ok(true); }
    let approved = match c.query_row(
        "SELECT approved_domains_json FROM asset_ownership_profiles WHERE project_id=?1",
        [project], |row| row.get::<_, String>(0),
    ) {
        Ok(text) => text,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(error) => {
            if error.to_string().contains("no such table") { return Ok(false); }
            return Err(error.to_string());
        }
    };
    let host = reqwest::Url::parse(url).ok().and_then(|parsed| parsed.host_str().map(str::to_string)).unwrap_or_default();
    let Ok(Value::Array(rows)) = serde_json::from_str::<Value>(&approved) else { return Ok(false); };
    Ok(rows.iter().filter_map(Value::as_str).any(|rule| crate::commands::ownership::domain_matches(&host, rule)))
}

/// Same read-only checks start and resume already use. A refusal is the
/// existing code. An original allowance refuses a cap change, because that
/// would mint a second allowance. Adding an already authorized target does
/// not mint one, so that allowance does not refuse it. A failed read of this
/// attempt's root row is claim_ledger_unchecked. No row is not that failure.
/// A failed snapshot begin is claim_ledger_unchecked: that transaction never
/// started, so nothing from it was committed. A failed rollback is the same
/// code when the snapshot wrote no row and the connection is autocommit
/// again, so no refusal row from it was committed. A rollback failure that
/// may have committed a write still returns that error. This does not mint an
/// allowance, reserve a request, start the scan, or call preflight.
fn supplement_scope_budget_refusal(c: &rusqlite::Connection, project: i64, task: &str, attempt: i64, payload: &str) -> Result<Option<String>, String> {
    let Some((outside, budget_changed)) = supplement_scope_budget_request(c, task, payload)? else { return Ok(None); };
    let project_active: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN projects p ON p.id=s.project_id AND p.status='active' WHERE s.id=?1 AND s.project_id=?2 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id))",
        params![task, project], |row| row.get(0),
    ).unwrap_or(false);
    if !project_active {
        return Ok(Some("scan_project_unavailable".into()));
    }
    for url in &outside {
        if !supplement_url_in_project(c, project, url)? {
            return Ok(Some("scan_asset_unavailable".into()));
        }
    }
    let root = match c.query_row(
        "SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id='' ORDER BY id LIMIT 1",
        params![task, attempt], |row| row.get::<_, String>(0),
    ).optional() {
        Ok(root) => root,
        Err(_) => return Ok(Some("claim_ledger_unchecked".into())),
    };
    if let Some(root) = root {
        match crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(c, &root) {
            Ok((tokens, requests)) if tokens == Some(0) || requests == Some(0) => {
                return Ok(Some("original_request_balance_exhausted".into()));
            }
            Err(_) => return Ok(Some("claim_ledger_unchecked".into())),
            Ok(_) => {}
        }
    }
    let history = if c.is_autocommit() {
        let tx = match c.unchecked_transaction() {
            Ok(tx) => tx,
            Err(_) => return Ok(Some("claim_ledger_unchecked".into())),
        };
        // total_changes counts uncommitted writes and does not fall after
        // rollback. An equal count means this snapshot wrote no row. Autocommit
        // after the failed rollback means that transaction is closed, so it
        // did not commit a refusal row. A changed count, an unreadable count,
        // or a transaction that is still open keeps the raw rollback error.
        let changes_before = tx.query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0)).ok();
        let result = super::require_web_start_financial_history_in(&tx, task);
        let changes_after = tx.query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0)).ok();
        let snapshot_unwritten = matches!((changes_before, changes_after), (Some(before), Some(after)) if before == after);
        match tx.rollback() {
            Ok(()) => result,
            Err(_) if snapshot_unwritten && c.is_autocommit() => return Ok(Some("claim_ledger_unchecked".into())),
            Err(error) => return Err(error.to_string()),
        }
    } else {
        super::require_web_start_financial_history_in(c, task)
    };
    if let Err(code) = history {
        return Ok(Some(code));
    }
    if budget_changed {
        let existing_allowance: bool = match c.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts a
               WHERE EXISTS(SELECT 1 FROM agent_runs r WHERE r.id=a.root_run_id AND r.scan_id=?1
                 AND r.backend='native' AND r.role='coordinator' AND r.parent_run_id IS NULL AND r.assignment_id='')
               OR json_extract(a.contract_json,'$.root.scan')=?1)",
            [task], |row| row.get(0),
        ) {
            Ok(existing) => existing,
            Err(_) => return Ok(Some("claim_ledger_unchecked".into())),
        };
        if existing_allowance {
            return Ok(Some("web_resume_original_allowance_reused".into()));
        }
    }
    Ok(None)
}

struct SupplementGateSave {
    names: Vec<String>,
    added: usize,
    cap_not_raised: bool,
}

fn supplement_target_key_present(c: &rusqlite::Connection, task: &str, key: &str) -> Result<bool, String> {
    let mut statement = match c.prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1") {
        Ok(statement) => statement,
        Err(error) => {
            let message = error.to_string();
            if message.contains("no such table") { return Ok(false); }
            return Err(message);
        }
    };
    let rows = statement.query_map([task], |row| row.get::<_, String>(0)).map_err(|error| error.to_string())?;
    for row in rows {
        if supplement_target_key(&row.map_err(|error| error.to_string())?) == key {
            return Ok(true);
        }
    }
    Ok(false)
}

fn supplement_url_is_fused(c: &rusqlite::Connection, project: i64, url: &str) -> Result<bool, String> {
    let key = supplement_target_key(url);
    match c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_fuse_zone WHERE project_id=?1 AND normalized_url=?2)",
        params![project, key], |row| row.get(0),
    ) {
        Ok(fused) => Ok(fused),
        Err(error) => {
            let message = error.to_string();
            if message.contains("no such table") { Ok(false) } else { Err(message) }
        }
    }
}

/// The asset-draft insert, pointed at this scan. A closed or fused row is left
/// alone. This does not create a scan, mint an allowance, or change a cap.
fn supplement_persist_authorized_target(c: &rusqlite::Connection, project: i64, task: &str, url: &str) -> Result<Option<String>, String> {
    let key = supplement_target_key(url);
    if supplement_target_key_present(c, task, &key)? || supplement_url_is_fused(c, project, url)? {
        return Ok(None);
    }
    let asset = match c.query_row(
        "SELECT a.id, a.company, COALESCE(NULLIF(a.link,''), a.host) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.is_deleted=0 AND lower(rtrim(trim(COALESCE(NULLIF(a.link,''), a.host)), '/'))=?2 AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.project_id=pa.project_id AND f.normalized_url=lower(rtrim(trim(COALESCE(NULLIF(a.link,''), a.host)), '/'))) ORDER BY a.id LIMIT 1",
        params![project, key],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
    ) {
        Ok(row) => Some(row),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(error) => {
            let message = error.to_string();
            if message.contains("no such table") { None } else { return Err(message); }
        }
    };
    if let Some((asset_id, company, stored_url)) = asset {
        if supplement_target_key_present(c, task, &supplement_target_key(&stored_url))? {
            return Ok(None);
        }
        let inserted = c.execute(
            "INSERT INTO sentinel_targets(project_id,scan_id,asset_id,company,url) VALUES(?1,?2,?3,?4,?5)",
            params![project, task, asset_id, company, stored_url],
        ).map_err(|error| error.to_string())?;
        if inserted != 1 { return Err("supplement_authorized_target_not_persisted".into()); }
        return Ok(Some(company));
    }
    if !supplement_url_in_project(c, project, url)? { return Ok(None); }
    let project_name: String = c.query_row("SELECT project_name FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_default();
    let stored_url = url.trim().trim_end_matches('/').to_string();
    if supplement_target_key_present(c, task, &supplement_target_key(&stored_url))? { return Ok(None); }
    let inserted = c.execute(
        "INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,'queued')",
        params![project, task, project_name, stored_url],
    ).map_err(|error| error.to_string())?;
    if inserted != 1 { return Err("supplement_authorized_target_not_persisted".into()); }
    Ok(Some(project_name))
}

/// Persist only the authorized target the draft insert already knows how to
/// write. A requested cap stays unchanged: no ledger function can raise the
/// original allowance without minting another attempt.
fn supplement_save_gate_success(c: &rusqlite::Connection, project: i64, task: &str, payload: &str) -> Result<SupplementGateSave, String> {
    let Some((outside, budget_changed)) = supplement_scope_budget_request(c, task, payload)? else {
        return Ok(SupplementGateSave { names: Vec::new(), added: 0, cap_not_raised: false });
    };
    let mut names = Vec::new();
    let mut added = 0usize;
    for url in outside {
        if let Some(company) = supplement_persist_authorized_target(c, project, task, &url)? {
            added += 1;
            let company = company.trim().to_string();
            if !company.is_empty() && !names.iter().any(|saved| saved == &company) {
                names.push(company);
            }
        }
    }
    Ok(SupplementGateSave { names, added, cap_not_raised: budget_changed })
}

/// The accepted command and its pause/cancel control commit together. A failed
/// control insert does not leave an accepted command without a queue row.
fn persist_accepted_command(
    c: &rusqlite::Connection, id: &str, key: &str, binding: &str, project: i64, task: &str, attempt: i64,
    kind: &str, digest: &str, stored_payload: &str, boundary: &str, command_version: i64,
) -> Result<(), String> {
    let write = |c: &rusqlite::Connection| -> Result<(), String> {
        c.execute(
            "INSERT INTO nest_task_commands(command_id,idempotency_key,binding_id,project_id,task_id,attempt_number,kind,payload_hash,payload_text,plan_boundary,state,command_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'accepted',?11)",
            params![id, key, binding, project, task, attempt, kind, digest, stored_payload, boundary, command_version],
        ).map_err(|error| error.to_string())?;
        if matches!(kind, "pause" | "cancel") {
            c.execute(
                "INSERT INTO nest_execution_controls(control_id,command_id,project_id,task_id,attempt_number,kind,state) VALUES(?1,?2,?3,?4,?5,?6,'queued')",
                params![uuid::Uuid::new_v4().to_string(), id, project, task, attempt, kind],
            ).map_err(|error| error.to_string())?;
        }
        Ok(())
    };
    if !c.is_autocommit() {
        return write(c);
    }
    let tx = c.unchecked_transaction().map_err(|error| error.to_string())?;
    write(&tx)?;
    tx.commit().map_err(|error| error.to_string())
}

pub(crate) fn submit(c: &rusqlite::Connection, project: i64, conversation: &str, task: &str, key: &str, kind: &str, payload: &str) -> Result<Value, String> {
    let bound = bind(c, project, conversation, task)?;
    ensure_command_scope(c)?;
    let binding = bound["bindingId"].as_str().ok_or("binding_unavailable")?;
    let attempt: i64 = c.query_row(
        "SELECT attempt_count FROM sentinel_scans WHERE id=?1 AND project_id=?2",
        params![task, project], |row| row.get(0),
    ).map_err(|_| "scan_task_unavailable".to_string())?;
    let digest = format!("{:x}", Sha256::digest(payload.as_bytes()));
    let scope = if kind == "reanalyze" {
        crate::agent_runtime::secrets::redact_text_with(payload, None)
    } else {
        String::new()
    };
    let stored_payload = if kind == "supplement" {
        let scrubbed = crate::agent_runtime::secrets::redact_text_with(payload, None);
        if scrubbed.trim().is_empty() {
            return Err("supplement_text_invalid".into());
        }
        scrubbed
    } else {
        scope.clone()
    };
    let current_attempt = attempt;
    if let Some((command, stored, state, stored_scope, stored_attempt, stored_boundary, stored_failure, stored_task, stored_binding, stored_kind)) = c.query_row(
        "SELECT command_id, payload_hash, state, payload_text, attempt_number, plan_boundary, failure_reason, task_id, binding_id, kind FROM nest_task_commands WHERE project_id=?1 AND idempotency_key=?2",
        params![project, key], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?, row.get::<_, String>(9)?)),
    ).optional().map_err(|error| error.to_string())? {
        // A key replays only the same command: same task, conversation binding,
        // kind and attempt as well as the same payload.
        if stored_task != task || stored_binding != binding || stored_kind != kind || stored_attempt != attempt {
            return Err("command_identity_conflict".into());
        }
        if stored != digest { return Err("command_payload_conflict".into()); }
        let mut receipt = json!({"commandId": command, "state": state, "attemptNumber": stored_attempt, "replayed": true, "executionStarted": false, "verified": false});
        if matches!(kind, "pause" | "cancel") {
            receipt["dispatchStopped"] = json!(true);
            receipt["engineStopped"] = json!(false);
        }
        reanalyze_fields(&mut receipt, kind, &stored_scope);
        supplement_fields(&mut receipt, kind, &stored_boundary, stored_attempt, current_attempt);
        if kind == "supplement" && !stored_failure.is_empty() {
            receipt["failureReason"] = json!(stored_failure);
        }
        return Ok(receipt);
    }
    let boundary = if kind == "supplement" {
        supplement_boundary(c, task, attempt)
    } else {
        String::new()
    };
    let command_version: i64 = c.query_row(
        "SELECT COALESCE(MAX(command_version),0)+1 FROM nest_task_commands WHERE project_id=?1 AND task_id=?2",
        params![project, task], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    persist_accepted_command(c, &id, key, binding, project, task, attempt, kind, &digest, &stored_payload, &boundary, command_version)?;
    // Pause still replaces every unexecuted non-control command. Cancel leaves
    // supplements to the stored-version decision below; other kinds stay here.
    let supersede_sql = if kind == "cancel" {
        "UPDATE nest_task_commands SET state='superseded', failure_reason='superseded_by_control' WHERE task_id=?1 AND attempt_number=?2 AND project_id=?3 AND state IN ('accepted','delivered') AND kind NOT IN ('pause','cancel','supplement') RETURNING command_id"
    } else {
        "UPDATE nest_task_commands SET state='superseded', failure_reason='superseded_by_control' WHERE task_id=?1 AND attempt_number=?2 AND project_id=?3 AND state IN ('accepted','delivered') AND kind NOT IN ('pause','cancel') RETURNING command_id"
    };
    let mut superseded = if matches!(kind, "pause" | "cancel") {
        let superseded_ids = {
            let mut statement = c.prepare(supersede_sql).map_err(|error| error.to_string())?;
            let ids = statement.query_map(params![task, attempt, project], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            ids
        };
        for command_id in &superseded_ids {
            let _ = post_superseded(c, project, command_id, kind);
        }
        superseded_ids.len()
    } else {
        0
    };
    let mut command_state = "accepted".to_string();
    let mut cancelled_delivery = String::new();
    if kind == "cancel" {
        let changed = c.execute(
            "UPDATE nest_task_commands SET state='cancelled' WHERE command_id=?1 AND state='accepted'",
            [&id],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("command_cancel_not_persisted".into()); }
        command_state = "cancelled".to_string();
        cancelled_delivery = post_cancelled(c, project, &id);
        superseded += settle_supplements_for_cancel(c, project, task, attempt, &id)?;
    }
    let mut closed_work_refusal = String::new();
    let mut block_delivery = String::new();
    let mut applied_scope = false;
    let mut applied_cap_not_raised = false;
    if kind == "supplement" {
        settle_supplement_against_stored_cancels(c, project, task, attempt, &id)?;
        command_state = c.query_row(
            "SELECT state FROM nest_task_commands WHERE command_id=?1",
            [&id], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        // A still-accepted supplement may apply to queued, pending, empty, or
        // paused targets. Closed target rows, executed commands, budgets,
        // attempt_count, and the continuation holder stay where they are.
        // This path does not start a scan.
        if command_state == "accepted" && supplement_would_only_rewrite_closed_work(c, task)? {
            let changed = c.execute(
                "UPDATE nest_task_commands SET state='rejected', failure_reason=?2 WHERE command_id=?1 AND state='accepted'",
                params![id, SUPPLEMENT_CLOSED_WORK_UNCHANGED],
            ).map_err(|error| error.to_string())?;
            if changed == 1 {
                command_state = "rejected".to_string();
                closed_work_refusal = SUPPLEMENT_CLOSED_WORK_UNCHANGED.to_string();
            }
        }
        // Scope or budget changes use the start/resume authorization and quota
        // gate. A refusal is stored on this command and posted once to the
        // bound conversation. An accepted supplement that did not change the
        // saved budget cap and did not add an out-of-scope target uses that
        // same post. When the gate allows, only an already authorized target
        // is inserted. An original allowance blocks a cap change and does not
        // block that insert. The cap is not raised, because no ledger function
        // can change the original allowance without minting. Neither path mints
        // an allowance, reserves a request, starts the scan, or calls preflight.
        if command_state == "accepted" {
            if supplement_scope_budget_request(c, task, payload)?.is_some() {
                if let Some(reason) = supplement_scope_budget_refusal(c, project, task, attempt, payload)? {
                    let changed = c.execute(
                        "UPDATE nest_task_commands SET state='rejected', failure_reason=?2 WHERE command_id=?1 AND state='accepted'",
                        params![id, reason],
                    ).map_err(|error| error.to_string())?;
                    if changed == 1 {
                        command_state = "rejected".to_string();
                        closed_work_refusal = reason.clone();
                        block_delivery = post_block_reason(c, project, &id, "rejected", &reason);
                    }
                } else {
                    let saved = supplement_save_gate_success(c, project, task, payload)?;
                    applied_scope = saved.added > 0;
                    applied_cap_not_raised = saved.cap_not_raised;
                    if saved.cap_not_raised {
                        c.execute(
                            "UPDATE nest_task_commands SET decision_reason=?2 WHERE command_id=?1 AND state='accepted' AND decision_reason=''",
                            params![id, GATE_ALLOWED_CAP_NOT_RAISED],
                        ).map_err(|error| error.to_string())?;
                    }
                    let note = applied_gate_note(c, task, attempt, &id, &saved.names, saved.added, saved.cap_not_raised);
                    block_delivery = post_prepared_block_note(c, project, &id, &note);
                }
            } else {
                block_delivery = post_block_reason(c, project, &id, "accepted", "");
            }
        }
    }
    let backend_status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_default();
    let mut receipt = json!({"commandId": id, "state": command_state, "attemptNumber": attempt, "replayed": false, "executionStarted": false, "budgetReset": false, "backendStatus": backend_status, "verified": false, "supersededCount": superseded});
    if matches!(kind, "pause" | "cancel") {
        receipt["dispatchStopped"] = json!(true);
        receipt["engineStopped"] = json!(false);
    }
    if kind == "cancel" {
        receipt["cancelledDelivery"] = json!(cancelled_delivery);
    }
    reanalyze_fields(&mut receipt, kind, scope.as_str());
    supplement_fields(&mut receipt, kind, &boundary, attempt, current_attempt);
    if applied_scope {
        receipt["scopeExpanded"] = json!(true);
    }
    if applied_cap_not_raised {
        receipt["decisionReason"] = json!(GATE_ALLOWED_CAP_NOT_RAISED);
    }
    if !closed_work_refusal.is_empty() {
        receipt["failureReason"] = json!(closed_work_refusal);
    }
    if !block_delivery.is_empty() {
        receipt["blockDelivery"] = json!(block_delivery);
    }
    Ok(receipt)
}

/// Accepted supplements of the current attempt become confirmed directives in
/// that attempt's live Coordinator queue, through the same draft and
/// confirmation path as a human directive, so the Coordinator relays, rewrites
/// or rejects them under its normal rules. A command is `delivered` only once
/// the Coordinator queue holds it; without a live Coordinator it stays
/// `accepted` and is tried again. A supplement saved for an older attempt is
/// never applied to a later one.
pub(crate) fn deliver_supplements(c: &rusqlite::Connection, project: i64, task: Option<&str>) -> Result<usize, String> {
    ensure_command_scope(c)?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS nest_command_directives(
            command_id TEXT PRIMARY KEY,
            draft_id TEXT NOT NULL,
            revision INTEGER NOT NULL,
            draft_hash TEXT NOT NULL,
            directive_id TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    ).map_err(|error| error.to_string())?;
    let pending: Vec<(String, String, String)> = {
        let mut statement = c.prepare(
            "SELECT k.command_id, k.task_id, k.payload_text FROM nest_task_commands k JOIN sentinel_scans s ON s.id=k.task_id
             WHERE k.project_id=?1 AND (?2 IS NULL OR k.task_id=?2) AND k.kind='supplement' AND k.state='accepted'
               AND k.attempt_number=s.attempt_count AND s.status IN ('scanning','pausing') ORDER BY k.rowid",
        ).map_err(|error| error.to_string())?;
        let rows = statement.query_map(params![project, task], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let mut delivered = 0;
    for (command, task, payload) in pending {
        if payload.trim().is_empty() { continue; }
        let mapped: Option<(String, i64, String)> = c.query_row(
            "SELECT draft_id, revision, draft_hash FROM nest_command_directives WHERE command_id=?1",
            [&command], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|error| error.to_string())?;
        let (draft_id, revision, draft_hash) = match mapped {
            Some(mapped) => mapped,
            None => {
                let short = command.get(..8).unwrap_or(&command);
                let text = format!("主会话补充（指令 {short}）：{payload}");
                let draft = match super::draft_scan_directive_in(c, &task, &text, "team") {
                    Ok(draft) => draft,
                    // No live Coordinator for this attempt yet: try again later.
                    Err(_) => continue,
                };
                let id = draft["id"].as_str().unwrap_or("").to_string();
                let revision = draft["revision"].as_i64().unwrap_or(0);
                let hash = draft["draftHash"].as_str().unwrap_or("").to_string();
                if id.is_empty() || hash.is_empty() { continue; }
                // Remembered before confirmation: a crash in between confirms
                // this same draft on the next pass instead of drafting again.
                c.execute(
                    "INSERT OR IGNORE INTO nest_command_directives(command_id,draft_id,revision,draft_hash) VALUES(?1,?2,?3,?4)",
                    params![command, id, revision, hash],
                ).map_err(|error| error.to_string())?;
                (id, revision, hash)
            }
        };
        let confirmed = crate::agent_runtime::multi_agent::directive::confirm_bound_draft(c, &task, &draft_id, revision, &draft_hash);
        let directive = match confirmed {
            Ok(directive) => directive.id,
            Err(error) => {
                let already: Option<String> = c.query_row(
                    "SELECT confirmed_directive_id FROM agent_directive_drafts WHERE id=?1 AND confirmed_directive_id<>''",
                    [&draft_id], |row| row.get(0),
                ).optional().unwrap_or(None);
                match already {
                    Some(directive) => directive,
                    None => {
                        // The lease that bound this draft is gone; draft again
                        // for the next Coordinator instead of retrying a dead one.
                        if error.contains("lease") || error.contains("coordinator") || error.contains("stale") || error.contains("binding") || error.contains("attempt") {
                            let _ = c.execute("DELETE FROM nest_command_directives WHERE command_id=?1 AND directive_id=''", [&command]);
                        }
                        continue;
                    }
                }
            }
        };
        let tx = c.unchecked_transaction().map_err(|error| error.to_string())?;
        tx.execute("UPDATE nest_command_directives SET directive_id=?2 WHERE command_id=?1", params![command, directive])
            .map_err(|error| error.to_string())?;
        let changed = tx.execute(
            "UPDATE nest_task_commands SET state='delivered', execution_actor=CASE WHEN execution_actor='' THEN ?2 ELSE execution_actor END WHERE command_id=?1 AND state='accepted'",
            params![command, format!("coordinator_directive:{directive}")],
        ).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        delivered += changed;
    }
    Ok(delivered)
}

/// The running attempt hands its saved supplements to the live Coordinator.
pub(crate) fn deliver_supplements_for_scan(c: &rusqlite::Connection, task: &str) -> Result<usize, String> {
    let project: i64 = c.query_row("SELECT project_id FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0))
        .map_err(|_| "scan_task_unavailable".to_string())?;
    deliver_supplements(c, project, Some(task))
}

/// Cancel is claimed ahead of pause. Superseded commands stay out. This does not
/// change the scan status, delete the task, or mint an allowance.
fn claim_queued_execution_control_in(c: &rusqlite::Connection, task: &str, attempt: i64) -> Result<Option<String>, String> {
    let row: Option<(String, String)> = c.query_row(
        "SELECT c.control_id, c.kind FROM nest_execution_controls c
         JOIN nest_task_commands k ON k.command_id=c.command_id
         WHERE c.task_id=?1 AND c.attempt_number=?2 AND c.state='queued'
           AND k.state NOT IN ('superseded','rejected')
         ORDER BY CASE c.kind WHEN 'cancel' THEN 0 WHEN 'pause' THEN 1 ELSE 2 END, c.rowid
         LIMIT 1",
        params![task, attempt],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|error| error.to_string())?;
    let Some((id, kind)) = row else { return Ok(None); };
    let changed = c.execute(
        "UPDATE nest_execution_controls SET state='claimed' WHERE control_id=?1 AND state='queued'",
        [&id],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Ok(None); }
    Ok(Some(kind))
}

pub(crate) fn claim_queued_execution_control(c: &rusqlite::Connection, task: &str, attempt: i64) -> Result<Option<String>, String> {
    if c.prepare("SELECT 1 FROM nest_execution_controls LIMIT 0").is_err() {
        return Ok(None);
    }
    if !c.is_autocommit() {
        return claim_queued_execution_control_in(c, task, attempt);
    }
    let tx = c.unchecked_transaction().map_err(|error| error.to_string())?;
    let kind = claim_queued_execution_control_in(&tx, task, attempt)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(kind)
}

pub(crate) fn deliver(c: &rusqlite::Connection, project: i64, command: &str, conversation: &str) -> Result<Value, String> {
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    let (binding_conversation, binding_project): (String, i64) = c.query_row(
        "SELECT b.conversation_id, b.project_id FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1",
        [command], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "command_unavailable".to_string())?;
    if binding_project != project { return Err("scan_project_mismatch".into()); }
    if binding_conversation != conversation {
        return Err("result_delivery_conversation_mismatch".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO nest_result_deliveries(delivery_id,command_id,conversation_id,state) VALUES(?1,?2,?3,'pending') ON CONFLICT(command_id, conversation_id) DO NOTHING",
        params![id, command, conversation],
    ).map_err(|error| error.to_string())?;
    Ok(json!({"commandId": command, "conversationId": conversation, "state": "pending", "deliveredToAnotherConversation": false}))
}

/// Replay a pending result only into the bound conversation. A second try does not add another message.
/// An accepted command that died before any delivery row is created can be posted once to that same conversation.
/// A verification conclusion left pending uses that same conversation: archived or missing stays pending,
/// and once the conversation is available again one redacted message is posted. A second try is a replay.
/// A block reason left pending uses that same conversation and the same retry. It is not sent twice.
/// An accepted supplement that kept the saved budget cap uses that same block post and the same retry. It is not sent twice.
/// A supplement the quota gate allowed uses that same block post and the same retry. It names the authorized target that was saved and that the cap stayed unchanged. It is not sent twice.
/// A waiting_for_pause note left pending uses that same conversation and the same retry. It is not sent twice.
/// A superseded note left pending uses that same conversation and the same retry. It is not sent twice.
/// A cancelled note for the cancel command itself uses that same conversation and the same retry. It is not sent twice.
/// A note that a cancel left an executed supplement in place uses that same conversation and the same retry. It is not sent twice.
pub(crate) fn retry_pending_delivery(c: &rusqlite::Connection, project: i64, command: &str) -> Result<Value, String> {
    ensure_result_event(c)?;
    let (conversation, binding_project, task, attempt, command_state): (String, i64, String, i64, String) = c.query_row(
        "SELECT b.conversation_id, b.project_id, k.task_id, k.attempt_number, k.state FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1",
        [command], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).map_err(|_| "command_unavailable".to_string())?;
    if binding_project != project { return Err("scan_project_mismatch".into()); }
    let stored: Option<(String, String, String, String, String, String, String, String, String, String, String)> = c.query_row(
        "SELECT state, verification_delivery, verification_note, block_delivery, block_note, waiting_delivery, waiting_note, superseded_delivery, superseded_note, cancelled_delivery, cancelled_note FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?)),
    ).optional().map_err(|error| error.to_string())?;
    if let Some((_, _, _, block, block_note, _, _, _, _, _, _)) = stored.as_ref() {
        if matches!(block.as_str(), "pending" | "delivery_failed") {
            return retry_pending_block(c, project, command, &conversation, &task, block_note);
        }
    }
    if let Some((_, verification, note, _, _, _, _, _, _, _, _)) = stored.as_ref() {
        if matches!(verification.as_str(), "pending" | "delivery_failed") {
            return retry_pending_verification(c, project, command, &conversation, &task, note);
        }
    }
    if let Some((_, _, _, _, _, waiting, waiting_note, superseded, superseded_note, cancelled, cancelled_note)) = stored.as_ref() {
        if matches!(waiting.as_str(), "pending" | "delivery_failed") {
            return retry_pending_waiting(c, project, command, &conversation, &task, waiting_note);
        }
        if matches!(superseded.as_str(), "pending" | "delivery_failed") {
            return retry_pending_superseded(c, project, command, &conversation, &task, superseded_note);
        }
        if matches!(cancelled.as_str(), "pending" | "delivery_failed") {
            return retry_pending_cancelled(c, project, command, &conversation, &task, cancelled_note);
        }
        if matches!(waiting.as_str(), "posted" | "dropped") {
            return Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": true, "deliveredToAnotherConversation": false, "waitingDelivery": waiting}));
        }
        if matches!(superseded.as_str(), "posted" | "dropped") {
            return Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": true, "deliveredToAnotherConversation": false, "supersededDelivery": superseded}));
        }
        if matches!(cancelled.as_str(), "posted" | "dropped") {
            return Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": true, "deliveredToAnotherConversation": false, "cancelledDelivery": cancelled}));
        }
    }
    let kept_pending: Option<(String, String)> = c.query_row(
        "SELECT kept_delivery, kept_note FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ).optional().map_err(|error| error.to_string())?;
    if let Some((kept, kept_note)) = kept_pending {
        if matches!(kept.as_str(), "pending" | "delivery_failed") {
            return retry_pending_kept(c, project, command, &conversation, &task, &kept_note);
        }
    }
    let delivery_state = stored.as_ref().map(|(state, _, _, _, _, _, _, _, _, _, _)| state.clone());
    if delivery_state.as_deref() == Some("posted") {
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": true, "deliveredToAnotherConversation": false}));
    }
    if delivery_state.is_none() {
        let current_attempt: i64 = c.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [&task], |row| row.get(0)).unwrap_or(0);
        if command_state != "accepted" || attempt != current_attempt {
            return Err("result_delivery_not_pending".into());
        }
        c.execute(
            "INSERT INTO nest_result_deliveries(delivery_id,command_id,conversation_id,state,task_id,attempt_number) VALUES(?1,?2,?3,'pending',?4,?5) ON CONFLICT(command_id, conversation_id) DO NOTHING",
            params![uuid::Uuid::new_v4().to_string(), command, conversation, task, attempt],
        ).map_err(|error| error.to_string())?;
    } else if delivery_state.as_deref() != Some("pending") && delivery_state.as_deref() != Some("delivery_failed") {
        return Err("result_delivery_not_pending".into());
    }
    let available: i64 = bound_conversation_available(c, project, &conversation);
    if available != 1 {
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "pending", "replayed": false, "deliveredToAnotherConversation": false}));
    }
    let note = format!("待投递结果回到原会话。任务 {task}，attempt {attempt}，指令 {command}。重试不改变复核状态，也没有发往其他会话。");
    post_bound_message(c, project, &conversation, "retry_pending_delivery", command, &task, &note)?;
    let changed = c.execute(
        "UPDATE nest_result_deliveries SET state='posted' WHERE command_id=?1 AND conversation_id=?2 AND state IN ('pending','delivery_failed')",
        params![command, conversation],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("result_delivery_not_pending".into()); }
    // Posting the note tells the conversation; it is not the executor taking
    // the command, so the command state stays as it was.
    Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": false, "deliveredToAnotherConversation": false}))
}

/// Posts one stored verification conclusion to the bound conversation after it is available again.
/// An empty redaction is dropped. The raw note is not sent, and no other conversation is chosen.
/// A still-pending execution row is closed with that conclusion so the next retry does not add the generic substitute.
fn retry_pending_verification(c: &rusqlite::Connection, project: i64, command: &str, conversation: &str, task: &str, stored_note: &str) -> Result<Value, String> {
    let available: i64 = bound_conversation_available(c, project, &conversation);
    let note = omit_stored_target_urls(c, task, &crate::agent_runtime::secrets::redact_text_with(stored_note, None));
    if available != 1 {
        let _ = c.execute(
            "UPDATE nest_result_deliveries SET verification_note=?3 WHERE command_id=?1 AND conversation_id=?2 AND verification_delivery IN ('pending','delivery_failed')",
            params![command, conversation, note],
        );
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "pending", "replayed": false, "deliveredToAnotherConversation": false, "verificationDelivery": "pending"}));
    }
    if note.trim().is_empty() {
        let changed = c.execute(
            "UPDATE nest_result_deliveries SET verification_delivery='dropped', verification_note='' WHERE command_id=?1 AND conversation_id=?2 AND verification_delivery IN ('pending','delivery_failed')",
            params![command, conversation],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("result_delivery_not_pending".into()); }
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "dropped", "replayed": false, "deliveredToAnotherConversation": false, "verificationDelivery": "dropped"}));
    }
    post_bound_message(c, project, &conversation, "retry_pending_verification", command, &task, &note)?;
    let changed = c.execute(
        "UPDATE nest_result_deliveries SET verification_delivery='posted', verification_note=?3, state=CASE WHEN state='pending' THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND verification_delivery IN ('pending','delivery_failed')",
        params![command, conversation, note],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("result_delivery_not_pending".into()); }
    Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": false, "deliveredToAnotherConversation": false, "verificationDelivery": "posted"}))
}

/// Posts one stored block reason to the bound conversation after it is available again.
/// An empty redaction is dropped and the raw note is not sent. No other conversation is chosen.
fn scrubbed_delivery_note(c: &rusqlite::Connection, task: &str, stored_note: &str) -> String {
    omit_stored_target_urls(c, task, &crate::agent_runtime::secrets::redact_text_with(stored_note, None))
}

fn remember_held_note(c: &rusqlite::Connection, command: &str, conversation: &str, column: &str, note: &str) {
    let sql = match column {
        "block_note" => "UPDATE nest_result_deliveries SET block_note=?3 WHERE command_id=?1 AND conversation_id=?2 AND block_delivery IN ('pending','delivery_failed')",
        "waiting_note" => "UPDATE nest_result_deliveries SET waiting_note=?3 WHERE command_id=?1 AND conversation_id=?2 AND waiting_delivery IN ('pending','delivery_failed')",
        "superseded_note" => "UPDATE nest_result_deliveries SET superseded_note=?3 WHERE command_id=?1 AND conversation_id=?2 AND superseded_delivery IN ('pending','delivery_failed')",
        "cancelled_note" => "UPDATE nest_result_deliveries SET cancelled_note=?3 WHERE command_id=?1 AND conversation_id=?2 AND cancelled_delivery IN ('pending','delivery_failed')",
        "kept_note" => "UPDATE nest_result_deliveries SET kept_note=?3 WHERE command_id=?1 AND conversation_id=?2 AND kept_delivery IN ('pending','delivery_failed')",
        _ => return,
    };
    let _ = c.execute(sql, params![command, conversation, note]);
}

fn retry_pending_block(c: &rusqlite::Connection, project: i64, command: &str, conversation: &str, task: &str, stored_note: &str) -> Result<Value, String> {
    let available: i64 = bound_conversation_available(c, project, &conversation);
    let note = scrubbed_delivery_note(c, task, stored_note);
    if available != 1 {
        remember_held_note(c, command, conversation, "block_note", &note);
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "pending", "replayed": false, "deliveredToAnotherConversation": false, "blockDelivery": "pending"}));
    }
    if note.trim().is_empty() {
        let changed = c.execute(
            "UPDATE nest_result_deliveries SET block_delivery='dropped', block_note='', state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND block_delivery IN ('pending','delivery_failed')",
            params![command, conversation],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("result_delivery_not_pending".into()); }
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "dropped", "replayed": false, "deliveredToAnotherConversation": false, "blockDelivery": "dropped"}));
    }
    post_bound_message(c, project, &conversation, "retry_pending_block", command, &task, &note)?;
    let changed = c.execute(
        "UPDATE nest_result_deliveries SET block_delivery='posted', block_note=?3, state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND block_delivery IN ('pending','delivery_failed')",
        params![command, conversation, note],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("result_delivery_not_pending".into()); }
    Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": false, "deliveredToAnotherConversation": false, "blockDelivery": "posted"}))
}

/// Posts one stored waiting_for_pause note after the bound conversation is available again.
/// An empty redaction is dropped and the raw note is not sent. No other conversation is chosen.
/// A block reason or verification note already stored on this row is not copied into this message.
fn retry_pending_waiting(c: &rusqlite::Connection, project: i64, command: &str, conversation: &str, task: &str, stored_note: &str) -> Result<Value, String> {
    let available: i64 = bound_conversation_available(c, project, &conversation);
    let note = scrubbed_delivery_note(c, task, stored_note);
    if available != 1 {
        remember_held_note(c, command, conversation, "waiting_note", &note);
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "pending", "replayed": false, "deliveredToAnotherConversation": false, "waitingDelivery": "pending"}));
    }
    if note.trim().is_empty() {
        let changed = c.execute(
            "UPDATE nest_result_deliveries SET waiting_delivery='dropped', waiting_note='', state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND waiting_delivery IN ('pending','delivery_failed')",
            params![command, conversation],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("result_delivery_not_pending".into()); }
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "dropped", "replayed": false, "deliveredToAnotherConversation": false, "waitingDelivery": "dropped"}));
    }
    post_bound_message(c, project, &conversation, "retry_pending_waiting", command, &task, &note)?;
    let changed = c.execute(
        "UPDATE nest_result_deliveries SET waiting_delivery='posted', waiting_note=?3, state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND waiting_delivery IN ('pending','delivery_failed')",
        params![command, conversation, note],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("result_delivery_not_pending".into()); }
    Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": false, "deliveredToAnotherConversation": false, "waitingDelivery": "posted"}))
}

/// Posts one stored superseded note after the bound conversation is available again.
/// An empty redaction is dropped and the raw note is not sent. No other conversation is chosen.
/// A waiting or block note already stored on this row is not copied into this message.
fn retry_pending_superseded(c: &rusqlite::Connection, project: i64, command: &str, conversation: &str, task: &str, stored_note: &str) -> Result<Value, String> {
    let available: i64 = bound_conversation_available(c, project, &conversation);
    let note = scrubbed_delivery_note(c, task, stored_note);
    if available != 1 {
        remember_held_note(c, command, conversation, "superseded_note", &note);
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "pending", "replayed": false, "deliveredToAnotherConversation": false, "supersededDelivery": "pending"}));
    }
    if note.trim().is_empty() {
        let changed = c.execute(
            "UPDATE nest_result_deliveries SET superseded_delivery='dropped', superseded_note='', state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND superseded_delivery IN ('pending','delivery_failed')",
            params![command, conversation],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("result_delivery_not_pending".into()); }
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "dropped", "replayed": false, "deliveredToAnotherConversation": false, "supersededDelivery": "dropped"}));
    }
    post_bound_message(c, project, &conversation, "retry_pending_superseded", command, &task, &note)?;
    let changed = c.execute(
        "UPDATE nest_result_deliveries SET superseded_delivery='posted', superseded_note=?3, state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND superseded_delivery IN ('pending','delivery_failed')",
        params![command, conversation, note],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("result_delivery_not_pending".into()); }
    Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": false, "deliveredToAnotherConversation": false, "supersededDelivery": "posted"}))
}

/// Posts one stored cancelled note after the cancel command's bound conversation is available again.
/// An empty redaction is dropped and the raw note is not sent. No other conversation is chosen.
/// The stored note is this command's cancellation, not a superseded note for another command.
fn retry_pending_cancelled(c: &rusqlite::Connection, project: i64, command: &str, conversation: &str, task: &str, stored_note: &str) -> Result<Value, String> {
    let available: i64 = bound_conversation_available(c, project, &conversation);
    let note = scrubbed_delivery_note(c, task, stored_note);
    if available != 1 {
        remember_held_note(c, command, conversation, "cancelled_note", &note);
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "pending", "replayed": false, "deliveredToAnotherConversation": false, "cancelledDelivery": "pending"}));
    }
    if note.trim().is_empty() {
        let changed = c.execute(
            "UPDATE nest_result_deliveries SET cancelled_delivery='dropped', cancelled_note='', state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND cancelled_delivery IN ('pending','delivery_failed')",
            params![command, conversation],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("result_delivery_not_pending".into()); }
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "dropped", "replayed": false, "deliveredToAnotherConversation": false, "cancelledDelivery": "dropped"}));
    }
    post_bound_message(c, project, &conversation, "retry_pending_cancelled", command, &task, &note)?;
    let changed = c.execute(
        "UPDATE nest_result_deliveries SET cancelled_delivery='posted', cancelled_note=?3, state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND cancelled_delivery IN ('pending','delivery_failed')",
        params![command, conversation, note],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("result_delivery_not_pending".into()); }
    Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": false, "deliveredToAnotherConversation": false, "cancelledDelivery": "posted"}))
}

/// The controller has the command. This is delivery, not execution.
pub(crate) fn mark_delivered(c: &rusqlite::Connection, project: i64, command: &str) -> Result<Value, String> {
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    let (state, scans_before): (String, i64) = {
        let state: String = c.query_row(
            "SELECT state FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
            params![command, project], |row| row.get(0),
        ).map_err(|_| "command_unavailable".to_string())?;
        let scans: i64 = c.query_row("SELECT COUNT(*) FROM sentinel_scans", [], |row| row.get(0)).map_err(|error| error.to_string())?;
        (state, scans)
    };
    let _ = scans_before;
    // `delivered` is set by the executor that took the command (the Coordinator
    // queue for a supplement, the execution control for pause/cancel). This
    // call only reports that state; a caller cannot declare it.
    if state == "delivered" {
        return Ok(json!({"commandId": command, "state": "delivered", "replayed": true, "executionStarted": false}));
    }
    if state == "accepted" { return Err("command_not_claimed_by_executor".into()); }
    Err("command_state_not_deliverable".into())
}

fn ensure_result_event(c: &rusqlite::Connection) -> Result<(), String> {
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    for (column, definition) in [
        ("task_id", "TEXT NOT NULL DEFAULT ''"),
        ("attempt_number", "INTEGER NOT NULL DEFAULT 0"),
        ("execution_method", "TEXT NOT NULL DEFAULT ''"),
        ("review_state", "TEXT NOT NULL DEFAULT 'unverified'"),
        ("evidence_ref", "TEXT NOT NULL DEFAULT ''"),
        ("sequence", "INTEGER NOT NULL DEFAULT 0"),
        ("model_requests", "INTEGER NOT NULL DEFAULT 0"),
        ("total_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("verification_delivery", "TEXT NOT NULL DEFAULT ''"),
        ("verification_note", "TEXT NOT NULL DEFAULT ''"),
        ("block_delivery", "TEXT NOT NULL DEFAULT ''"),
        ("block_note", "TEXT NOT NULL DEFAULT ''"),
        ("waiting_delivery", "TEXT NOT NULL DEFAULT ''"),
        ("waiting_note", "TEXT NOT NULL DEFAULT ''"),
        ("superseded_delivery", "TEXT NOT NULL DEFAULT ''"),
        ("superseded_note", "TEXT NOT NULL DEFAULT ''"),
        ("cancelled_delivery", "TEXT NOT NULL DEFAULT ''"),
        ("cancelled_note", "TEXT NOT NULL DEFAULT ''"),
        ("kept_delivery", "TEXT NOT NULL DEFAULT ''"),
        ("kept_note", "TEXT NOT NULL DEFAULT ''"),
    ] {
        let _ = c.execute(&format!("ALTER TABLE nest_result_deliveries ADD COLUMN {column} {definition}"), []);
    }
    Ok(())
}

fn scan_execution_started(kind: &str) -> bool {
    matches!(kind, "continue_targets" | "resume")
}

fn execution_method(kind: &str) -> &'static str {
    match kind {
        "continue_targets" => "continuation_fence",
        "pause" => "backend_paused",
        "resume" => "backend_scanning_same_attempt",
        _ => "unspecified",
    }
}

fn evidence_ref(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64) -> String {
    if kind == "continue_targets" {
        return c.query_row(
            "SELECT id FROM scan_target_continuation_fences WHERE scan_id=?1 AND attempt_number=?2 AND launched=1 LIMIT 1",
            params![task, attempt], |row| row.get(0),
        ).unwrap_or_default();
    }
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_default();
    format!("scan_status:{status}")
}

fn record_result_event_row(c: &rusqlite::Connection, command: &str, conversation: &str, task: &str, attempt: i64, kind: &str, state: &str) -> Result<(i64, i64, i64), String> {
    let (requests, tokens): (i64, i64) = c.query_row(
        "SELECT llm_requests, total_tokens FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or((0, 0));
    let existing: i64 = c.query_row(
        "SELECT sequence FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).unwrap_or(0);
    let sequence = if existing > 0 {
        existing
    } else {
        c.query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM nest_result_deliveries WHERE conversation_id=?1",
            [conversation], |row| row.get(0),
        ).unwrap_or(1)
    };
    let id = uuid::Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO nest_result_deliveries(delivery_id,command_id,conversation_id,state,task_id,attempt_number,execution_method,review_state,evidence_ref,sequence,model_requests,total_tokens) VALUES(?1,?2,?3,?4,?5,?6,?7,'unverified',?8,?9,?10,?11)
         ON CONFLICT(command_id, conversation_id) DO UPDATE SET state=excluded.state, task_id=excluded.task_id, attempt_number=excluded.attempt_number, execution_method=excluded.execution_method, evidence_ref=excluded.evidence_ref,
         sequence=CASE WHEN nest_result_deliveries.sequence>0 THEN nest_result_deliveries.sequence ELSE excluded.sequence END,
         model_requests=CASE WHEN nest_result_deliveries.sequence>0 THEN nest_result_deliveries.model_requests ELSE excluded.model_requests END,
         total_tokens=CASE WHEN nest_result_deliveries.sequence>0 THEN nest_result_deliveries.total_tokens ELSE excluded.total_tokens END,
         review_state=CASE WHEN nest_result_deliveries.review_state='unverified' THEN 'unverified' ELSE nest_result_deliveries.review_state END",
        params![id, command, conversation, state, task, attempt, execution_method(kind), evidence_ref(c, kind, task, attempt), sequence, requests, tokens],
    ).map_err(|error| error.to_string())?;
    Ok((sequence, requests, tokens))
}

fn cost_snapshot(sequence: i64, requests: i64, tokens: i64) -> Value {
    json!({"sequence": sequence, "modelRequests": requests, "totalTokens": tokens, "freshAllowanceMinted": false})
}

fn set_review_state(c: &rusqlite::Connection, command: &str, review_state: &str) {
    if ensure_result_event(c).is_err() { return; }
    let _ = c.execute("UPDATE nest_result_deliveries SET review_state=?2 WHERE command_id=?1", params![command, review_state]);
}

fn remember_name(names: &mut Vec<String>, company: &str) {
    let name = company.trim();
    let label = if name.is_empty() {
        "未命名".to_string()
    } else {
        let redacted = crate::agent_runtime::secrets::redact_text_with(name, None);
        if redacted.trim().is_empty() { return; }
        // A leftover "Bearer " prefix would let a later scrub treat the rest of the sentence as the token.
        if redacted.to_ascii_lowercase().contains("bearer ") {
            crate::agent_runtime::secrets::RedactionContext::durable_marker("bearer", name)
        } else {
            redacted
        }
    };
    if label.trim().is_empty() || names.iter().any(|item| item == &label) { return; }
    names.push(label);
}

fn listed_names(names: &[String]) -> String {
    if names.is_empty() { return "无".into(); }
    let mut text = names.iter().take(8).map(String::as_str).collect::<Vec<_>>().join("、");
    if names.len() > 8 { text.push_str("等"); }
    text
}

/// Names only. Target URLs stay out of the session note so query secrets never ride along.
/// A read error is not reported as an empty remaining set.
fn target_progress_phrase(c: &rusqlite::Connection, task: &str) -> String {
    let mut statement = match c.prepare("SELECT company, status FROM sentinel_targets WHERE scan_id=?1 ORDER BY id") {
        Ok(statement) => statement,
        Err(_) => return "目标进度没有读到，不能当成没有剩余目标。".into(),
    };
    let rows = match statement.query_map([task], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))) {
        Ok(rows) => rows,
        Err(_) => return "目标进度没有读到，不能当成没有剩余目标。".into(),
    };
    let mut completed = Vec::new();
    let mut remaining = Vec::new();
    let mut held = Vec::new();
    for row in rows {
        let Ok((company, status)) = row else {
            return "目标进度没有读到，不能当成没有剩余目标。".into();
        };
        match status.as_str() {
            "completed" | "recon_only" => remember_name(&mut completed, &company),
            "queued" | "pending" | "" | "paused" => remember_name(&mut remaining, &company),
            _ => remember_name(&mut held, &company),
        }
    }
    format!(
        "已完成目标：{}。仍未完成：{}。不在自动剩余集合：{}。",
        listed_names(&completed), listed_names(&remaining), listed_names(&held)
    )
}

/// Pending Host outbox events for one task, returned with scan results so the
/// Host can store them in the original conversation. Reading changes nothing;
/// the Host acknowledges what it stored through the outbox ack.
pub(crate) fn host_result_deliveries(c: &rusqlite::Connection, task: &str) -> Vec<Value> {
    let mut statement = match c.prepare(
        "SELECT event_id, host_conversation_id, task_id, kind, text FROM nest_host_outbox WHERE task_id=?1 AND state='pending' ORDER BY created_at, rowid LIMIT 32",
    ) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    let rows = statement.query_map([task], |row| {
        Ok(json!({"eventId": row.get::<_, String>(0)?, "conversationId": row.get::<_, String>(1)?, "taskId": row.get::<_, String>(2)?, "kind": row.get::<_, String>(3)?, "text": row.get::<_, String>(4)?}))
    });
    match rows {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => Vec::new(),
    }
}

fn session_result_note(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64, command: &str) -> String {
    let usage = match c.query_row(
        "SELECT llm_requests, total_tokens FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    ) {
        Ok((requests, tokens)) => format!("引擎用量 {requests} 次请求、{tokens} token。"),
        Err(_) => "引擎用量没有读到，不能当成 0。".into(),
    };
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0))
        .unwrap_or_else(|_| "没有读到".into());
    let current_attempt: i64 = c.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or(attempt);
    let parent_resume = kind == "resume" && current_attempt != attempt;
    let lead = match kind {
        "pause" => "暂停指令已执行：后端状态是暂停，仍是原 attempt。这还不是已验证。".to_string(),
        "resume" if parent_resume => format!("恢复指令已执行：父 attempt {attempt} 的恢复指令已执行，因为子 attempt {current_attempt} 正在扫描。这还不是新的额度。"),
        "resume" => "恢复指令已执行：后端状态是扫描中，仍是原 attempt。这还不是已验证，也不是新的额度。".to_string(),
        _ => "指令已执行：该 attempt 的续跑执行权已被领取。复核尚未完成，这不是已验证。".to_string(),
    };
    let method = if parent_resume { "backend_scanning_child_attempt" } else { execution_method(kind) };
    let evidence = evidence_ref(c, kind, task, attempt);
    let evidence_text = if evidence.is_empty() { "无".to_string() } else { evidence };
    format!(
        "{lead} 任务 {task}，attempt {attempt}，指令 {command}，方式 {method}，证据 {evidence_text}。{progress}{usage}没有新开额度。剩余额度没有在这条回传里重算，不能当成还有额度或已经用尽。扫描状态是 {status}。结论依据是上述执行方式，不是浏览器验证，也不是对已有证据的重新分析。复核未完成。下一步是按该指令自己的验证规则核对；未覆盖的目标不能说成已经完成。",
        progress = target_progress_phrase(c, task),
    )
}

/// Executed only after the matching backend fact exists.
/// Verification stays unset. The note and delivery row stay on the bound conversation.
pub(crate) fn mark_executed(c: &rusqlite::Connection, project: i64, command: &str) -> Result<Value, String> {
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    let (state, task, attempt, conversation, kind): (String, String, i64, String, String) = c.query_row(
        "SELECT k.state, k.task_id, k.attempt_number, b.conversation_id, k.kind FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1 AND k.project_id=?2",
        params![command, project], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).map_err(|_| "command_unavailable".to_string())?;
    if kind == "reanalyze" { return Err("reanalyze_is_not_execution".into()); }
    if state == "executed" {
        return Ok(json!({"commandId": command, "state": "executed", "replayed": true, "executionStarted": scan_execution_started(&kind), "verified": false}));
    }
    if state != "delivered" { return Err("command_execution_not_observed".into()); }
    if !execution_observed(c, &kind, &task, attempt) { return Err("command_execution_not_observed".into()); }
    ensure_result_event(c)?;
    let _ = c.execute("ALTER TABLE nest_task_commands ADD COLUMN execution_actor TEXT NOT NULL DEFAULT ''", []);
    let actor = observed_execution_actor(c, &kind, &task, attempt);
    let write = |c: &rusqlite::Connection| -> Result<Value, String> {
        let changed = c.execute(
            "UPDATE nest_task_commands SET state='executed', execution_actor=CASE WHEN execution_actor='' THEN ?3 ELSE execution_actor END WHERE command_id=?1 AND project_id=?2 AND state='delivered'",
            params![command, project, actor],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("command_execution_not_observed".into()); }
        let available: i64 = bound_conversation_available(c, project, &conversation);
        if available != 1 {
            let (sequence, requests, tokens) = record_result_event_row(c, command, &conversation, &task, attempt, &kind, "pending")?;
            return Ok(json!({"commandId": command, "state": "executed", "replayed": false, "executionStarted": scan_execution_started(&kind), "verified": false, "conversationId": conversation, "deliveryState": "pending", "reviewState": "unverified", "executionMethod": execution_method(&kind), "attemptNumber": attempt, "costSnapshot": cost_snapshot(sequence, requests, tokens)}));
        }
        let note = crate::agent_runtime::secrets::redact_text_with(&session_result_note(c, &kind, &task, attempt, command), None);
        if note.trim().is_empty() {
            let (sequence, requests, tokens) = record_result_event_row(c, command, &conversation, &task, attempt, &kind, "delivery_failed")?;
            return Ok(json!({"commandId": command, "state": "delivery_failed", "commandState": "executed", "replayed": false, "executionStarted": scan_execution_started(&kind), "verified": false, "conversationId": conversation, "deliveryState": "delivery_failed", "reviewState": "unverified", "executionMethod": execution_method(&kind), "attemptNumber": attempt, "costSnapshot": cost_snapshot(sequence, requests, tokens)}));
        }
        let posted = post_bound_message(c, project, &conversation, "mark_executed", command, &task, &note);
        if posted.is_err() {
            let (sequence, requests, tokens) = record_result_event_row(c, command, &conversation, &task, attempt, &kind, "delivery_failed")?;
            return Ok(json!({"commandId": command, "state": "delivery_failed", "commandState": "executed", "replayed": false, "executionStarted": scan_execution_started(&kind), "verified": false, "conversationId": conversation, "deliveryState": "delivery_failed", "reviewState": "unverified", "executionMethod": execution_method(&kind), "attemptNumber": attempt, "costSnapshot": cost_snapshot(sequence, requests, tokens)}));
        }
        let (sequence, requests, tokens) = record_result_event_row(c, command, &conversation, &task, attempt, &kind, "posted")?;
        Ok(json!({"commandId": command, "state": "executed", "replayed": false, "executionStarted": scan_execution_started(&kind), "verified": false, "conversationId": conversation, "reviewState": "unverified", "executionMethod": execution_method(&kind), "attemptNumber": attempt, "costSnapshot": cost_snapshot(sequence, requests, tokens)}))
    };
    if c.is_autocommit() {
        let tx = c.unchecked_transaction().map_err(|error| error.to_string())?;
        let value = write(&tx)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(value)
    } else {
        write(c)
    }
}

pub(crate) fn latest_command(c: &rusqlite::Connection, task: &str) -> Value {
    if ensure_command_scope(c).is_err() { return Value::Null; }
    let row = c.query_row(
        "SELECT k.command_id, k.state, k.attempt_number, k.kind, k.payload_text, k.plan_boundary, k.failure_reason, s.attempt_count FROM nest_task_commands k JOIN sentinel_scans s ON s.id=k.task_id WHERE k.task_id=?1 ORDER BY k.rowid DESC LIMIT 1",
        [task], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, i64>(7)?)),
    );
    let Ok((id, state, attempt, kind, scope, boundary, failure, current)) = row else { return Value::Null; };
    if attempt != current {
        // One case: executed or verified resume on the immediate parent of this scanning child.
        // Accepted or delivered stays a mismatch. The attempt number is not rewritten.
        if !parent_resume_on_scanning_child(c, task, &kind, &state, attempt, current) {
            return json!({"commandId": id, "state": "attempt_mismatch", "attemptNumber": attempt, "currentAttempt": current, "appliesToCurrentAttempt": false, "executionStarted": false, "verified": false});
        }
    }
    let mut value = json!({"commandId": id, "kind": kind, "state": state, "attemptNumber": attempt, "executionStarted": scan_execution_started(&kind) && (state == "executed" || state == "verified"), "verified": kind != "reanalyze" && state == "verified"});
    if !failure.is_empty() { value["failureReason"] = json!(failure); }
    reanalyze_fields(&mut value, &kind, &scope);
    supplement_fields(&mut value, &kind, &boundary, attempt, current);
    if ensure_result_event(c).is_ok() {
        if let Ok((method, review, evidence, sequence, requests, tokens, delivery_state)) = c.query_row(
            "SELECT execution_method, review_state, evidence_ref, sequence, model_requests, total_tokens, state FROM nest_result_deliveries WHERE command_id=?1 ORDER BY created_at DESC LIMIT 1",
            [&id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?, row.get::<_, String>(6)?)),
        ) {
            if !method.is_empty() { value["executionMethod"] = json!(method); }
            if !review.is_empty() { value["reviewState"] = json!(review); }
            if !evidence.is_empty() { value["evidenceRef"] = json!(evidence); }
            if sequence > 0 { value["costSnapshot"] = cost_snapshot(sequence, requests, tokens); }
            if delivery_state == "delivery_failed" {
                value["state"] = json!("delivery_failed");
                value["deliveryState"] = json!("delivery_failed");
                value["verified"] = json!(false);
                value["executionStarted"] = json!(scan_execution_started(&kind));
            }
        }
    }
    let scan_status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_default();
    if kind == "continue_targets" && state == "executed" && scan_status == "failed" {
        value["state"] = json!("execution_failed");
        value["verified"] = json!(false);
        value["executionStarted"] = json!(false);
        value["executionOutcome"] = json!("failed");
        value["preflightCoversResult"] = json!(false);
        value["failureReason"] = json!("execution_failed_not_covered_by_preflight");
    }
    if matches!(kind.as_str(), "pause" | "cancel") {
        value["dispatchStopped"] = json!(true);
        value["engineStopped"] = json!(kind == "pause" && matches!(state.as_str(), "executed" | "verified"));
        value["executionStarted"] = json!(false);
    }
    value
}

/// Executed resume on the immediate parent of the scanning child, while its
/// own execution result is still pending or delivery_failed. Empty verification
/// is not a conclusion. Accepted, delivered, a posted result, and every other
/// attempt stay outside this predicate. This is a read filter, not a write.
const PARENT_RESUME_RESULT_PENDING: &str = "\
k.kind='resume' AND k.state='executed' AND s.status='scanning' \
AND k.attempt_number!=s.attempt_count \
AND EXISTS ( \
  SELECT 1 FROM sentinel_scan_attempts a \
  WHERE a.scan_id=k.task_id AND a.attempt_number=s.attempt_count \
    AND a.execution_mode='resume' AND a.parent_attempt_number=k.attempt_number \
    AND a.parent_attempt_number!=a.attempt_number \
) \
AND EXISTS ( \
  SELECT 1 FROM nest_result_deliveries d \
  WHERE d.command_id=k.command_id AND d.state IN ('pending','delivery_failed') \
)";

/// Executed resume on the immediate parent of the scanning child, while its
/// execution result is already posted and its verification conclusion is still
/// pending or delivery_failed. A pending execution result stays with
/// `PARENT_RESUME_RESULT_PENDING`. An empty conclusion, a posted conclusion,
/// and every other attempt stay outside this predicate. This is a read filter,
/// not a write.
const PARENT_RESUME_VERIFICATION_PENDING: &str = "\
k.kind='resume' AND k.state='executed' AND s.status='scanning' \
AND k.attempt_number!=s.attempt_count \
AND EXISTS ( \
  SELECT 1 FROM sentinel_scan_attempts a \
  WHERE a.scan_id=k.task_id AND a.attempt_number=s.attempt_count \
    AND a.execution_mode='resume' AND a.parent_attempt_number=k.attempt_number \
    AND a.parent_attempt_number!=a.attempt_number \
) \
AND EXISTS ( \
  SELECT 1 FROM nest_result_deliveries d \
  WHERE d.command_id=k.command_id AND d.state='posted' \
    AND d.verification_delivery IN ('pending','delivery_failed') \
) \
AND NOT EXISTS ( \
  SELECT 1 FROM nest_result_deliveries d \
  WHERE d.command_id=k.command_id AND d.state IN ('pending','delivery_failed') \
) \
AND NOT EXISTS ( \
  SELECT 1 FROM nest_result_deliveries d \
  WHERE d.command_id=k.command_id AND d.verification_delivery='posted' \
)";

/// Commands still waiting on this attempt: accepted, a result delivery stuck
/// pending / delivery_failed, or an execution result already posted whose
/// verification conclusion is still pending / delivery_failed. Also the one
/// executed parent resume in `PARENT_RESUME_RESULT_PENDING` or
/// `PARENT_RESUME_VERIFICATION_PENDING`. A posted execution result with a
/// posted verification conclusion is not waiting. A newer command does not
/// hide them. This read does not insert work.
pub(crate) fn pending_command_count(c: &rusqlite::Connection, task: &str) -> i64 {
    if ensure_command_scope(c).is_err() || ensure_result_event(c).is_err() { return 0; }
    let sql = format!(
        "SELECT COUNT(DISTINCT k.command_id) FROM nest_task_commands k
         JOIN sentinel_scans s ON s.id=k.task_id
         WHERE k.task_id=?1 AND ((k.attempt_number=s.attempt_count
           AND (k.state='accepted' OR EXISTS (
             SELECT 1 FROM nest_result_deliveries d
             WHERE d.command_id=k.command_id AND (
               d.state IN ('pending','delivery_failed')
               OR (d.state='posted' AND d.verification_delivery IN ('pending','delivery_failed'))
             )
           ))) OR ({PARENT_RESUME_RESULT_PENDING}) OR ({PARENT_RESUME_VERIFICATION_PENDING}))"
    );
    c.query_row(&sql, [task], |row| row.get(0)).unwrap_or(0)
}

/// Ordinary rows still waiting on this attempt, plus the one executed parent
/// resume in `PARENT_RESUME_RESULT_PENDING`. Accepted, or a result delivery
/// still pending / delivery_failed. A verification-conclusion delivery is not
/// included. The whole current attempt is counted. This read does not insert work.
pub(crate) fn pending_instruction_count(c: &rusqlite::Connection, task: &str) -> i64 {
    if ensure_command_scope(c).is_err() || ensure_result_event(c).is_err() { return 0; }
    let sql = format!(
        "SELECT COUNT(DISTINCT k.command_id) FROM nest_task_commands k
         JOIN sentinel_scans s ON s.id=k.task_id
         WHERE k.task_id=?1 AND ((k.attempt_number=s.attempt_count
           AND (k.state='accepted' OR EXISTS (
             SELECT 1 FROM nest_result_deliveries d
             WHERE d.command_id=k.command_id AND d.state IN ('pending','delivery_failed')
           ))) OR ({PARENT_RESUME_RESULT_PENDING}))"
    );
    c.query_row(&sql, [task], |row| row.get(0)).unwrap_or(0)
}

/// Verification conclusions still waiting on this attempt, plus the one
/// executed parent resume in `PARENT_RESUME_VERIFICATION_PENDING`. The
/// execution result is already posted, and verification_delivery is pending
/// or delivery_failed. A parent resume whose execution result is still
/// pending or delivery_failed stays in `pending_instruction_count`. Posted
/// execution alone, a posted verification, an empty verification, a verified
/// parent resume, and other attempts are not counted. This read does not insert work.
pub(crate) fn pending_verification_count(c: &rusqlite::Connection, task: &str) -> i64 {
    if ensure_command_scope(c).is_err() || ensure_result_event(c).is_err() { return 0; }
    let sql = format!(
        "SELECT COUNT(DISTINCT k.command_id) FROM nest_task_commands k
         JOIN sentinel_scans s ON s.id=k.task_id
         WHERE k.task_id=?1 AND ((k.attempt_number=s.attempt_count
           AND k.state!='accepted'
           AND NOT EXISTS (
             SELECT 1 FROM nest_result_deliveries d
             WHERE d.command_id=k.command_id AND d.state IN ('pending','delivery_failed')
           )
           AND EXISTS (
             SELECT 1 FROM nest_result_deliveries d
             WHERE d.command_id=k.command_id AND d.state='posted'
               AND d.verification_delivery IN ('pending','delivery_failed')
           )) OR ({PARENT_RESUME_VERIFICATION_PENDING}))"
    );
    c.query_row(&sql, [task], |row| row.get(0)).unwrap_or(0)
}

/// Ids still waiting, same set as `pending_command_count`. At most 8, oldest
/// first. No payload or notes. A parent resume whose execution result is still
/// pending or delivery_failed reports that same state. A posted execution whose
/// verification conclusion is still pending reports verification_pending or
/// verification_delivery_failed.
pub(crate) fn pending_commands(c: &rusqlite::Connection, task: &str) -> Vec<Value> {
    if ensure_command_scope(c).is_err() || ensure_result_event(c).is_err() { return Vec::new(); }
    let sql = format!(
        "SELECT k.command_id,
            CASE
              WHEN {PARENT_RESUME_RESULT_PENDING} THEN (
                SELECT d.state FROM nest_result_deliveries d
                WHERE d.command_id=k.command_id AND d.state IN ('pending','delivery_failed')
                ORDER BY d.rowid DESC LIMIT 1
              )
              WHEN {PARENT_RESUME_VERIFICATION_PENDING} THEN (
                SELECT CASE d.verification_delivery
                  WHEN 'delivery_failed' THEN 'verification_delivery_failed'
                  ELSE 'verification_pending'
                END
                FROM nest_result_deliveries d
                WHERE d.command_id=k.command_id
                  AND d.verification_delivery IN ('pending','delivery_failed')
                ORDER BY d.rowid DESC LIMIT 1
              )
              WHEN k.state='accepted' THEN 'accepted'
              WHEN EXISTS (
                SELECT 1 FROM nest_result_deliveries d
                WHERE d.command_id=k.command_id AND d.state IN ('pending','delivery_failed')
              ) THEN (
                SELECT d.state FROM nest_result_deliveries d
                WHERE d.command_id=k.command_id AND d.state IN ('pending','delivery_failed')
                ORDER BY d.rowid DESC LIMIT 1
              )
              ELSE (
                SELECT CASE d.verification_delivery
                  WHEN 'delivery_failed' THEN 'verification_delivery_failed'
                  ELSE 'verification_pending'
                END
                FROM nest_result_deliveries d
                WHERE d.command_id=k.command_id AND d.state='posted'
                  AND d.verification_delivery IN ('pending','delivery_failed')
                ORDER BY d.rowid DESC LIMIT 1
              )
            END
         FROM nest_task_commands k
         JOIN sentinel_scans s ON s.id=k.task_id
         WHERE k.task_id=?1 AND ((k.attempt_number=s.attempt_count
           AND (k.state='accepted' OR EXISTS (
             SELECT 1 FROM nest_result_deliveries d
             WHERE d.command_id=k.command_id AND (
               d.state IN ('pending','delivery_failed')
               OR (d.state='posted' AND d.verification_delivery IN ('pending','delivery_failed'))
             )
           ))) OR ({PARENT_RESUME_RESULT_PENDING}) OR ({PARENT_RESUME_VERIFICATION_PENDING}))
         ORDER BY k.rowid ASC LIMIT 8"
    );
    let Ok(mut statement) = c.prepare(&sql) else { return Vec::new(); };
    let Ok(rows) = statement.query_map([task], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))) else { return Vec::new(); };
    rows.filter_map(|row| row.ok()).filter_map(|(id, state)| {
        let state = state?;
        if !matches!(state.as_str(), "accepted" | "pending" | "delivery_failed" | "verification_pending" | "verification_delivery_failed") { return None; }
        Some(json!({"commandId": id, "state": state}))
    }).collect()
}

/// Parent stored on this child attempt when the child itself is a resume.
/// Empty, a different mode, or a self-link is not a parent. Ancestors are not walked.
fn immediate_resume_parent(c: &rusqlite::Connection, task: &str, child: i64) -> Option<i64> {
    c.query_row(
        "SELECT parent_attempt_number FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2 AND execution_mode='resume'",
        params![task, child], |row| row.get::<_, Option<i64>>(0),
    ).ok().flatten().filter(|parent| *parent != child)
}

/// Executed or verified resume whose attempt is the immediate parent of the scanning child.
/// Accepted, delivered, a non-scanning status, an empty parent, and older ancestors stay out.
fn parent_resume_on_scanning_child(c: &rusqlite::Connection, task: &str, kind: &str, state: &str, attempt: i64, current: i64) -> bool {
    kind == "resume"
        && matches!(state, "executed" | "verified")
        && immediate_resume_parent(c, task, current) == Some(attempt)
        && c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get::<_, String>(0)).unwrap_or_default() == "scanning"
}

/// The continuation holder that is claiming this attempt. Pause and resume
/// have no separate worker identity, so they stay blank.
fn observed_execution_actor(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64) -> String {
    if kind != "continue_targets" {
        return String::new();
    }
    c.query_row(
        "SELECT holder_id FROM scan_continuation_holders WHERE scan_id=?1 AND attempt_number=?2 AND state='claiming' AND holder_id<>'' LIMIT 1",
        params![task, attempt], |row| row.get(0),
    ).unwrap_or_default()
}

fn execution_observed(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64) -> bool {
    match kind {
        "continue_targets" => c.query_row(
            "SELECT launched FROM scan_target_continuation_fences WHERE scan_id=?1 AND attempt_number=?2 AND launched=1 LIMIT 1",
            params![task, attempt], |row| row.get::<_, i64>(0),
        ).unwrap_or(0) == 1,
        "pause" => c.query_row(
            "SELECT status, attempt_count FROM sentinel_scans WHERE id=?1",
            [task], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        ).ok().is_some_and(|(status, current)| status == "paused" && current == attempt),
        "resume" => c.query_row(
            "SELECT status, attempt_count FROM sentinel_scans WHERE id=?1",
            [task], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        ).ok().is_some_and(|(status, current)| {
            status == "scanning" && (current == attempt || immediate_resume_parent(c, task, current) == Some(attempt))
        }),
        _ => false,
    }
}

fn execution_failure_code(reason: &str) -> String {
    let token = reason.split(':').next().unwrap_or(reason).trim();
    if !token.is_empty() && token.len() <= 80 && token.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        token.to_string()
    } else {
        "execution_failed".into()
    }
}

/// A continuation that did not claim execution is a failure, not an in-progress accept.
/// Already executed, verified, or superseded commands stay as they are.
pub(crate) fn mark_execution_failed(c: &rusqlite::Connection, project: i64, command: &str, reason: &str) -> Result<Value, String> {
    ensure_command_scope(c)?;
    let (state, kind): (String, String) = c.query_row(
        "SELECT state, kind FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
        params![command, project], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "command_unavailable".to_string())?;
    if kind != "continue_targets" { return Err("command_not_failing".into()); }
    if matches!(state.as_str(), "executed" | "verified" | "verification_failed" | "superseded" | "cancelled" | "rejected" | "waiting") {
        return Ok(json!({"commandId": command, "state": state, "changed": false, "executionStarted": state == "executed" || state == "verified", "verified": state == "verified"}));
    }
    let code = execution_failure_code(reason);
    if state == "execution_failed" {
        let stored: String = c.query_row(
            "SELECT failure_reason FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
            params![command, project], |row| row.get(0),
        ).unwrap_or(code);
        return Ok(json!({"commandId": command, "state": "execution_failed", "changed": false, "executionStarted": false, "verified": false, "failureReason": stored}));
    }
    if !matches!(state.as_str(), "accepted" | "delivered") { return Err("command_not_failing".into()); }
    let changed = c.execute(
        "UPDATE nest_task_commands SET state='execution_failed', failure_reason=?3 WHERE command_id=?1 AND project_id=?2 AND state IN ('accepted','delivered')",
        params![command, project, code],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("command_not_failing".into()); }
    let block_delivery = post_block_reason(c, project, command, "execution_failed", &code);
    Ok(json!({"commandId": command, "state": "execution_failed", "changed": true, "executionStarted": false, "verified": false, "failureReason": code, "blockDelivery": block_delivery}))
}

/// A preflight refusal happens before execution. It is not an execution failure.
pub(crate) fn mark_rejected(c: &rusqlite::Connection, project: i64, command: &str, reason: &str) -> Result<Value, String> {
    ensure_command_scope(c)?;
    let (state, kind): (String, String) = c.query_row(
        "SELECT state, kind FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
        params![command, project], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "command_unavailable".to_string())?;
    if kind != "continue_targets" { return Err("command_not_failing".into()); }
    if matches!(state.as_str(), "executed" | "verified" | "verification_failed" | "superseded" | "cancelled" | "execution_failed") {
        return Ok(json!({"commandId": command, "state": state, "changed": false, "executionStarted": state == "executed" || state == "verified", "verified": state == "verified"}));
    }
    let code = execution_failure_code(reason);
    if state == "rejected" {
        let stored: String = c.query_row(
            "SELECT failure_reason FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
            params![command, project], |row| row.get(0),
        ).unwrap_or(code);
        return Ok(json!({"commandId": command, "state": "rejected", "changed": false, "executionStarted": false, "verified": false, "failureReason": stored}));
    }
    if !matches!(state.as_str(), "accepted" | "delivered") { return Err("command_not_failing".into()); }
    let changed = c.execute(
        "UPDATE nest_task_commands SET state='rejected', failure_reason=?3 WHERE command_id=?1 AND project_id=?2 AND state IN ('accepted','delivered')",
        params![command, project, code],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("command_not_failing".into()); }
    let block_delivery = post_block_reason(c, project, command, "rejected", &code);
    Ok(json!({"commandId": command, "state": "rejected", "changed": true, "executionStarted": false, "verified": false, "failureReason": code, "blockDelivery": block_delivery}))
}

/// The pause has been requested and has not settled. Continuation waits; it has not failed or started.
/// One waiting_for_pause note goes to the bound conversation. This does not pause the scan.
pub(crate) fn mark_waiting(c: &rusqlite::Connection, project: i64, command: &str) -> Result<Value, String> {
    ensure_command_scope(c)?;
    let (state, kind): (String, String) = c.query_row(
        "SELECT state, kind FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
        params![command, project], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "command_unavailable".to_string())?;
    if kind != "continue_targets" { return Err("command_not_failing".into()); }
    if state == "waiting" {
        return Ok(json!({"commandId": command, "state": "waiting", "changed": false, "executionStarted": false, "verified": false, "failureReason": "waiting_for_pause"}));
    }
    if !matches!(state.as_str(), "accepted" | "delivered") { return Err("command_not_failing".into()); }
    let changed = c.execute(
        "UPDATE nest_task_commands SET state='waiting', failure_reason='waiting_for_pause' WHERE command_id=?1 AND project_id=?2 AND state IN ('accepted','delivered')",
        params![command, project],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("command_not_failing".into()); }
    let waiting_delivery = post_waiting_for_pause(c, project, command);
    Ok(json!({"commandId": command, "state": "waiting", "changed": true, "executionStarted": false, "verified": false, "failureReason": "waiting_for_pause", "waitingDelivery": waiting_delivery}))
}

const REVIEW_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS nest_command_reviews(command_id TEXT PRIMARY KEY, reviewer_id TEXT NOT NULL, note TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))";

/// An independent reviewer records a check. A passing note does not itself
/// verify the command. A failing note is a verification failure, not "in progress".
pub(crate) fn record_review(c: &rusqlite::Connection, project: i64, command: &str, reviewer_id: &str, note: &str, outcome: &str) -> Result<Value, String> {
    c.execute_batch(REVIEW_SCHEMA).map_err(|error| error.to_string())?;
    if reviewer_id.trim().is_empty() || note.trim().is_empty() || !matches!(outcome, "passed" | "failed") {
        return Err("review_not_recorded".into());
    }
    let _ = c.execute(
        "ALTER TABLE nest_task_commands ADD COLUMN execution_actor TEXT NOT NULL DEFAULT ''",
        [],
    );
    let (state, kind, execution_actor): (String, String, String) = c.query_row(
        "SELECT state, kind, execution_actor FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
        params![command, project], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(|_| "command_unavailable".to_string())?;
    if kind == "reanalyze" { return Err("reanalyze_is_not_a_review".into()); }
    if !execution_actor.is_empty() && execution_actor == reviewer_id.trim() {
        return Err("review_not_independent".into());
    }
    // The reviewer must be a reviewer run of this task that concluded a review
    // request on the task's own evidence. A name the caller typed is not one.
    let task: String = c.query_row("SELECT task_id FROM nest_task_commands WHERE command_id=?1", [command], |row| row.get(0))
        .map_err(|_| "command_unavailable".to_string())?;
    let review_status: Option<String> = c.query_row(
        "SELECT r.status FROM agent_review_requests r JOIN agent_runs root ON root.id=r.root_run_id JOIN agent_runs reviewer ON reviewer.id=r.reviewer_run_id
         WHERE r.reviewer_run_id=?1 AND root.scan_id=?2 AND reviewer.scan_id=?2 AND reviewer.role='evidence_reviewer'
           AND r.status IN ('confirmed','rejected','insufficient_evidence','needs_evidence')
         ORDER BY r.updated_at DESC LIMIT 1",
        params![reviewer_id.trim(), task], |row| row.get(0),
    ).optional().unwrap_or(None);
    let Some(review_status) = review_status else { return Err("review_reviewer_not_independent_run".into()); };
    if (outcome == "passed") != (review_status == "confirmed") {
        return Err("review_outcome_not_supported_by_reviewer_run".into());
    }
    if state == "verification_failed" {
        return Ok(json!({"commandId": command, "reviewRecorded": true, "verified": false, "state": "verification_failed", "replayed": true}));
    }
    if state == "verified" { return Err("verification_already_recorded".into()); }
    if state != "executed" { return Err("review_not_recorded".into()); }
    let stored_note = crate::agent_runtime::secrets::redact_text_with(note.trim(), None);
    if stored_note.trim().is_empty() { return Err("review_not_recorded".into()); }
    let _ = c.execute("ALTER TABLE nest_command_reviews ADD COLUMN execution_actor TEXT NOT NULL DEFAULT ''", []);
    c.execute(
        "INSERT INTO nest_command_reviews(command_id,reviewer_id,note,execution_actor) VALUES(?1,?2,?3,?4) ON CONFLICT(command_id) DO NOTHING",
        params![command, reviewer_id.trim(), stored_note, execution_actor],
    ).map_err(|error| error.to_string())?;
    if outcome == "failed" {
        let changed = c.execute(
            "UPDATE nest_task_commands SET state='verification_failed', failure_reason='verification_failed' WHERE command_id=?1 AND project_id=?2 AND state='executed'",
            params![command, project],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("review_not_recorded".into()); }
        let verification_delivery = post_verification_failure(c, project, command, &stored_note);
        set_review_state(c, command, "verification_failed");
        return Ok(json!({"commandId": command, "reviewRecorded": true, "verified": false, "state": "verification_failed", "replayed": false, "executionStarted": false, "verificationDelivery": verification_delivery}));
    }
    Ok(json!({"commandId": command, "reviewRecorded": true, "verified": false, "state": "executed"}))
}

fn field_text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn without_target_urls(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(relative) = ["https://", "http://"].into_iter().filter_map(|scheme| lower[cursor..].find(scheme)).min() {
        let start = cursor + relative;
        let end = text[start..].char_indices()
            .find(|(_, character)| character.is_whitespace() || "。，；、,;)".contains(*character))
            .map(|(offset, _)| start + offset)
            .unwrap_or(text.len());
        out.push_str(&text[cursor..start]);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn omit_stored_target_urls(c: &rusqlite::Connection, task: &str, text: &str) -> String {
    let mut text = without_target_urls(text);
    let Ok(mut statement) = c.prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1") else { return text; };
    let Ok(rows) = statement.query_map([task], |row| row.get::<_, String>(0)) else { return text; };
    for url in rows.flatten() {
        if url.contains("://") { text = text.replace(&url, ""); }
    }
    text
}

fn verification_conclusion_note(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64, command: &str, review_state: &str, rule: &str) -> String {
    let usage = match c.query_row(
        "SELECT llm_requests, total_tokens FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    ) {
        Ok((requests, tokens)) => format!("引擎用量 {requests} 次请求、{tokens} token。"),
        Err(_) => "引擎用量没有读到，不能当成 0。".into(),
    };
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_else(|_| "没有读到".into());
    let current_attempt: i64 = c.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or(attempt);
    let command_state: String = c.query_row("SELECT state FROM nest_task_commands WHERE command_id=?1", [command], |row| row.get(0)).unwrap_or_default();
    let parent_child = parent_resume_on_scanning_child(c, task, kind, &command_state, attempt, current_attempt);
    let evidence = evidence_ref(c, kind, task, attempt);
    let evidence_text = if evidence.is_empty() { "无".to_string() } else { evidence };
    let parent_lead = format!("已验证：恢复指令符合本次验证规则。核对的是父 attempt {attempt}，子 attempt {current_attempt} 正在扫描。这不是扫描发现的独立复核，也不是所有目标都已完成。");
    let lead = match rule {
        "paused_same_attempt" => "已验证：暂停指令符合本次验证规则。核对的是持久暂停状态和原 attempt。这不是扫描发现的独立复核，也不是所有目标都已完成。",
        "resumed_same_attempt" if parent_child => parent_lead.as_str(),
        "resumed_same_attempt" => "已验证：恢复指令符合本次验证规则。核对的是仍在扫描且 attempt 没有换新。这不是扫描发现的独立复核，也不是所有目标都已完成。",
        _ => "已验证：继续剩余目标的指令符合本次验证规则。独立复核已记录。这不是所有目标都已完成，未覆盖的目标不能说成已经完成。",
    };
    let method = if parent_child { "backend_scanning_child_attempt" } else { execution_method(kind) };
    let redacted = crate::agent_runtime::secrets::redact_json_with(&json!({
        "lead": lead,
        "task": task,
        "attempt": attempt.to_string(),
        "command": command,
        "method": method,
        "evidence": evidence_text,
        "reviewState": review_state,
        "rule": rule,
        "progress": target_progress_phrase(c, task),
        "usage": usage,
        "status": status,
    }), None);
    let note = format!(
        "{lead} 验证规则 {rule}。任务 {task}，attempt {attempt}，指令 {command}，方式 {method}，证据 {evidence}。复核状态 {review}。{progress}{usage}没有新开额度。剩余额度没有在这条回传里重算，不能当成还有额度或已经用尽。扫描状态是 {status}。结论依据是本次验证规则，不是浏览器验证，也不是对已有证据的重新分析。",
        lead = field_text(&redacted, "lead"),
        rule = field_text(&redacted, "rule"),
        task = field_text(&redacted, "task"),
        attempt = field_text(&redacted, "attempt"),
        command = field_text(&redacted, "command"),
        method = field_text(&redacted, "method"),
        evidence = field_text(&redacted, "evidence"),
        review = field_text(&redacted, "reviewState"),
        progress = field_text(&redacted, "progress"),
        usage = field_text(&redacted, "usage"),
        status = field_text(&redacted, "status"),
    );
    let note = crate::agent_runtime::secrets::redact_text_with(&note, None);
    omit_stored_target_urls(c, task, &note)
}

fn store_verification_delivery(c: &rusqlite::Connection, command: &str, conversation: &str, task: &str, attempt: i64, kind: &str, review_state: &str, delivery: &str, note: &str) -> Result<(), String> {
    ensure_result_event(c)?;
    let saved = if delivery == "dropped" { "" } else { note };
    let current: Option<String> = c.query_row(
        "SELECT verification_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if let Some(existing) = current {
        if !existing.is_empty() { return Ok(()); }
        c.execute(
            "UPDATE nest_result_deliveries SET verification_delivery=?3, verification_note=?4 WHERE command_id=?1 AND conversation_id=?2 AND verification_delivery=''",
            params![command, conversation, delivery, saved],
        ).map_err(|error| error.to_string())?;
        return Ok(());
    }
    c.execute(
        "INSERT INTO nest_result_deliveries(delivery_id,command_id,conversation_id,state,task_id,attempt_number,execution_method,review_state,evidence_ref,verification_delivery,verification_note) VALUES(?1,?2,?3,'recorded',?4,?5,?6,?7,?8,?9,?10)",
        params![uuid::Uuid::new_v4().to_string(), command, conversation, task, attempt, execution_method(kind), review_state, evidence_ref(c, kind, task, attempt), delivery, saved],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn verification_failure_note(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64, command: &str, review_note: &str) -> String {
    let usage = match c.query_row(
        "SELECT llm_requests, total_tokens FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    ) {
        Ok((requests, tokens)) => format!("引擎用量 {requests} 次请求、{tokens} token。"),
        Err(_) => "引擎用量没有读到，不能当成 0。".into(),
    };
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_else(|_| "没有读到".into());
    let evidence = evidence_ref(c, kind, task, attempt);
    let evidence_text = if evidence.is_empty() { "无".to_string() } else { evidence };
    let lead = "验证失败：独立复核没有通过。这不是已验证，也不是扫描发现已经确认。未覆盖的目标不能说成已经完成。";
    let redacted = crate::agent_runtime::secrets::redact_json_with(&json!({
        "lead": lead,
        "task": task,
        "attempt": attempt.to_string(),
        "command": command,
        "method": execution_method(kind),
        "evidence": evidence_text,
        "reason": review_note,
        "progress": target_progress_phrase(c, task),
        "usage": usage,
        "status": status,
    }), None);
    let note = format!(
        "{lead} 失败说明：{reason}。任务 {task}，attempt {attempt}，指令 {command}，方式 {method}，证据 {evidence}。复核状态 verification_failed。{progress}{usage}没有新开额度。剩余额度没有在这条回传里重算，不能当成还有额度或已经用尽。扫描状态是 {status}。结论依据是这次未通过的复核，不是浏览器验证，也不是对已有证据的重新分析。",
        lead = field_text(&redacted, "lead"),
        reason = field_text(&redacted, "reason"),
        task = field_text(&redacted, "task"),
        attempt = field_text(&redacted, "attempt"),
        command = field_text(&redacted, "command"),
        method = field_text(&redacted, "method"),
        evidence = field_text(&redacted, "evidence"),
        progress = field_text(&redacted, "progress"),
        usage = field_text(&redacted, "usage"),
        status = field_text(&redacted, "status"),
    );
    let note = crate::agent_runtime::secrets::redact_text_with(&note, None);
    omit_stored_target_urls(c, task, &note)
}

/// Posts one verification-failure conclusion to the bound conversation.
/// An archived or missing conversation keeps it pending. An empty redaction is dropped.
/// A second conversation is never chosen, and a later retry posts the stored note once.
fn post_verification_failure(c: &rusqlite::Connection, project: i64, command: &str, review_note: &str) -> String {
    if ensure_result_event(c).is_err() { return "pending".into(); }
    let bound = c.query_row(
        "SELECT b.conversation_id, k.task_id, k.attempt_number, k.kind FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1 AND k.project_id=?2",
        params![command, project], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?)),
    );
    let Ok((conversation, task, attempt, kind)) = bound else { return "unbound".into(); };
    if conversation.is_empty() { return "unbound".into(); }
    let existing: String = c.query_row(
        "SELECT verification_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).unwrap_or_default();
    if !existing.is_empty() { return existing; }
    let note = verification_failure_note(c, &kind, &task, attempt, command, review_note);
    if note.trim().is_empty() {
        let _ = store_verification_delivery(c, command, &conversation, &task, attempt, &kind, "verification_failed", "dropped", "");
        return "dropped".into();
    }
    let available: i64 = bound_conversation_available(c, project, &conversation);
    if available != 1 {
        let _ = store_verification_delivery(c, command, &conversation, &task, attempt, &kind, "verification_failed", "pending", &note);
        return "pending".into();
    }
    let posted = post_bound_message(c, project, &conversation, "post_verification_failure", command, &task, &note);
    if posted.is_err() {
        let _ = store_verification_delivery(c, command, &conversation, &task, attempt, &kind, "verification_failed", "delivery_failed", &note);
        return "delivery_failed".into();
    }
    let _ = store_verification_delivery(c, command, &conversation, &task, attempt, &kind, "verification_failed", "posted", &note);
    "posted".into()
}

/// Says the supplement was accepted and the saved budget cap stayed unchanged.
/// Names the schedule boundary already stored on the command. A missing boundary
/// is left unnamed. This does not say the scan started, that a new allowance was
/// minted, or that closed work changed.
fn accepted_budget_unchanged_note(c: &rusqlite::Connection, task: &str, attempt: i64, command: &str) -> String {
    let boundary: String = c.query_row(
        "SELECT plan_boundary FROM nest_task_commands WHERE command_id=?1",
        [command], |row| row.get(0),
    ).unwrap_or_default();
    let boundary_sentence = if boundary.is_empty() {
        String::new()
    } else {
        format!("调度边界是 {boundary}。")
    };
    let lead = "已接受：补充方向已保存到任务，预算保持原上限。";
    let redacted = crate::agent_runtime::secrets::redact_json_with(&json!({
        "lead": lead,
        "boundary": boundary_sentence,
        "task": task,
        "attempt": attempt.to_string(),
        "command": command,
    }), None);
    let note = format!(
        "{lead}{boundary}任务 {task}，attempt {attempt}，指令 {command}。没有新开额度。",
        lead = field_text(&redacted, "lead"),
        boundary = field_text(&redacted, "boundary"),
        task = field_text(&redacted, "task"),
        attempt = field_text(&redacted, "attempt"),
        command = field_text(&redacted, "command"),
    );
    let note = crate::agent_runtime::secrets::redact_text_with(&note, None);
    omit_stored_target_urls(c, task, &note)
}

/// Names the authorized target that was actually inserted, and whether the cap
/// stayed unchanged. It does not say the scan started or that an allowance was minted.
fn applied_gate_note(c: &rusqlite::Connection, task: &str, attempt: i64, command: &str, names: &[String], added: usize, cap_not_raised: bool) -> String {
    let boundary: String = c.query_row(
        "SELECT plan_boundary FROM nest_task_commands WHERE command_id=?1",
        [command], |row| row.get(0),
    ).unwrap_or_default();
    let boundary_sentence = if boundary.is_empty() {
        String::new()
    } else {
        format!("调度边界是 {boundary}。")
    };
    let target_sentence = if added == 0 {
        "没有加入新的已授权目标。".to_string()
    } else if names.is_empty() {
        format!("已加入 {added} 个已授权目标。")
    } else {
        format!("已加入已授权目标 {}。", listed_names(names))
    };
    let cap_sentence = if cap_not_raised {
        "额度门允许了这次请求，预算上限没有提高。"
    } else {
        "预算上限保持不变。"
    };
    let lead = format!("已保存：{target_sentence}{cap_sentence}");
    let redacted = crate::agent_runtime::secrets::redact_json_with(&json!({
        "lead": lead,
        "boundary": boundary_sentence,
        "task": task,
        "attempt": attempt.to_string(),
        "command": command,
    }), None);
    let note = format!(
        "{lead}{boundary}任务 {task}，attempt {attempt}，指令 {command}。没有新开额度。没有启动扫描。",
        lead = field_text(&redacted, "lead"),
        boundary = field_text(&redacted, "boundary"),
        task = field_text(&redacted, "task"),
        attempt = field_text(&redacted, "attempt"),
        command = field_text(&redacted, "command"),
    );
    let note = crate::agent_runtime::secrets::redact_text_with(&note, None);
    omit_stored_target_urls(c, task, &note)
}

fn block_reason_note(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64, command: &str, label: &str, code: &str) -> String {
    if label == "accepted" {
        return accepted_budget_unchanged_note(c, task, attempt, command);
    }
    let usage = match c.query_row(
        "SELECT llm_requests, total_tokens FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    ) {
        Ok((requests, tokens)) => format!("引擎用量 {requests} 次请求、{tokens} token。"),
        Err(_) => "引擎用量没有读到，不能当成 0。".into(),
    };
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_else(|_| "没有读到".into());
    let evidence = evidence_ref(c, kind, task, attempt);
    let evidence_text = if evidence.is_empty() { "无".to_string() } else { evidence };
    let lead = if label == "rejected" {
        "预检拒绝：没有启动续跑。这不是已验证。未覆盖的目标不能说成已经完成。"
    } else {
        "执行失败：续跑没有完成。这不是已验证。未覆盖的目标不能说成已经完成。"
    };
    let named = if label == "rejected" {
        code.to_string()
    } else if code.is_empty() || code == "execution_failed" {
        "execution_failed".to_string()
    } else {
        format!("execution_failed，{code}")
    };
    let redacted = crate::agent_runtime::secrets::redact_json_with(&json!({
        "lead": lead,
        "task": task,
        "attempt": attempt.to_string(),
        "command": command,
        "method": execution_method(kind),
        "evidence": evidence_text,
        "reason": named,
        "progress": target_progress_phrase(c, task),
        "usage": usage,
        "status": status,
    }), None);
    let note = format!(
        "{lead} 阻塞原因是 {reason}。任务 {task}，attempt {attempt}，指令 {command}，方式 {method}，证据 {evidence}。{progress}{usage}没有新开额度。剩余额度没有在这条回传里重算，不能当成还有额度或已经用尽。扫描状态是 {status}。结论依据是这次阻塞，不是浏览器验证，也不是对已有证据的重新分析。",
        lead = field_text(&redacted, "lead"),
        reason = field_text(&redacted, "reason"),
        task = field_text(&redacted, "task"),
        attempt = field_text(&redacted, "attempt"),
        command = field_text(&redacted, "command"),
        method = field_text(&redacted, "method"),
        evidence = field_text(&redacted, "evidence"),
        progress = field_text(&redacted, "progress"),
        usage = field_text(&redacted, "usage"),
        status = field_text(&redacted, "status"),
    );
    let note = crate::agent_runtime::secrets::redact_text_with(&note, None);
    omit_stored_target_urls(c, task, &note)
}

fn store_block_delivery(c: &rusqlite::Connection, command: &str, conversation: &str, task: &str, attempt: i64, kind: &str, delivery: &str, note: &str) -> Result<(), String> {
    ensure_result_event(c)?;
    let saved = if delivery == "dropped" { "" } else { note };
    let current: Option<String> = c.query_row(
        "SELECT block_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if let Some(existing) = current {
        if !existing.is_empty() { return Ok(()); }
        c.execute(
            "UPDATE nest_result_deliveries SET block_delivery=?3, block_note=?4, state=CASE WHEN ?3 IN ('posted','dropped') AND state IN ('pending','delivery_failed','recorded','') THEN 'posted' WHEN ?3='delivery_failed' AND state IN ('pending','recorded','') THEN 'delivery_failed' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND block_delivery=''",
            params![command, conversation, delivery, saved],
        ).map_err(|error| error.to_string())?;
        return Ok(());
    }
    let row_state = match delivery {
        "posted" | "dropped" => "posted",
        "delivery_failed" => "delivery_failed",
        _ => "recorded",
    };
    c.execute(
        "INSERT INTO nest_result_deliveries(delivery_id,command_id,conversation_id,state,task_id,attempt_number,execution_method,review_state,evidence_ref,block_delivery,block_note) VALUES(?1,?2,?3,?4,?5,?6,?7,'unverified',?8,?9,?10)",
        params![uuid::Uuid::new_v4().to_string(), command, conversation, row_state, task, attempt, execution_method(kind), evidence_ref(c, kind, task, attempt), delivery, saved],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// One block reason to the bound conversation. Execution failure names execution_failed.
/// A preflight refusal names the code already stored. The same receipt is not posted twice.
/// An accepted supplement that kept the saved budget cap and added no out-of-scope target
/// uses this same post. The note says it was accepted and the cap is unchanged, and names
/// the stored schedule boundary when one was saved.
/// This does not call preflight, start a scan, reserve budget, or mark the scan verified.
fn post_block_reason(c: &rusqlite::Connection, project: i64, command: &str, label: &str, code: &str) -> String {
    if ensure_result_event(c).is_err() { return "pending".into(); }
    let bound = c.query_row(
        "SELECT b.conversation_id, k.task_id, k.attempt_number, k.kind FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1 AND k.project_id=?2",
        params![command, project], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?)),
    );
    let Ok((conversation, task, attempt, kind)) = bound else { return "unbound".into(); };
    if conversation.is_empty() { return "unbound".into(); }
    let existing: String = c.query_row(
        "SELECT block_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).unwrap_or_default();
    if !existing.is_empty() { return existing; }
    let note = block_reason_note(c, &kind, &task, attempt, command, label, code);
    if note.trim().is_empty() {
        let _ = store_block_delivery(c, command, &conversation, &task, attempt, &kind, "dropped", "");
        return "dropped".into();
    }
    let available: i64 = bound_conversation_available(c, project, &conversation);
    if available != 1 {
        let _ = store_block_delivery(c, command, &conversation, &task, attempt, &kind, "pending", &note);
        return "pending".into();
    }
    let posted = post_bound_message(c, project, &conversation, "post_block_reason", command, &task, &note);
    if posted.is_err() {
        let _ = store_block_delivery(c, command, &conversation, &task, attempt, &kind, "delivery_failed", &note);
        return "delivery_failed".into();
    }
    let _ = store_block_delivery(c, command, &conversation, &task, attempt, &kind, "posted", &note);
    "posted".into()
}

/// One prepared note to the bound conversation. Replay and retry use the stored
/// block row, so a second call does not add another message. An empty note is dropped.
/// An empty conversation id does not choose another destination.
fn post_prepared_block_note(c: &rusqlite::Connection, project: i64, command: &str, note: &str) -> String {
    if ensure_result_event(c).is_err() { return "pending".into(); }
    let bound = c.query_row(
        "SELECT b.conversation_id, k.task_id, k.attempt_number, k.kind FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1 AND k.project_id=?2",
        params![command, project], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?)),
    );
    let Ok((conversation, task, attempt, kind)) = bound else { return "unbound".into(); };
    if conversation.is_empty() { return "unbound".into(); }
    let existing: String = c.query_row(
        "SELECT block_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).unwrap_or_default();
    if !existing.is_empty() { return existing; }
    if note.trim().is_empty() {
        let _ = store_block_delivery(c, command, &conversation, &task, attempt, &kind, "dropped", "");
        return "dropped".into();
    }
    let available: i64 = bound_conversation_available(c, project, &conversation);
    if available != 1 {
        let _ = store_block_delivery(c, command, &conversation, &task, attempt, &kind, "pending", note);
        return "pending".into();
    }
    let posted = post_bound_message(c, project, &conversation, "post_prepared_block_note", command, &task, &note);
    if posted.is_err() {
        let _ = store_block_delivery(c, command, &conversation, &task, attempt, &kind, "delivery_failed", note);
        return "delivery_failed".into();
    }
    let _ = store_block_delivery(c, command, &conversation, &task, attempt, &kind, "posted", note);
    "posted".into()
}

fn waiting_pause_note(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64, command: &str) -> String {
    let usage = match c.query_row(
        "SELECT llm_requests, total_tokens FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    ) {
        Ok((requests, tokens)) => format!("引擎用量 {requests} 次请求、{tokens} token。"),
        Err(_) => "引擎用量没有读到，不能当成 0。".into(),
    };
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_else(|_| "没有读到".into());
    let evidence = evidence_ref(c, kind, task, attempt);
    let evidence_text = if evidence.is_empty() { "无".to_string() } else { evidence };
    let lead = "正在等待：扫描正在暂停，续跑还没开始。记录的状态是 waiting_for_pause。";
    let redacted = crate::agent_runtime::secrets::redact_json_with(&json!({
        "lead": lead,
        "task": task,
        "attempt": attempt.to_string(),
        "command": command,
        "method": execution_method(kind),
        "evidence": evidence_text,
        "progress": target_progress_phrase(c, task),
        "usage": usage,
        "status": status,
    }), None);
    let note = format!(
        "{lead} 任务 {task}，attempt {attempt}，指令 {command}，方式 {method}，证据 {evidence}。复核状态 unverified。{progress}{usage}没有新开额度。剩余额度没有在这条回传里重算，不能当成还有额度或已经用尽。扫描状态是 {status}。结论依据是 waiting_for_pause，不是浏览器验证，也不是对已有证据的重新分析。",
        lead = field_text(&redacted, "lead"),
        task = field_text(&redacted, "task"),
        attempt = field_text(&redacted, "attempt"),
        command = field_text(&redacted, "command"),
        method = field_text(&redacted, "method"),
        evidence = field_text(&redacted, "evidence"),
        progress = field_text(&redacted, "progress"),
        usage = field_text(&redacted, "usage"),
        status = field_text(&redacted, "status"),
    );
    let note = crate::agent_runtime::secrets::redact_text_with(&note, None);
    omit_stored_target_urls(c, task, &note)
}

fn store_waiting_delivery(c: &rusqlite::Connection, command: &str, conversation: &str, task: &str, attempt: i64, kind: &str, delivery: &str, note: &str) -> Result<(), String> {
    ensure_result_event(c)?;
    let saved = if delivery == "dropped" { "" } else { note };
    let current: Option<String> = c.query_row(
        "SELECT waiting_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if let Some(existing) = current {
        if !existing.is_empty() { return Ok(()); }
        c.execute(
            "UPDATE nest_result_deliveries SET waiting_delivery=?3, waiting_note=?4, state=CASE WHEN ?3 IN ('posted','dropped') AND state IN ('pending','delivery_failed','recorded','') THEN 'posted' WHEN ?3='delivery_failed' AND state IN ('pending','recorded','') THEN 'delivery_failed' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND waiting_delivery=''",
            params![command, conversation, delivery, saved],
        ).map_err(|error| error.to_string())?;
        return Ok(());
    }
    let row_state = match delivery {
        "posted" | "dropped" => "posted",
        "delivery_failed" => "delivery_failed",
        _ => "recorded",
    };
    c.execute(
        "INSERT INTO nest_result_deliveries(delivery_id,command_id,conversation_id,state,task_id,attempt_number,execution_method,review_state,evidence_ref,waiting_delivery,waiting_note) VALUES(?1,?2,?3,?4,?5,?6,?7,'unverified',?8,?9,?10)",
        params![uuid::Uuid::new_v4().to_string(), command, conversation, row_state, task, attempt, execution_method(kind), evidence_ref(c, kind, task, attempt), delivery, saved],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// One waiting_for_pause note to the bound conversation.
/// It says the continuation is waiting because the scan is pausing.
/// It does not say the scan has paused, completed, or failed, and it does not copy a block or verification note.
/// An archived or missing conversation keeps it pending. An empty redaction is dropped. No second conversation is chosen.
fn post_waiting_for_pause(c: &rusqlite::Connection, project: i64, command: &str) -> String {
    if ensure_result_event(c).is_err() { return "pending".into(); }
    let bound = c.query_row(
        "SELECT b.conversation_id, k.task_id, k.attempt_number, k.kind, k.state, k.failure_reason FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1 AND k.project_id=?2",
        params![command, project], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?)),
    );
    let Ok((conversation, task, attempt, kind, state, reason)) = bound else { return "unbound".into(); };
    if conversation.is_empty() || state != "waiting" || reason != "waiting_for_pause" { return "unbound".into(); }
    let (existing, block_note, verification_note): (String, String, String) = c.query_row(
        "SELECT waiting_delivery, block_note, verification_note FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap_or_default();
    if !existing.is_empty() { return existing; }
    let note = waiting_pause_note(c, &kind, &task, attempt, command);
    if (!block_note.is_empty() && note == block_note) || (!verification_note.is_empty() && note == verification_note) {
        return "posted".into();
    }
    if note.trim().is_empty() {
        let _ = store_waiting_delivery(c, command, &conversation, &task, attempt, &kind, "dropped", "");
        return "dropped".into();
    }
    let available: i64 = bound_conversation_available(c, project, &conversation);
    if available != 1 {
        let _ = store_waiting_delivery(c, command, &conversation, &task, attempt, &kind, "pending", &note);
        return "pending".into();
    }
    let posted = post_bound_message(c, project, &conversation, "post_waiting_for_pause", command, &task, &note);
    if posted.is_err() {
        let _ = store_waiting_delivery(c, command, &conversation, &task, attempt, &kind, "delivery_failed", &note);
        return "delivery_failed".into();
    }
    let _ = store_waiting_delivery(c, command, &conversation, &task, attempt, &kind, "posted", &note);
    "posted".into()
}

fn superseded_note(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64, command: &str, control: &str) -> String {
    let usage = match c.query_row(
        "SELECT llm_requests, total_tokens FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    ) {
        Ok((requests, tokens)) => format!("引擎用量 {requests} 次请求、{tokens} token。"),
        Err(_) => "引擎用量没有读到，不能当成 0。".into(),
    };
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_else(|_| "没有读到".into());
    let evidence = evidence_ref(c, kind, task, attempt);
    let evidence_text = if evidence.is_empty() { "无".to_string() } else { evidence };
    let lead = format!("已被取代：后来的暂停或取消在执行前替换了这条指令。记录的状态是 superseded。这次替换来自 {control}。");
    let redacted = crate::agent_runtime::secrets::redact_json_with(&json!({
        "lead": lead,
        "task": task,
        "attempt": attempt.to_string(),
        "command": command,
        "method": execution_method(kind),
        "evidence": evidence_text,
        "progress": target_progress_phrase(c, task),
        "usage": usage,
        "status": status,
    }), None);
    let note = format!(
        "{lead} 任务 {task}，attempt {attempt}，指令 {command}，方式 {method}，证据 {evidence}。复核状态 unverified。{progress}{usage}没有新开额度。剩余额度没有在这条回传里重算，不能当成还有额度或已经用尽。扫描状态是 {status}。结论依据是 superseded，不是浏览器验证，也不是对已有证据的重新分析。",
        lead = field_text(&redacted, "lead"),
        task = field_text(&redacted, "task"),
        attempt = field_text(&redacted, "attempt"),
        command = field_text(&redacted, "command"),
        method = field_text(&redacted, "method"),
        evidence = field_text(&redacted, "evidence"),
        progress = field_text(&redacted, "progress"),
        usage = field_text(&redacted, "usage"),
        status = field_text(&redacted, "status"),
    );
    let note = crate::agent_runtime::secrets::redact_text_with(&note, None);
    omit_stored_target_urls(c, task, &note)
}

fn store_superseded_delivery(c: &rusqlite::Connection, command: &str, conversation: &str, task: &str, attempt: i64, kind: &str, delivery: &str, note: &str) -> Result<(), String> {
    ensure_result_event(c)?;
    let saved = if delivery == "dropped" { "" } else { note };
    let current: Option<String> = c.query_row(
        "SELECT superseded_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if let Some(existing) = current {
        if !existing.is_empty() { return Ok(()); }
        c.execute(
            "UPDATE nest_result_deliveries SET superseded_delivery=?3, superseded_note=?4, state=CASE WHEN ?3 IN ('posted','dropped') AND state IN ('pending','delivery_failed','recorded','') THEN 'posted' WHEN ?3='delivery_failed' AND state IN ('pending','recorded','') THEN 'delivery_failed' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND superseded_delivery=''",
            params![command, conversation, delivery, saved],
        ).map_err(|error| error.to_string())?;
        return Ok(());
    }
    let row_state = match delivery {
        "posted" | "dropped" => "posted",
        "delivery_failed" => "delivery_failed",
        _ => "recorded",
    };
    c.execute(
        "INSERT INTO nest_result_deliveries(delivery_id,command_id,conversation_id,state,task_id,attempt_number,execution_method,review_state,evidence_ref,superseded_delivery,superseded_note) VALUES(?1,?2,?3,?4,?5,?6,?7,'unverified',?8,?9,?10)",
        params![uuid::Uuid::new_v4().to_string(), command, conversation, row_state, task, attempt, execution_method(kind), evidence_ref(c, kind, task, attempt), delivery, saved],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// One superseded note to the original bound conversation.
/// It names superseded and that a later pause or cancel replaced the command before execution.
/// It does not say the command executed, failed, or was verified.
/// A waiting_for_pause or block note that already says the same thing is not copied again.
/// An archived or missing conversation keeps it pending. An empty conversation is not a destination.
/// An empty redaction is dropped. No second conversation is chosen. This does not pause or cancel.
fn post_superseded(c: &rusqlite::Connection, project: i64, command: &str, control: &str) -> String {
    if !matches!(control, "pause" | "cancel") { return "unbound".into(); }
    if ensure_result_event(c).is_err() { return "pending".into(); }
    let bound = c.query_row(
        "SELECT b.conversation_id, k.task_id, k.attempt_number, k.kind, k.state, k.failure_reason FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1 AND k.project_id=?2",
        params![command, project], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?)),
    );
    let Ok((conversation, task, attempt, kind, state, reason)) = bound else { return "unbound".into(); };
    if conversation.is_empty() || state != "superseded" || reason != "superseded_by_control" { return "unbound".into(); }
    let (existing, waiting_note, block_note): (String, String, String) = c.query_row(
        "SELECT superseded_delivery, waiting_note, block_note FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap_or_default();
    if !existing.is_empty() { return existing; }
    let note = superseded_note(c, &kind, &task, attempt, command, control);
    if (!waiting_note.is_empty() && note == waiting_note) || (!block_note.is_empty() && note == block_note) {
        let _ = store_superseded_delivery(c, command, &conversation, &task, attempt, &kind, "posted", "");
        return "posted".into();
    }
    if note.trim().is_empty() {
        let _ = store_superseded_delivery(c, command, &conversation, &task, attempt, &kind, "dropped", "");
        return "dropped".into();
    }
    let available: i64 = bound_conversation_available(c, project, &conversation);
    if available != 1 {
        let _ = store_superseded_delivery(c, command, &conversation, &task, attempt, &kind, "pending", &note);
        return "pending".into();
    }
    let posted = post_bound_message(c, project, &conversation, "post_superseded", command, &task, &note);
    if posted.is_err() {
        let _ = store_superseded_delivery(c, command, &conversation, &task, attempt, &kind, "delivery_failed", &note);
        return "delivery_failed".into();
    }
    let _ = store_superseded_delivery(c, command, &conversation, &task, attempt, &kind, "posted", &note);
    "posted".into()
}

fn cancelled_note(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64, command: &str) -> String {
    let usage = match c.query_row(
        "SELECT llm_requests, total_tokens FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    ) {
        Ok((requests, tokens)) => format!("引擎用量 {requests} 次请求、{tokens} token。"),
        Err(_) => "引擎用量没有读到，不能当成 0。".into(),
    };
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_else(|_| "没有读到".into());
    let evidence = evidence_ref(c, kind, task, attempt);
    let evidence_text = if evidence.is_empty() { "无".to_string() } else { evidence };
    let lead = "已取消：这条取消指令自己的记录状态是 cancelled。";
    let redacted = crate::agent_runtime::secrets::redact_json_with(&json!({
        "lead": lead,
        "task": task,
        "attempt": attempt.to_string(),
        "command": command,
        "method": execution_method(kind),
        "evidence": evidence_text,
        "progress": target_progress_phrase(c, task),
        "usage": usage,
        "status": status,
    }), None);
    let note = format!(
        "{lead} 任务 {task}，attempt {attempt}，指令 {command}，方式 {method}，证据 {evidence}。复核状态 unverified。{progress}{usage}没有新开额度。剩余额度没有在这条回传里重算，不能当成还有额度或已经用尽。扫描状态是 {status}。结论依据是 cancelled，不是浏览器验证，也不是对已有证据的重新分析。",
        lead = field_text(&redacted, "lead"),
        task = field_text(&redacted, "task"),
        attempt = field_text(&redacted, "attempt"),
        command = field_text(&redacted, "command"),
        method = field_text(&redacted, "method"),
        evidence = field_text(&redacted, "evidence"),
        progress = field_text(&redacted, "progress"),
        usage = field_text(&redacted, "usage"),
        status = field_text(&redacted, "status"),
    );
    let note = crate::agent_runtime::secrets::redact_text_with(&note, None);
    omit_stored_target_urls(c, task, &note)
}

fn store_cancelled_delivery(c: &rusqlite::Connection, command: &str, conversation: &str, task: &str, attempt: i64, kind: &str, delivery: &str, note: &str) -> Result<(), String> {
    ensure_result_event(c)?;
    let saved = if delivery == "dropped" { "" } else { note };
    let current: Option<String> = c.query_row(
        "SELECT cancelled_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if let Some(existing) = current {
        if !existing.is_empty() { return Ok(()); }
        c.execute(
            "UPDATE nest_result_deliveries SET cancelled_delivery=?3, cancelled_note=?4, state=CASE WHEN ?3 IN ('posted','dropped') AND state IN ('pending','delivery_failed','recorded','') THEN 'posted' WHEN ?3='delivery_failed' AND state IN ('pending','recorded','') THEN 'delivery_failed' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND cancelled_delivery=''",
            params![command, conversation, delivery, saved],
        ).map_err(|error| error.to_string())?;
        return Ok(());
    }
    let row_state = match delivery {
        "posted" | "dropped" => "posted",
        "delivery_failed" => "delivery_failed",
        _ => "recorded",
    };
    c.execute(
        "INSERT INTO nest_result_deliveries(delivery_id,command_id,conversation_id,state,task_id,attempt_number,execution_method,review_state,evidence_ref,cancelled_delivery,cancelled_note) VALUES(?1,?2,?3,?4,?5,?6,?7,'unverified',?8,?9,?10)",
        params![uuid::Uuid::new_v4().to_string(), command, conversation, row_state, task, attempt, execution_method(kind), evidence_ref(c, kind, task, attempt), delivery, saved],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// One cancelled note to the original bound conversation of this cancel command.
/// It names cancelled. It does not say the scan executed, failed, or was verified.
/// It does not repeat a superseded, waiting, or block note already stored on this row.
/// An archived or missing conversation keeps it pending on that same conversation.
/// An empty conversation is not a destination. An empty redaction is dropped.
/// No second conversation is chosen. This does not cancel again and does not supersede anything.
fn post_cancelled(c: &rusqlite::Connection, project: i64, command: &str) -> String {
    if ensure_result_event(c).is_err() { return "pending".into(); }
    let bound = c.query_row(
        "SELECT b.conversation_id, k.task_id, k.attempt_number, k.kind, k.state FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1 AND k.project_id=?2",
        params![command, project], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)),
    );
    let Ok((conversation, task, attempt, kind, state)) = bound else { return "unbound".into(); };
    if conversation.is_empty() || state != "cancelled" || kind != "cancel" { return "unbound".into(); }
    let (existing, superseded_note, waiting_note, block_note): (String, String, String, String) = c.query_row(
        "SELECT cancelled_delivery, superseded_note, waiting_note, block_note FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap_or_default();
    if !existing.is_empty() { return existing; }
    let note = cancelled_note(c, &kind, &task, attempt, command);
    if (!superseded_note.is_empty() && note == superseded_note) || (!waiting_note.is_empty() && note == waiting_note) || (!block_note.is_empty() && note == block_note) {
        let _ = store_cancelled_delivery(c, command, &conversation, &task, attempt, &kind, "posted", "");
        return "posted".into();
    }
    if note.trim().is_empty() {
        let _ = store_cancelled_delivery(c, command, &conversation, &task, attempt, &kind, "dropped", "");
        return "dropped".into();
    }
    let available: i64 = bound_conversation_available(c, project, &conversation);
    if available != 1 {
        let _ = store_cancelled_delivery(c, command, &conversation, &task, attempt, &kind, "pending", &note);
        return "pending".into();
    }
    let posted = post_bound_message(c, project, &conversation, "post_cancelled", command, &task, &note);
    if posted.is_err() {
        let _ = store_cancelled_delivery(c, command, &conversation, &task, attempt, &kind, "delivery_failed", &note);
        return "delivery_failed".into();
    }
    let _ = store_cancelled_delivery(c, command, &conversation, &task, attempt, &kind, "posted", &note);
    "posted".into()
}

const CANCEL_DID_NOT_UNDO_EXECUTED: &str = "cancel_did_not_undo_executed";

fn load_command_row(c: &rusqlite::Connection, project: i64, command: &str) -> Result<(String, String, i64, String, String, String, i64), String> {
    c.query_row(
        "SELECT kind, state, command_version, failure_reason, decision_reason, task_id, attempt_number FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
        params![command, project],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    ).map_err(|_| "command_unavailable".to_string())
}

fn command_ids_for(c: &rusqlite::Connection, sql: &str, task: &str, attempt: i64, project: i64) -> Result<Vec<String>, String> {
    let mut statement = c.prepare(sql).map_err(|error| error.to_string())?;
    let ids = statement.query_map(params![project, task, attempt], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(ids)
}

/// Supplement and cancel are already stored. The winner is the stored
/// command_version and state, reread here, not the order of the two ids.
fn decide_stored_supplement_cancel(c: &rusqlite::Connection, project: i64, supplement_id: &str, cancel_id: &str) -> Result<Value, String> {
    ensure_command_scope(c)?;
    let (supplement_kind, supplement_state, supplement_version, supplement_failure, _, supplement_task, supplement_attempt) = load_command_row(c, project, supplement_id)?;
    let (cancel_kind, cancel_state, cancel_version, _, _, cancel_task, cancel_attempt) = load_command_row(c, project, cancel_id)?;
    if supplement_kind != "supplement" || cancel_kind != "cancel" || supplement_task != cancel_task || supplement_attempt != cancel_attempt {
        return Ok(json!({"wrote": false, "reason": "", "supplementState": supplement_state, "cancelState": cancel_state, "executedCommandId": "", "skippedCommandId": ""}));
    }
    if matches!(supplement_state.as_str(), "executed" | "verified") {
        c.execute(
            "UPDATE nest_task_commands SET decision_reason=?2 WHERE command_id=?1 AND state=?3 AND command_version=?4 AND decision_reason IN ('', ?2)",
            params![supplement_id, CANCEL_DID_NOT_UNDO_EXECUTED, supplement_state, supplement_version],
        ).map_err(|error| error.to_string())?;
        let (state_after, failure_after, decision_after): (String, String, String) = c.query_row(
            "SELECT state, failure_reason, decision_reason FROM nest_task_commands WHERE command_id=?1",
            [supplement_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).map_err(|error| error.to_string())?;
        if state_after != supplement_state || failure_after != supplement_failure || decision_after != CANCEL_DID_NOT_UNDO_EXECUTED {
            return Ok(json!({"wrote": false, "reason": "", "supplementState": state_after, "cancelState": cancel_state, "executedCommandId": supplement_id, "skippedCommandId": ""}));
        }
        let delivery = post_cancel_did_not_undo(c, project, supplement_id);
        return Ok(json!({"wrote": true, "reason": CANCEL_DID_NOT_UNDO_EXECUTED, "supplementState": state_after, "cancelState": cancel_state, "executedCommandId": supplement_id, "skippedCommandId": cancel_id, "delivery": delivery, "supplementFailureReason": failure_after}));
    }
    if cancel_version >= supplement_version && matches!(supplement_state.as_str(), "accepted" | "delivered") {
        let changed = c.execute(
            "UPDATE nest_task_commands SET state='superseded', failure_reason='superseded_by_control' WHERE command_id=?1 AND state IN ('accepted','delivered') AND command_version=?2",
            params![supplement_id, supplement_version],
        ).map_err(|error| error.to_string())?;
        let delivery = if changed == 1 { post_superseded(c, project, supplement_id, "cancel") } else { String::new() };
        let state_after: String = c.query_row("SELECT state FROM nest_task_commands WHERE command_id=?1", [supplement_id], |row| row.get(0)).unwrap_or(supplement_state);
        return Ok(json!({"wrote": changed == 1, "reason": "superseded_by_control", "supplementState": state_after, "cancelState": cancel_state, "executedCommandId": cancel_id, "skippedCommandId": supplement_id, "delivery": delivery}));
    }
    let executed = if supplement_version > cancel_version { supplement_id } else { cancel_id };
    Ok(json!({"wrote": false, "reason": "", "supplementState": supplement_state, "cancelState": cancel_state, "executedCommandId": executed, "skippedCommandId": ""}))
}

fn settle_supplements_for_cancel(c: &rusqlite::Connection, project: i64, task: &str, attempt: i64, cancel_id: &str) -> Result<usize, String> {
    let ids = command_ids_for(
        c,
        "SELECT command_id FROM nest_task_commands WHERE project_id=?1 AND task_id=?2 AND attempt_number=?3 AND kind='supplement' ORDER BY command_version",
        task,
        attempt,
        project,
    )?;
    let mut count = 0;
    for supplement_id in ids {
        let before: String = c.query_row("SELECT state FROM nest_task_commands WHERE command_id=?1", [&supplement_id], |row| row.get(0)).unwrap_or_default();
        decide_stored_supplement_cancel(c, project, &supplement_id, cancel_id)?;
        let after: String = c.query_row("SELECT state FROM nest_task_commands WHERE command_id=?1", [&supplement_id], |row| row.get(0)).unwrap_or_default();
        if before != "superseded" && after == "superseded" { count += 1; }
    }
    Ok(count)
}

fn settle_supplement_against_stored_cancels(c: &rusqlite::Connection, project: i64, task: &str, attempt: i64, supplement_id: &str) -> Result<(), String> {
    let ids = command_ids_for(
        c,
        "SELECT command_id FROM nest_task_commands WHERE project_id=?1 AND task_id=?2 AND attempt_number=?3 AND kind='cancel' ORDER BY command_version DESC",
        task,
        attempt,
        project,
    )?;
    for cancel_id in ids {
        decide_stored_supplement_cancel(c, project, supplement_id, &cancel_id)?;
    }
    Ok(())
}

/// Both commands are already stored. A stale binding version or attempt on
/// either side writes nothing. The stored command_version decides the rest.
pub(crate) fn resolve_supplement_and_cancel(
    c: &rusqlite::Connection,
    project: i64,
    supplement_id: &str,
    cancel_id: &str,
    supplement_binding_version: Option<i64>,
    supplement_attempt: Option<i64>,
    cancel_binding_version: Option<i64>,
    cancel_attempt: Option<i64>,
) -> Result<Value, String> {
    ensure_command_scope(c)?;
    let (_, _, _, _, _, task, _) = load_command_row(c, project, supplement_id)?;
    let (_, _, _, _, _, cancel_task, _) = load_command_row(c, project, cancel_id)?;
    if task != cancel_task { return Err("command_unavailable".into()); }
    refuse_stale_page_write(c, project, &task, supplement_binding_version, supplement_attempt)?;
    refuse_stale_page_write(c, project, &task, cancel_binding_version, cancel_attempt)?;
    decide_stored_supplement_cancel(c, project, supplement_id, cancel_id)
}

fn cancel_did_not_undo_note(c: &rusqlite::Connection, kind: &str, task: &str, attempt: i64, command: &str) -> String {
    let usage = match c.query_row(
        "SELECT llm_requests, total_tokens FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    ) {
        Ok((requests, tokens)) => format!("引擎用量 {requests} 次请求、{tokens} token。"),
        Err(_) => "引擎用量没有读到，不能当成 0。".into(),
    };
    let status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [task], |row| row.get(0)).unwrap_or_else(|_| "没有读到".into());
    let evidence = evidence_ref(c, kind, task, attempt);
    let evidence_text = if evidence.is_empty() { "无".to_string() } else { evidence };
    let lead = format!("取消没有退回这条补充。记录的原因是 {CANCEL_DID_NOT_UNDO_EXECUTED}。补充指令保持原记录。");
    let redacted = crate::agent_runtime::secrets::redact_json_with(&json!({
        "lead": lead,
        "task": task,
        "attempt": attempt.to_string(),
        "command": command,
        "method": execution_method(kind),
        "evidence": evidence_text,
        "progress": target_progress_phrase(c, task),
        "usage": usage,
        "status": status,
        "reason": CANCEL_DID_NOT_UNDO_EXECUTED,
    }), None);
    let note = format!(
        "{lead} 任务 {task}，attempt {attempt}，指令 {command}，方式 {method}，证据 {evidence}。复核状态 unverified。{progress}{usage}没有新开额度。剩余额度没有在这条回传里重算，不能当成还有额度或已经用尽。扫描状态是 {status}。结论依据是 {reason}，不是浏览器验证，也不是对已有证据的重新分析。",
        lead = field_text(&redacted, "lead"),
        task = field_text(&redacted, "task"),
        attempt = field_text(&redacted, "attempt"),
        command = field_text(&redacted, "command"),
        method = field_text(&redacted, "method"),
        evidence = field_text(&redacted, "evidence"),
        progress = field_text(&redacted, "progress"),
        usage = field_text(&redacted, "usage"),
        status = field_text(&redacted, "status"),
        reason = field_text(&redacted, "reason"),
    );
    let note = crate::agent_runtime::secrets::redact_text_with(&note, None);
    omit_stored_target_urls(c, task, &note)
}

fn store_kept_delivery(c: &rusqlite::Connection, command: &str, conversation: &str, task: &str, attempt: i64, kind: &str, delivery: &str, note: &str) -> Result<(), String> {
    ensure_result_event(c)?;
    let saved = if delivery == "dropped" { "" } else { note };
    let current: Option<String> = c.query_row(
        "SELECT kept_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if let Some(existing) = current {
        if !existing.is_empty() { return Ok(()); }
        c.execute(
            "UPDATE nest_result_deliveries SET kept_delivery=?3, kept_note=?4, state=CASE WHEN ?3 IN ('posted','dropped') AND state IN ('pending','delivery_failed','recorded','') THEN 'posted' WHEN ?3='delivery_failed' AND state IN ('pending','recorded','') THEN 'delivery_failed' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND kept_delivery=''",
            params![command, conversation, delivery, saved],
        ).map_err(|error| error.to_string())?;
        return Ok(());
    }
    let row_state = match delivery {
        "posted" | "dropped" => "posted",
        "delivery_failed" => "delivery_failed",
        _ => "recorded",
    };
    c.execute(
        "INSERT INTO nest_result_deliveries(delivery_id,command_id,conversation_id,state,task_id,attempt_number,execution_method,review_state,evidence_ref,kept_delivery,kept_note) VALUES(?1,?2,?3,?4,?5,?6,?7,'unverified',?8,?9,?10)",
        params![uuid::Uuid::new_v4().to_string(), command, conversation, row_state, task, attempt, execution_method(kind), evidence_ref(c, kind, task, attempt), delivery, saved],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// One note that this cancel did not undo an executed supplement.
/// The supplement row stays executed. An existing superseded or cancelled note
/// that already says the same thing is not posted again.
/// An archived or missing conversation keeps it pending. An empty redaction is dropped.
/// No second conversation is chosen.
fn post_cancel_did_not_undo(c: &rusqlite::Connection, project: i64, command: &str) -> String {
    if ensure_result_event(c).is_err() { return "pending".into(); }
    let bound = c.query_row(
        "SELECT b.conversation_id, k.task_id, k.attempt_number, k.kind, k.state, k.decision_reason FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1 AND k.project_id=?2",
        params![command, project], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?)),
    );
    let Ok((conversation, task, attempt, kind, state, reason)) = bound else { return "unbound".into(); };
    if conversation.is_empty() || !matches!(state.as_str(), "executed" | "verified") || reason != CANCEL_DID_NOT_UNDO_EXECUTED { return "unbound".into(); }
    let (existing, superseded_note, cancelled_note): (String, String, String) = c.query_row(
        "SELECT kept_delivery, superseded_note, cancelled_note FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap_or_default();
    if !existing.is_empty() { return existing; }
    let note = cancel_did_not_undo_note(c, &kind, &task, attempt, command);
    if (!superseded_note.is_empty() && note == superseded_note) || (!cancelled_note.is_empty() && note == cancelled_note) {
        let _ = store_kept_delivery(c, command, &conversation, &task, attempt, &kind, "posted", "");
        return "posted".into();
    }
    if note.trim().is_empty() {
        let _ = store_kept_delivery(c, command, &conversation, &task, attempt, &kind, "dropped", "");
        return "dropped".into();
    }
    let available: i64 = bound_conversation_available(c, project, &conversation);
    if available != 1 {
        let _ = store_kept_delivery(c, command, &conversation, &task, attempt, &kind, "pending", &note);
        return "pending".into();
    }
    let posted = post_bound_message(c, project, &conversation, "post_cancel_did_not_undo", command, &task, &note);
    if posted.is_err() {
        let _ = store_kept_delivery(c, command, &conversation, &task, attempt, &kind, "delivery_failed", &note);
        return "delivery_failed".into();
    }
    let _ = store_kept_delivery(c, command, &conversation, &task, attempt, &kind, "posted", &note);
    "posted".into()
}

fn retry_pending_kept(c: &rusqlite::Connection, project: i64, command: &str, conversation: &str, task: &str, stored_note: &str) -> Result<Value, String> {
    let available: i64 = bound_conversation_available(c, project, &conversation);
    let note = scrubbed_delivery_note(c, task, stored_note);
    if available != 1 {
        remember_held_note(c, command, conversation, "kept_note", &note);
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "pending", "replayed": false, "deliveredToAnotherConversation": false, "keptDelivery": "pending"}));
    }
    if note.trim().is_empty() {
        let changed = c.execute(
            "UPDATE nest_result_deliveries SET kept_delivery='dropped', kept_note='', state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND kept_delivery IN ('pending','delivery_failed')",
            params![command, conversation],
        ).map_err(|error| error.to_string())?;
        if changed != 1 { return Err("result_delivery_not_pending".into()); }
        return Ok(json!({"commandId": command, "conversationId": conversation, "state": "dropped", "replayed": false, "deliveredToAnotherConversation": false, "keptDelivery": "dropped"}));
    }
    post_bound_message(c, project, &conversation, "retry_pending_kept", command, &task, &note)?;
    let changed = c.execute(
        "UPDATE nest_result_deliveries SET kept_delivery='posted', kept_note=?3, state=CASE WHEN state IN ('pending','delivery_failed','recorded','') THEN 'posted' ELSE state END WHERE command_id=?1 AND conversation_id=?2 AND kept_delivery IN ('pending','delivery_failed')",
        params![command, conversation, note],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err("result_delivery_not_pending".into()); }
    Ok(json!({"commandId": command, "conversationId": conversation, "state": "posted", "replayed": false, "deliveredToAnotherConversation": false, "keptDelivery": "posted"}))
}

/// Posts the verification conclusion once, and only to the conversation already bound to this command.
/// An archived or missing conversation keeps that conclusion pending. No second conversation is chosen.
fn post_verification_conclusion(c: &rusqlite::Connection, project: i64, command: &str, kind: &str, review_state: &str, rule: &str) -> String {
    if ensure_result_event(c).is_err() { return "pending".into(); }
    let bound = c.query_row(
        "SELECT b.conversation_id, k.task_id, k.attempt_number FROM nest_task_commands k JOIN nest_session_task_bindings b ON b.binding_id=k.binding_id WHERE k.command_id=?1 AND k.project_id=?2",
        params![command, project], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
    );
    let Ok((conversation, task, attempt)) = bound else { return "unbound".into(); };
    if conversation.is_empty() { return "unbound".into(); }
    let existing: String = c.query_row(
        "SELECT verification_delivery FROM nest_result_deliveries WHERE command_id=?1 AND conversation_id=?2",
        params![command, conversation], |row| row.get(0),
    ).unwrap_or_default();
    if !existing.is_empty() { return existing; }
    let note = verification_conclusion_note(c, kind, &task, attempt, command, review_state, rule);
    if note.trim().is_empty() {
        let _ = store_verification_delivery(c, command, &conversation, &task, attempt, kind, review_state, "dropped", "");
        return "dropped".into();
    }
    let available: i64 = bound_conversation_available(c, project, &conversation);
    if available != 1 {
        let _ = store_verification_delivery(c, command, &conversation, &task, attempt, kind, review_state, "pending", &note);
        return "pending".into();
    }
    let posted = post_bound_message(c, project, &conversation, "post_verification_conclusion", command, &task, &note);
    if posted.is_err() {
        let _ = store_verification_delivery(c, command, &conversation, &task, attempt, kind, review_state, "delivery_failed", &note);
        return "delivery_failed".into();
    }
    let _ = store_verification_delivery(c, command, &conversation, &task, attempt, kind, review_state, "posted", &note);
    "posted".into()
}

pub(crate) fn mark_verified(c: &rusqlite::Connection, project: i64, command: &str) -> Result<Value, String> {
    c.execute_batch(REVIEW_SCHEMA).map_err(|error| error.to_string())?;
    let (state, kind): (String, String) = c.query_row(
        "SELECT state, kind FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
        params![command, project], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or_default();
    if kind.is_empty() && state.is_empty() { return Err("review_not_recorded".into()); }
    if kind == "reanalyze" { return Err("reanalyze_is_not_a_review".into()); }
    if kind == "pause" { return verify_same_attempt(c, project, command, &state, "pause", "pause_not_persistent", "paused_same_attempt"); }
    if kind == "resume" { return verify_same_attempt(c, project, command, &state, "resume", "resume_not_same_attempt", "resumed_same_attempt"); }
    if kind != "continue_targets" { return Err("automatic_verification_unavailable".into()); }
    if state == "verification_failed" { return Err("verification_failed".into()); }
    let task: String = c.query_row("SELECT task_id FROM nest_task_commands WHERE command_id=?1 AND project_id=?2", params![command, project], |row| row.get(0)).unwrap_or_default();
    let scan_status: String = c.query_row("SELECT status FROM sentinel_scans WHERE id=?1", [&task], |row| row.get(0)).unwrap_or_default();
    if scan_status == "failed" { return Err("execution_failed_not_covered_by_preflight".into()); }
    let recorded: i64 = c.query_row(
        "SELECT COUNT(*) FROM nest_command_reviews WHERE command_id=?1",
        [command], |row| row.get(0),
    ).unwrap_or(0);
    if recorded != 1 { return Err("review_not_recorded".into()); }
    let changed = c.execute(
        "UPDATE nest_task_commands SET state='verified' WHERE command_id=?1 AND project_id=?2 AND state='executed'",
        params![command, project],
    ).map_err(|error| error.to_string())?;
    if changed != 1 {
        let state: String = c.query_row("SELECT state FROM nest_task_commands WHERE command_id=?1 AND project_id=?2", params![command, project], |row| row.get(0)).unwrap_or_default();
        if state == "verified" {
            return Ok(json!({"commandId": command, "state": "verified", "verified": true, "replayed": true}));
        }
        return Err("review_not_recorded".into());
    }
    set_review_state(c, command, "finding_review_recorded");
    let verification_delivery = post_verification_conclusion(c, project, command, &kind, "finding_review_recorded", "finding_review_recorded");
    Ok(json!({"commandId": command, "state": "verified", "verified": true, "replayed": false, "reviewState": "finding_review_recorded", "verificationDelivery": verification_delivery}))
}

fn verify_same_attempt(c: &rusqlite::Connection, project: i64, command: &str, state: &str, kind: &str, error: &str, rule: &str) -> Result<Value, String> {
    if state == "verified" {
        return Ok(json!({"commandId": command, "state": "verified", "verified": true, "replayed": true, "verificationRule": rule}));
    }
    let (task, attempt): (String, i64) = c.query_row(
        "SELECT task_id, attempt_number FROM nest_task_commands WHERE command_id=?1 AND project_id=?2",
        params![command, project], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| error.to_string())?;
    let current_attempt: i64 = c.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [&task], |row| row.get(0)).unwrap_or(-1);
    let parent_resume = parent_resume_on_scanning_child(c, &task, kind, state, attempt, current_attempt);
    if state != "executed" || (kind == "resume" && current_attempt != attempt && !parent_resume) || !execution_observed(c, kind, &task, attempt) {
        return Err(error.into());
    }
    let changed = c.execute(
        "UPDATE nest_task_commands SET state='verified' WHERE command_id=?1 AND project_id=?2 AND state='executed'",
        params![command, project],
    ).map_err(|error| error.to_string())?;
    if changed != 1 { return Err(error.into()); }
    let review_state = match rule {
        "paused_same_attempt" => "pause_state_verified",
        "resumed_same_attempt" => "resume_state_verified",
        _ => "verified",
    };
    set_review_state(c, command, review_state);
    let verification_delivery = post_verification_conclusion(c, project, command, kind, review_state, rule);
    Ok(json!({"commandId": command, "state": "verified", "verified": true, "replayed": false, "verificationRule": rule, "reviewState": review_state, "verificationDelivery": verification_delivery}))
}

/// The scan is already paused on this attempt. An accepted pause command is
/// delivered and then executed, so the existing executed note posts once.
/// An already executed pause command is left untouched. This does not pause
/// the scan or change its status, attempt, budget, or holder.
pub(crate) fn promote_completed_pause(c: &rusqlite::Connection, task: &str, attempt: i64) -> Result<Vec<Value>, String> {
    let (status, current): (String, i64) = match c.query_row(
        "SELECT status, attempt_count FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(row) => row,
        Err(_) => return Ok(Vec::new()),
    };
    if status != "paused" || current != attempt {
        return Ok(Vec::new());
    }
    let ready: i64 = c.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='nest_task_commands'",
        [], |row| row.get(0),
    ).unwrap_or(0);
    if ready != 1 { return Ok(Vec::new()); }
    let mut statement = c.prepare(
        "SELECT command_id, project_id, state FROM nest_task_commands WHERE task_id=?1 AND attempt_number=?2 AND kind='pause' AND state IN ('accepted','delivered')",
    ).map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![task, attempt], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
    }).map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    let mut updated = Vec::new();
    for (id, project, state) in rows {
        if state == "executed" { continue; }
        if state == "accepted" {
            mark_delivered(c, project, &id)?;
        }
        updated.push(mark_executed(c, project, &id)?);
    }
    Ok(updated)
}

/// The scan is already scanning on this attempt. An accepted resume command on
/// this attempt, or on this attempt's immediate resume parent, is delivered and
/// then executed once. An already executed resume command is left untouched.
/// This does not resume the scan or change its status, attempt, budget, or holder.
/// A spawned thread is not this signal.
pub(crate) fn promote_completed_resume(c: &rusqlite::Connection, task: &str, attempt: i64) -> Result<Vec<Value>, String> {
    let (status, current): (String, i64) = match c.query_row(
        "SELECT status, attempt_count FROM sentinel_scans WHERE id=?1",
        [task], |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(row) => row,
        Err(_) => return Ok(Vec::new()),
    };
    if status != "scanning" || current != attempt {
        return Ok(Vec::new());
    }
    let ready: i64 = c.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='nest_task_commands'",
        [], |row| row.get(0),
    ).unwrap_or(0);
    if ready != 1 { return Ok(Vec::new()); }
    let parent = immediate_resume_parent(c, task, attempt).unwrap_or(-1);
    let mut statement = c.prepare(
        "SELECT command_id, project_id, state FROM nest_task_commands WHERE task_id=?1 AND kind='resume' AND state IN ('accepted','delivered') AND (attempt_number=?2 OR attempt_number=?3)",
    ).map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![task, attempt, parent], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
    }).map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    let mut updated = Vec::new();
    for (id, project, state) in rows {
        if state == "executed" { continue; }
        if state == "accepted" {
            mark_delivered(c, project, &id)?;
        }
        updated.push(mark_executed(c, project, &id)?);
    }
    Ok(updated)
}

/// After a worker claim, move continuation commands to executed. Accepted means
/// saved only; this step records delivery and execution together because the
/// claim is the executor's receipt and the state change. Verification stays false.
pub(crate) fn promote_claimed_continuation(c: &rusqlite::Connection, task: &str, attempt: i64) -> Result<Vec<Value>, String> {
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    let launched: i64 = c.query_row(
        "SELECT launched FROM scan_target_continuation_fences WHERE scan_id=?1 AND attempt_number=?2 AND launched=1 LIMIT 1",
        params![task, attempt], |row| row.get(0),
    ).unwrap_or(0);
    if launched != 1 { return Ok(Vec::new()); }
    let mut statement = c.prepare(
        "SELECT command_id, project_id, state FROM nest_task_commands WHERE task_id=?1 AND attempt_number=?2 AND kind='continue_targets' AND state IN ('accepted','delivered')",
    ).map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![task, attempt], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)))
        .map_err(|error| error.to_string())?;
    let mut updated = Vec::new();
    for row in rows {
        let (id, project, state) = row.map_err(|error| error.to_string())?;
        if state == "accepted" {
            c.execute("UPDATE nest_task_commands SET state='delivered' WHERE command_id=?1 AND state='accepted'", [&id]).map_err(|error| error.to_string())?;
        }
        updated.push(mark_executed(c, project, &id)?);
    }
    Ok(updated)
}

/// One conversation may be bound to several tasks. Without an explicit task, a
/// single binding can be used; more than one is a stop, not a guess.
pub(crate) fn resolve_bound_task(c: &rusqlite::Connection, project: i64, conversation: &str, explicit_task: Option<&str>) -> Result<String, String> {
    c.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
    if let Some(task) = explicit_task.filter(|value| !value.is_empty()) {
        let found: i64 = c.query_row(
            "SELECT COUNT(*) FROM nest_session_task_bindings WHERE conversation_id=?1 AND project_id=?2 AND task_id=?3",
            params![conversation, project, task], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if found != 1 { return Err("task_binding_missing".into()); }
        return Ok(task.to_string());
    }
    let mut statement = c.prepare(
        "SELECT task_id FROM nest_session_task_bindings WHERE conversation_id=?1 AND project_id=?2 ORDER BY task_id",
    ).map_err(|error| error.to_string())?;
    let tasks = statement.query_map(params![conversation, project], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    match tasks.len() {
        1 => Ok(tasks[0].clone()),
        0 => Err("task_binding_missing".into()),
        _ => Err("task_not_unique".into()),
    }
}

fn ensure_reassociation(c: &rusqlite::Connection) -> Result<(), String> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS nest_binding_reassociations(
            reassociation_id TEXT PRIMARY KEY,
            project_id INTEGER NOT NULL,
            task_id TEXT NOT NULL,
            from_binding_id TEXT NOT NULL,
            to_binding_id TEXT NOT NULL,
            from_conversation_id TEXT NOT NULL,
            to_conversation_id TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    ).map_err(|error| error.to_string())
}

/// The source conversation was deleted, or it no longer belongs to this project.
/// Pending deliveries move only to the conversation the caller names, and the move is stored.
/// An archived source stays pending for retry. Already posted rows stay on the conversation
/// that received them. This call does not post a message, create a scan, or pick another chat.
pub(crate) fn reassociate_revoked_binding(c: &rusqlite::Connection, project: i64, task: &str, from_conversation: &str, to_conversation: &str) -> Result<Value, String> {
    reassociate_revoked_binding_at(c, project, task, from_conversation, to_conversation, None, None)
}

pub(crate) fn reassociate_revoked_binding_at(
    c: &rusqlite::Connection,
    project: i64,
    task: &str,
    from_conversation: &str,
    to_conversation: &str,
    binding_version: Option<i64>,
    attempt: Option<i64>,
) -> Result<Value, String> {
    if task.is_empty() { return Err("scan_task_unavailable".into()); }
    if from_conversation.is_empty() || to_conversation.is_empty() { return Err("conversation_unavailable".into()); }
    if from_conversation == to_conversation { return Err("reassociation_same_conversation".into()); }
    ensure_binding_store(c)?;
    ensure_result_event(c)?;
    ensure_reassociation(c)?;
    let tx = c.unchecked_transaction().map_err(|error| error.to_string())?;
    let receipt = reassociate_revoked_binding_in(&tx, project, task, from_conversation, to_conversation, binding_version, attempt)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(receipt)
}

fn reassociate_revoked_binding_in(
    c: &rusqlite::Connection,
    project: i64,
    task: &str,
    from_conversation: &str,
    to_conversation: &str,
    binding_version: Option<i64>,
    attempt: Option<i64>,
) -> Result<Value, String> {
    let scans_before: i64 = c.query_row("SELECT COUNT(*) FROM sentinel_scans", [], |row| row.get(0)).map_err(|error| error.to_string())?;
    let task_found: i64 = c.query_row(
        "SELECT COUNT(*) FROM sentinel_scans WHERE id=?1 AND project_id=?2",
        params![task, project], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if task_found != 1 { return Err("scan_task_unavailable".into()); }
    refuse_stale_page_write(c, project, task, binding_version, attempt)?;
    let binding: String = c.query_row(
        "SELECT binding_id FROM nest_session_task_bindings WHERE conversation_id=?1 AND project_id=?2 AND task_id=?3",
        params![from_conversation, project, task], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?.ok_or_else(|| "task_binding_missing".to_string())?;
    // Host conversations are owned by the Host, which only asks for a move
    // after it found the source conversation gone.
    let source: Option<(i64, i64)> = if host_conversation(from_conversation).is_some() { None } else { c.query_row(
        "SELECT project_id, archived FROM conversations WHERE id=?1",
        [from_conversation], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|error| error.to_string())? };
    match source {
        Some((source_project, archived)) if source_project == project && archived == 0 => return Err("reassociation_source_still_available".into()),
        Some((source_project, _)) if source_project == project => return Err("reassociation_source_archived".into()),
        _ => {}
    }
    let destination: Option<(i64, i64)> = if host_conversation(to_conversation).is_some() { Some((project, 0)) } else { c.query_row(
        "SELECT project_id, archived FROM conversations WHERE id=?1",
        [to_conversation], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|error| error.to_string())? };
    match destination {
        None => return Err("conversation_unavailable".into()),
        Some((destination_project, _)) if destination_project != project => return Err("conversation_project_mismatch".into()),
        Some((_, archived)) if archived != 0 => return Err("reassociation_destination_archived".into()),
        Some(_) => {}
    }
    let destination_binding: Option<String> = c.query_row(
        "SELECT binding_id FROM nest_session_task_bindings WHERE conversation_id=?1 AND project_id=?2 AND task_id=?3",
        params![to_conversation, project, task], |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if destination_binding.is_some() { return Err("reassociation_destination_already_bound".into()); }
    let open = "(state IN ('pending','delivery_failed') OR verification_delivery IN ('pending','delivery_failed') OR block_delivery IN ('pending','delivery_failed') OR waiting_delivery IN ('pending','delivery_failed') OR superseded_delivery IN ('pending','delivery_failed') OR cancelled_delivery IN ('pending','delivery_failed') OR kept_delivery IN ('pending','delivery_failed'))";
    let conflict: i64 = c.query_row(
        &format!("SELECT COUNT(*) FROM nest_result_deliveries d WHERE d.conversation_id=?1 AND d.command_id IN (SELECT command_id FROM nest_task_commands WHERE binding_id=?2 AND project_id=?3 AND task_id=?4) AND {open} AND EXISTS (SELECT 1 FROM nest_result_deliveries other WHERE other.command_id=d.command_id AND other.conversation_id=?5)"),
        params![from_conversation, binding, project, task, to_conversation], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if conflict != 0 { return Err("reassociation_delivery_conflict".into()); }
    let moved = c.execute(
        &format!("UPDATE nest_result_deliveries SET conversation_id=?1 WHERE conversation_id=?2 AND command_id IN (SELECT command_id FROM nest_task_commands WHERE binding_id=?3 AND project_id=?4 AND task_id=?5) AND {open}"),
        params![to_conversation, from_conversation, binding, project, task],
    ).map_err(|error| error.to_string())?;
    let changed = match binding_version {
        Some(version) => c.execute(
            "UPDATE nest_session_task_bindings SET conversation_id=?1, binding_version=binding_version+1, updated_at=datetime('now') WHERE binding_id=?2 AND conversation_id=?3 AND project_id=?4 AND task_id=?5 AND binding_version=?6",
            params![to_conversation, binding, from_conversation, project, task, version],
        ),
        None => c.execute(
            "UPDATE nest_session_task_bindings SET conversation_id=?1, binding_version=binding_version+1, updated_at=datetime('now') WHERE binding_id=?2 AND conversation_id=?3 AND project_id=?4 AND task_id=?5",
            params![to_conversation, binding, from_conversation, project, task],
        ),
    }.map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err(if binding_version.is_some() { "stale_page_write" } else { "task_binding_missing" }.into());
    }
    if let (Some(from_host), Some(to_host)) = (host_conversation(from_conversation), host_conversation(to_conversation)) {
        c.execute(
            "UPDATE nest_host_outbox SET host_conversation_id=?1 WHERE host_conversation_id=?2 AND project_id=?3 AND task_id=?4 AND state='pending'",
            params![to_host, from_host, project, task],
        ).map_err(|error| error.to_string())?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO nest_binding_reassociations(reassociation_id,project_id,task_id,from_binding_id,to_binding_id,from_conversation_id,to_conversation_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![id, project, task, binding, binding, from_conversation, to_conversation],
    ).map_err(|error| error.to_string())?;
    let scans_after: i64 = c.query_row("SELECT COUNT(*) FROM sentinel_scans", [], |row| row.get(0)).map_err(|error| error.to_string())?;
    if scans_after != scans_before { return Err("reassociation_created_a_scan".into()); }
    Ok(json!({
        "reassociationId": id,
        "taskId": task,
        "fromConversationId": from_conversation,
        "toConversationId": to_conversation,
        "fromBindingId": binding,
        "toBindingId": binding,
        "movedDeliveries": moved,
        "posted": false,
        "deliveredToAnotherConversation": false,
        "bindingVersion": c.query_row("SELECT binding_version FROM nest_session_task_bindings WHERE binding_id=?1", [binding.as_str()], |row| row.get::<_, i64>(0)).map_err(|error| error.to_string())?,
    }))
}


#[cfg(test)]
mod host_loop_tests {
    use super::*;

    struct Fixture {
        dir: std::path::PathBuf,
        c: rusqlite::Connection,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Real startup schema in a throwaway directory, one project and one web scan.
    fn fixture(status: &str, attempt: i64) -> Fixture {
        let dir = std::env::temp_dir().join(format!("oviraptor-host-loop-{}", uuid::Uuid::new_v4()));
        let path = crate::db::initialize(&dir).unwrap();
        let c = crate::db::open(&path).unwrap();
        c.execute("INSERT INTO projects(id,name) VALUES(1,'p')", []).unwrap();
        c.execute(
            "INSERT INTO sentinel_scans(id,project_id,status,attempt_count,scan_type) VALUES('scan-1',1,?1,?2,'web')",
            params![status, attempt],
        ).unwrap();
        c.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(1,'scan-1','示例','https://a.example/',?1)", [status]).unwrap();
        Fixture { dir, c }
    }

    fn outbox_rows(c: &rusqlite::Connection) -> i64 {
        c.query_row("SELECT COUNT(*) FROM nest_host_outbox", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn host_binding_needs_no_desktop_conversation() {
        let f = fixture("completed", 1);
        let bound = bind(&f.c, 1, "host:conv-1", "scan-1").unwrap();
        assert_eq!(bound["created"], json!(true));
        assert_eq!(bind(&f.c, 1, "host:conv-1", "scan-1").unwrap()["created"], json!(false));
        assert_eq!(bind(&f.c, 1, "desktop-conv", "scan-1").unwrap_err(), "conversation_unavailable");
    }

    #[test]
    fn a_settled_attempt_produces_one_event_with_evidence_until_acknowledged() {
        let f = fixture("completed", 1);
        f.c.execute("INSERT INTO sentinel_validations(scan_id,url,finding_key,verdict) VALUES('scan-1','https://a.example/','k1','confirmed')", []).unwrap();
        bind(&f.c, 1, "host:conv-1", "scan-1").unwrap();
        let pulled = host_outbox_pull(&f.c, 1, 32).unwrap();
        let deliveries = pulled["deliveries"].as_array().unwrap();
        assert_eq!(deliveries.len(), 1);
        let event = &deliveries[0];
        assert_eq!(event["conversationId"], json!("conv-1"));
        assert_eq!(event["kind"], json!("task_result"));
        assert_eq!(event["evidence"]["scanId"], json!("scan-1"));
        assert_eq!(event["evidence"]["validationIds"].as_array().unwrap().len(), 1);
        // Pulling again (or after a restart) reconciles to the same single event.
        assert_eq!(host_outbox_pull(&f.c, 1, 32).unwrap()["deliveries"], pulled["deliveries"]);
        assert_eq!(outbox_rows(&f.c), 1);
        let event_id = event["eventId"].as_str().unwrap().to_string();
        let acked = host_outbox_ack(&f.c, 1, &[(event_id.clone(), "delivered".into())]).unwrap();
        assert_eq!(acked["acks"][0]["changed"], json!(true));
        assert!(host_outbox_pull(&f.c, 1, 32).unwrap()["deliveries"].as_array().unwrap().is_empty());
        assert_eq!(outbox_rows(&f.c), 1, "an acknowledged event is not produced again");
        assert_eq!(host_outbox_ack(&f.c, 1, &[(event_id, "delivered".into())]).unwrap()["acks"][0]["changed"], json!(false));
        assert_eq!(host_outbox_ack(&f.c, 1, &[("x".into(), "posted".into())]).unwrap_err(), "scan_arguments_invalid");
    }

    #[test]
    fn a_running_attempt_produces_no_result_event_and_other_projects_see_nothing() {
        let f = fixture("scanning", 1);
        bind(&f.c, 1, "host:conv-1", "scan-1").unwrap();
        assert!(host_outbox_pull(&f.c, 1, 32).unwrap()["deliveries"].as_array().unwrap().is_empty());
        f.c.execute("UPDATE sentinel_scans SET status='paused'", []).unwrap();
        assert_eq!(host_outbox_pull(&f.c, 1, 32).unwrap()["deliveries"].as_array().unwrap().len(), 1);
        f.c.execute("INSERT INTO projects(id,name) VALUES(2,'q')", []).unwrap();
        assert!(host_outbox_pull(&f.c, 2, 32).unwrap()["deliveries"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_retried_post_reuses_its_event_id() {
        let f = fixture("completed", 1);
        ensure_binding_store(&f.c).unwrap();
        for _ in 0..2 {
            post_bound_message(&f.c, 1, "host:conv-1", "post_block_reason", "cmd-1", "scan-1", "note").unwrap();
        }
        assert_eq!(outbox_rows(&f.c), 1);
    }

    #[test]
    fn an_idempotency_key_replays_only_the_same_command() {
        let f = fixture("paused", 1);
        let first = submit(&f.c, 1, "host:conv-1", "scan-1", "key-1", "pause", "pause now").unwrap();
        let replay = submit(&f.c, 1, "host:conv-1", "scan-1", "key-1", "pause", "pause now").unwrap();
        assert_eq!(replay["commandId"], first["commandId"]);
        assert_eq!(replay["replayed"], json!(true));
        assert_eq!(submit(&f.c, 1, "host:conv-1", "scan-1", "key-1", "cancel", "pause now").unwrap_err(), "command_identity_conflict");
        assert_eq!(submit(&f.c, 1, "host:conv-2", "scan-1", "key-1", "pause", "pause now").unwrap_err(), "command_identity_conflict");
        assert_eq!(submit(&f.c, 1, "host:conv-1", "scan-1", "key-1", "pause", "other").unwrap_err(), "command_payload_conflict");
    }

    #[test]
    fn a_caller_cannot_mark_a_command_delivered() {
        let f = fixture("paused", 1);
        let receipt = submit(&f.c, 1, "host:conv-1", "scan-1", "key-1", "pause", "pause now").unwrap();
        let command = receipt["commandId"].as_str().unwrap();
        assert_eq!(mark_delivered(&f.c, 1, command).unwrap_err(), "command_not_claimed_by_executor");
        let state: String = f.c.query_row("SELECT state FROM nest_task_commands WHERE command_id=?1", [command], |r| r.get(0)).unwrap();
        assert_eq!(state, "accepted");
    }

    #[test]
    fn a_supplement_waits_for_a_live_coordinator_and_never_applies_to_another_attempt() {
        let f = fixture("scanning", 1);
        let receipt = submit(&f.c, 1, "host:conv-1", "scan-1", "key-1", "supplement", "优先检查证书线索").unwrap();
        let command = receipt["commandId"].as_str().unwrap().to_string();
        let payload: String = f.c.query_row("SELECT payload_text FROM nest_task_commands WHERE command_id=?1", [&command], |r| r.get(0)).unwrap();
        assert!(payload.contains("证书线索"), "the body is kept for a later consumer: {payload}");
        if receipt["state"] != json!("accepted") { return; }
        // No Coordinator is running: nothing is delivered and nothing is claimed.
        assert_eq!(deliver_supplements(&f.c, 1, Some("scan-1")).unwrap(), 0);
        let state: String = f.c.query_row("SELECT state FROM nest_task_commands WHERE command_id=?1", [&command], |r| r.get(0)).unwrap();
        assert_eq!(state, "accepted");
        // A later attempt does not pick up a supplement saved for attempt 1.
        f.c.execute("UPDATE sentinel_scans SET attempt_count=2", []).unwrap();
        assert_eq!(deliver_supplements(&f.c, 1, Some("scan-1")).unwrap(), 0);
    }

    #[test]
    fn reviews_need_a_concluded_reviewer_run_of_the_task() {
        let f = fixture("completed", 1);
        let receipt = submit(&f.c, 1, "host:conv-1", "scan-1", "key-1", "pause", "pause now").unwrap();
        let command = receipt["commandId"].as_str().unwrap();
        f.c.execute("UPDATE nest_task_commands SET state='executed' WHERE command_id=?1", [command]).unwrap();
        assert_eq!(
            record_review(&f.c, 1, command, "someone", "looks fine", "passed").unwrap_err(),
            "review_reviewer_not_independent_run"
        );
    }
}
