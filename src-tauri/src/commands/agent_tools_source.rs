// §10.2 items 6, 10-11 — the model-facing half of the source broker. The tools are only
// advertised when the attempt has a frozen snapshot, and every call is answered from that
// snapshot through `SourceBroker`, which refuses shell, absolute paths, writes, self-chosen
// binaries, downloaded rules and network access.

#[cfg(test)]
fn agent_source_snapshot_revision(connection: &rusqlite::Connection, root_run_id: &str) -> Result<i64,String> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(revision),1) FROM agent_evidence_nodes WHERE root_run_id=?1",
            [root_run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error|error.to_string())
}

/// The broker for this attempt, or `None` when the scan never froze a repository.
fn agent_source_broker(
    context: &AgentRunContext,
) -> Option<(
    rusqlite::Connection,
    crate::native_pipeline::tools::SourceBroker,
    WorkbenchSourceScope,
)> {
    if context.target_url.starts_with("source:") {
        let connection=db::open(&context.db_path).ok()?;
        let authority=agent_native_source_tool_authority(&connection,context,"repo.inventory").ok()?;
        let scope=load_workbench_source_scope(&connection,&context.scan_id,context.attempt_number).ok()?;
        let snapshot=crate::native_pipeline::snapshot::RepositorySnapshot::restore(
            &connection,&context.scan_id,context.attempt_number).ok()??;
        let broker=crate::native_pipeline::tools::SourceBroker::scoped(snapshot,authority.view,
            &authority.lease.root_run_id,&context.run.as_ref()?.run_id,authority.revision).ok()?;
        return Some((connection,broker,scope));
    }
    // Pre-native single-Web fixtures exercise frozen-file parsing only. They
    // are not an executable production source identity or a fallback grant.
    #[cfg(not(test))]
    { None }
    #[cfg(test)]
    { agent_legacy_source_fixture_broker(context) }
}

#[cfg(test)]
fn agent_legacy_source_fixture_broker(context: &AgentRunContext) -> Option<(
    rusqlite::Connection,crate::native_pipeline::tools::SourceBroker,WorkbenchSourceScope,
)> {
    let connection = db::open(&context.db_path).ok()?;
    let scope=load_workbench_source_scope(&connection,&context.scan_id,context.attempt_number).ok()?;
    let snapshot = crate::native_pipeline::snapshot::RepositorySnapshot::restore(
        &connection,
        &context.scan_id,
        context.attempt_number,
    )
    .ok()
    .flatten()?;
    let manifest=crate::native_pipeline::analysis_view::AnalysisManifest::select(
        &snapshot,&context.scan_id,context.attempt_number,&serde_json::to_value(&scope).ok()?).ok()?;
    let view=crate::native_pipeline::analysis_view::SourceAnalysisView::restore(&connection,&manifest,&snapshot).ok()?;
    view.verify_source_receipt(&connection).ok()?;
    let ledger=context.run.as_ref()?;
    if ledger.db_path!=context.db_path { return None; }
    let run_id=&ledger.run_id;
    let root_run_id: String = connection
        .query_row(
            "SELECT root_run_id FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4
             AND root_run_id<>'' AND status IN ('prepared','running') AND cancel_requested_at=''",
            params![run_id,context.scan_id,context.attempt_number,context.target_url],
            |row| row.get(0),
        )
        .ok()?;
    crate::agent_runtime::multi_agent::lease::require_open_coordinator(
        &connection,&context.scan_id,context.attempt_number,&context.target_url,&root_run_id).ok()?;
    let revision = agent_source_snapshot_revision(&connection, &root_run_id).ok()?;
    let broker=crate::native_pipeline::tools::SourceBroker::scoped(snapshot,view,&root_run_id,run_id,revision).ok()?;
    if !agent_source_identity_matches(&connection,context,&broker) { return None; }
    Some((
        connection,
        broker,
        scope,
    ))
}

fn agent_native_source_tool_authority(connection:&rusqlite::Connection,context:&AgentRunContext,name:&str)
    -> Result<crate::agent_runtime::multi_agent::source::ToolAuthority,&'static str> {
    if connection.is_autocommit() {
        let tx=rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate)
            .map_err(|_|"source_tool_authorization_unavailable")?;
        let authority=agent_native_source_tool_authority(&tx,context,name)?;
        tx.commit().map_err(|_|"source_tool_authorization_unconfirmed")?;
        return Ok(authority);
    }
    let run=context.run.as_ref().ok_or("source_tool_run_missing")?;
    if run.db_path!=context.db_path { return Err("source_tool_database_changed"); }
    let authority=crate::agent_runtime::multi_agent::source::authorize_tool(connection,&context.scan_id,
        context.attempt_number,&context.target_url,&run.run_id,name).map_err(|_|"source_tool_authority_denied")?;
    let work:String=connection.query_row("SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
        params![context.scan_id,context.attempt_number],|r|r.get(0)).map_err(|_|"source_tool_work_directory_missing")?;
    let model=model_runtime_env(&sentinel_settings(connection)).map_err(|_|"source_tool_runtime_changed")?;
    let runtime=resolve_agent_web_pipeline_runtime(connection,&context.scan_id,&model.deployment,PathBuf::new())
        .map_err(|_|"source_tool_runtime_changed")?;
    let directory=WebBindingDirectory::open(Path::new(&work)).map_err(|_|"source_tool_runtime_changed")?;
    let contract=verify_source_runtime_in(connection,&context.scan_id,context.attempt_number,&directory,&model,&runtime)
        .map_err(|_|"source_tool_runtime_changed")?;
    let plan:String=connection.query_row("SELECT plan_json FROM agent_runs WHERE id=?1",[&authority.lease.root_run_id],|r|r.get(0))
        .map_err(|_|"source_tool_root_missing")?;
    let plan:JsonValue=serde_json::from_str(&plan).map_err(|_|"source_tool_root_invalid")?;
    if plan["runtime"]!=contract { return Err("source_tool_runtime_changed"); }
    let active:bool=connection.query_row(
        "SELECT status='running' AND started_at<>'' FROM agent_runs WHERE id=?1",
        [&authority.lease.root_run_id],|r|r.get(0)).map_err(|_|"source_tool_root_missing")?;
    if !active { return Err("source_tool_root_not_running"); }
    let _remaining=native_source_fresh_finance::remaining_for_new_work(connection,&authority.lease)
        .map_err(|reason|if reason=="budget_wall_time_exhausted" {"source_tool_deadline_exceeded"}
            else {"source_tool_original_finance_unavailable"})?;
    Ok(authority)
}

/// Whether this run may be offered the source tools at all.
fn agent_run_has_frozen_source(context: &AgentRunContext) -> bool {
    agent_source_broker(context).is_some()
}

fn agent_execute_source_tool(
    context: &AgentRunContext,
    name: &str,
    arguments: &JsonValue,
) -> JsonValue {
    let Some((connection, mut broker, scope)) = agent_source_broker(context) else {
        // No snapshot means no source answers — and no fallback to another engine.
        return agent_tool_error(
            "本轮没有有效运行身份、源码范围合同或分析视图；源码工具不可用，不回退整仓或其它引擎",
            "source_snapshot_unavailable",
        );
    };
    let Ok(transaction)=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate) else {
        return agent_tool_error("无法锁定源码工具权限与证据事务","source_authorization_unavailable");
    };
    if let Err(code)=agent_authorize_tool_on(&transaction,context,name) {
        return agent_tool_error("当前 run 没有此源码工具的有效权限",code);
    }
    if verify_workbench_source_scope(&transaction,&context.scan_id,context.attempt_number,&scope).is_err() {
        return agent_tool_error("源码请求合同已失效","source_scope_unavailable");
    }
    if !agent_source_identity_matches(&transaction,context,&broker) {
        return agent_tool_error("源码运行身份已变化","source_run_binding_changed");
    }
    match broker.call(&transaction, name, arguments) {
        Ok(value) => {
            if agent_authorize_tool_on(&transaction,context,name).is_err()
                || verify_workbench_source_scope(&transaction,&context.scan_id,context.attempt_number,&scope).is_err()
                || !agent_source_identity_matches(&transaction,context,&broker) {
                return agent_tool_error("源码执行期间权限发生变化，未提交工具结果","source_authorization_changed");
            }
            match transaction.commit() {
                Ok(())=>value,
                Err(_)=>agent_tool_error("源码工具事务提交失败，不能报告成功","source_tool_not_committed"),
            }
        },
        Err(denial) => {
            append_runner_log(
                &context.log_path,
                &format!("源码工具 {name} 被拒绝：{}", denial.code),
            );
            denial.as_json()
        }
    }
}

fn agent_source_identity_matches(connection: &rusqlite::Connection,context: &AgentRunContext,
    broker: &crate::native_pipeline::tools::SourceBroker) -> bool {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_runs root ON root.id=r.root_run_id
         WHERE r.id=?1 AND r.root_run_id=?2 AND r.scan_id=?3 AND r.attempt_number=?4 AND r.target_url=?5
         AND r.status IN ('prepared','running') AND r.cancel_requested_at=''
         AND root.root_run_id=root.id AND root.role='coordinator' AND root.scan_id=r.scan_id
         AND root.attempt_number=r.attempt_number AND root.target_url=r.target_url
         AND root.status IN ('prepared','running') AND root.cancel_requested_at='')",
        params![broker.run_id,broker.root_run_id,context.scan_id,context.attempt_number,context.target_url],
        |row|row.get::<_,bool>(0)).unwrap_or(false)
}
