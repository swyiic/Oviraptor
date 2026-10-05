struct NativeAgentBackend;

/// Keep the fenced executor alive during slow local model generations as well
/// as between tool calls. Dropping the guard wakes and joins the worker, so a
/// completed run never leaves a background lease renewer behind.
struct ExecutorLeaseHeartbeat {
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ExecutorLeaseHeartbeat {
    fn start(context: &AgentRunContext) -> Result<Option<Self>, String> {
        let Some(run) = &context.run else { return Ok(None) };
        if run.db_path!=context.db_path {return Err("tool_authorization_unavailable".into());}
        let connection = db::open(&run.db_path)?;
        let identity: Option<(String, String)> = connection
            .query_row(
                "SELECT orchestration_policy,role FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND backend='native'",
                params![run.run_id,context.scan_id,context.attempt_number,context.target_url],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("无法读取 executor lease 身份：{error}"))?;
        if identity.is_none() {return Err("tool_run_not_found".into());}
        if !matches!(identity, Some((ref policy, ref role)) if policy == "multi" && role == "web_executor") {
            return Ok(None);
        }
        crate::agent_runtime::multi_agent::scheduler::refresh_running_executor_leases_authorized(
            &connection, &run.run_id,|db|native_model_reentry_guard(db,context),
        )?;
        let (stop, receiver) = mpsc::channel();
        let path = run.db_path.clone();
        let run_id = run.run_id.clone();
        let worker = thread::spawn(move || {
            while matches!(receiver.recv_timeout(Duration::from_secs(60)), Err(mpsc::RecvTimeoutError::Timeout)) {
                if let Ok(connection) = db::open(&path) {
                    // A transient lock may fail this tick; the next tick can
                    // retry. A stale fence never becomes valid again.
                    let _ = crate::agent_runtime::multi_agent::scheduler::refresh_running_executor_leases(
                        &connection, &run_id,
                    );
                }
            }
        });
        Ok(Some(Self { stop, worker: Some(worker) }))
    }
}

impl Drop for ExecutorLeaseHeartbeat {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl AgentBackend for NativeAgentBackend {
    fn kind(&self) -> AgentBackendKind {
        AgentBackendKind::Native
    }

    fn execute(&self, context: &AgentRunContext) -> AgentTargetOutcome {
        native_run_with_single_finally(context)
    }
}
