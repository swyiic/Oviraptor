//! Field-bound ownership observations. Provider association lists are not SANs.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OwnershipObservation {
    pub(super) host: String,
    pub(super) organization: String,
    pub(super) source: String,
    pub(super) certificate_sha256: Option<String>,
    pub(super) wildcard: bool,
}

fn objects(v: &Value) -> Vec<std::borrow::Cow<'_, Value>> {
    use std::borrow::Cow;
    let mut result = vec![Cow::Borrowed(v)];
    for key in ["cert", "certificate", "ssl_certificate"] {
        let node = &v[key];
        if node.is_object() {
            result.push(Cow::Borrowed(node));
        } else if let Some(raw) = node.as_str().filter(|s| s.len() <= 262144) {
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                if parsed.is_object() {
                    result.push(Cow::Owned(parsed));
                }
            }
        }
    }
    result
}

fn leaves(v: &Value) -> Vec<crate::certificates::Certificate> {
    // A current TLS leaf supersedes historical provider certificate fields.
    if let Some(leaf) = v["probe_cert_der_base64"]
        .as_str()
        .and_then(crate::certificates::from_text)
    {
        return vec![leaf];
    }
    let mut result = Vec::new();
    for node in objects(v) {
        for key in [
            "certificate_der_base64", "certificate_pem", "ssl_certificate",
            "pem", "der_base64",
        ] {
            if let Some(leaf) = node[key].as_str().and_then(crate::certificates::from_text) {
                result.push(leaf);
            }
        }
    }
    result
}

/// Organizations belong only to the DNS names in their own leaf/metadata
/// object. `cert.domain` is a provider association list, never identity proof.
pub(super) fn ownership_observations(v: &Value, endpoint: &str) -> Vec<OwnershipObservation> {
    let mut result = BTreeSet::new();
    let leafs = leaves(v);
    if !leafs.is_empty() {
        for leaf in leafs {
            for name in leaf.server_names() {
                let Some(h) = host(name) else { continue; };
                for organization in &leaf.organizations {
                    if organization.trim().is_empty() { continue; }
                    result.insert(OwnershipObservation {
                        host: h.clone(), organization: clipped(organization),
                        source: "leaf_subject".into(),
                        certificate_sha256: Some(leaf.sha256.clone()),
                        wildcard: name.starts_with("*."),
                    });
                }
            }
        }
    } else {
        for node in objects(v) {
            let organizations = [
                &node["cert.subject.org"], &node["cert.subject.o"],
                &node["subject"]["O"], &node["subject"]["organizationName"],
            ].into_iter().filter_map(Value::as_str)
                .map(clipped).filter(|s| !s.is_empty()).collect::<BTreeSet<_>>();
            for (source, fields) in [
                ("provider_subject_cn", vec![&node["cert.subject.cn"], &node["subject"]["CN"], &node["subject"]["commonName"], &node["subject"]["cn"]]),
                ("provider_subject_san", vec![&node["dns_names"], &node["san"]["dns_names"], &node["subject_alt_names"]]),
            ] {
                for value in fields {
                    for (h, wildcard) in field_hosts(value) {
                        for organization in &organizations {
                            result.insert(OwnershipObservation {
                                host: h.clone(), organization: organization.clone(),
                                source: source.into(), certificate_sha256: None, wildcard,
                            });
                        }
                    }
                }
            }
        }
    }
    // A site registration describes the observed endpoint, not co-located
    // certificates, CNAME targets, or every name in an association list.
    if let Some(h) = host(endpoint) {
        for organization in [
            &v["icp.name"], &v["icp_name"], &v["icp_company"], &v["icp"]["name"],
        ].into_iter().filter_map(Value::as_str).map(clipped).filter(|s| !s.is_empty()) {
            result.insert(OwnershipObservation {
                host: h.clone(), organization, source: "endpoint_icp".into(),
                certificate_sha256: None, wildcard: false,
            });
        }
    }
    result.into_iter().collect()
}

pub(super) fn observations_for_names(v: &Value, endpoint: &str, names: &BTreeSet<String>) -> Vec<OwnershipObservation> {
    ownership_observations(v, endpoint).into_iter()
        .filter(|observation| observation_matches(observation, names)).collect()
}

fn observation_matches(observation: &OwnershipObservation, names: &BTreeSet<String>) -> bool {
    names.contains(&observation.host)
        || observation.wildcard && names.iter().any(|name| in_scope(name, &observation.host))
}

pub(super) fn apply_observations(e: &mut Value, observations: Vec<Value>) {
    let current_observations = json!(observations.iter().take(128).collect::<Vec<_>>());
    if e["domainEvidence"] != current_observations {
        if let Some(object) = e.as_object_mut() {
            object.remove("currentAdvice");
            object.remove("currentReason");
        }
    }
    let organizations = |source: &str| observations.iter()
        .filter(|o| o["source"].as_str() == Some(source))
        .filter_map(|o| o["organization"].as_str()).map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut subjects = organizations("leaf_subject");
    subjects.extend(organizations("public_ct_subject"));
    e["subjectOrganizations"] = json!(subjects);
    e["icpOrganizations"] = json!(organizations("endpoint_icp"));
    let mut hints = organizations("provider_subject_cn");
    hints.extend(organizations("provider_subject_san"));
    e["organizationHints"] = json!(hints);
    e["ownershipEvidenceVersion"] = json!(2);
    e["ownershipEvidenceTruncated"] = json!(observations.len() > 128);
    e["domainEvidence"] = current_observations;
    let supported = e["domainEvidence"].as_array().into_iter().flatten()
        .filter_map(|o| o["host"].as_str()).collect::<BTreeSet<_>>();
    e["weakAssociationDomains"] = json!(e["domains"].as_array().into_iter().flatten()
        .filter_map(Value::as_str).filter(|name| !supported.contains(name))
        .take(64).collect::<Vec<_>>());
}

/// Rebuild derived provenance from the current original records, including for
/// legacy review rows. No paid response or human decision is edited/replayed.
/// One joined read and one parse per asset keep this linear on large projects.
pub(in crate::asset_discovery) fn current_projections(
    c: &Connection, project: i64, run: &str, items: &[(String, Value)],
) -> Result<HashMap<String, Value>, String> {
    let names = items.iter().map(|(key, e)| (key.clone(), e["domains"].as_array()
        .into_iter().flatten().filter_map(Value::as_str).map(str::to_owned)
        .collect::<BTreeSet<_>>())).collect::<HashMap<_, _>>();
    let keys = json!(names.keys().collect::<Vec<_>>()).to_string();
    let mut q = c.prepare(&format!("SELECT refs.item_key,a.id,a.host,a.link,a.extra_json FROM json_each(?3) keys CROSS JOIN asset_seed_review_refs refs JOIN assets a ON a.id=refs.asset_id JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?2 JOIN asset_seed_reviews review ON review.id=refs.run_id WHERE refs.run_id=?1 AND refs.item_key=keys.value AND (review.include_isolated=1 OR NOT {ISOLATED})")).map_err(err)?;
    let mut cached = HashMap::<i64, Vec<OwnershipObservation>>::new();
    let mut bound = HashMap::<String, BTreeMap<OwnershipObservation, BTreeSet<i64>>>::new();
    for row in q.query_map(params![run, project, keys], |r| Ok((
        r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?,
        r.get::<_, String>(3)?, r.get::<_, String>(4)?,
    ))).map_err(err)? {
        let (key, asset, h, l, raw) = row.map_err(err)?;
        let observations = cached.entry(asset).or_insert_with(|| {
            let v = serde_json::from_str(&raw).unwrap_or(Value::Null);
            ownership_observations(&v, if l.is_empty() { &h } else { &l })
        });
        let Some(ns) = names.get(&key) else { continue; };
        for observation in observations.iter().filter(|o| observation_matches(o, ns)) {
            bound.entry(key.clone()).or_default().entry(observation.clone()).or_default().insert(asset);
        }
    }
    let all_names = names.values().flat_map(|v| v.iter().cloned()).collect::<BTreeSet<_>>();
    let public_observations = super::super::passive_evidence::observations(c, project, run, &all_names)?;
    let mut result = HashMap::new();
    for (key, original) in items {
        let mut evidence = original.clone();
        let mut observations: Vec<Value> = bound.remove(key).unwrap_or_default().into_iter().map(|(observation, assets)| {
            let mut value = serde_json::to_value(observation).expect("serializable ownership observation");
            value["assetIds"] = json!(assets.into_iter().take(5).collect::<Vec<_>>());
            value
        }).collect();
        if let Some(candidate_names) = names.get(key) {
            observations.extend(public_observations.iter().filter(|o| o["candidate"].as_str().is_some_and(|n| candidate_names.contains(n))).cloned());
        }
        apply_observations(&mut evidence, observations);
        result.insert(key.clone(), evidence);
    }
    Ok(result)
}

