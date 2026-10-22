//! Whole-project scope proposals. Suggestions are read-only; one explicit,
//! revision-bound confirmation authorizes the displayed domain families.
use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposalInput {
    project_id: i64,
    run_id: String,
    #[serde(default)]
    item_key: Option<String>,
}
#[tauri::command]
pub async fn seed_review_scope_proposal(
    state: tauri::State<'_, AppState>,
    input: ProposalInput,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || proposal_on(&db::open(&path)?, &input))
        .await
        .map_err(err)?
}
fn names(e: &Value) -> Vec<String> {
    e["domains"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(host)
        .collect()
}
fn shared(root: &str, configured: &[String]) -> bool {
    // PSL PRIVATE domains already have tenant-level registrable boundaries.
    // These infrastructure roots are absent from PSL; never promote them to a
    // platform-wide grant merely because one customer's certificate matched.
    matches!(
        root,
        "cmecloud.cn"
            | "myqcloud.com"
            | "aliyuncs.com"
            | "cloudfront.net"
            | "akamaiedge.net"
            | "akadns.net"
            | "azureedge.net"
            | "chinacloudsites.cn"
    ) || configured
        .iter()
        .any(|s| in_scope(root, s) || in_scope(s, root))
}
fn configured_shared(c: &Connection, p: i64) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let has_column = c
        .prepare("PRAGMA table_info(asset_ownership_profiles)")
        .map_err(err)?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?
        .iter()
        .any(|s| s == "shared_domains_json");
    if has_column {
        let raw: Option<String> = c
            .query_row(
                "SELECT shared_domains_json FROM asset_ownership_profiles WHERE project_id=?1",
                [p],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if let Some(raw) = raw {
            result.extend(serde_json::from_str::<Vec<String>>(&raw).map_err(err)?);
        }
    }
    Ok(result)
}
struct Row {
    key: String,
    kind: String,
    evidence: Value,
    state: String,
    advice: String,
    decision: String,
    confidence: f64,
    revision: i64,
    reason: String,
}
#[derive(Default)]
struct Family {
    names: BTreeSet<String>,
    scopes: BTreeSet<String>,
    keys: BTreeSet<String>,
    certs: BTreeSet<String>,
    pending: BTreeSet<String>,
    conflicts: BTreeSet<String>,
    reasons: BTreeSet<String>,
    assets: BTreeSet<i64>,
}
// A certificate Subject belongs to its CN/SAN, not every host or CNAME in
// the same asset row. Bind legacy advice to current local names before using
// it to suggest an entire domain family. This is read-only and never replays AI.
fn automatic_witnesses(c:&Connection,p:i64,id:&str)->Result<HashMap<String,BTreeSet<String>>,String>{
    let mut result=HashMap::<String,BTreeSet<String>>::new();
    let mut cached=HashMap::<i64,BTreeSet<String>>::new();
    let mut q=c.prepare("SELECT refs.item_key,a.id,a.host,a.link,a.extra_json FROM asset_seed_review_refs refs JOIN asset_seed_review_items i ON i.run_id=refs.run_id AND i.item_key=refs.item_key JOIN assets a ON a.id=refs.asset_id JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?2 WHERE refs.run_id=?1 AND pa.is_deleted=0 AND i.decision='pending' AND i.state='done' AND i.advice='retain' AND i.confidence>=0.8").map_err(err)?;
    let rows=q.query_map(params![id,p],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?))).map_err(err)?;
    for row in rows{
        let(key,asset,h,l,raw)=row.map_err(err)?;
        let families=cached.entry(asset).or_insert_with(||{
            let v:Value=serde_json::from_str(&raw).unwrap_or(Value::Null);
            let identities=evidence::identities(&v);
            let names:BTreeSet<String>=if identities.is_empty(){local::certificate_facts(&v).names.into_iter().map(|(n,_)|n).collect()}else{identities.into_iter().flat_map(|(_,names)|names).collect()};
            let mut families=names.into_iter().filter_map(|n|crate::commands::ownership::ownership_domain_parts(&n).and_then(|p|p.registrable)).collect::<BTreeSet<_>>();
            let independent_icp=[&v["icp.name"],&v["icp_name"],&v["icp_company"],&v["icp"]["name"]].into_iter().filter_map(Value::as_str).any(|s|!s.trim().is_empty());
            if independent_icp{if let Some(root)=host(if l.is_empty(){&h}else{&l}).and_then(|n|crate::commands::ownership::ownership_domain_parts(&n)).and_then(|p|p.registrable){families.insert(root);}}
            families
        });
        result.entry(key).or_default().extend(families.iter().cloned());
    }
    Ok(result)
}
pub(super) fn proposal_on(c: &Connection, i: &ProposalInput) -> Result<Value, String> {
    project(c, i.project_id)?;
    owned(c, i.project_id, &i.run_id)?;
    let (cursor, max): (i64, i64) = c
        .query_row(
            "SELECT cursor,max_id FROM asset_seed_reviews WHERE id=?1",
            [&i.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    if cursor < max {
        return Err("请先完成全量本地整理，再汇总项目范围".into());
    }
    let current = roots(c, i.project_id)?;
    let blocked = exclusions::rules(c, i.project_id)?;
    let configured = configured_shared(c, i.project_id)?;
    let rows: Vec<Row> = {
        let mut q=c.prepare("SELECT item_key,kind,evidence_json,state,advice,decision,confidence,revision,reason FROM asset_seed_review_items WHERE run_id=?1 ORDER BY item_key").map_err(err)?;
        let rows = q
            .query_map([&i.run_id], |r| {
                Ok(Row {
                    key: r.get(0)?,
                    kind: r.get(1)?,
                    evidence: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null),
                    state: r.get(3)?,
                    advice: r.get(4)?,
                    decision: r.get(5)?,
                    confidence: r.get(6)?,
                    revision: r.get(7)?,
                    reason: r.get(8)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)?
    };
    if i.item_key
        .as_ref()
        .is_some_and(|k| !rows.iter().any(|r| &r.key == k))
    {
        return Err("所选证据已不存在，请刷新".into());
    }
    let automatic_witnesses = automatic_witnesses(c,i.project_id,&i.run_id)?;
    let mut digest = Sha256::new();
    digest.update(json!([i.run_id, i.item_key, current, blocked, configured]).to_string());
    let mut families = BTreeMap::<String, Family>::new();
    let mut chosen_families = BTreeSet::new();
    for r in &rows {
        let ns = names(&r.evidence);
        let complete =
            r.evidence["evidenceTruncated"] != true && r.evidence["correlationTruncated"] != true;
        // Legacy model output could mistake collection labels for independent
        // ownership evidence. Apply today's relevance gate to automatic
        // suggestions without changing original receipts or operator decisions.
        let mut current_evidence = r.evidence.clone();
        current_evidence["confirmedDomainMatches"] = json!(ns.iter().filter(|n| current.iter().any(|root| in_scope(n,root))).collect::<Vec<_>>());
        let meaningful = has_relevance_basis(&current_evidence);
        let clear = complete
            && (!cross_domain_families(&r.evidence) || r.decision == "accepted")
            && r.decision != "rejected"
            && (r.decision == "accepted"
                || r.state == "done" && r.advice == "retain" && r.confidence >= 0.8 && meaningful);
        let explicit =
            i.item_key.as_deref() == Some(r.key.as_str()) && r.decision != "rejected" && complete;
        for n in ns {
            let Some(root) =
                crate::commands::ownership::ownership_domain_parts(&n).and_then(|p| p.registrable)
            else {
                continue;
            };
            if explicit {
                chosen_families.insert(root.clone());
            }
            let f = families.entry(root.clone()).or_default();
            f.names.insert(n.clone());
            f.keys.insert(r.key.clone());
            if r.kind == "certificate" {
                f.certs.insert(r.key.clone());
            }
            if r.state == "deferred" || r.advice == "evidence" || r.advice == "review" || r.advice == "retain" && !meaningful {
                f.pending.insert(r.key.clone());
            }
            if r.decision == "rejected"
                || matches!(r.advice.as_str(), "conflict")
                || cross_domain_families(&r.evidence)
            {
                f.conflicts.insert(r.key.clone());
            }
            let current_scope_anchor=current.iter().any(|scope|in_scope(&n,scope));
            let witnessed=automatic_witnesses.get(&r.key).is_some_and(|families|families.contains(&root));
            if (explicit || clear && (r.decision=="accepted" || current_scope_anchor || witnessed)) && !exclusions::matches(&n, &blocked) {
                // Platform tenants retain their exact observed host boundary.
                let candidate = if shared(&root, &configured) {
                    if n == root {
                        continue;
                    }
                    n
                } else {
                    root
                };
                if !current.iter().any(|s| in_scope(&candidate, s))
                    && !exclusions::matches(&candidate, &blocked)
                {
                    f.scopes.insert(candidate);
                    f.reasons.insert(clipped(&r.reason));
                }
            }
        }
    }
    // Only candidate-family evidence binds this confirmation. A background
    // refresh of an already confirmed, unrelated family cannot invalidate the
    // user's pending selection. Scope/exclusions/shared boundaries remain bound.
    let selected_keys: BTreeSet<_> = families
        .iter()
        .filter(|(root,f)| !f.scopes.is_empty() && (i.item_key.is_none() || chosen_families.contains(*root)))
        .flat_map(|(_,f)| f.keys.iter().cloned())
        .collect();
    for r in rows.iter().filter(|r| selected_keys.contains(&r.key)) {
        digest.update(json!([r.key,r.evidence,r.state,r.advice,r.decision,r.confidence,r.revision]).to_string());
    }
    // Exact distinct assets, not a sum that double-counts certificate/domain rows.
    let mut by_key = HashMap::<String, Vec<i64>>::new();
    let mut refs=c.prepare("SELECT refs.item_key,refs.asset_id,pa.is_deleted FROM asset_seed_review_refs refs JOIN project_assets pa ON pa.project_id=?2 AND pa.asset_id=refs.asset_id WHERE refs.run_id=?1 ORDER BY refs.item_key,refs.asset_id").map_err(err)?;
    for row in refs
        .query_map(params![i.run_id, i.project_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(err)?
    {
        let (key, asset, deleted) = row.map_err(err)?;
        if selected_keys.contains(&key) {
            digest.update(json!([key, asset, deleted]).to_string());
            by_key.entry(key).or_default().push(asset);
        }
    }
    let mut groups = Vec::new();
    let mut all_pending = 0usize;
    let mut proposed_pending = 0usize;
    for (root, mut f) in families {
        all_pending += f.pending.len();
        if f.scopes.is_empty() || i.item_key.is_some() && !chosen_families.contains(&root) {
            continue;
        }
        for key in &f.keys {
            f.assets
                .extend(by_key.get(key).into_iter().flatten().copied());
        }
        proposed_pending += f.pending.len();
        groups.push(json!({"root":root,"shared":shared(&root,&configured),"scopes":f.scopes,"observedDomains":f.names.iter().take(12).collect::<Vec<_>>(),"observedCount":f.names.len(),"evidenceGroups":f.keys.len(),"certificateGroups":f.certs.len(),"pendingEvidenceGroups":f.pending.len(),"conflictGroups":f.conflicts.len(),"assetCount":f.assets.len(),"reasons":f.reasons.iter().take(3).collect::<Vec<_>>()}));
    }
    Ok(
        json!({"runId":i.run_id,"token":format!("{:x}",digest.finalize()),"roots":current,"groups":groups,"totalEvidenceGroups":rows.len(),"pendingFamilyReferences":all_pending,"proposedPendingFamilyReferences":proposed_pending,"scopeExpanded":false,"hunterRequests":0}),
    )
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    project_id: i64,
    run_id: String,
    hosts: Vec<String>,
    expected_roots: Vec<String>,
    proposal_token: String,
    #[serde(default)]
    item_key: Option<String>,
    user_confirmed: bool,
}
pub(super) fn confirm_on(c: &mut Connection, i: &Selection) -> Result<Value, String> {
    if !i.user_confirmed || i.hosts.is_empty() || i.hosts.len() > 500 {
        return Err("请核对并确认新增范围（最多500项）".into());
    }
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let proposal = proposal_on(
        &tx,
        &ProposalInput {
            project_id: i.project_id,
            run_id: i.run_id.clone(),
            item_key: i.item_key.clone(),
        },
    )?;
    if proposal["token"] != i.proposal_token || proposal["roots"] != json!(i.expected_roots) {
        return Err("证据、人工结论或项目范围已变化，请重新汇总后确认".into());
    }
    let allowed: BTreeSet<String> = proposal["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|g| g["scopes"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let mut current = roots(&tx, i.project_id)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    for name in &i.hosts {
        if !allowed.contains(name) {
            return Err("所选范围不属于本次项目汇总，请重新核对".into());
        }
        // A parent proposal is explicit in the UI. It must still have a live
        // observed child; a stale/trash-only AI receipt cannot authorize a root.
        let mut q=tx.prepare("SELECT DISTINCT a.host,a.link,a.extra_json FROM asset_seed_review_refs refs JOIN assets a ON a.id=refs.asset_id JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?2 WHERE refs.run_id=?1 AND pa.is_deleted=0 AND refs.item_key IN (SELECT item_key FROM asset_seed_review_items WHERE run_id=?1 AND decision!='rejected' AND EXISTS(SELECT 1 FROM json_each(evidence_json,'$.domains') d WHERE d.value=?3 OR d.value LIKE '%.'||?3))").map_err(err)?;
        let witnessed = q
            .query_map(params![i.run_id, i.project_id, name], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(err)?
            .filter_map(Result::ok)
            .any(|(h, l, e)| {
                units(
                    &serde_json::from_str(&e).unwrap_or(Value::Null),
                    if l.is_empty() { &h } else { &l },
                )
                .iter()
                .any(|u| u.domains.iter().any(|n| in_scope(n, name)))
            });
        if !witnessed {
            return Err(format!(
                "{name} 的当前本地依据已变化或仅在回收站，请重新整理；未保存任何新增范围"
            ));
        }
        current.insert(name.clone());
    }
    // Collapse redundant children only within this explicit new grant; retained
    // historical records/decisions and exclusions are not modified.
    let normalized = current
        .iter()
        .filter(|n| !current.iter().any(|p| p != *n && in_scope(n, p)))
        .cloned()
        .collect::<Vec<_>>();
    if normalized.len() > 500 {
        return Err("项目确认范围超过500项".into());
    }
    tx.execute("INSERT INTO asset_discovery_scopes VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET roots_json=excluded.roots_json",params![i.project_id,json!(normalized).to_string()]).map_err(err)?;
    // Queue all related observed names, including SAN names attached to an
    // out-of-scope IP endpoint. Endpoint-only scanning would miss precisely
    // those project certificates the operator has just confirmed.
    let mut seeds = BTreeSet::new();
    {
        let mut q=tx.prepare("SELECT item_key,evidence_json FROM asset_seed_review_items WHERE run_id=?1 AND decision!='rejected' AND advice!='exclude'").map_err(err)?;
        for row in q
            .query_map([&i.run_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(err)?
        {
            let (key, raw) = row.map_err(err)?;
            for n in names(&serde_json::from_str(&raw).unwrap_or(Value::Null)) {
                if i.hosts.iter().any(|root| in_scope(&n, root))
                    && !exclusions::blocked_name(&tx, i.project_id, &n)?
                {
                    seeds.insert((n, key.clone()));
                }
            }
        }
    }
    for (name, key) in &seeds {
        insert_seed(
            &tx,
            i.project_id,
            name,
            "confirmed_review_name",
            &format!("review:{}:{key}", i.run_id),
            false,
        )?;
    }
    assist::record_enrichment_intent(&tx, i.project_id, &i.run_id, &i.proposal_token)?;
    tx.commit().map_err(err)?;
    Ok(
        json!({"roots":normalized,"scopeSaved":true,"queuedNameReferences":seeds.len(),"hunterStarted":false}),
    )
}
#[tauri::command]
pub async fn confirm_seed_review_proposal(
    state: tauri::State<'_, AppState>,
    manager: tauri::State<'_, enrichment::Manager>,
    input: Selection,
) -> Result<Value, String> {
    let path = state.db_path.clone();
    let m = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut c = db::open(&path)?;
        let mut result = match confirm_on(&mut c, &input) {
            Ok(result) => result,
            Err(error) => {
                // Background certificate enrichment may advance the evidence
                // digest while the user reads the cached proposal. Replace the
                // cache once on rejection so the next poll can offer a new token.
                return Err(match assist::refresh_proposal(&c, input.project_id, &input.run_id) {
                    Ok(()) => error,
                    Err(refresh) => format!("{error}；建议刷新失败：{refresh}"),
                });
            }
        };
        if let Err(e) = triage::run(&mut c, input.project_id, &input.run_id) {
            result["triageError"] = json!(e);
        }
        if let Err(e) = assist::refresh_proposal(&c, input.project_id, &input.run_id) {
            result["proposalRefreshError"] = json!(e);
        }
        // Scope persistence and queue activation have separate durable receipts.
        // A failed launch never pretends the scope write was rolled back.
        match m.dispatch(
            input.project_id,
            "start",
            json!({}),
            &format!("scope-{}", input.proposal_token),
        ) {
            Ok(task) => {
                if let Err(e) = assist::enrichment_receipt(&c, input.project_id, &input.run_id, &input.proposal_token, None) { result["enrichmentReceiptError"] = json!(e); }
                result["enrichment"] = task;
                result["enrichmentStarted"] = json!(true);
            }
            Err(e) => {
                let _ = assist::enrichment_receipt(&c, input.project_id, &input.run_id, &input.proposal_token, Some(&e));
                result["enrichmentStarted"] = json!(false);
                result["enrichmentError"] = json!(e);
            }
        }
        Ok(result)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Connection, String) {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT);INSERT INTO projects VALUES(1,'移动','active'),(2,'其他','active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT,shared_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,company TEXT,extra_json TEXT,content_category TEXT,probe_outcome TEXT DEFAULT '');CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,decision TEXT DEFAULT 'pending');").unwrap();
        ensure(&c).unwrap();
        c.execute_batch(TABLES).unwrap();
        for id in 1..=90 {
            let name = if id < 89 {
                format!("n{id}.10086.cn")
            } else {
                format!("eos.tenant{id}.cmecloud.cn")
            };
            c.execute(
                "INSERT INTO assets VALUES(?1,?2,?2,'',?3,'','')",
                params![
                    id,
                    format!("https://{name}"),
                    json!({"cert.domain":name,"cert.subject.org":"目标单位"}).to_string()
                ],
            )
            .unwrap();
            c.execute("INSERT INTO project_assets VALUES(1,?1,0,'pending')", [id])
                .unwrap();
        }
        let id = prepare_on(&mut c, 1, "中国移动集团", true, &HashSet::new()).unwrap();
        while !harvest_page(&mut c, 1, &id).unwrap() {}
        c.execute("UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=.95,reason='目标单位证据' WHERE run_id=?1 AND kind='certificate_metadata'",[&id]).unwrap();
        (c, id)
    }
    fn input(id: &str) -> ProposalInput {
        ProposalInput {
            project_id: 1,
            run_id: id.into(),
            item_key: None,
        }
    }
    fn selection(id: &str, v: &Value, hosts: Vec<String>) -> Selection {
        Selection {
            project_id: 1,
            run_id: id.into(),
            hosts,
            expected_roots: serde_json::from_value(v["roots"].clone()).unwrap(),
            proposal_token: v["token"].as_str().unwrap().into(),
            item_key: None,
            user_confirmed: true,
        }
    }
    #[test]
    fn entire_project_groups_other_pages_and_accepted_certificates_without_platform_grant() {
        let (c, id) = fixture();
        c.execute("UPDATE asset_seed_review_items SET decision='accepted' WHERE run_id=?1 AND kind='certificate_metadata' AND EXISTS(SELECT 1 FROM json_each(evidence_json,'$.domains') WHERE value='n88.10086.cn')",[&id]).unwrap();
        c.execute("UPDATE asset_seed_review_items SET evidence_json=json_set(evidence_json,'$.ambiguousIdentity',json('true')) WHERE kind='certificate_metadata'",[]).unwrap();
        let v = proposal_on(&c, &input(&id)).unwrap();
        let groups = v["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        let ordinary = groups.iter().find(|g| g["root"] == "10086.cn").unwrap();
        assert_eq!(ordinary["scopes"], json!(["10086.cn"]));
        assert_eq!(ordinary["assetCount"], 88);
        assert!(ordinary["evidenceGroups"].as_u64().unwrap() > 30);
        let cloud = groups.iter().find(|g| g["root"] == "cmecloud.cn").unwrap();
        assert_eq!(cloud["shared"], true);
        assert_eq!(
            cloud["scopes"],
            json!(["eos.tenant89.cmecloud.cn", "eos.tenant90.cmecloud.cn"])
        );
        assert!(roots(&c, 1).unwrap().is_empty());
    }
    #[test]
    fn selecting_one_certificate_includes_the_whole_domain_family_but_not_other_families() {
        let (c, id) = fixture();
        let key:String=c.query_row("SELECT item_key FROM asset_seed_review_items WHERE run_id=?1 AND kind='certificate_metadata' AND EXISTS(SELECT 1 FROM json_each(evidence_json,'$.domains') WHERE value='n2.10086.cn')",[&id],|r|r.get(0)).unwrap();
        let mut i = input(&id);
        i.item_key = Some(key);
        let v = proposal_on(&c, &i).unwrap();
        assert_eq!(v["groups"].as_array().unwrap().len(), 1);
        assert_eq!(v["groups"][0]["assetCount"], 88);
        assert_eq!(v["groups"][0]["scopes"], json!(["10086.cn"]));
    }
    #[test]
    fn explicit_parent_consent_is_atomic_and_stale_decisions_or_forged_platforms_are_rejected() {
        let (mut c, id) = fixture();
        let v = proposal_on(&c, &input(&id)).unwrap();
        assert!(confirm_on(&mut c, &selection(&id, &v, vec!["cmecloud.cn".into()])).is_err());
        assert!(roots(&c, 1).unwrap().is_empty());
        c.execute("UPDATE asset_seed_review_items SET decision='rejected',revision=revision+1 WHERE run_id=?1 AND kind='domain_group'",[&id]).unwrap();
        assert!(
            confirm_on(&mut c, &selection(&id, &v, vec!["10086.cn".into()]))
                .unwrap_err()
                .contains("已变化")
        );
        let v = proposal_on(&c, &input(&id)).unwrap();
        let mut denied = selection(&id, &v, vec!["10086.cn".into()]);
        denied.user_confirmed = false;
        assert!(confirm_on(&mut c, &denied).is_err());
        let before: i64 = c
            .query_row(
                "SELECT count(*) FROM asset_seed_review_items WHERE decision='rejected'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        confirm_on(&mut c, &selection(&id, &v, vec!["10086.cn".into()])).unwrap();
        assert_eq!(roots(&c, 1).unwrap(), vec!["10086.cn"]);
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM asset_seed_review_items WHERE decision='rejected'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            before
        );
    }
    #[test]
    fn rejected_truncated_and_other_project_evidence_cannot_supply_automatic_proposals() {
        let (c, id) = fixture();
        c.execute("UPDATE asset_seed_review_items SET decision='rejected' WHERE kind='certificate_metadata'",[]).unwrap();
        let v = proposal_on(&c, &input(&id)).unwrap();
        assert!(v["groups"].as_array().unwrap().is_empty());
        c.execute("UPDATE asset_seed_review_items SET decision='pending',evidence_json=json_set(evidence_json,'$.evidenceTruncated',json('true'))",[]).unwrap();
        assert!(proposal_on(&c, &input(&id)).unwrap()["groups"]
            .as_array()
            .unwrap()
            .is_empty());
        let mut other = input(&id);
        other.project_id = 2;
        assert!(proposal_on(&c, &other).is_err());
    }
    #[test]
    fn removed_local_witness_does_not_authorize_a_root_and_preserves_previous_scopes() {
        let (mut c, id) = fixture();
        c.execute("UPDATE assets SET extra_json='{}',host='https://different.invalid',link='https://different.invalid'",[]).unwrap();
        let v = proposal_on(&c, &input(&id)).unwrap();
        assert!(confirm_on(&mut c, &selection(&id, &v, vec!["10086.cn".into()])).is_err());
        assert!(roots(&c, 1).unwrap().is_empty());
    }
    #[test]
    #[ignore = "private real-project snapshot, opt-in read-only acceptance"]
    fn real_project_scope_snapshot() {
        let path = std::env::var("OVI_SCOPE_SNAPSHOT").unwrap();
        let output = std::env::var("OVI_SCOPE_RECEIPT").unwrap();
        let c =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let id: String = c
            .query_row(
                "SELECT id FROM asset_seed_reviews WHERE project_id=3 ORDER BY rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let before = roots(&c, 3).unwrap();
        let mut i = input(&id);
        i.project_id = 3;
        let v = proposal_on(&c, &i).unwrap();
        assert!(!v["groups"].as_array().unwrap().is_empty());
        assert!(v["totalEvidenceGroups"].as_u64().unwrap() > 10000);
        for expected in ["10086.cn", "cmecloud.cn"] {
            assert!(
                v["groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|g| g["root"] == expected),
                "real project family missing: {expected}"
            );
        }
        assert_eq!(roots(&c, 3).unwrap(), before);
        for group in v["groups"].as_array().unwrap() {
            if group["shared"] == true {
                assert!(!group["scopes"].as_array().unwrap().contains(&group["root"]));
            }
        }
        std::fs::write(output, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    }
}
