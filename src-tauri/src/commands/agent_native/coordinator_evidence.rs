// Original signed attempt path + actual frozen frontend evidence; no UI JSON.
fn native_coordinator_frozen_evidence(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<(JsonValue, String), String> {
    let mode = private_web_mode_on(db, &context.scan_id, context.attempt_number)?;
    let directory = checked_evidence_directory(
        Path::new(&mode.fact().work_dir),
        &context.target_dir,
        &context.scan_id,
    )?;
    let file = directory.join("frontend-evidence.json");
    let metadata = fs::symlink_metadata(&file).map_err(|_| "root_tick_evidence_missing")?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return Err("root_tick_evidence_invalid".into());
    }
    let bytes = fs::read(&file).map_err(|_| "root_tick_evidence_unreadable")?;
    let evidence: JsonValue =
        serde_json::from_slice(&bytes).map_err(|_| "root_tick_evidence_invalid")?;
    let mut observed = context.evidence.clone();
    if let Some(object) = observed.as_object_mut() {
        object.retain(|key, _| !key.starts_with("multiAgent") && key != "followupObjective");
    }
    if evidence.as_object().is_none()
        || evidence.get("error").is_some()
        || evidence["url"].as_str() != Some(&context.target_url)
        || evidence != observed
    {
        return Err("root_tick_original_evidence_conflict".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "root_tick_evidence_invalid")?;
    Ok((
        crate::agent_runtime::secrets::redact_json(&evidence),
        crate::agent_runtime::store::stable_hash(text),
    ))
}
