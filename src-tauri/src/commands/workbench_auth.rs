// Historical authentication JSON supplies identity handles, never renewed
// authority or credential material. Resolve handles against the live store.
fn workbench_browser_auth_in(
    connection: &rusqlite::Connection,
    session_ids: &[String],
    project_id: i64,
    owner_scan_id: Option<&str>,
) -> Result<JsonValue, String> {
    let mut unique = std::collections::BTreeSet::new();
    if session_ids.is_empty() || session_ids.len() > 5
        || session_ids.iter().any(|id| id.trim().is_empty() || id.trim() != id || !unique.insert(id)) {
        return Err("workbench_auth_identity_set_invalid".into());
    }
    if let Some(scan_id) = owner_scan_id {
        for id in session_ids {
            let owned: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM browser_auth_sessions WHERE id=?1 AND project_id=?2 AND owner_scan_id=?3 AND draft_scope_id='')",
                params![id,project_id,scan_id], |row| row.get(0),
            ).map_err(|_| "workbench_auth_identity_unavailable")?;
            if !owned { return Err("workbench_auth_identity_not_owned".into()); }
        }
    }
    let mut documents = crate::auth_session::distinct_session_documents_for_scan(connection,session_ids,project_id)?;
    for (id,document) in session_ids.iter().zip(&documents) {
        let scope_valid = document.get("scopeHosts").and_then(JsonValue::as_array)
            .is_some_and(|hosts| !hosts.is_empty() && hosts.iter().all(|host| host.as_str().is_some_and(|v| !v.trim().is_empty())));
        let expiry_valid = document.get("expiresAt").and_then(JsonValue::as_str)
            .and_then(|expiry| chrono::DateTime::parse_from_rfc3339(expiry).ok())
            .is_some_and(|expiry| expiry > chrono::Utc::now());
        if document.get("schemaVersion").and_then(JsonValue::as_i64) != Some(1)
            || document.get("id").and_then(JsonValue::as_str) != Some(id.as_str())
            || document.get("projectId").and_then(JsonValue::as_i64) != Some(project_id)
            || !scope_valid || !expiry_valid {
            return Err("workbench_auth_document_invalid".into());
        }
    }
    Ok(if documents.len() == 1 { documents.remove(0) } else {
        json!({"schemaVersion":2,"kind":"identity-matrix","sessions":documents,
            "comparisonPolicy":"same-target-same-action-plan",
            "identityIsolation":"dedicated-webview-and-distinct-auth-material"})
    })
}

fn workbench_current_browser_auth(
    connection: &rusqlite::Connection,
    session_ids: &[String],
    project_id: i64,
    owner_scan_id: Option<&str>,
) -> Result<JsonValue, String> {
    // Read ownership, status, expiry and all identities from one DB snapshot.
    let tx = rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Deferred)
        .map_err(|_| "workbench_auth_snapshot_unavailable")?;
    let document = workbench_browser_auth_in(&tx,session_ids,project_id,owner_scan_id)?;
    tx.commit().map_err(|_| "workbench_auth_snapshot_unavailable")?;
    Ok(document)
}

fn restore_workbench_browser_auth(
    connection: &rusqlite::Connection,
    persisted: &JsonValue,
    project_id: i64,
    scan_id: &str,
) -> Result<Option<(Vec<String>,JsonValue)>,String> {
    let object = persisted.as_object().ok_or("workbench_auth_history_invalid")?;
    let session_values: Vec<&JsonValue> = match persisted.get("schemaVersion").and_then(JsonValue::as_i64) {
        Some(1) if !object.contains_key("sessions") => vec![persisted],
        Some(2) if persisted.get("kind").and_then(JsonValue::as_str) == Some("identity-matrix") => {
            persisted.get("sessions").and_then(JsonValue::as_array)
                .ok_or("workbench_auth_history_invalid")?.iter().collect()
        },
        None if !["schemaVersion","sessions","id","kind"].iter().any(|key| object.contains_key(*key)) => return Ok(None),
        _ => return Err("workbench_auth_history_invalid".into()),
    };
    let mut ids = session_values.iter().map(|value| {
        value.get("id").and_then(JsonValue::as_str).map(str::to_owned)
            .ok_or_else(|| "workbench_auth_history_invalid".to_string())
    }).collect::<Result<Vec<_>,_>>()?;
    ids.sort(); // Stable policy order, without silently discarding duplicates.
    let document = workbench_current_browser_auth(connection,&ids,project_id,Some(scan_id))?;
    Ok(Some((ids,document)))
}

fn verify_workbench_browser_auth_publication(
    connection: &rusqlite::Connection,
    record: &WorkbenchStartRecord,
    work_dir: &Path,
) -> Result<(),String> {
    if record.auth_session_ids.is_empty() {
        if matches!(record.auth_type.as_str(),"browser_session"|"browser_session_matrix") {
            return Err("workbench_auth_identity_set_invalid".into());
        }
        return Ok(());
    }
    if record.scan_type != "greybox" || !record.authenticated
        || !matches!(record.auth_type.as_str(),"browser_session"|"browser_session_matrix") {
        return Err("workbench_auth_context_mismatch".into());
    }
    let mut ids = record.auth_session_ids.clone();
    ids.sort();
    if record.policy.get("authSessionIds") != Some(&json!(ids))
        || record.policy.get("authSessionId") != Some(&json!(ids.first().cloned().unwrap_or_default())) {
        return Err("workbench_auth_policy_mismatch".into());
    }
    let staged: JsonValue = fs::read(work_dir.join("auth-session.json")).ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("workbench_auth_staged_document_unavailable")?;
    let current = workbench_browser_auth_in(connection,&ids,record.project_id,Some(&record.scan_id))?;
    if staged != current { return Err("workbench_auth_inputs_changed".into()); }
    Ok(())
}
