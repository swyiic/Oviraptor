fn prepare_current_attempt_surface(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<(), String> {
    let marker_key = format!("agent-current-attempt:{scan_id}");
    require_scan_without_retired_data(connection,scan_id)?;
    let expected = attempt_number.to_string();
    let previous = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [&marker_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if previous.as_deref() == Some(expected.as_str()) {
        return Ok(());
    }

    let (scan_type, execution_mode): (String, String) = connection
        .query_row(
            "SELECT s.scan_type,COALESCE(a.execution_mode,'initial') FROM sentinel_scans s LEFT JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=?2 WHERE s.id=?1",
            params![scan_id, attempt_number],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;

    if scan_type == "web" && execution_mode == "fresh" {
        // A full rerun owns a new current-result surface. Confirmed human
        // validations and task-scoped login sessions remain durable, while
        // machine-generated evidence is rebuilt from the new browser/Agent
        // artifacts. The immutable attempt directory remains the audit copy.
        for table in [
            "investigation_identity_diffs",
            "investigation_metrics",
            "investigation_edges",
            "investigation_nodes",
            "investigation_actions",
            "investigation_api_models",
            "investigation_hypotheses",
        ] {
            connection
                .execute(&format!("DELETE FROM {table} WHERE scan_id=?1"), [scan_id])
                .map_err(|error| error.to_string())?;
        }
        connection
            .execute(
                "DELETE FROM sentinel_checkpoints WHERE scan_id=?1",
                [scan_id],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute("DELETE FROM sentinel_findings WHERE scan_id=?1", [scan_id])
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM sentinel_opportunities WHERE scan_id=?1 AND status IN ('queued','ready')",
                [scan_id],
            )
            .map_err(|error| error.to_string())?;
    } else if scan_type == "web" && execution_mode == "resume" {
        // A continuation retains existing evidence; only pending opportunities
        // for targets in the new attempt are rebuilt.
        connection
            .execute(
                "DELETE FROM sentinel_opportunities WHERE scan_id=?1 AND status IN ('queued','ready') AND target_url IN (SELECT url FROM sentinel_targets WHERE scan_id=?1 AND last_attempt_number=?2)",
                params![scan_id, attempt_number],
            )
            .map_err(|error| error.to_string())?;
    }
    if scan_type == "web" && matches!(execution_mode.as_str(), "fresh" | "resume") {
        connection
            .execute(
                "DELETE FROM agent_learning_candidates WHERE scan_id=?1 AND status='pending'",
                [scan_id],
            )
            .map_err(|error| error.to_string())?;
    }

    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![marker_key, expected],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn asset_match_keys(value: &str) -> Vec<String> {
    let normalized = value.trim().trim_end_matches('/').to_ascii_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut keys = vec![normalized.clone()];
    let without_scheme = normalized
        .strip_prefix("https://")
        .or_else(|| normalized.strip_prefix("http://"))
        .unwrap_or(&normalized);
    keys.push(without_scheme.to_string());
    if let Some((host, _)) = without_scheme.split_once('/') {
        keys.push(host.to_string());
        if normalized.starts_with("https://") {
            keys.push(format!("https://{host}"));
        } else if normalized.starts_with("http://") {
            keys.push(format!("http://{host}"));
        }
    }
    keys.sort();
    keys.dedup();
    keys
}
