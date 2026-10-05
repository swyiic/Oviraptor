/// Model transport has no Web execution plan, browser, identity or tool authority.
/// Source orchestration can use it without manufacturing a Web AgentRunContext.
#[derive(Clone)]
struct SpecialistTransportContext<'a> {
    supervision: Option<crate::agent_runtime::multi_agent::supervision_ticket::SupervisionTicket>,
    db_path: &'a Path,
    scan_id: &'a str,
    attempt_number: i64,
    target_key: &'a str,
    run_id: &'a str,
    environment: &'a ModelRuntimeEnv,
    proxy: Option<&'a str>,
    usage_dir: &'a Path,
    deadline: Option<std::time::Instant>,
}

impl SpecialistTransportContext<'_> {
    fn require_supervision(
        &self,
        lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        if let Some(ticket) = &self.supervision {
            ticket.check_actor(self.db_path, lease)?;
        }
        Ok(())
    }
}
