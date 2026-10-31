/// Side-effect class of a granted capability (design §3). The scanner's entire
/// live surface is `Standard`; the rest are high-risk pentest capability
/// families that do not exist as tools yet. They are refused by default at
/// scheduling (and, later, per call) so that adding any such capability to a
/// role allow-list, a lease, or a capability bundle cannot silently make it
/// usable — a task must explicitly authorize the class first (opt-in + per-
/// action approval, added in a later step). A target page can drive the agent,
/// so this is a fixed rule, never the model's judgement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CapabilityClass {
    Standard,
    CodeExec,
    TargetWrite,
    Egress,
    Persistence,
    Exfil,
    WafEvasion,
}

impl CapabilityClass {
    fn is_high_risk(self) -> bool {
        !matches!(self, CapabilityClass::Standard)
    }
    fn as_str(self) -> &'static str {
        match self {
            CapabilityClass::Standard => "none",
            CapabilityClass::CodeExec => "code_exec",
            CapabilityClass::TargetWrite => "target_write",
            CapabilityClass::Egress => "egress",
            CapabilityClass::Persistence => "persistence",
            CapabilityClass::Exfil => "exfil",
            CapabilityClass::WafEvasion => "waf_evasion",
        }
    }
}

/// Classify a capability string by name. The whole current surface
/// (evidence.read, review.write, mailbox.*, public_surface_get,
/// authorization_probe, and the evidence/source tool names) is `Standard`. The
/// patterns below name the high-risk families, so a future capability added
/// with such a name is refused by default; a new high-risk capability must also
/// be registered with its class explicitly — patterns are a floor, not the sole
/// classifier.
fn capability_class(capability: &str) -> CapabilityClass {
    let name = capability.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| name.contains(n));
    if has(&["shell", "/bin/", "subprocess", "rce", "command_exec", "exec_command", "run_command", "system_command"]) {
        CapabilityClass::CodeExec
    } else if has(&["persist", "implant", "backdoor", "autorun", "startup_item", "scheduled_task_install"]) {
        CapabilityClass::Persistence
    } else if has(&["exfil", "data_out", "beacon", "oob_exfiltrate"]) {
        CapabilityClass::Exfil
    } else if has(&["waf_bypass", "waf_evade", "bypass_waf", "evade_waf", "challenge_bypass"]) {
        CapabilityClass::WafEvasion
    } else if has(&["unrestricted_net", "egress_any", "raw_socket", "connect_any", "proxy_any"]) {
        CapabilityClass::Egress
    } else if has(&["asset_write", "target_write", "modify_target", "mutate_asset", "deface"]) {
        CapabilityClass::TargetWrite
    } else {
        CapabilityClass::Standard
    }
}

/// Whether this scan attempt authorized the given high-risk class. Reads the
/// immutable per-scan authorization record (written only by the operator via
/// `set_scan_capability_authorization`; the agent has no tool that writes it, so
/// it cannot self-authorize). A missing table, a missing row, a missing legal
/// attestation, or a class not in the authorized set each mean "not authorized".
/// Standard (non-high-risk) classes are always allowed and never query the DB.
fn scan_class_authorized(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    class: CapabilityClass,
) -> Result<bool, String> {
    if !class.is_high_risk() {
        return Ok(true);
    }
    let storage: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='scan_capability_authorizations')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "scan_capability_authorization_storage_unavailable")?;
    if !storage {
        return Ok(false);
    }
    let row = connection.query_row(
        "SELECT legal_attestation, authorized_classes FROM scan_capability_authorizations \
         WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt_number],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
    );
    let (attested, classes_json) = match row {
        Ok(found) => found,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    if attested != 1 {
        return Ok(false);
    }
    let classes: Vec<String> = serde_json::from_str(&classes_json).unwrap_or_default();
    Ok(classes.iter().any(|entry| entry == class.as_str()))
}

/// Default-closed floor: refuse any requested capability whose class is
/// high-risk and not authorized for this scan attempt, independent of the
/// per-role allow-lists. A target page can drive the agent, so this is decided
/// against the operator's immutable authorization record, never by the model.
fn refuse_high_risk_capabilities(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    capabilities: &[String],
) -> Result<(), String> {
    for capability in capabilities {
        let class = capability_class(capability);
        if class.is_high_risk()
            && !scan_class_authorized(connection, scan_id, attempt_number, class)?
        {
            return Err(format!("high_risk_capability_not_authorized:{}", class.as_str()));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_child_contract(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    lane: AgentLane,
    task_slice: &JsonValue,
    evidence_revision: i64,
    capabilities: &[String],
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<(), String> {
    super::source::validate_surface_role(connection, lease, role)?;
    if role == AgentRole::Coordinator {
        return Err("child_role_must_not_be_coordinator".into());
    }
    // A caller cannot grant target tools to a review/read-only child simply
    // by choosing another lane or passing an arbitrary capability string.
    if (role == AgentRole::EvidenceReviewer) != (lane == AgentLane::Review) {
        return Err("reviewer_requires_exclusive_review_lane".into());
    }
    if role.requires_read_only_analysis_lane() && lane != AgentLane::ReadOnlyAnalysis {
        return Err("analysis_specialist_requires_read_only_lane".into());
    }
    if role == AgentRole::DeepInvestigator && lane != AgentLane::ReadOnlyAnalysis {
        return Err("deep_investigator_requires_read_only_lane".into());
    }
    if role == AgentRole::WebExecutor && lane != AgentLane::TargetTouching {
        return Err("web_executor_requires_target_lane".into());
    }
    if role == AgentRole::Authorization && lane != AgentLane::TargetTouching {
        return Err("authorization_requires_target_lane".into());
    }
    if role == AgentRole::ExternalSurface && lane != AgentLane::TargetTouching {
        return Err("external_surface_requires_target_lane".into());
    }
    let allowed: Option<&[&str]> = match role {
        AgentRole::EvidenceReviewer => Some(&["evidence.read", "review.write"][..]),
        AgentRole::SpaApiMapper => Some(&["evidence.read", "mailbox.read", "mailbox.write"][..]),
        AgentRole::IdentitySession => Some(&["evidence.read", "mailbox.read", "mailbox.write"][..]),
        AgentRole::ClientSide => Some(&["evidence.read", "mailbox.write"][..]),
        AgentRole::InputParser => Some(&["evidence.read", "mailbox.write"][..]),
        AgentRole::BusinessLogic => Some(&["evidence.read", "mailbox.write"][..]),
        // Source validates the complete frozen phase/capability contract below.
        AgentRole::RepoMapper | AgentRole::SourceAnalyst => None,
        AgentRole::DeepInvestigator => {
            Some(&["evidence.read", "mailbox.read", "mailbox.write"][..])
        }
        AgentRole::WebExecutor => {
            // The executor's tool set is additionally checked per invocation by
            // the Broker against the exact lease and current fencing token.
            None
        }
        AgentRole::Authorization => Some(&["authorization_probe"][..]),
        AgentRole::ExternalSurface => {
            Some(&["public_surface_get", "evidence.read", "mailbox.write"][..])
        }
        AgentRole::Upload => return Err("upload_role_not_implemented".into()),
        AgentRole::Concurrency => return Err("concurrency_role_not_implemented".into()),
        _ => return Err("specialist_role_not_implemented".into()),
    };
    if let Some(allowed) = allowed {
        if capabilities
            .iter()
            .any(|capability| !allowed.contains(&capability.as_str()))
        {
            return Err("role_capability_not_allowed".into());
        }
    }
    // Class-based hard floor on top of the per-role allow-lists: a high-risk
    // capability family (command execution, target-asset mutation, scope-
    // widening egress, persistence, data exfiltration, WAF evasion) is refused
    // for every role unless this scan attempt holds the operator's immutable
    // authorization for that class. Applies even to the broker-checked roles
    // (WebExecutor / source), whose current capabilities are all Standard, so
    // this is a no-op today and a floor for the future.
    refuse_high_risk_capabilities(connection, &lease.scan_id, lease.attempt_number, capabilities)?;
    if super::source::is_source_role(role) {
        super::source::validate_scheduled_slice(
            connection,
            lease,
            role,
            task_slice,
            evidence_revision,
            capabilities,
        )?;
    }
    if role == AgentRole::EvidenceReviewer
        && super::source::uses_source_runtime(connection, lease, role)?
    {
        super::source_review_subject::validate_slice(
            connection,
            lease,
            task_slice,
            evidence_revision,
            capabilities,
        )?;
    }
    if role == AgentRole::ClientSide {
        super::client_side::validate_scheduled(task_slice, lease, evidence_revision, capabilities)?;
    }
    if role == AgentRole::IdentitySession {
        let (mode, _) = crate::auth_session::validated_scan_identities(
            connection,
            &lease.scan_id,
            &lease.target_key,
        )?;
        if mode == crate::auth_session::ScanIdentityMode::AnonymousOnly {
            return Err("identity_session_requires_bound_identity".into());
        }
    }
    if role == AgentRole::Authorization {
        if capabilities != ["authorization_probe"] {
            return Err("authorization_capability_invalid".into());
        }
        let (mode, identities) = crate::auth_session::validated_scan_identities(
            connection,
            &lease.scan_id,
            &lease.target_key,
        )?;
        let (count, invalid): (i64, i64) = connection.query_row(
            "SELECT COUNT(*),COALESCE(SUM(CASE WHEN method!='GET' OR owner_identity=tester_identity THEN 1 ELSE 0 END),0) \
             FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3",
            params![lease.scan_id, lease.attempt_number, lease.target_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|_| "authorization_control_lookup_failed".to_string())?;
        if mode != crate::auth_session::ScanIdentityMode::IdentitySet
            || count == 0
            || count > 4
            || invalid != 0
            || reserved_tokens != 0
            // No model invocation: target requests are claimed by their own
            // broker and never reserved from the model-request ledger.
            || reserved_requests != 0
        {
            return Err("authorization_control_or_budget_invalid".into());
        }
        let unbound: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 \
             AND (owner_identity NOT IN (SELECT value FROM json_each(?4)) OR tester_identity NOT IN (SELECT value FROM json_each(?4))))",
            params![lease.scan_id, lease.attempt_number, lease.target_key, serde_json::json!(identities).to_string()],
            |row| row.get(0),
        ).map_err(|_| "authorization_control_identity_lookup_failed".to_string())?;
        if unbound {
            return Err("authorization_control_identity_unbound".into());
        }
    }
    Ok(())
}

/// A capability PROFILE the single controller (design Option A) operates under
/// for one bounded phase. In the single-controller model the former specialist
/// roles become phases the one controller run enters and leaves, rather than
/// separate child agents. A phase validates EXACTLY like the equivalent child
/// contract — same lane, same allow-list, same B1/B2 high-risk gate — via
/// `validate_controller_phase`, so moving to one controller relaxes no per-role
/// boundary. `Planner` is the controller's own deliberation and holds no tool
/// capabilities. The `Validator` phase deliberately stays on the Review lane so
/// the "don't grade your own homework" separation is kept. Not wired into
/// leasing/execution yet: this is the vocabulary + gate the controller leasing
/// path (B3, next step) builds on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ControllerProfile {
    Planner,
    Recon,
    Web,
    Api,
    Validator,
}

impl ControllerProfile {
    /// The equivalent child role + hard lane a non-deliberation phase validates
    /// as. `None` for `Planner`, which executes nothing and takes no lane.
    fn execution_role_and_lane(self) -> Option<(AgentRole, AgentLane)> {
        match self {
            ControllerProfile::Planner => None,
            ControllerProfile::Recon => Some((AgentRole::ExternalSurface, AgentLane::TargetTouching)),
            ControllerProfile::Web => Some((AgentRole::WebExecutor, AgentLane::TargetTouching)),
            ControllerProfile::Api => Some((AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis)),
            ControllerProfile::Validator => {
                Some((AgentRole::EvidenceReviewer, AgentLane::Review))
            }
        }
    }
}

/// Validate that the controller may enter `profile` with `capabilities`, reusing
/// the exact per-role contract (lanes, allow-list) and the B1/B2 high-risk gate.
/// The single-controller execution path (B3) calls this before issuing the
/// phase's leases to the controller's own run; it is a pure gate that issues
/// nothing.
#[allow(dead_code, clippy::too_many_arguments)] // wired by the controller leasing path (B3, next step)
fn validate_controller_phase(
    connection: &Connection,
    lease: &CoordinatorLease,
    profile: ControllerProfile,
    task_slice: &JsonValue,
    evidence_revision: i64,
    capabilities: &[String],
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<(), String> {
    match profile.execution_role_and_lane() {
        // Pure deliberation phase: no tool capability may be leased to it.
        None => {
            if capabilities.is_empty() {
                Ok(())
            } else {
                Err("controller_planner_phase_grants_no_capabilities".into())
            }
        }
        Some((role, lane)) => validate_child_contract(
            connection,
            lease,
            role,
            lane,
            task_slice,
            evidence_revision,
            capabilities,
            reserved_tokens,
            reserved_requests,
        ),
    }
}

#[cfg(test)]
mod capability_class_tests {
    use super::{capability_class, refuse_high_risk_capabilities, scan_class_authorized, CapabilityClass};
    use rusqlite::{params, Connection};

    // In-memory DB with the authorization table (mirrors the schema the writer
    // module owns). `authorize` inserts an immutable-shaped row.
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE scan_capability_authorizations(
               scan_id TEXT NOT NULL, attempt_number INTEGER NOT NULL,
               legal_attestation INTEGER NOT NULL, authorized_classes TEXT NOT NULL,
               attested_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL DEFAULT '',
               PRIMARY KEY(scan_id,attempt_number));",
        )
        .unwrap();
        c
    }
    fn authorize(c: &Connection, scan: &str, attempt: i64, attested: i64, classes: &str) {
        c.execute(
            "INSERT INTO scan_capability_authorizations(scan_id,attempt_number,legal_attestation,authorized_classes) VALUES(?1,?2,?3,?4)",
            params![scan, attempt, attested, classes],
        )
        .unwrap();
    }

    #[test]
    fn current_surface_is_standard_and_allowed() {
        let caps = [
            "evidence.read", "review.write", "mailbox.read", "mailbox.write",
            "public_surface_get", "authorization_probe",
            "inspect_evidence", "replay_http", "compare_identities", "targeted_discovery",
            "browser_action", "record_hypothesis_result", "finish_target",
            "repo.inventory", "repo.read_slice", "analyzer.get_result",
            "evidence.submit_candidate", "assignment.finish",
        ];
        for c in caps {
            assert_eq!(capability_class(c), CapabilityClass::Standard, "{c} must be Standard");
        }
        // Standard capabilities need no authorization and never touch the table:
        // a bare connection with no auth table still passes.
        let conn = Connection::open_in_memory().unwrap();
        let owned: Vec<String> = caps.iter().map(|s| s.to_string()).collect();
        assert!(refuse_high_risk_capabilities(&conn, "scan-x", 1, &owned).is_ok());
    }

    #[test]
    fn high_risk_names_are_classified() {
        for (c, class) in [
            ("shell_exec", CapabilityClass::CodeExec),
            ("run_command", CapabilityClass::CodeExec),
            ("target.asset_write", CapabilityClass::TargetWrite),
            ("unrestricted_net", CapabilityClass::Egress),
            ("persist_cron", CapabilityClass::Persistence),
            ("dns_exfil", CapabilityClass::Exfil),
            ("waf_bypass", CapabilityClass::WafEvasion),
        ] {
            assert_eq!(capability_class(c), class, "{c}");
        }
    }

    #[test]
    fn high_risk_refused_without_authorization() {
        // Empty auth table → every high-risk class refused.
        let conn = db();
        for c in ["shell_exec", "unrestricted_net", "persist_cron", "dns_exfil", "waf_bypass", "target.asset_write"] {
            assert!(refuse_high_risk_capabilities(&conn, "scan-x", 1, &[c.to_string()]).is_err(), "{c}");
        }
        // No auth table at all → still refused (storage missing = not authorized).
        let bare = Connection::open_in_memory().unwrap();
        assert!(refuse_high_risk_capabilities(&bare, "scan-x", 1, &["shell_exec".to_string()]).is_err());
    }

    #[test]
    fn authorized_class_passes_only_when_attested_and_listed() {
        let conn = db();
        authorize(&conn, "scan-a", 1, 1, r#"["code_exec"]"#);
        assert!(scan_class_authorized(&conn, "scan-a", 1, CapabilityClass::CodeExec).unwrap());
        assert!(!scan_class_authorized(&conn, "scan-a", 1, CapabilityClass::Exfil).unwrap());
        // Listed but NOT attested → still refused (the legal-attestation gate).
        authorize(&conn, "scan-b", 1, 0, r#"["code_exec"]"#);
        assert!(!scan_class_authorized(&conn, "scan-b", 1, CapabilityClass::CodeExec).unwrap());
        // The refuse gate honors the record: authorized class allowed, others not.
        assert!(refuse_high_risk_capabilities(&conn, "scan-a", 1, &["run_command".to_string()]).is_ok());
        assert!(refuse_high_risk_capabilities(&conn, "scan-a", 1, &["dns_exfil".to_string()]).is_err());
        // A different attempt has no row → refused.
        assert!(refuse_high_risk_capabilities(&conn, "scan-a", 2, &["run_command".to_string()]).is_err());
    }
}

#[cfg(test)]
mod controller_profile_tests {
    use super::{AgentLane, AgentRole, ControllerProfile};

    #[test]
    fn profiles_map_to_the_equivalent_role_and_hard_lane() {
        // Planner is pure deliberation: no execution role, no lane.
        assert!(ControllerProfile::Planner.execution_role_and_lane().is_none());
        assert!(matches!(
            ControllerProfile::Recon.execution_role_and_lane(),
            Some((AgentRole::ExternalSurface, AgentLane::TargetTouching))
        ));
        assert!(matches!(
            ControllerProfile::Web.execution_role_and_lane(),
            Some((AgentRole::WebExecutor, AgentLane::TargetTouching))
        ));
        assert!(matches!(
            ControllerProfile::Api.execution_role_and_lane(),
            Some((AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis))
        ));
        // The validator phase stays on the Review lane — independent validation.
        assert!(matches!(
            ControllerProfile::Validator.execution_role_and_lane(),
            Some((AgentRole::EvidenceReviewer, AgentLane::Review))
        ));
    }
}
