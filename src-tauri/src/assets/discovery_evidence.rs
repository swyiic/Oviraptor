//! Paid-query admission uses the current certificate identity, never a co-located host.
use super::*;

fn identity_unit(
    v: &Value,
    hash_keys: &[&str],
    raw_keys: &[&str],
    probe: bool,
) -> Vec<(String, BTreeSet<String>)> {
    let raw: Vec<_> = raw_keys
        .iter()
        .filter_map(|k| v[*k].as_str())
        .filter_map(crate::certificates::from_text)
        .collect();
    if !raw.is_empty() {
        return raw
            .into_iter()
            .map(|c| {
                (
                    c.sha256,
                    c.dns_names.into_iter().filter_map(|n| host(&n)).collect(),
                )
            })
            .collect();
    }
    let names: BTreeSet<String> = if probe {
        field_hosts(&v["probe_cert_dns_names_json"])
            .into_iter()
            .map(|(h, _)| h)
            .collect()
    } else {
        let mut names = BTreeSet::new();
        for value in [
            &v["cert.domain"],
            &v["cert.subject.cn"],
            &v["dns_names"],
            &v["subject"]["CN"],
            &v["subject"]["commonName"],
            &v["subject"]["cn"],
            &v["san"]["dns_names"],
            &v["subject_alt_names"],
        ] {
            names.extend(field_hosts(value).into_iter().map(|(h, _)| h));
        }
        names
    };
    hash_keys
        .iter()
        .filter_map(|k| v[*k].as_str().and_then(hunter_expansion::fingerprint))
        .map(|h| (h, names.clone()))
        .collect()
}

pub(super) fn identities(v: &Value) -> Vec<(String, BTreeSet<String>)> {
    let mut result = identity_unit(v, &["probe_cert_sha256"], &["probe_cert_der_base64"], true);
    if !result.is_empty() {
        return result;
    } // Fresh TLS identity supersedes historical provider metadata.
    let provider_raw: Vec<_> = [
        "certificate_der_base64",
        "certificate_pem",
        "ssl_certificate",
    ]
    .iter()
    .filter_map(|k| v[*k].as_str())
    .filter_map(crate::certificates::from_text)
    .collect();
    if provider_raw.is_empty() {
        result.extend(identity_unit(
            v,
            &["cert_sha256", "cert.sha256", "cert.sha-256", "tls_sha256"],
            &[],
            false,
        ));
    } else {
        result.extend(provider_raw.into_iter().map(|c| {
            (
                c.sha256,
                c.dns_names.into_iter().filter_map(|n| host(&n)).collect(),
            )
        }));
    }
    for key in ["ssl_certificate", "cert", "certificate"] {
        let parsed;
        let unit = if v[key].is_object() {
            &v[key]
        } else if let Some(raw) = v[key].as_str() {
            if raw.len() > 262144 {
                continue;
            }
            parsed = serde_json::from_str::<Value>(raw).unwrap_or(Value::Null);
            &parsed
        } else {
            continue;
        };
        result.extend(identity_unit(
            unit,
            &["sha256", "cert_sha256", "cert.sha256"],
            &[
                "pem",
                "der_base64",
                "certificate_pem",
                "certificate_der_base64",
            ],
            false,
        ));
    }
    result
}

pub(super) fn certificate_anchor(
    v: &Value,
    fingerprint: &str,
    confirmed: &[String],
) -> Option<String> {
    identities(v).into_iter().find_map(|(hash, names)| {
        if hash != fingerprint
            || names.is_empty()
            || !names
                .iter()
                .all(|n| confirmed.iter().any(|root| in_scope(n, root)))
        {
            return None;
        }
        names.into_iter().next()
    })
}

pub(super) fn asset_certificate_anchor(
    c: &Connection,
    p: i64,
    id: i64,
    hash: &str,
    confirmed: &[String],
) -> Result<Option<String>, String> {
    let extra: Option<String> = c.query_row("SELECT a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND a.id=?2 AND pa.is_deleted=0", params![p,id], |r|r.get(0)).optional().map_err(err)?;
    let excluded = exclusions::rules(c, p)?;
    Ok(extra
        .and_then(|e| serde_json::from_str::<Value>(&e).ok())
        .and_then(|v| {
            if identities(&v).into_iter().any(|(h, names)| {
                h == hash && names.iter().any(|n| exclusions::matches(n, &excluded))
            }) {
                return None;
            }
            certificate_anchor(&v, hash, confirmed)
        }))
}

#[tauri::command]
pub async fn preview_hunter_evidence(
    state: tauri::State<'_, AppState>,
    input: hunter_expansion::ExpansionInput,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let c=harvest::open_existing(&path,false)?;
        project(&c,input.project_id)?;
        let decision=hunter_expansion::normalized(&input).and_then(|target|hunter_expansion::evidence_anchor(&c,input.project_id,&target,&input.kind));
        Ok(match decision {
            Ok(anchor)=>json!({"eligible":true,"anchor":anchor,"reason":"当前线索通过范围与证书关联检查；结果仍需核验归属","networkRequested":false}),
            Err(reason)=>json!({"eligible":false,"reason":reason,"networkRequested":false}),
        })
    }).await.map_err(err)?
}

/// Read-only batches include rule-isolated/trash rows so valid leads can be reviewed.
pub(super) fn quarantine_batch(c: &Connection, p: i64, cursor: i64) -> Result<Value, String> {
    project(c, p)?;
    if cursor < 0 {
        return Err("复核游标无效".into());
    }
    let confirmed = roots(c, p)?;
    if confirmed.is_empty() {
        return Err("请先保存当前项目确认的域名范围".into());
    }
    let mut s=c.prepare("SELECT a.id,a.host,a.link,a.extra_json,a.probe_outcome,a.content_category,pa.is_deleted,pa.decision,pa.note FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND a.id>?2 AND (pa.is_deleted=1 OR pa.decision='rejected' OR a.probe_outcome='blocked_content' OR a.content_category IN ('gambling','porn','custom_rule')) ORDER BY a.id LIMIT 500").map_err(err)?;
    let mut rows = s.query(params![p, cursor]).map_err(err)?;
    let mut items = vec![];
    let mut scanned = 0;
    let mut next = cursor;
    while let Some(row) = rows.next().map_err(err)? {
        scanned += 1;
        next = row.get::<_, i64>(0).map_err(err)?;
        let extra: String = row.get(3).map_err(err)?;
        let v: Value = serde_json::from_str(&extra).unwrap_or(Value::Null);
        let matches: Vec<_> = identities(&v)
            .into_iter()
            .filter_map(|(h, _)| {
                certificate_anchor(&v, &h, &confirmed)
                    .map(|anchor| json!({"fingerprint":h,"anchor":anchor}))
            })
            .collect();
        if !matches.is_empty() {
            items.push(json!({"assetId":next,"host":row.get::<_,String>(1).map_err(err)?,"link":row.get::<_,String>(2).map_err(err)?,"probeOutcome":row.get::<_,String>(4).map_err(err)?,"contentCategory":row.get::<_,String>(5).map_err(err)?,"isDeleted":row.get::<_,bool>(6).map_err(err)?,"decision":row.get::<_,String>(7).map_err(err)?,"note":row.get::<_,String>(8).map_err(err)?,"certificates":matches}));
        }
    }
    Ok(
        json!({"items":items,"scanned":scanned,"nextCursor":next,"hasMore":scanned==500,"roots":confirmed,"networkRequested":false,"dataModified":false}),
    )
}

#[tauri::command]
pub async fn review_quarantined_certificates(
    state: tauri::State<'_, AppState>,
    project_id: i64,
    cursor: i64,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        quarantine_batch(&harvest::open_existing(&path, false)?, project_id, cursor)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
#[path = "discovery_evidence_tests.rs"]
mod tests;
