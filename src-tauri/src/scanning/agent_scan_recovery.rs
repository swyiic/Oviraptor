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
        "originalRequestsReplayAllowed":false,"remainingTargetWorkImplemented":false,"originalLedgerPreserved":true,
        "costs":costs,"issuedAssessment":issued,"originalRoots":roots,"originalRootsTruncated":roots_truncated,
        "neverDispatchedRecoveryAvailable":super::super::web_recovery_available_in(c,task,attempt),
        "handoff":{"surface":"scanning","section":"original_costs","taskId":task,"attemptNumber":attempt},
        "reason":"保留原任务、原费用、剩余额度和截止时间。已有专用费用接续只支持冻结证据的新评估，尚不支持恢复已付费的目标剩余工作；不能用新任务替代原额度。未决请求先核对，不重放。"}))
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
