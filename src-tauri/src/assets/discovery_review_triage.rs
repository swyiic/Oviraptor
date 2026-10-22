//! Local correlation and routing. Never grants scope, deletes assets or calls a provider.
use super::*;

fn strings(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

// Deliberately excludes observation counts and collection labels. A changed
// organization, domain, trust level or truncated record invalidates reuse.
pub(super) fn signature(v: &Value) -> String {
    let mut value = serde_json::Map::new();
    for key in [
        "domains",
        "subjectOrganizations",
        "organizationHints",
        "icpOrganizations",
        "sources",
        "correlatedOrganizations",
    ] {
        value.insert(
            key.into(),
            json!(strings(v, key).into_iter().collect::<BTreeSet<_>>()),
        );
    }
    for key in [
        "evidenceTruncated",
        "ambiguousIdentity",
        "correlationTruncated",
    ] {
        value.insert(key.into(), json!(v[key].as_bool().unwrap_or(false)));
    }
    json!(value).to_string()
}

// Re-evaluate only this workflow's versioned derived labels. The original paid
// response and operator decisions remain intact; no model request is replayed.
fn reconcile_receipts(c: &Connection, id: &str) -> Result<(), String> {
    let receipts: Vec<(String, String)> = {
        let mut q=c.prepare("SELECT request_keys_json,response_text FROM asset_seed_review_model_receipts r WHERE run_id=?1 AND EXISTS(SELECT 1 FROM json_each(r.request_keys_json) k JOIN asset_seed_review_items i ON i.run_id=r.run_id AND i.item_key=json_extract(k.value,'$.key') WHERE i.state='done' AND i.decision='pending' AND json_extract(i.evidence_json,'$.automation.version')=1)").map_err(err)?;
        let rows = q
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        rows
    };
    for (raw, response) in receipts {
        let mapping: Vec<Value> = serde_json::from_str(&raw).map_err(err)?;
        let aliases = mapping
            .iter()
            .filter_map(|v| Some((v["alias"].as_str()?.to_string(), Value::Null)))
            .collect::<Vec<_>>();
        let Ok(parsed) = parse_advice(&response, &aliases) else {
            continue;
        };
        for advice in parsed.valid {
            let Some(key) = mapping
                .iter()
                .find(|v| v["alias"] == advice.key)
                .and_then(|v| v["key"].as_str())
            else {
                continue;
            };
            let saved:Option<(String,String,String)>=c.query_row("SELECT evidence_json,advice,reason FROM asset_seed_review_items WHERE run_id=?1 AND item_key=?2 AND state='done' AND decision='pending' AND json_extract(evidence_json,'$.automation.version')=1",params![id,key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
            if let Some((raw, old_label, old_reason)) = saved {
                let evidence: Value = serde_json::from_str(&raw).map_err(err)?;
                let label = relevance_label(&advice, &evidence);
                let reason = advice_reason(&advice, label);
                if label != old_label || reason != old_reason {
                    c.execute("UPDATE asset_seed_review_items SET advice=?3,reason=?4,revision=revision+1 WHERE run_id=?1 AND item_key=?2 AND state='done' AND decision='pending'",params![id,key,label,reason]).map_err(err)?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn run(c: &mut Connection, p: i64, id: &str) -> Result<Value, String> {
    owned(c, p, id)?;
    let (cursor, max, organization): (i64, i64, String) = c
        .query_row(
            "SELECT cursor,max_id,organization FROM asset_seed_reviews WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(err)?;
    if cursor < max {
        return Err("请先完成本地证据整理".into());
    }
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let confirmed = roots(&tx, p)?;
    let blocked = exclusions::rules(&tx, p)?;
    reconcile_receipts(&tx, id)?;
    let rows: Vec<(String, String, Value, String, String)> = {
        let mut q = tx.prepare("SELECT item_key,kind,evidence_json,state,decision FROM asset_seed_review_items WHERE run_id=?1").map_err(err)?;
        let rows = q
            .query_map([id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    serde_json::from_str::<Value>(&r.get::<_, String>(2)?).unwrap_or(Value::Null),
                    r.get(3)?,
                    r.get(4)?,
                ))
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)?
    };
    // Exact observed names only: never inherit the platform certificate's
    // organization merely because a tenant shares its registrable domain.
    let mut by_name = HashMap::<String, Vec<(String, BTreeSet<String>)>>::new();
    for (key, kind, e, _, _) in &rows {
        if !matches!(kind.as_str(), "certificate" | "certificate_metadata") {
            continue;
        }
        let orgs = [
            "subjectOrganizations",
            "organizationHints",
            "icpOrganizations",
        ]
        .into_iter()
        .flat_map(|k| strings(e, k))
        .collect::<BTreeSet<_>>();
        if orgs.is_empty() || e["evidenceTruncated"] == true {
            continue;
        }
        for name in strings(e, "domains") {
            by_name
                .entry(name)
                .or_default()
                .push((key.clone(), orgs.clone()));
        }
    }
    let previous: HashMap<String, (Value, String, String)> = {
        let mut q = tx.prepare("SELECT i.item_key,i.evidence_json,i.decision,r.id FROM asset_seed_review_items i JOIN asset_seed_reviews r ON r.id=i.run_id WHERE r.project_id=?1 AND r.organization=?2 AND r.rowid<(SELECT rowid FROM asset_seed_reviews WHERE id=?3) ORDER BY r.rowid DESC").map_err(err)?;
        let mut out = HashMap::new();
        for row in q
            .query_map(params![p, organization, id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(err)?
        {
            let (key, raw, mut decision, mut run) = row.map_err(err)?;
            if out.contains_key(&key) {
                continue;
            }
            let evidence: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            if decision == "pending"
                && matches!(
                    evidence["automation"]["route"].as_str(),
                    Some("reused_accepted" | "reused_rejected")
                )
            {
                if let Some(source) = evidence["automation"]["sourceRun"].as_str() {
                    let original: Option<(String,String)> = tx.query_row("SELECT i.decision,i.evidence_json FROM asset_seed_review_items i JOIN asset_seed_reviews r ON r.id=i.run_id WHERE i.run_id=?1 AND i.item_key=?2 AND r.project_id=?3 AND r.organization=?4",params![source,key,p,organization],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
                    if let Some((d, raw)) = original {
                        if signature(&serde_json::from_str(&raw).unwrap_or(Value::Null))
                            == signature(&evidence)
                        {
                            decision = d;
                            run = source.into();
                        }
                    }
                }
            }
            out.insert(key, (evidence, decision, run));
        }
        out
    };
    let mut counts = BTreeMap::<String, usize>::new();
    for (key, kind, mut e, state, decision) in rows {
        // Original provider receipts and operator decisions are retained. A
        // newly confirmed family resolves its old missing-evidence projection.
        let old_e = e.clone();
        let names = strings(&e, "domains");
        let matches = names
            .iter()
            .filter(|name| confirmed.iter().any(|root| in_scope(name, root)))
            .cloned()
            .collect::<Vec<_>>();
        e["confirmedDomainMatches"] = json!(matches);
        let mut linked_orgs = BTreeSet::new();
        let mut links = BTreeSet::new();
        if kind == "domain_group" {
            for name in &names {
                for (source, orgs) in by_name.get(name).into_iter().flatten() {
                    links.insert(source.clone());
                    linked_orgs.extend(orgs.clone());
                }
            }
        }
        let correlated_truncated = links.len() > 16 || linked_orgs.len() > 16;
        e["correlatedOrganizations"] = json!(linked_orgs.into_iter().take(16).collect::<Vec<_>>());
        e["localEvidenceLinks"] = json!(links.into_iter().take(16).collect::<Vec<_>>());
        e["correlationTruncated"] = json!(correlated_truncated);
        // Current correlation is mutable even when a model response or human
        // decision is immutable. Never put an unknown request back on the paid
        // queue merely because its current local facts have improved.
        if decision == "pending" && state == "done" && !cross_domain_families(&e)
            && e["evidenceTruncated"] != true && !names.is_empty()
            && matches.len() == names.len()
            && !names.iter().any(|n| exclusions::matches(n, &blocked))
        {
            let advice: String = tx.query_row("SELECT advice FROM asset_seed_review_items WHERE run_id=?1 AND item_key=?2",params![id,key],|r|r.get(0)).map_err(err)?;
            if matches!(advice.as_str(), "review" | "evidence" | "retain") {
                e["automation"] = json!({"version":1,"route":"existing_scope"});
                tx.execute("UPDATE asset_seed_review_items SET state='resolved',evidence_json=?3,revision=revision+1 WHERE run_id=?1 AND item_key=?2",params![id,key,e.to_string()]).map_err(err)?;
                *counts.entry("existing_scope".into()).or_default() += 1;
                continue;
            }
        }
        if decision != "pending" || !matches!(state.as_str(), "pending" | "deferred" | "resolved") {
            if old_e != e {
                tx.execute("UPDATE asset_seed_review_items SET evidence_json=?3,revision=revision+1 WHERE run_id=?1 AND item_key=?2",params![id,key,e.to_string()]).map_err(err)?;
            }
            continue;
        }
        let has_names = !names.is_empty();
        let complete = e["evidenceTruncated"] != true
            || (kind == "certificate" && e["enrichmentVerifiedScopeRoots"] == json!(confirmed));
        let policy_conflict = matches.iter().any(|n| exclusions::matches(n, &blocked));
        let reused = previous.get(&key).filter(|(old, d, _)| {
            ["accepted", "rejected"].contains(&d.as_str()) && signature(old) == signature(&e)
        });
        let (next, advice, route, reason) = if complete && policy_conflict {
            (
                "done",
                "conflict",
                "policy_conflict",
                "已确认范围与排除规则重叠，只需处理这项规则冲突",
            )
        } else if complete && has_names && names.iter().all(|n| exclusions::matches(n, &blocked)) {
            (
                "resolved",
                "exclude",
                "excluded",
                "已按本项目排除域名规则自动分流，无需重复审核",
            )
        } else if complete && has_names && matches.len() == names.len() {
            (
                "resolved",
                "retain",
                "existing_scope",
                "全部已观察域名均在已确认范围内，无需重复确认；不等于已证明资产归属",
            )
        } else if let Some((_, d, _)) = reused {
            if d == "accepted" {
                (
                    "resolved",
                    "retain",
                    "reused_accepted",
                    "相同目标、相同证据的人工认可已复用；不扩大探测范围",
                )
            } else {
                (
                    "resolved",
                    "exclude",
                    "reused_rejected",
                    "相同目标、相同证据的人工排除已复用，无需重复审核",
                )
            }
        } else if !has_relevance_basis(&e) {
            ("deferred","evidence","missing_evidence","本地证据关联后仍只有名称或采集标签，自动暂存待补证；无需逐条审核，不发送 AI 或 Hunter")
        } else {
            (
                "pending",
                "review",
                "ai_pending",
                "已关联本地单位或范围线索，交由 AI 判断相关性与反证",
            )
        };
        e["automation"] =
            json!({"version":1,"route":route,"sourceRun":reused.map(|(_,_,run)|run.as_str())});
        let unchanged = old_e == e && state == next && tx.query_row("SELECT advice=?3 AND reason=?4 FROM asset_seed_review_items WHERE run_id=?1 AND item_key=?2",params![id,key,advice,reason],|r|r.get::<_,bool>(0)).map_err(err)?;
        if !unchanged {
            tx.execute("UPDATE asset_seed_review_items SET evidence_json=?3,state=?4,advice=?5,reason=?6,confidence=0,revision=revision+1 WHERE run_id=?1 AND item_key=?2 AND decision='pending'", params![id,key,e.to_string(),next,advice,reason]).map_err(err)?;
        }
        *counts.entry(route.into()).or_default() += 1;
    }
    tx.commit().map_err(err)?;
    Ok(json!(counts))
}
