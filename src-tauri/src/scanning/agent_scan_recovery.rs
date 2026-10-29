//! Original-owner recovery inventory. Never issues a fresh financial contract,
//! confirms a bill or replays target/model requests as a side effect of a read.
use super::*;

pub(super) fn financial_reason(code:&str)->bool {
    matches!(code,"scan_financial_continuation_required"|"scan_financial_reconciliation_required"|"scan_financial_history_unavailable")
}

pub(super) fn view(c:&rusqlite::Connection, project:i64, task:&str)->Result<Value,String> {
    let scan=owned(c,project,task)?;
    if scan.scan_type!="web" {return Err("scan_channel_web_only".into());}
    let reason=super::super::scan_resume_financial_preflight_in(c,task).err();
    let original_reason=super::super::require_web_start_financial_history_in(c,task).err()
        .map(|v|diagnostics::reason_code(&v));
    let attempt=i64::from(scan.attempt_count);
    let mut q=c.prepare("SELECT id,orchestration_policy,terminal_state,used_requests,used_tokens
        FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND backend='native' AND role='coordinator'
        AND parent_run_id IS NULL AND assignment_id='' ORDER BY id LIMIT 101").map_err(|_|"scan_recovery_unavailable")?;
    let mut roots=q.query_map(params![task,attempt],|r|Ok(json!({"rootRunId":r.get::<_,String>(0)?,
        "mode":r.get::<_,String>(1)?,"terminalState":r.get::<_,String>(2)?,
        "recordedModelRequests":r.get::<_,i64>(3)?,"recordedTokens":r.get::<_,i64>(4)?,
        "countersAreNotRemainingAllowance":true})))
        .map_err(|_|"scan_recovery_unavailable")?.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_|"scan_recovery_unavailable")?;
    let roots_truncated=roots.len()>100;roots.truncate(100);
    let costs=if attempt>0 {
        match super::super::native_root_cost_reconciliations_for_attempt(c,task,attempt) {
            Ok(v)=>json!({"checked":true,"pendingBills":v["items"].as_array().map(Vec::len),
                "blockedBills":v["blocked"].as_array().map(Vec::len),"confirmedReceipts":v["receipts"].as_array().map(Vec::len),"truncated":v["truncated"]}),
            Err(reason)=>json!({"checked":false,"reasonCode":diagnostics::reason_code(&reason)}),
        }
    }else{json!({"checked":true,"pendingBills":0,"blockedBills":0,"confirmedReceipts":0,"truncated":false})};
    let continuation=remaining_target_continuation(c,task,attempt);
    let target_work_started=continuation["workerStarted"].as_bool().unwrap_or(false);
    let issued=if attempt>0 {
        match super::super::native_financial_continuation_discovery_on(c,task,attempt) {
            Ok(v)=>{let i=&v["issued"];json!({"checked":true,"exists":!i.is_null(),"status":i.pointer("/status/status"),
                "assessmentCompleted":i.pointer("/status/assessmentCompleted"),"attemptClosed":i.pointer("/status/attemptClosed")})},
            Err(reason)=>json!({"checked":false,"reasonCode":diagnostics::reason_code(&reason)}),
        }
    }else{json!({"checked":true,"exists":false})};
    Ok(json!({"schemaVersion":1,"kind":"original_owner_financial_recovery","taskId":task,"attemptNumber":attempt,
        "reasonCode":reason,"originalReasonCode":original_reason,
        "ordinaryResumeAllowed":reason.is_none() && matches!(scan.status.as_str(),"paused"|"partial"),
        "ordinaryResumeFinanciallyAllowed":reason.is_none(),"executionAllowed":false,"automaticResumeAllowed":false,
        "originalRequestsReplayAllowed":false,"remainingTargetWorkImplemented":target_work_started,"originalLedgerPreserved":true,
        "costs":costs,"issuedAssessment":issued,"originalRoots":roots,"originalRootsTruncated":roots_truncated,
        "neverDispatchedRecoveryAvailable":super::super::web_recovery_available_in(c,task,attempt),
        "remainingTargetContinuation":continuation,
        "handoff":{"surface":"scanning","section":"original_costs","taskId":task,"attemptNumber":attempt},
        "reason":"保留原任务、原费用、剩余额度和截止时间。已有专用费用接续只支持冻结证据的新评估，尚不支持恢复已付费的目标剩余工作；不能用新任务替代原额度。未决请求先核对，不重放。"}))
}



fn queued_continuation_targets(c:&rusqlite::Connection, task:&str)->Result<Vec<Value>, String> {
    let mut statement=c.prepare("SELECT url,status FROM sentinel_targets WHERE scan_id=?1 AND status IN ('queued','pending','','paused') ORDER BY id LIMIT 32").map_err(|_| "claim_ledger_unchecked".to_string())?;
    let rows=statement.query_map([task],|row| Ok(json!({"url":row.get::<_,String>(0)?,"status":row.get::<_,String>(1)?}))).map_err(|_| "claim_ledger_unchecked".to_string())?;
    rows.collect::<Result<Vec<_>,_>>().map_err(|_| "claim_ledger_unchecked".to_string())
}

fn continuation_targets_unchecked()->Value {
    json!({"implemented":false,"executionStarted":false,"replaysUnknownRequests":false,"mutatesOriginalExit":false,"freshAllowanceMinted":false,
        "pipelineCalled":false,"workerStarted":false,"queuedTargets":null,"queuedTargetCount":null,"blockedReason":"claim_ledger_unchecked",
        "pipelineAdmission":{"checked":false,"called":false,"workerStarted":false,"freshAllowanceMinted":false}})
}

fn remaining_target_continuation(c:&rusqlite::Connection, task:&str, attempt:i64)->Value {
    // A failed read is not an empty remaining set, and it is not a manual-review result.
    let Ok(queued)=queued_continuation_targets(c, task) else { return continuation_targets_unchecked(); };
    let unknown=c.query_row(
        "SELECT COUNT(*) FROM agent_http_request_claims WHERE scan_id=?1 AND attempt_number=?2 AND received_at=''",
        params![task, attempt],
        |row| row.get::<_,i64>(0),
    ).ok();
    let root: Option<String> = c.query_row(
        "SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id='' ORDER BY id LIMIT 1",
        params![task, attempt], |row| row.get(0),
    ).ok();
    // A failed remaining-balance read stays a Result. It is not remaining budget,
    // an unlimited grant, or zero, and it does not mint an allowance.
    let balance=match root.as_deref() {
        Some(id)=>Some(crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(c, id)),
        None=>None,
    };
    let status: Option<String>=c.query_row("SELECT status FROM sentinel_scans WHERE id=?1",[task],|row| row.get(0)).ok();
    let deleted: Option<bool>=c.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",[task],|row| row.get(0)).ok();
    // sentinel_scan_is_active is the pipeline's own gate: status IN ('scanning','pausing')
    // and the scan is not deleted. launch_sentinel_url_pipeline returns before any target
    // work when that predicate is false. Promoting paused/partial into it rewrites the
    // original exit. An already active attempt is still not launched here: that pipeline
    // does not spend remaining_child_capacity of this original root, and ordinary
    // Confirm/Resume/Retry stays on require_web_start_financial_history_in, which
    // refuses a fresh allowance. This function does not call either path.
    let pipeline_active=matches!(status.as_deref(),Some("scanning"|"pausing")) && deleted==Some(false);
    let terminal_exit: Option<bool>=c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_web_attempt_closures WHERE scan_id=?1 AND attempt_number=?2)",
        params![task, attempt], |row| row.get(0),
    ).ok();
    let manual=c.query_row("SELECT COUNT(*) FROM sentinel_targets WHERE scan_id=?1 AND status='manual_review'", [task], |row| row.get::<_, i64>(0));
    let started=c.query_row(
        "SELECT launched FROM scan_target_continuation_fences WHERE scan_id=?1 AND attempt_number=?2 ORDER BY created_at DESC LIMIT 1",
        params![task, attempt], |row| row.get::<_, i64>(0),
    ).unwrap_or(0)==1;
    let blocked=if started {"continuation_worker_started"}
        else if unknown.is_none() {"claim_ledger_unchecked"}
        else if unknown==Some(0) && queued.is_empty() && matches!(manual, Ok(count) if count>0) {"continuation_needs_manual_review"}
        else if unknown==Some(0) && queued.is_empty() && manual.is_err() {"claim_ledger_unchecked"}
        else if unknown==Some(0) && queued.is_empty() {"no_remaining_targets"}
        else if unknown.unwrap_or(0)>0 {"unknown_request_not_replayable"}
        else if matches!(&balance, Some(Err(_))) {"claim_ledger_unchecked"}
        else if matches!(&balance, Some(Ok((tokens, requests))) if *tokens==Some(0) || *requests==Some(0)) {"original_request_balance_exhausted"}
        else if pipeline_active && terminal_exit==Some(false) {"original_root_pipeline_unbound"}
        else {"continuation_launch_not_admitted"};
    let (remaining_tokens, remaining_requests)=match &balance {
        Some(Ok((tokens, requests)))=>(Some(json!(tokens)), Some(json!(requests))),
        None=>(Some(json!(null)), Some(json!(null))),
        Some(Err(_))=>(None, None),
    };
    let mut body=json!({"implemented":started,"executionStarted":started,"replaysUnknownRequests":false,"mutatesOriginalExit":false,"freshAllowanceMinted":false,
        "pipelineCalled":started,"workerStarted":started,
        "originalRootRunId":root,
        "queuedTargets":queued,"queuedTargetCount":queued.len(),"unknownRequestCount":unknown,
        "blockedReason":blocked,
        "pipelineAdmission":{"checked":true,"called":started,"workerStarted":started,"scanStatus":status,
            "requiresStatus":["scanning","pausing"],"pipelineWouldRun":pipeline_active,"terminalAttemptExit":terminal_exit,"freshAllowanceMinted":false},
        "refusalCodes":{
            "unknown_request_not_replayable":"agent_http_request_claims rows with received_at='' are not resent",
            "original_request_balance_exhausted":"the original root has no remaining tokens or requests",
            "continuation_needs_manual_review":"queued work is gone and at least one target still needs a person",
            "continuation_launch_not_admitted":"launch_sentinel_url_pipeline only continues while status is scanning or pausing; a paused or closed attempt is left unchanged and no fresh allowance is minted",
            "original_root_pipeline_unbound":"the attempt is active and open, but the URL pipeline is not called because it does not spend the original root remaining capacity"
        },
        "detail":"已绑定原任务、原 Root 和剩余额度。URL 管线只在 scanning 或 pausing 时继续，且不消耗这笔原 Root 剩余额度；本路径不调用它，不改原退出，不新开额度，也不重发 received_at 为空的请求。"});
    if let (Some(tokens), Some(requests))=(remaining_tokens, remaining_requests) {
        body["remainingTokens"]=tokens;
        body["remainingRequests"]=requests;
    }
    body
}


pub(super) fn read(c:&rusqlite::Connection, project:i64, task:&str)->Result<Value,String> {
    let tx=c.unchecked_transaction().map_err(|_|"scan_recovery_unavailable")?;
    let scan=owned(&tx,project,task)?;
    Ok(json!({"projectId":project,"task":projection(&tx,&scan)?,"recovery":view(&tx,project,task)?}))
}

pub(super) fn refusal(c:&rusqlite::Connection, project:i64, operation:&str,args:&Value,code:&str)->Result<Value,String> {
    let task=args["taskId"].as_str().ok_or("scan_arguments_invalid")?;
    let mut value=read(c,project,task)?;
    let object=value.as_object_mut().ok_or("scan_recovery_unavailable")?;
    object.extend(json!({"operationId":operation,"operationState":"rejected","action":"resume","taskId":task,
        "phase":"not_started","reasonCode":code,"outcomeUnknown":false,"executionStartedByThisOperation":false,
        "retryable":false,"nextAction":"scan_recovery","reason":"普通恢复被原费用保护门禁拒绝；没有创建执行尝试或发送请求。已附原任务恢复诊断，不是授权丢失，不要反复请求权限或创建新额度。"}).as_object().unwrap().clone());
    Ok(value)
}
