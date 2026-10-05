//! Read-only request accounting. A claim consumes budget, but is not proof of
//! a response. These readers never authorize execution or refund missing files.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalRequestUsage {
    pub received: i64,
    pub unresolved: i64,
}

/// Follow the same typed attempt lineage used by native checkpoint inheritance.
/// Fresh attempts stop the chain; absent legacy mode rows retain initial scope.
pub fn budget_attempts(
    connection: &Connection,
    scan: &str,
    attempt: i64,
) -> Result<Vec<i64>, String> {
    if attempt < 1 {
        return Err("request_accounting_attempt_invalid".into());
    }
    let mut result = Vec::new();
    let mut current = attempt;
    loop {
        // The validated parent is strictly smaller, so cycles are impossible.
        // Do not add an arbitrary attempt-count gate to a valid budget chain.
        result.push(current);
        let mode: Option<String> = connection.query_row(
            "SELECT execution_mode FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
            params![scan,current], |r|r.get(0),
        ).optional().map_err(|_| "request_accounting_lineage_unavailable")?;
        if !mode.is_some_and(|mode| mode.trim().eq_ignore_ascii_case("resume")) {
            break;
        }
        let parent: Option<i64> = connection.query_row(
            "SELECT MAX(attempt_number) FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number<?2",
            params![scan,current], |r|r.get(0),
        ).map_err(|_| "request_accounting_lineage_unavailable")?;
        current = parent
            .filter(|p| *p >= 1 && *p < current)
            .ok_or("request_accounting_parent_missing")?;
    }
    Ok(result)
}

pub fn external_usage(
    connection: &Connection,
    scan: &str,
    attempts: &[i64],
    target: &str,
) -> Result<ExternalRequestUsage, String> {
    let mut usage = ExternalRequestUsage::default();
    let mut seen = std::collections::HashSet::new();
    for attempt in attempts {
        if !seen.insert(*attempt) {
            return Err("request_accounting_duplicate_attempt".into());
        }
        let mut statement = connection.prepare(
            "SELECT c.state,c.assignment_id,c.artifact_id,c.response_json,c.response_hash, \
             EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
               JOIN agent_runs root ON root.id=a.coordinator_run_id \
               WHERE a.id=c.assignment_id AND r.id=c.child_run_id AND r.assignment_id=a.id \
                 AND r.root_run_id=root.id AND root.role='coordinator' \
                 AND r.scan_id=c.scan_id AND r.attempt_number=c.attempt_number AND r.target_url=c.target_url \
                 AND root.scan_id=c.scan_id AND root.attempt_number=c.attempt_number AND root.target_url=c.target_url \
                 AND a.target_key=c.target_url AND a.role='external_surface' AND r.role=a.role \
                 AND a.lane='target_touching' AND r.lane=a.lane) \
             FROM agent_external_surface_captures c WHERE c.scan_id=?1 AND c.attempt_number=?2 AND c.target_url=?3"
        ).map_err(|_| "request_accounting_capture_unavailable")?;
        let rows = statement
            .query_map(params![scan, attempt, target], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, bool>(5)?,
                ))
            })
            .map_err(|_| "request_accounting_capture_unavailable")?;
        for row in rows {
            let (state, assignment, artifact, response, hash, bound) =
                row.map_err(|_| "request_accounting_capture_invalid")?;
            if !bound {
                return Err("request_accounting_capture_binding_invalid".into());
            }
            match state.as_str() {
                "claimed" if artifact.is_empty() && response == "{}" && hash.is_empty() => {
                    usage.unresolved += 1
                }
                "received" => {
                    let value: Value = serde_json::from_str(&response)
                        .map_err(|_| "request_accounting_receipt_invalid")?;
                    let mut url = reqwest::Url::parse(target)
                        .map_err(|_| "request_accounting_target_invalid")?;
                    url.set_fragment(None);
                    if artifact.is_empty()
                        || value["artifactId"] != artifact
                        || value["targetRequests"] != 1
                        || hash != super::store::stable_hash(&response)
                        || value["capture"]["requestId"] != format!("public-surface:{assignment}")
                        || value["capture"]["url"] != url.as_str()
                        || value["capture"]["method"] != "GET"
                        || value["capture"]["identity"] != "anonymous"
                    {
                        return Err("request_accounting_receipt_invalid".into());
                    }
                    usage.received += 1;
                }
                _ => return Err("request_accounting_capture_invalid".into()),
            }
        }
    }
    Ok(usage)
}

/// A durable artifact reference records a response; an empty reference remains
/// uncertain (including send/read/persistence failures). Evidence-file loss must
/// never refund traffic. This is accounting, not finding verification.
pub fn authorization_usage(
    connection: &Connection,
    scan: &str,
    attempts: &[i64],
    target: &str,
) -> Result<ExternalRequestUsage, String> {
    let mut usage = ExternalRequestUsage::default();
    let mut seen = std::collections::HashSet::new();
    for attempt in attempts {
        if !seen.insert(*attempt) {
            return Err("request_accounting_duplicate_attempt".into());
        }
        let mut statement = connection.prepare(
            "SELECT c.artifact_id, EXISTS(SELECT 1 FROM agent_assignments a \
             JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_runs root ON root.id=a.coordinator_run_id \
             JOIN agent_authorization_controls k ON k.scan_id=c.scan_id AND k.attempt_number=c.attempt_number \
               AND k.target_url=c.target_url AND k.contract_key=c.contract_key AND k.method='GET' \
             WHERE a.id=c.assignment_id AND r.id=c.child_run_id AND r.assignment_id=a.id \
               AND r.root_run_id=root.id AND root.role='coordinator' \
               AND r.scan_id=c.scan_id AND r.attempt_number=c.attempt_number AND r.target_url=c.target_url \
               AND root.scan_id=c.scan_id AND root.attempt_number=c.attempt_number AND root.target_url=c.target_url \
               AND a.target_key=c.target_url AND a.role='authorization' AND r.role=a.role \
               AND a.lane='target_touching' AND r.lane=a.lane AND c.side IN ('owner','cross','tester')) \
             FROM agent_authorization_probe_claims c WHERE c.scan_id=?1 AND c.attempt_number=?2 AND c.target_url=?3"
        ).map_err(|_| "request_accounting_authorization_unavailable")?;
        let rows = statement
            .query_map(params![scan, attempt, target], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
            })
            .map_err(|_| "request_accounting_authorization_unavailable")?;
        for row in rows {
            let (artifact, bound) = row.map_err(|_| "request_accounting_authorization_invalid")?;
            if !bound {
                return Err("request_accounting_authorization_binding_invalid".into());
            }
            if artifact.is_empty() {
                usage.unresolved += 1;
            } else {
                usage.received += 1;
            }
        }
    }
    Ok(usage)
}

/// Specialist snapshots own only their own received target request, not the
/// coordinator total. A missing receipt never turns into a fabricated request.
pub fn child_received(connection: &Connection, run: &str) -> Result<i64, String> {
    let (scan, attempt, target, role): (String, i64, String, String) = connection
        .query_row(
            "SELECT scan_id,attempt_number,target_url,role FROM agent_runs WHERE id=?1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| "request_accounting_run_missing")?;
    if role != "external_surface" {
        return Ok(0);
    }
    let own: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_external_surface_captures WHERE child_run_id=?1 AND state='received')",
        [run], |r|r.get(0),
    ).map_err(|_| "request_accounting_capture_unavailable")?;
    if !own {
        return Err("request_accounting_received_required".into());
    }
    let usage = external_usage(connection, &scan, &[attempt], &target)?;
    if usage.received != 1 || usage.unresolved != 0 {
        return Err("request_accounting_received_required".into());
    }
    Ok(1)
}
