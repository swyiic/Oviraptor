//! Active exposure probe + technology fingerprinting — the deterministic
//! identification + targeted-check layer (docs/AGENT_NEST_INTELLIGENT_FLOW_DESIGN.md
//! §10b, §12).
//!
//! Two deterministic, read-only capabilities, both driven by data (JSON) so new
//! products/checks are added without recompiling:
//!   1. Fingerprinting: match a target's root response (headers, cookies, body,
//!      characteristic paths) against a signal catalog to identify products
//!      (DedeCMS, WordPress, ...). A larger corpus is meant to be synced as a
//!      rule-pack; the shipped set is the vetted default.
//!   2. Exposure checks: GET a small, high-signal set of sensitive paths (git
//!      leak, .env, backups, and product-specific disclosure paths). A check runs
//!      when its `appliesTo` is empty, names "http", or names an identified
//!      product — so product-specific checks fire only on matching targets.
//!
//! A soft-404 baseline plus content matching keeps sites that answer 200 for
//! everything from producing false positives. Pure: the caller records the
//! identified products and hits as findings.

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

// ---------------------------------------------------------------- exposure checks

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

pub(crate) fn builtin_checks() -> Vec<ExposureCheck> {
    serde_json::from_str::<ExposureCatalog>(include_str!(
        "../../resources/rules/active_exposures.json"
    ))
    .map(|catalog| catalog.checks)
    .unwrap_or_default()
}

// ---------------------------------------------------------------- fingerprints

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FingerprintCatalog {
    #[serde(default)]
    fingerprints: Vec<Fingerprint>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fingerprint {
    id: String,
    name: String,
    #[serde(default)]
    header_regex: std::collections::HashMap<String, String>,
    #[serde(default)]
    set_cookie_regex: Vec<String>,
    #[serde(default)]
    body_regex: Vec<String>,
    #[serde(default)]
    path_signals: Vec<PathSignal>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathSignal {
    path: String,
    #[serde(default)]
    status: Vec<u16>,
    #[serde(default)]
    body_regex: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Identified {
    pub id: String,
    pub name: String,
}

fn fingerprint_catalog() -> Vec<Fingerprint> {
    serde_json::from_str::<FingerprintCatalog>(include_str!(
        "../../resources/rules/fingerprints.json"
    ))
    .map(|catalog| catalog.fingerprints)
    .unwrap_or_default()
}

// ---------------------------------------------------------------- HTTP

struct Probe {
    status: u16,
    headers: Vec<(String, String)>,
    set_cookie: Vec<String>,
    body: String,
}

const MAX_BODY: usize = 65536;
const MAX_EVIDENCE: usize = 160;

fn build_client(follow_redirects: bool) -> Option<reqwest::blocking::Client> {
    // Mirror the proven native-recon client: accept invalid certs (a scanner must
    // probe targets with broken/self-signed certs; the default-verifying rustls
    // build can also fail in-app) and present a browser-like UA. Fingerprinting
    // follows redirects so the real homepage is matched (many sites 302 the root);
    // exposure path-checks do NOT follow, so a redirect-to-home is never mistaken
    // for a path existing.
    let redirect = if follow_redirects {
        reqwest::redirect::Policy::limited(5)
    } else {
        reqwest::redirect::Policy::none()
    };
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(redirect)
        .danger_accept_invalid_certs(true)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36")
        .build()
        .ok()
}

fn fetch(client: &reqwest::blocking::Client, base: &str, path: &str) -> Option<Probe> {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let response = client.get(&url).send().ok()?;
    let status = response.status().as_u16();
    let set_cookie = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let bytes = response.bytes().ok()?;
    let slice = &bytes[..bytes.len().min(MAX_BODY)];
    Some(Probe {
        status,
        headers,
        set_cookie,
        body: String::from_utf8_lossy(slice).to_string(),
    })
}

fn regex_matches(pattern: &str, haystack: &str) -> bool {
    regex::Regex::new(pattern).is_ok_and(|regex| regex.is_match(haystack))
}

// ---------------------------------------------------------------- identification

/// Identify products running on `base_url` from its root response (and a few
/// characteristic-path probes). Signal-based: any matching signal identifies.
pub(crate) fn identify_products(base_url: &str) -> Vec<Identified> {
    let Some(client) = build_client(true) else {
        return Vec::new();
    };
    let Some(root) = fetch(&client, base_url, "/") else {
        return Vec::new();
    };
    let mut identified = Vec::new();
    for fingerprint in fingerprint_catalog() {
        if fingerprint_matches(&client, base_url, &fingerprint, &root) {
            identified.push(Identified {
                id: fingerprint.id,
                name: fingerprint.name,
            });
        }
    }
    identified
}

fn fingerprint_matches(
    client: &reqwest::blocking::Client,
    base_url: &str,
    fingerprint: &Fingerprint,
    root: &Probe,
) -> bool {
    for (name, pattern) in &fingerprint.header_regex {
        let lower = name.to_ascii_lowercase();
        if root
            .headers
            .iter()
            .any(|(header, value)| *header == lower && regex_matches(pattern, value))
        {
            return true;
        }
    }
    if fingerprint
        .set_cookie_regex
        .iter()
        .any(|pattern| root.set_cookie.iter().any(|cookie| regex_matches(pattern, cookie)))
    {
        return true;
    }
    if fingerprint
        .body_regex
        .iter()
        .any(|pattern| regex_matches(pattern, &root.body))
    {
        return true;
    }
    for signal in &fingerprint.path_signals {
        if let Some(probe) = fetch(client, base_url, &signal.path) {
            let status_ok = signal.status.is_empty() || signal.status.contains(&probe.status);
            let body_ok = signal
                .body_regex
                .as_ref()
                .is_none_or(|pattern| regex_matches(pattern, &probe.body));
            if status_ok && body_ok && !probe.body.to_ascii_lowercase().contains("<html") {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------- exposure probe

/// Probe `base_url` for the applicable checks. A check runs when its `appliesTo`
/// is empty, names "http", or names one of `products` (so product-specific checks
/// fire only on matching targets). Never panics; a dead host yields no hits.
pub(crate) fn probe_exposures(
    base_url: &str,
    checks: &[ExposureCheck],
    products: &[String],
) -> Vec<ExposureHit> {
    let Some(client) = build_client(false) else {
        return Vec::new();
    };
    // Soft-404 baseline: a path that cannot exist. A site that answers 200 for
    // everything reveals its generic page here, so a genuine hit must differ.
    let baseline = fetch(&client, base_url, "/oviraptor-nonexistent-9f3a2b7c1e5d");
    let mut hits = Vec::new();
    for check in checks {
        let applicable = check.applies_to.is_empty()
            || check
                .applies_to
                .iter()
                .any(|tag| tag == "http" || products.iter().any(|product| product == tag));
        if !applicable {
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
        if !regex_matches(pattern, &probe.body) {
            return false;
        }
    }
    // A bare status match is never a hit: require a positive content signal.
    rule.body_regex.is_some() || !rule.body_contains.is_empty()
}

// ---------------------------------------------------------------- findings

/// Record exposure hits as findings (independent of the model/value gate).
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

/// Record identified products as info findings so the identification is visible
/// and so later phases can target it.
pub(crate) fn record_identifications(
    db_path: &Path,
    scan_id: &str,
    target_url: &str,
    products: &[Identified],
) -> usize {
    let Ok(connection) = crate::db::open(db_path) else {
        return 0;
    };
    let mut recorded = 0;
    for product in products {
        let record = serde_json::json!({
            "source": "fingerprint",
            "productId": product.id,
            "product": product.name,
        })
        .to_string();
        let written = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'fingerprint','tech_identified',?3,?4,'info',?5) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            rusqlite::params![
                scan_id,
                target_url,
                product.id,
                format!("技术识别：{}", product.name),
                record
            ],
        );
        if written.is_ok() {
            recorded += 1;
        }
    }
    recorded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    // A site that leaks .git/HEAD and .env, serves a DedeCMS-flavoured root, and
    // answers 200 + an HTML page for every other path (the soft-404 case).
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
                let (extra, body): (&str, &[u8]) = match path {
                    "/" => ("Set-Cookie: DedeLoginTime=1; path=/\r\n", b"<html><body>Power by DedeCMS. <a href=/plus/list.php>list</a></body></html>"),
                    "/.git/HEAD" => ("", b"ref: refs/heads/main\n"),
                    "/.env" => ("", b"SECRET_KEY=topsecret\n"),
                    "/data/admin/ver.txt" => ("", b"20230512\n"),
                    _ => ("", b"<html><body>Not Found</body></html>"),
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
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
    fn identifies_dedecms_from_cookie_and_body() {
        let port = start_site();
        let base = format!("http://127.0.0.1:{port}");
        let ids: Vec<String> = identify_products(&base)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert!(ids.contains(&"dedecms".to_string()), "dedecms not identified: {ids:?}");
        assert!(!ids.contains(&"wordpress".to_string()), "false WordPress id");
    }

    #[test]
    fn detects_generic_and_product_gated_exposures() {
        let port = start_site();
        let base = format!("http://127.0.0.1:{port}");
        // Without identifying dedecms, the product-gated check must NOT run.
        let generic = probe_exposures(&base, &builtin_checks(), &[]);
        let generic_ids: Vec<&str> = generic.iter().map(|h| h.id.as_str()).collect();
        assert!(generic_ids.contains(&"git-head"), "git leak missed: {generic_ids:?}");
        assert!(generic_ids.contains(&"dotenv"), ".env leak missed");
        assert!(!generic_ids.contains(&"dedecms-version"), "product check ran without identification");
        // With dedecms identified, the product-gated check runs.
        let targeted = probe_exposures(&base, &builtin_checks(), &["dedecms".to_string()]);
        let targeted_ids: Vec<&str> = targeted.iter().map(|h| h.id.as_str()).collect();
        assert!(targeted_ids.contains(&"dedecms-version"), "product check missed: {targeted_ids:?}");
    }

    #[test]
    fn builtin_catalogs_parse() {
        assert!(builtin_checks().len() >= 6);
        assert!(fingerprint_catalog().iter().any(|f| f.id == "dedecms"));
    }
}
