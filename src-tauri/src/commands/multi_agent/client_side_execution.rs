// Dedicated production ClientSide worker. It reads supplier facts, no new target I/O.
fn multi_agent_client_side_readonly(
    context: &AgentRunContext,
    session: &MultiAgentSession,
) -> Result<(), String> {
    session.supervisor.check()?;
    let db = db::open(&context.db_path)?;
    let Some(task) = client_side_frozen_http_task(&db, &session.lease, &context.target_dir)? else {
        return Ok(());
    };
    let child = client_grant_writer::prepare(context, &session.lease, &task)?;
    let result=multi_agent_child_round_transport(context,&session.lease,&child,
        "你是独立ClientSide配置事实专家。只分析冻结本地配置观察，不调用工具、浏览器或目标。输出严格JSON：summary、observationRefs、gaps、candidates。引用已有观察id；candidates=[]；gaps包含missing_browser_validation。配置缺失不是影响证明，不能confirmed。",task.clone());
    let (text, usage) = match result {
        Ok(value) => value,
        Err(code) if code.starts_with("client_side_sdk_dispatch_") => return Err(code),
        Err(code) => return Err(failed_specialist_error(&db, &session.lease, &child, &code)),
    };
    session.supervisor.check()?;
    deliver_readonly_assessment(
        &db,
        &session.lease,
        &child,
        &usage,
        &json!({"summary":text,"clientSideTask":task}),
        Some(&context.target_dir),
    )?;
    session.supervisor.check()?;
    // The normal prepared pipeline has an original paid independent Mapper.
    // Standalone readonly supplier contexts do not fabricate Root SDK history.
    if session.mapper.role == crate::agent_runtime::contract::AgentRole::SpaApiMapper {
        native_coordinator_client_feedback(context, session, &child)?;
    }
    session.supervisor.check()
}
