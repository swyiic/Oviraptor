// An agent policy version is an execution input, never an execution grant.
// Only the existing startup publication transaction can mint this snapshot.
// Historical attempts are not backfilled by reads or recovery.
const AGENT_POLICY_SNAPSHOT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS scan_agent_policy_snapshots (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    payload_digest TEXT NOT NULL,
    PRIMARY KEY(scan_id,attempt_number)
);
CREATE TRIGGER IF NOT EXISTS scan_agent_policy_no_update
BEFORE UPDATE ON scan_agent_policy_snapshots BEGIN SELECT RAISE(ABORT,'scan_agent_policy_immutable'); END;
CREATE TRIGGER IF NOT EXISTS scan_agent_policy_no_replace
BEFORE INSERT ON scan_agent_policy_snapshots
WHEN EXISTS(SELECT 1 FROM scan_agent_policy_snapshots WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)
BEGIN SELECT RAISE(ABORT,'scan_agent_policy_immutable'); END;
CREATE TRIGGER IF NOT EXISTS scan_agent_policy_no_delete
BEFORE DELETE ON scan_agent_policy_snapshots
WHEN EXISTS(SELECT 1 FROM sentinel_scans WHERE id=OLD.scan_id)
BEGIN SELECT RAISE(ABORT,'scan_agent_policy_immutable'); END;
"#;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentPolicySnapshot {
    schema_version: u32,
    scan_id: String,
    attempt_number: i64,
    profile_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    configuration_source: Option<String>,
    policy_reference: Option<crate::configuration_policies::PolicyReference>,
    effective_policy: JsonValue,
}

// Freeze defaults and clamping as well as explicitly configured values. Model
// connections, proxy credentials and host OAuth can never enter this object.
fn effective_agent_policy(settings: &JsonValue) -> JsonValue {
    let budget = AgentBudgetSettings::from_json(settings);
    let audit = match settings.get("agentPromptAuditMode").and_then(JsonValue::as_str) {
        Some("metadata") => "metadata",
        Some("full") => "full",
        _ => "off",
    };
    let packet_mode = match settings.get("agentFrontendPacketMode").and_then(JsonValue::as_str) {
        Some("compact") => "compact",
        Some("custom") => "custom",
        _ => "balanced",
    };
    json!({
        "agentLocalFullPower":settings.get("agentLocalFullPower").and_then(JsonValue::as_bool).unwrap_or(false),
        "agentPromptAuditMode":audit,
        "agentFrontendPacketMode":packet_mode,
        "agentFrontendPacketBudgetKb":setting_u64(settings,"agentFrontendPacketBudgetKb",24,4,64),
        "agentQuickScore":budget.quick_score,"agentStandardScore":budget.standard_score,"agentDeepScore":budget.deep_score,
        "agentQuickTimeout":budget.quick_timeout,"agentStandardTimeout":budget.standard_timeout,"agentDeepTimeout":budget.deep_timeout,
        "agentQuickTokenLimit":budget.quick_tokens,"agentStandardTokenLimit":budget.standard_tokens,"agentDeepTokenLimit":budget.deep_tokens,
        "agentQuickRequestLimit":budget.quick_requests,"agentStandardRequestLimit":budget.standard_requests,"agentDeepRequestLimit":budget.deep_requests,
        "agentNoToolTurnLimit":budget.no_tool_turn_limit,
    })
}

fn agent_policy_snapshot_storage_exists(connection: &rusqlite::Connection) -> Result<bool,String> {
    connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='scan_agent_policy_snapshots')",[],|r|r.get(0))
        .map_err(|_|"scan_agent_policy_storage_unavailable".into())
}

fn read_agent_policy_snapshot_in(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
) -> Result<Option<AgentPolicySnapshot>,String> {
    if !agent_policy_snapshot_storage_exists(connection)? { return Ok(None); }
    let row:Option<(String,String)>=connection.query_row(
        "SELECT snapshot_json,payload_digest FROM scan_agent_policy_snapshots WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id,attempt],|r|Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(|_|"scan_agent_policy_snapshot_unavailable")?;
    let Some((text,digest))=row else { return Ok(None); };
    if digest!=format!("{:x}",Sha256::digest(text.as_bytes())) { return Err("scan_agent_policy_snapshot_digest_mismatch".into()); }
    let snapshot:AgentPolicySnapshot=serde_json::from_str(&text).map_err(|_|"scan_agent_policy_snapshot_invalid")?;
    let source_valid = match snapshot.schema_version {
        1 => snapshot.profile_id > 0 && snapshot.configuration_source.is_none(),
        2 => snapshot.profile_id == 0 && snapshot.configuration_source.as_deref() == Some("category-defaults"),
        _ => false,
    };
    if !source_valid || snapshot.scan_id!=scan_id || snapshot.attempt_number!=attempt
        || snapshot.policy_reference.as_ref().is_some_and(|r|r.kind!="agent"||r.revision<=0||r.policy_id.is_empty())
        || crate::configuration_policies::project(&snapshot.effective_policy,"agent")?!=snapshot.effective_policy
        || effective_agent_policy(&snapshot.effective_policy)!=snapshot.effective_policy {
        return Err("scan_agent_policy_snapshot_invalid".into());
    }
    Ok(Some(snapshot))
}

fn freeze_agent_policy_in(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64, resume_parent: Option<i64>,
) -> Result<AgentPolicySnapshot,String> {
    if connection.is_autocommit() || attempt<=0 { return Err("scan_agent_policy_publication_requires_transaction".into()); }
    // A continuation must never substitute today's default for an unknown
    // historical policy. The operator can choose an independently admitted
    // fresh attempt; sealed financial and lifecycle checks remain unchanged.
    let mut snapshot=if let Some(parent)=resume_parent {
        if parent<=0 || parent>=attempt { return Err("scan_agent_policy_parent_invalid".into()); }
        read_agent_policy_snapshot_in(connection,scan_id,parent)?
            .ok_or("scan_agent_policy_history_unknown_requires_fresh_attempt")?
    } else {
        let settings=crate::model_configuration::default_settings_for(connection, &["agent"]);
        if !settings.is_object() || settings.get("_configurationError").is_some() {
            return Err("scan_agent_policy_profile_unavailable".into());
        }
        AgentPolicySnapshot { schema_version:2,scan_id:scan_id.into(),attempt_number:attempt,profile_id:0,
            configuration_source:Some("category-defaults".into()),
            policy_reference:crate::configuration_policies::reference(&settings,"agent")?,
            effective_policy:effective_agent_policy(&settings) }
    };
    snapshot.attempt_number=attempt;
    connection.execute_batch(AGENT_POLICY_SNAPSHOT_SCHEMA).map_err(|_|"scan_agent_policy_storage_unavailable")?;
    let text=serde_json::to_string(&snapshot).map_err(|_|"scan_agent_policy_snapshot_invalid")?;
    let changed=connection.execute("INSERT INTO scan_agent_policy_snapshots(scan_id,attempt_number,snapshot_json,payload_digest) VALUES(?1,?2,?3,?4)",
        params![scan_id,attempt,text,format!("{:x}",Sha256::digest(text.as_bytes()))])
        .map_err(|_|"scan_agent_policy_publication_failed")?;
    if changed!=1 { return Err("scan_agent_policy_publication_failed".into()); }
    let stored=read_agent_policy_snapshot_in(connection,scan_id,attempt)?.ok_or("scan_agent_policy_publication_failed")?;
    if serde_json::to_value(&stored).map_err(|_|"scan_agent_policy_snapshot_invalid")?
        !=serde_json::to_value(&snapshot).map_err(|_|"scan_agent_policy_snapshot_invalid")? {
        return Err("scan_agent_policy_publication_failed".into());
    }
    Ok(stored)
}

fn agent_policy_settings_for_attempt(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
) -> Result<JsonValue,String> {
    let mut settings=sentinel_settings(connection);
    let Some(snapshot)=read_agent_policy_snapshot_in(connection,scan_id,attempt)? else {
        // Preserve the historical binding/sealed-budget refusal path. Reading
        // old rows must not mint a policy or authorize automatic replay.
        return Ok(settings);
    };
    if settings.get("_configurationError").is_some() { return Err("scan_agent_policy_live_configuration_unavailable".into()); }
    let object=settings.as_object_mut().ok_or("scan_agent_policy_live_configuration_unavailable")?;
    for key in crate::configuration_policies::policy_keys("agent")? { object.remove(*key); }
    object.extend(snapshot.effective_policy.as_object().ok_or("scan_agent_policy_snapshot_invalid")?.clone());
    if let Some(references)=object.get_mut("_configurationRefs").and_then(JsonValue::as_object_mut) { references.remove("agent"); }
    if let Some(reference)=snapshot.policy_reference {
        let references=object.entry("_configurationRefs").or_insert_with(||json!({})).as_object_mut()
            .ok_or("scan_agent_policy_live_configuration_unavailable")?;
        references.insert("agent".into(),serde_json::to_value(reference).map_err(|_|"scan_agent_policy_snapshot_invalid")?);
    }
    if object.get("_configurationRefs").and_then(JsonValue::as_object).is_some_and(|r|r.is_empty()) { object.remove("_configurationRefs"); }
    Ok(settings)
}

fn agent_policy_settings_for_scan(connection: &rusqlite::Connection,scan_id: &str) -> Result<JsonValue,String> {
    let attempt:Option<i64>=connection.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1",[scan_id],|r|r.get(0))
        .optional().map_err(|_|"scan_agent_policy_attempt_unavailable")?;
    match attempt { Some(attempt)=>agent_policy_settings_for_attempt(connection,scan_id,attempt),None=>Ok(sentinel_settings(connection)) }
}

fn agent_model_runtime_for_scan(connection: &rusqlite::Connection,scan_id: &str) -> Result<ModelRuntimeEnv,String> {
    model_runtime_env(&agent_policy_settings_for_scan(connection,scan_id)?)
}
