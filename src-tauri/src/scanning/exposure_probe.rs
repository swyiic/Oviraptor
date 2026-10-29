//! Active exposure probe — Tier 1 deterministic checks
//! (docs/AGENT_NEST_INTELLIGENT_FLOW_DESIGN.md §10b).
//!
//! Read-only GETs of a small, high-signal set of sensitive paths (git leak, .env,
//! backups, ...). This is the missing "active" counterpart to the passive content
//! rules in resources/rules: the browser recon only sees what a page requests of
//! itself, so an unlinked `.git/` leak is invisible to it and must be probed for.
//!
//! A soft-404 baseline plus content matching keeps sites that answer 200 for every
//! path from producing false positives. Pure: (base_url, checks) -> hits; the
//! caller records hits as findings.

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExposureCatalog {
    #[serde(default)]
    #[allow(dead_code)]
    pub schema_version: u32,
    pub checks: Vec<ExposureCheck>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExposureCheck {
    pub id: String,
    pub title: String,
    pub severity: String,
    #[serde(default)]
    pub applies_to: Vec<String>,
    pub path: String,
    #[serde(default, rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default)]
    pub negate_body_contains: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MatchRule {
    #[serde(default)]
    pub status: Vec<u16>,
    #[serde(default)]
    pub body_contains: Vec<String>,
    #[serde(default)]
    pub body_regex: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExposureHit {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub url: String,
    pub evidence: String,
}

/// The built-in, shipped check set. A malformed resource yields no checks (the
/// probe simply finds nothing) rather than failing a scan.
pub(crate) fn builtin_checks() -> Vec<ExposureCheck> {
    serde_json::from_str::<ExposureCatalog>(include_str!(
        "../../resources/rules/active_exposures.json"
    ))
    .map(|catalog| catalog.checks)
    .unwrap_or_default()
}

struct Probe {
    status: u16,
    body: String,
}

const MAX_BODY: usize = 65536;
const MAX_EVIDENCE: usize = 160;

fn fetch(client: &reqwest::blocking::Client, base: &str, path: &str) -> Option<Probe> {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let response = client.get(&url).send().ok()?;
    let status = response.status().as_u16();
    let bytes = response.bytes().ok()?;
    let slice = &bytes[..bytes.len().min(MAX_BODY)];
    Some(Probe {
        status,
        body: String::from_utf8_lossy(slice).to_string(),
    })
}

/// Probe `base_url` for the applicable checks. A check runs when its `appliesTo`
/// is empty or names "http" (fingerprint-scoped checks are a later refinement).
/// Never panics; a dead host or a build error simply yields no hits.
pub(crate) fn probe_exposures(base_url: &str, checks: &[ExposureCheck]) -> Vec<ExposureHit> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(false)
        .user_agent("Oviraptor-ExposureProbe/1")
        .build()
    {
        Ok(client) => client,
        Err(_) => return Vec::new(),
    };
    // Soft-404 baseline: a path that cannot exist. A site that answers 200 for
    // everything reveals its generic page here, so a genuine hit must differ.
    let baseline = fetch(&client, base_url, "/oviraptor-nonexistent-9f3a2b7c1e5d");
    let mut hits = Vec::new();
    for check in checks {
        if !(check.applies_to.is_empty() || check.applies_to.iter().any(|tag| tag == "http")) {
            continue;
        }
        let Some(probe) = fetch(&client, base_url, &check.path) else {
            continue;
        };
        if let Some(base) = &baseline {
            if base.status == probe.status && base.body == probe.body {
                continue; // identical to the not-found baseline → soft 404
            }
        }
        if matches_rule(&check.match_rule, &check.negate_body_contains, &probe) {
            hits.push(ExposureHit {
                id: check.id.clone(),
                title: check.title.clone(),
                severity: check.severity.clone(),
                url: format!("{}{}", base_url.trim_end_matches('/'), check.path),
                evidence: probe.body.chars().take(MAX_EVIDENCE).collect(),
            });
        }
    }
    hits
}

/// Record probe hits as findings so they surface in results directly, without
/// depending on the model/value gate. Best-effort: a DB error drops the write
/// rather than failing the scan. Returns how many rows were recorded.
pub(crate) fn record_exposures(
    db_path: &Path,
    scan_id: &str,
    target_url: &str,
    hits: &[ExposureHit],
) -> usize {
    let Ok(connection) = crate::db::open(db_path) else {
        return 0;
    };
    let mut recorded = 0;
    for hit in hits {
        let record = serde_json::json!({
            "source": "exposure_probe",
            "checkId": hit.id,
            "url": hit.url,
            "severity": hit.severity,
            "evidence": hit.evidence,
        })
        .to_string();
        let written = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'exposure','vulnerability',?3,?4,?5,?6) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            rusqlite::params![scan_id, target_url, hit.id, hit.title, hit.severity, record],
        );
        if written.is_ok() {
            recorded += 1;
        }
    }
    recorded
}

fn matches_rule(rule: &MatchRule, negate: &[String], probe: &Probe) -> bool {
    if !rule.status.is_empty() && !rule.status.contains(&probe.status) {
        return false;
    }
    if negate.iter().any(|needle| probe.body.contains(needle.as_str())) {
        return false;
    }
    if !rule
        .body_contains
        .iter()
        .all(|needle| probe.body.contains(needle.as_str()))
    {
        return false;
    }
    if let Some(pattern) = &rule.body_regex {
        match regex::Regex::new(pattern) {
            Ok(regex) if regex.is_match(&probe.body) => {}
            _ => return false,
        }
    }
    // A bare status match is never a hit: require at least one positive content
    // signal so an everything-returns-200 site cannot trip a check.
    rule.body_regex.is_some() || !rule.body_contains.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    // A site that leaks .git/HEAD and .env but answers 200 + an HTML page for every
    // other path (the common soft-404 case that must not produce false positives).
    fn start_site() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body: &[u8] = match path {
                    "/.git/HEAD" => b"ref: refs/heads/main\n",
                    "/.env" => b"SECRET_KEY=topsecret\nDB_PASSWORD=hunter2\n",
                    _ => b"<html><body>Not Found</body></html>",
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
                let _ = stream.flush();
            }
        });
        port
    }

    #[test]
    fn detects_git_and_env_but_not_the_soft_404_paths() {
        let port = start_site();
        let base = format!("http://127.0.0.1:{port}");
        let hits = probe_exposures(&base, &builtin_checks());
        let ids: Vec<&str> = hits.iter().map(|hit| hit.id.as_str()).collect();
        assert!(ids.contains(&"git-head"), "git leak missed: {ids:?}");
        assert!(ids.contains(&"dotenv"), ".env leak missed: {ids:?}");
        // Paths that return the generic 200 HTML page must not be false positives.
        assert!(!ids.contains(&"git-config"), "false positive on git-config");
        assert!(!ids.contains(&"security-txt"), "false positive on security-txt");
        assert!(!ids.contains(&"backup-sql"), "false positive on backup-sql");
    }

    #[test]
    fn builtin_catalog_parses() {
        let checks = builtin_checks();
        assert!(checks.len() >= 6, "expected the shipped checks to parse");
        assert!(checks.iter().any(|check| check.id == "git-head"));
    }
}
