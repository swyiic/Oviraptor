//! Trusted desktop adapter. No independent service store or snapshot is adopted.
use super::*;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS agent_scan_operations(project_id INTEGER NOT NULL REFERENCES projects(id),operation_id TEXT NOT NULL,action TEXT NOT NULL,arguments_sha256 TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('prepared','completed','uncertain')),receipt_json TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL DEFAULT(datetime('now')),PRIMARY KEY(project_id,operation_id));";

#[path = "agent_results.rs"]
mod results;
#[path = "agent_scan_recovery.rs"]
mod recovery;
#[path = "agent_scan_diagnostics.rs"]
mod diagnostics;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FromAssets {
    asset_ids: Vec<i64>,
    scan_mode: Option<String>,
    max_model_requests: Option<i64>,
    max_model_tokens: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Task {
    task_id: String,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct List {
    limit: Option<i64>,
    offset: Option<i64>,
}

fn active(c: &rusqlite::Connection, project: i64) -> Result<(), String> {
    if project < 1
        || !c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1 AND status='active')",
                [project],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|_| "scan_project_unavailable")?
    {
        return Err("scan_project_unavailable".into());
    }
    Ok(())
}
fn present(c: &rusqlite::Connection, project: i64) -> Result<(), String> {
    if project < 1
        || !c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
                [project],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|_| "scan_project_unavailable")?
    {
        return Err("scan_project_unavailable".into());
    }
    Ok(())
}
fn owned_scope(c: &rusqlite::Connection, project: i64, task: &str) -> Result<(), String> {
    if task.is_empty() || task.len() > 128 || task.chars().any(char::is_control) {
        return Err("scan_task_invalid".into());
    }
    let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans s WHERE id=?1 AND project_id=?2 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id))",params![task,project],|r|r.get(0)).map_err(|_|"scan_task_unavailable")?;
    if !exists {
        return Err("scan_task_unavailable".into());
    }
    Ok(())
}
fn owned(c: &rusqlite::Connection, project: i64, task: &str) -> Result<SentinelScan, String> {
    owned_scope(c, project, task)?;
    sentinel_scan_by_id(c, task)
}
fn projection(c: &rusqlite::Connection, scan: &SentinelScan) -> Result<Value, String> {
    let mut progress = serde_json::Map::new();
    let mut s = c
        .prepare("SELECT status,COUNT(*) FROM sentinel_targets WHERE scan_id=?1 GROUP BY status")
        .map_err(|_| "scan_progress_unavailable")?;
    let rows = s
        .query_map([&scan.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|_| "scan_progress_unavailable")?;
    let mut total = 0;
    for row in rows {
        let (status, count) = row.map_err(|_| "scan_progress_unavailable")?;
        total += count;
        progress.insert(status, json!(count));
    }
    // Keep path/config/raw logs and target session material outside model context.
    Ok(
        json!({"id":scan.id,"title":scan.task_name,"status":scan.status,"scanType":scan.scan_type,
        "attempt":scan.attempt_count,"latestAttemptStatus":scan.latest_attempt_status,
        "targetCount":total,"progress":progress,"modelRequests":scan.llm_requests,"tokens":scan.total_tokens,
        "createdAt":scan.created_at,"updatedAt":scan.updated_at,"executionAvailable":scan.scan_type=="web",
        "executionStarted":scan.attempt_count>0,"modelLimits":saved_model_limits(c,&scan.id)?,"source":"desktop_scanning_owner"}),
    )
}
fn saved_model_limits(c: &rusqlite::Connection, task: &str) -> Result<Value, String> {
    let raw: Option<String> = c
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [task],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "scan_policy_unavailable")?;
    let policy: Value = raw
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map_err(|_| "scan_policy_unavailable")?
        .unwrap_or(Value::Null);
    Ok(
        json!({"maxModelRequests":policy.get("maxModelRequests"),"maxModelTokens":policy.get("maxModelTokens"),
        "requestedMode":match policy.get("webModeCeiling").and_then(Value::as_str){Some("quick")=>"quick",Some("deep")=>"deep",_=>"standard"},
        "scope":"per_target_per_attempt","requestedMaxBudgetUsd":policy.get("maxBudgetUsd"),"usdLimitEnforced":false}),
    )
}
fn preflight(c: &rusqlite::Connection, project: i64, task: &str) -> Result<Value, String> {
    let tx = c
        .unchecked_transaction()
        .map_err(|_| "scan_preflight_unavailable")?;
    let scan = owned(&tx, project, task)?;
    let settings = agent_policy_settings_for_scan(&tx, task)?;
    let (configured, model) = match model_runtime_env(&settings).and_then(|value| {
        crate::model_configuration::gateway_profile(&value.as_connection(), None).map(|_| value)
    }) {
        Ok(value) => (
            true,
            Some(
                json!({"name":crate::model_configuration::openai_chat_completion_model(&value.llm),"deployment":value.deployment,"endpointHost":reqwest::Url::parse(&value.api_base).ok().and_then(|u|u.host_str().map(str::to_owned))}),
            ),
        ),
        Err(_) => (false, None),
    };
    let financial_resume=if scan.scan_type=="web" {
        match super::scan_resume_financial_preflight_in(&tx,task) {
            Ok(())=>json!({"checked":true,"ordinaryResumeBlocked":false,"onlyFinancialHistoryChecked":true}),
            Err(code)=>json!({"checked":true,"ordinaryResumeBlocked":true,"reasonCode":code,"onlyFinancialHistoryChecked":true,
                "reason":"普通恢复被原生费用保护策略阻止；请保留原账单与预算，使用原 owner 的费用核对或剩余额度接续流程。不是应用读取或执行授权丢失；本次没有创建执行尝试或发送请求。"}),
        }
    }else{json!({"checked":false,"ordinaryResumeBlocked":true,"reasonCode":"scan_channel_web_only"})};
    Ok(
        json!({"projectId":project,"task":projection(&tx,&scan)?,"modelConfigured":configured,"model":model,"resumeFinancialCheck":financial_resume,
        "configurationSource":"desktop_models_category","networkTested":false,"engineAdmissionValidated":false,
        "recovery":if scan.scan_type=="web"{Some(recovery::view(&tx,project,task)?)}else{None},
        "executionAuthorizedByThisRead":false,"reason":if configured{"已解析保存的扫描模型；启动仍须重新校验范围、授权、预算、工具与执行状态"}else{"扫描模型连接未就绪，请到配置中心 → 模型检查；本次没有发送模型或目标请求"}}),
    )
}
fn arguments(action: &str, args: &Value) -> Result<(), String> {
    match action {
        "list" => {
            let input: List =
                serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
            if input.limit.is_some_and(|v| !(1..=20).contains(&v))
                || input.offset.is_some_and(|v| !(0..=1_000_000).contains(&v))
            {
                return Err("scan_arguments_invalid".into());
            }
        }
        "from_assets" => {
            let input: FromAssets =
                serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
            if input.asset_ids.is_empty()
                || input.asset_ids.len() > 128
                || input.asset_ids.iter().any(|v| *v < 1)
                || input.asset_ids.iter().collect::<HashSet<_>>().len() != input.asset_ids.len()
                || input
                    .max_model_requests
                    .is_some_and(|v| !(1..=96).contains(&v))
                || input
                    .max_model_tokens
                    .is_some_and(|v| !(512..=2_000_000).contains(&v))
                || input
                    .scan_mode
                    .as_deref()
                    .is_some_and(|v| !matches!(v, "quick" | "standard" | "deep"))
            {
                return Err("scan_arguments_invalid".into());
            }
        }
        "status" | "preflight" | "recovery" | "start" | "pause" | "resume" => {
            let input: Task =
                serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
            if input.task_id.is_empty()
                || input.task_id.len() > 128
                || input.task_id.chars().any(char::is_control)
            {
                return Err("scan_arguments_invalid".into());
            }
        }
        "results" => { results::Results::parse(args)?; }
        _ => return Err("scan_action_unavailable".into()),
    }
    Ok(())
}
fn creation_receipt(
    c: &rusqlite::Connection,
    project: i64,
    operation: &str,
    scan: &SentinelScan,
) -> Result<Value, String> {
    let receipt = json!({"projectId":project,"operationId":operation,"operationState":"completed","action":"from_assets","task":projection(c,scan)?,"receiptIsHistorical":true});
    let changed=c.execute("UPDATE agent_scan_operations SET state='completed',receipt_json=?3 WHERE project_id=?1 AND operation_id=?2 AND state='prepared'",params![project,operation,receipt.to_string()]).map_err(|_|"scan_receipt_unavailable")?;
    if changed != 1 {
        return Err("scan_operation_state_changed".into());
    }
    Ok(receipt)
}

/// A crash after preparation is deliberately not replayed. Live status remains
/// authoritative; a historical receipt never asserts current completion.
#[cfg(test)]
fn claim(c:&rusqlite::Connection,project:i64,operation:&str,action:&str,args:&Value)->Result<Option<Value>,String>{
    claim_checked(c,project,operation,action,args,|_|Ok(()))
}
fn claim_checked(
    c: &rusqlite::Connection,
    project: i64,
    operation: &str,
    action: &str,
    args: &Value,
    admission:impl FnOnce(&rusqlite::Connection)->Result<(),String>,
) -> Result<Option<Value>, String> {
    if operation.is_empty() || operation.len() > 128 || operation.chars().any(char::is_control) {
        return Err("scan_operation_invalid".into());
    }
    c.execute_batch(SCHEMA)
        .map_err(|_| "scan_operation_store_unavailable")?;
    let tx = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "scan_operation_store_unavailable")?;
    if action == "pause" {
        present(&tx, project)?;
    } else {
        active(&tx, project)?;
    }
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(args).map_err(|_| "scan_arguments_invalid")?)
    );
    let old:Option<(String,String,String,String)>=tx.query_row("SELECT action,arguments_sha256,state,receipt_json FROM agent_scan_operations WHERE project_id=?1 AND operation_id=?2",params![project,operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|"scan_operation_store_unavailable")?;
    if let Some((verb, hash, state, receipt)) = old {
        if verb != action || hash != digest {
            return Err("scan_operation_conflict".into());
        }
        return if state == "completed" {
            serde_json::from_str(&receipt)
                .map(Some)
                .map_err(|_| "scan_receipt_unavailable".into())
        } else {
            Ok(Some(
                json!({"projectId":project,"operationId":operation,"operationState":"uncertain","outcomeUnknown":true,"retryable":false,"action":action,"taskId":args.get("taskId"),"reason":"上次操作的结果未确认；请读取真实任务状态，不自动重发"}),
            ))
        };
    }
    admission(&tx)?;
    tx.execute("INSERT INTO agent_scan_operations(project_id,operation_id,action,arguments_sha256,state) VALUES(?1,?2,?3,?4,'prepared')",params![project,operation,action,digest]).map_err(|_|"scan_operation_store_unavailable")?;
    tx.commit()
        .map_err(|_| "scan_operation_prepare_unconfirmed")?;
    Ok(None)
}

pub(crate) fn dispatch(
    app: &AppHandle,
    project: i64,
    action: &str,
    args: Value,
    operation: &str,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    arguments(action, &args)?;
    let c = db::open(&state.db_path)?;
    if matches!(action, "list" | "status" | "preflight" | "recovery" | "results" | "pause") {
        present(&c, project)?;
    } else {
        active(&c, project)?;
    }
    if action == "list" {
        let input: List = serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
        let limit = input.limit.unwrap_or(20);
        let offset = input.offset.unwrap_or(0);
        let mut statement=c.prepare("SELECT id FROM sentinel_scans s WHERE project_id=?1 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id) ORDER BY updated_at DESC,id DESC LIMIT ?2 OFFSET ?3").map_err(|_|"scan_list_unavailable")?;
        let ids: Vec<String> = statement
            .query_map(params![project, limit, offset], |r| r.get(0))
            .map_err(|_| "scan_list_unavailable")?
            .collect::<rusqlite::Result<_>>()
            .map_err(|_| "scan_list_unavailable")?;
        let total:i64=c.query_row("SELECT COUNT(*) FROM sentinel_scans s WHERE project_id=?1 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id)",[project],|r|r.get(0)).map_err(|_|"scan_list_unavailable")?;
        let tasks: Vec<Value> = ids
            .iter()
            .map(|id| owned(&c, project, id).and_then(|s| projection(&c, &s)))
            .collect::<Result<_, _>>()?;
        return Ok(
            json!({"projectId":project,"tasks":tasks,"total":total,"offset":offset,"nextOffset":if offset+limit<total{Some(offset+limit)}else{None},"source":"desktop_scanning_owner"}),
        );
    }
    if action == "status" {
        let task: Task = serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
        return Ok(
            json!({"projectId":project,"task":projection(&c,&owned(&c,project,&task.task_id)?)?}),
        );
    }
    if action == "preflight" {
        let task: Task = serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
        return preflight(&c, project, &task.task_id);
    }
    if action == "recovery" {
        let task: Task = serde_json::from_value(args).map_err(|_| "scan_arguments_invalid")?;
        return recovery::read(&c, project, &task.task_id);
    }
    if action == "results" {
        return results::read(&c, project, results::Results::parse(&args)?);
    }
    // Validate scope before storing a mutation receipt. The original owner also
    // checks admission after this adapter yields; this is no replacement gate.
    if action == "from_assets" {
        let input: FromAssets =
            serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
        for id in input.asset_ids {
            let url:Option<String>=c.query_row("SELECT COALESCE(NULLIF(a.link,''),a.host) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.asset_id=?2 AND pa.is_deleted=0",params![project,id],|r|r.get(0)).optional().map_err(|_|"scan_asset_unavailable")?;
            let url = url
                .and_then(|u| reqwest::Url::parse(&u).ok())
                .ok_or("scan_asset_unavailable")?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err("scan_asset_url_invalid".into());
            }
        }
    } else {
        let task: Task =
            serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
        let scan = owned(&c, project, &task.task_id)?;
        if scan.scan_type != "web" {
            return Err("scan_channel_web_only".into());
        }
    }
    let claimed = claim_checked(&c, project, operation, action, &args,|snapshot|{
        if action=="resume" {super::scan_resume_financial_preflight_in(snapshot,args["taskId"].as_str().unwrap())}else{Ok(())}
    });
    let receipt = match claimed {
        Ok(receipt) => receipt,
        Err(code) if action == "resume" && recovery::financial_reason(&code) => {
            return recovery::refusal(&c, project, operation, &args, &code);
        }
        Err(reason) => return Err(reason),
    };
    if let Some(receipt) = receipt {
        return Ok(receipt);
    }
    let before = args.get("taskId").and_then(Value::as_str)
        .and_then(|id| diagnostics::execution_snapshot(&c, project, id).ok());
    let original_dispatch = if action=="resume" {
        let task=args["taskId"].as_str().unwrap();
        let scan=owned(&c,project,task)?;
        super::web_recovery_available_in(&c,task,i64::from(scan.attempt_count))
            .then_some(i64::from(scan.attempt_count))
    }else{None};
    let outcome = match action {
        "from_assets" => {
            let input: FromAssets =
                serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
            create_sentinel_asset_scan_with_receipt_in(
                &c,
                project,
                input.asset_ids,
                input.scan_mode,
                (input.max_model_requests, input.max_model_tokens),
                |tx, scan| creation_receipt(tx, project, operation, scan).map(|_| ()),
            )
        }
        "start" => confirm_sentinel_scan(
            app.clone(),
            app.state(),
            args["taskId"].as_str().unwrap().into(),
        ),
        "pause" => pause_sentinel_scan(app.state(), args["taskId"].as_str().unwrap().into()),
        "resume" => if let Some(attempt)=original_dispatch {
            let task=args["taskId"].as_str().unwrap();
            resolve_frontend_recon_worker(app).and_then(|worker|
                super::recover_web_attempt_impl(&state.db_path,&state.app_data_dir,task,attempt,true,&worker)
            ).and_then(|_|sentinel_scan_by_id(&c,task))
        }else{resume_sentinel_scan(app.clone(),app.state(),args["taskId"].as_str().unwrap().into())},
        _ => unreachable!(),
    };
    let (phase, receipt) = match outcome {
        Ok(scan) => (
            "completed",
            json!({"projectId":project,"operationId":operation,"operationState":"completed","action":action,"task":projection(&c,&scan)?,
                "executionRoute":if original_dispatch.is_some(){Some("same_attempt_never_dispatched")}else{None},
                "receiptIsHistorical":true}),
        ),
        Err(reason) => diagnostics::failed_receipt(&c, project, operation, action, &args, before.as_ref(), &reason),
    };
    let changed=c.execute("UPDATE agent_scan_operations SET state=?3,receipt_json=?4 WHERE project_id=?1 AND operation_id=?2 AND state='prepared'",params![project,operation,phase,receipt.to_string()]).map_err(|_|"scan_operation_outcome_unknown")?;
    if changed!=1 {return Err("scan_operation_outcome_unknown".into());}
    Ok(receipt)
}

#[cfg(test)]
#[path = "agent_channel_tests.rs"]
mod tests;
