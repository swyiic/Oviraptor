//! Read local observations before any remote lookup. Never reads CAS/files or credentials.
use super::*;
#[derive(Default)]
pub(crate) struct CertificateFacts {
    pub(super) hashes: BTreeSet<String>,
    pub(super) names: BTreeSet<(String, bool)>,
    pub(super) raw: bool,
}
impl CertificateFacts {
    pub(crate) fn present(&self) -> bool {
        self.raw || !self.hashes.is_empty() || !self.names.is_empty()
    }
}
pub(crate) fn certificate_facts(v: &Value) -> CertificateFacts {
    let mut f = CertificateFacts::default();
    fn fields(v: &Value, f: &mut CertificateFacts, certificate_object: bool) {
        for key in [
            "cert_sha256",
            "cert.sha256",
            "cert.sha-256",
            "tls_sha256",
            "probe_cert_sha256",
        ] {
            if let Some(h) = v[key].as_str().and_then(hunter_expansion::fingerprint) {
                f.hashes.insert(h);
            }
        }
        if certificate_object {
            if let Some(h) = v["sha256"].as_str().and_then(hunter_expansion::fingerprint) { f.hashes.insert(h); }
        }
        for name in [
            &v["cert.domain"],
            &v["cert.subject.cn"],
            &v["dns_names"],
            &v["subject"]["CN"],
            &v["subject"]["commonName"],
            &v["subject"]["cn"],
            &v["san"]["dns_names"],
            &v["subject_alt_names"],
            &v["probe_cert_dns_names_json"],
        ] {
            f.names.extend(field_hosts(name));
        }
    }
    fn raw_certificate(s: &str, f: &mut CertificateFacts) {
        if let Some(cert) = crate::certificates::from_text(s) {
            f.hashes.insert(cert.sha256);
            for name in cert.dns_names { f.names.extend(field_hosts(&json!(name))); }
        }
    }
    fields(v, &mut f, false);
    for key in ["probe_cert_der_base64", "certificate_der_base64", "certificate_pem"] {
        if let Some(s) = v[key].as_str() { raw_certificate(s, &mut f); }
    }
    for key in ["ssl_certificate", "cert", "certificate"] {
        let raw = &v[key];
        if raw.as_object().is_some_and(|o| !o.is_empty()) {
            f.raw = true;
            fields(raw, &mut f, true);
            for key in ["pem", "der_base64", "certificate_pem", "certificate_der_base64"] {
                if let Some(s) = raw[key].as_str() { raw_certificate(s, &mut f); }
            }
        }
        if let Some(s) = raw.as_str().filter(|s| !s.trim().is_empty()) {
            f.raw = true;
            if s.len() <= 262144 {
                if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                    if parsed.is_object() {
                        fields(&parsed, &mut f, true);
                        for key in ["pem", "der_base64", "certificate_pem", "certificate_der_base64"] {
                            if let Some(s) = parsed[key].as_str() { raw_certificate(s, &mut f); }
                        }
                    }
                }
                raw_certificate(s, &mut f);
            }
        }
    }
    f
}
pub(super) fn ingest_certificate_metadata(
    c: &Connection,
    p: i64,
    reference: &str,
    v: &Value,
) -> Result<usize, String> {
    let f = certificate_facts(v);
    let mut added = 0;
    for h in f.hashes {
        added += insert_seed(c, p, &h, "certificate_sha256", reference, false)?;
    }
    for (h, w) in f.names {
        added += insert_seed(c, p, &h, "certificate_domain", reference, w)?;
    }
    Ok(added)
}
fn exists_table(c: &Connection, name: &str) -> Result<bool, String> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1 AND type='table')",
        [name],
        |r| r.get(0),
    )
    .map_err(err)
}
pub(super) fn lookup_on(c: &Connection, p: i64, input: &str) -> Result<Value, String> {
    project(c, p)?;
    if input.len() > 2048 {
        return Err("请输入单个域名或证书 SHA-256".into());
    }
    let hash = hunter_expansion::fingerprint(input);
    let target = hash
        .clone()
        .or_else(|| host(input))
        .ok_or("请输入有效域名、域名 URL 或 64 位证书 SHA-256")?;
    let parent = if hash.is_none() {
        target.split_once('.').map(|(_, p)| p).unwrap_or("")
    } else {
        ""
    };
    let mut references = String::new();
    if exists(c)? {
        let condition = if hash.is_some() {
            "host=?2 AND source='certificate_sha256' AND wildcard=0"
        } else {
            "((host=?2 AND wildcard=0) OR (host=?3 AND wildcard=1)) AND source IN ('observed_host','certificate_domain','certificate_cn')"
        };
        references=format!(" UNION SELECT CAST(substr(source_ref,7) AS INTEGER) FROM asset_discovery_seeds WHERE project_id=?1 AND {condition} AND source_ref LIKE 'asset:%' AND length(source_ref)>6 AND substr(source_ref,7) NOT GLOB '*[^0-9]*'");
        if exists_table(c, "asset_discovery_seed_evidence")? {
            references.push_str(&format!(" UNION SELECT CAST(substr(source_ref,7) AS INTEGER) FROM asset_discovery_seed_evidence WHERE project_id=?1 AND {condition} AND source_ref LIKE 'asset:%' AND length(source_ref)>6 AND substr(source_ref,7) NOT GLOB '*[^0-9]*'"));
        }
    }
    let endpoint = if hash.is_none() {
        "(lower(a.domain)=?2 OR lower(a.host) IN (?2,'http://'||?2,'https://'||?2,'http://'||?2||'/','https://'||?2||'/') OR lower(a.link) IN (?2,'http://'||?2,'https://'||?2,'http://'||?2||'/','https://'||?2||'/') OR lower(a.host) LIKE ?2||':%' OR lower(a.host) LIKE 'http://'||?2||'/%' OR lower(a.host) LIKE 'https://'||?2||'/%' OR lower(a.host) LIKE 'http://'||?2||':%' OR lower(a.host) LIKE 'https://'||?2||':%' OR lower(a.link) LIKE 'http://'||?2||'/%' OR lower(a.link) LIKE 'https://'||?2||'/%' OR lower(a.link) LIKE 'http://'||?2||':%' OR lower(a.link) LIKE 'https://'||?2||':%')"
    } else {
        "0=1"
    };
    // ?3 remains bound in the empty-index/hash case as well.
    let cte=format!("WITH ids AS (SELECT a.id FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.is_deleted=0 AND {endpoint} AND ?3 IS NOT NULL {references}), valid AS (SELECT DISTINCT a.id,a.host,a.link,a.domain,a.extra_json FROM ids JOIN assets a ON a.id=ids.id JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?1 AND pa.is_deleted=0)");
    let count: i64 = c
        .query_row(
            &format!("{cte} SELECT count(*) FROM valid"),
            params![p, target, parent],
            |r| r.get(0),
        )
        .map_err(err)?;
    let mut stmt = c
        .prepare(&format!(
            "{cte} SELECT id,host,link,domain,extra_json FROM valid ORDER BY id DESC LIMIT 200"
        ))
        .map_err(err)?;
    let mut assets = vec![];
    let mut certificates = std::collections::BTreeMap::<String, Value>::new();
    let mut raw_certificates = 0;
    let mut certificate_matches = 0;
    let mut endpoint_matches = 0;
    for row in stmt
        .query_map(params![p, target, parent], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(err)?
    {
        let (id, h, l, d, e) = row.map_err(err)?;
        let v = serde_json::from_str::<Value>(&e).unwrap_or(json!({}));
        let facts = certificate_facts(&v);
        let endpoint = hash.is_none()
            && [h.as_str(), l.as_str(), d.as_str()]
                .into_iter()
                .any(|s| host(s).as_deref() == Some(target.as_str()));
        let exact = facts.names.contains(&(target.clone(), false));
        let wildcard = hash.is_none()
            && !parent.is_empty()
            && facts.names.contains(&(parent.to_string(), true));
        let matching_hash = hash.as_ref().is_some_and(|h| facts.hashes.contains(h));
        let kind = if endpoint {
            "endpoint"
        } else if matching_hash {
            "fingerprint"
        } else if exact {
            "certificate_exact"
        } else if wildcard {
            "certificate_wildcard"
        } else {
            "saved_lead"
        };
        if endpoint {
            endpoint_matches += 1;
        }
        // A seed is historical evidence. Missing current metadata is never invented.
        if facts.present() && (endpoint || exact || wildcard || matching_hash) {
            certificate_matches += 1;
            if facts.raw {
                raw_certificates += 1;
            }
            for fingerprint in &facts.hashes {
                let entry=certificates.entry(fingerprint.clone()).or_insert_with(||json!({"fingerprint":fingerprint,"domains":facts.names.iter().map(|(d,w)|if *w{format!("*.{d}")}else{d.clone()}).collect::<Vec<_>>(),"assetIds":[],"eligible":hunter_expansion::evidence_anchor(c,p,fingerprint,"fingerprint").is_ok()}));
                entry["assetIds"].as_array_mut().unwrap().push(json!(id));
            }
        }
        if assets.len() < 10 {
            assets.push(json!({"id":id,"host":if l.is_empty(){h}else{l},"matchType":kind,"certificatePresent":facts.present()}));
        }
    }
    let certificate_count = certificates.len();
    let inspection_truncated = count > 200;
    Ok(
        json!({"target":target,"kind":if hash.is_some(){"fingerprint"}else{"domain"},"matchCount":count,"endpointMatches":endpoint_matches,"certificateMatches":certificate_matches,"rawCertificateMatches":raw_certificates,"certificateCount":certificate_count,"certificates":certificates.into_values().take(20).collect::<Vec<_>>(),"assets":assets,"inspectionTruncated":inspection_truncated,"certificatePreviewTruncated":certificate_count>20,"harvest":harvest::status_on(c,p)?,"networkRequested":false}),
    )
}
#[tauri::command]
pub async fn lookup_local_discovery(
    app: tauri::AppHandle,
    project_id: i64,
    target: String,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        lookup_on(
            &harvest::open_existing(&app.state::<AppState>().db_path, false)?,
            project_id,
            &target,
        )
    })
    .await
    .map_err(err)?
}
#[tauri::command]
pub async fn get_asset_discovery_page(
    app: tauri::AppHandle,
    project_id: i64,
    search: String,
    source: String,
    page: i64,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        view_filtered(
            &harvest::open_existing(&app.state::<AppState>().db_path, false)?,
            project_id,
            &search,
            &source,
            page,
            50,
        )
    })
    .await
    .map_err(err)?
}
#[cfg(test)]
#[path = "discovery_local_tests.rs"]
mod tests;
