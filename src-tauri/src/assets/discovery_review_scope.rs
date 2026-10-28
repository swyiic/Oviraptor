//! Whole-project scope proposals. Suggestions are read-only; one explicit,
//! revision-bound confirmation authorizes the displayed domain families.
use super::*;

pub(super) const VERIFICATION_POLICY_VERSION: i64 = 2;

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
pub(crate) fn shared(root: &str, configured: &[String]) -> bool {
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
pub(crate) fn configured_shared(c: &Connection, p: i64) -> Result<Vec<String>, String> {
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
    conflicting_names: BTreeSet<String>,
    reasons: BTreeSet<String>,
    assets: BTreeSet<i64>,
    verification_sources: BTreeMap<String, Value>,
}
// Scope proposals use independent, domain-bound facts. A model label and a
// historical "accepted" click only express relevance, never ownership of all
// co-located hostnames or SAN association lists.
#[derive(Default)]
pub(in crate::asset_discovery) struct TargetContext {
    pub(in crate::asset_discovery) names: BTreeSet<String>,
    official_domains: BTreeMap<String, String>,
}
pub(in crate::asset_discovery) fn organization_key(value: &str) -> String {
    let mut name = value.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    for suffix in ["及其下属单位", "及其下属公司", "及其子公司", "及下属单位", "及子公司"] {
        if let Some(prefix) = name.strip_suffix(suffix) { name = prefix.to_owned(); break; }
    }
    loop {
        let mut changed = false;
        for suffix in ["有限责任公司", "股份有限公司", "有限公司", "集团公司", "集团", "公司", "incorporated", "corporation", "limited", "ltd", "corp", "llc", "plc", "inc"] {
            if let Some(prefix) = name.strip_suffix(suffix).filter(|p| p.chars().count() >= 2) {
                name = prefix.to_owned(); changed = true; break;
            }
        }
        if !changed { break; }
    }
    name
}
fn has_table(c: &Connection, table: &str) -> Result<bool, String> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [table], |r| r.get(0)).map_err(err)
}
pub(in crate::asset_discovery) fn target_context(c: &Connection, p: i64, organization: &str) -> Result<TargetContext, String> {
    let mut context = TargetContext::default();
    let target = organization_key(organization);
    if target.chars().count() >= 2 { context.names.insert(target.clone()); }
    // Aliases/control relationships require a captured official statement,
    // scoped to this project and requested organization, with bounded freshness.
    // This only expands exact Subject/ICP identity matching, never network scope.
    for legal in crate::asset_discovery::organization_evidence::names(c,p,organization)? {
        context.names.insert(organization_key(&legal));
    }
    // A project profile is eligible only when this run targets that profile;
    // entering an unrelated target must not inherit all of the project's names.
    if has_table(c, "asset_ownership_profiles")? {
        let columns = c.prepare("PRAGMA table_info(asset_ownership_profiles)").map_err(err)?.query_map([], |r| r.get::<_,String>(1)).map_err(err)?.collect::<Result<HashSet<_>,_>>().map_err(err)?;
        if ["legal_name", "aliases_json"].iter().all(|k| columns.contains(*k)) {
            let profile: Option<(String,String)> = c.query_row("SELECT legal_name,aliases_json FROM asset_ownership_profiles WHERE project_id=?1", [p], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
            if let Some((legal,aliases)) = profile {
                let mut names = serde_json::from_str::<Vec<String>>(&aliases).map_err(err)?;
                names.push(legal.clone());
                if names.iter().any(|n| organization_key(n)==target) {
                    context.names.extend(names.iter().map(|n| organization_key(n)).filter(|n| n.chars().count()>=2));
                }
            }
        }
    }
    if ["project_org_bindings", "org_entities", "org_entity_identifiers"].iter().all(|t| has_table(c,t).unwrap_or(false)) {
        use crate::commands::ownership;
        let binding = ownership::load_project_org_binding(c,p)?;
        let entities = ownership::load_org_entities(c)?;
        if let Some(binding) = binding.as_ref() {
            let anchor = if binding.project_kind=="group_ledger" { Some(binding.group_entity_id) } else { binding.unit_entity_id };
            let anchored = entities.iter().find(|e| Some(e.id)==anchor).is_some_and(|e| {
                std::iter::once(e.legal_name.as_str()).chain(e.identifiers.iter().filter(|i| !matches!(i.kind.as_str(),"official_domain"|"shared_domain")).map(|i|i.value.as_str())).any(|n| organization_key(n)==target)
            });
            if anchored {
                let mut scope=ownership::org_binding_scope(&entities,Some(binding));
                // Suppliers and unknown relationships are not subsidiaries of
                // the requested group. Their descendants cannot inherit the
                // group relation either; an explicitly bound unit is separate.
                let mut unrelated=entities.iter().filter(|e| Some(e.id)!=anchor && matches!(e.entity_kind.as_str(),"supplier"|"unknown")).map(|e|e.id).collect::<HashSet<_>>();
                loop {
                    let before=unrelated.len();
                    for entity in &entities { if entity.parent_id.is_some_and(|p|unrelated.contains(&p)) { unrelated.insert(entity.id); } }
                    if unrelated.len()==before { break; }
                }
                scope.retain(|id|!unrelated.contains(id));
                for entity in entities.iter().filter(|e| scope.contains(&e.id)) {
                    context.names.insert(organization_key(&entity.legal_name));
                    for identifier in &entity.identifiers {
                        if identifier.kind=="official_domain" {
                            if let Some(h)=host(&identifier.value) { context.official_domains.insert(h,entity.legal_name.clone()); }
                        } else if identifier.kind!="shared_domain" {
                            let normalized=organization_key(&identifier.value);
                            if normalized.chars().count()>=2 { context.names.insert(normalized); }
                        }
                    }
                }
            }
        }
    }
    Ok(context)
}
/// Relevance hints may select fixed-site public research before ownership is
/// established. This does not admit provider association lists, active targets,
/// scope changes, or paid requests.
pub(crate) fn public_research_candidates(c:&Connection,p:i64,id:&str,items:&HashMap<String,Value>)->Result<Vec<(String,String)>,String>{
    let organization:String=c.query_row("SELECT organization FROM asset_seed_reviews WHERE id=?1 AND project_id=?2",params![id,p],|r|r.get(0)).map_err(err)?;
    let target=target_context(c,p,&organization)?;
    let mut out=BTreeSet::new();
    for(key,evidence)in items{
        for observation in evidence["domainEvidence"].as_array().into_iter().flatten(){
            let source=observation["source"].as_str().unwrap_or("");
            if !matches!(source,"provider_subject_cn"|"provider_subject_san"|"endpoint_icp"|"leaf_subject"|"public_ct_subject"){continue;}
            let(Some(name),Some(organization))=(observation["host"].as_str(),observation["organization"].as_str())else{continue;};
            if target.names.contains(&organization_key(organization)){
                if let Some(h)=host(name){out.insert((key.clone(),h));}
            }
        }
    }
    Ok(out.into_iter().collect())
}
#[derive(Clone)]
struct BoundWitness {
    host: String,
    organization: String,
    kind: String,
    asset_id: i64,
    wildcard: bool,
    certificate_sha256: Option<String>,
    target_match: bool,
    public_source: Option<Value>,
}
impl BoundWitness {
    fn source(&self) -> Value {
        let mut value = self.public_source.clone().unwrap_or_else(|| json!({"assetId":self.asset_id}));
        value["kind"] = json!(self.kind); value["host"] = json!(self.host); value["organization"] = json!(self.organization);
        value["certificateSha256"] = json!(self.certificate_sha256); value["wildcard"] = json!(self.wildcard);
        value
    }
}
fn automatic_witnesses(c:&Connection,p:i64,id:&str,target:&TargetContext,explicit:Option<&str>)->Result<HashMap<String,Vec<BoundWitness>>,String>{
    let mut result=HashMap::<String,Vec<BoundWitness>>::new();
    let mut cached=HashMap::<i64,Vec<BoundWitness>>::new();
    let mut q=c.prepare(&format!("SELECT refs.item_key,a.id,a.host,a.link,a.extra_json FROM asset_seed_review_refs refs JOIN asset_seed_reviews review ON review.id=refs.run_id AND review.project_id=?2 JOIN asset_seed_review_items i ON i.run_id=refs.run_id AND i.item_key=refs.item_key JOIN assets a ON a.id=refs.asset_id JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?2 WHERE refs.run_id=?1 AND pa.is_deleted=0 AND (review.include_isolated=1 OR NOT {ISOLATED}) AND i.decision!='rejected' AND (i.decision='accepted' OR (i.state='done' AND COALESCE(json_extract(i.evidence_json,'$.currentAdvice'),i.advice)='retain' AND i.confidence>=0.8) OR i.item_key=?3) ORDER BY refs.item_key,a.id")).map_err(err)?;
    for row in q.query_map(params![id,p,explicit],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?))).map_err(err)? {
        let(key,asset,h,l,raw)=row.map_err(err)?;
        let witnesses=cached.entry(asset).or_insert_with(||{
            let v:Value=serde_json::from_str(&raw).unwrap_or(Value::Null);
            provenance::ownership_observations(&v,if l.is_empty(){&h}else{&l}).into_iter()
                .filter(|o| matches!(o.source.as_str(),"leaf_subject"|"endpoint_icp"))
                .map(|o| BoundWitness { target_match:target.names.contains(&organization_key(&o.organization)),host:o.host,organization:o.organization,kind:(if o.source=="leaf_subject"{"tls_subject"}else{"icp"}).into(),asset_id:asset,wildcard:o.wildcard,certificate_sha256:o.certificate_sha256,public_source:None }).collect()
        });
        result.entry(key).or_default().extend(witnesses.iter().cloned());
    }
    // Public CT is bound to the run and exact candidate, then evaluated against
    // this target context. A cached certificate cannot inherit another
    // project's organization or turn a provider association list into SANs.
    let mut candidates = HashMap::<String, BTreeSet<String>>::new();
    let mut candidate_query = c.prepare(&format!("SELECT DISTINCT i.item_key,i.evidence_json FROM asset_seed_review_items i JOIN asset_seed_review_refs refs ON refs.run_id=i.run_id AND refs.item_key=i.item_key JOIN asset_seed_reviews review ON review.id=i.run_id AND review.project_id=?2 JOIN project_assets pa ON pa.asset_id=refs.asset_id AND pa.project_id=?2 JOIN assets a ON a.id=refs.asset_id WHERE i.run_id=?1 AND pa.is_deleted=0 AND (review.include_isolated=1 OR NOT {ISOLATED}) AND i.decision!='rejected'")).map_err(err)?;
    for row in candidate_query.query_map(params![id,p], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(err)? {
        let (key,raw) = row.map_err(err)?;
        candidates.insert(key,names(&serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null)).into_iter().collect());
    }
    let all_names = candidates.values().flat_map(|ns| ns.iter().cloned()).collect();
    let public = super::super::passive_evidence::observations(c,p,id,&all_names)?;
    let mut public_keys=BTreeSet::new();
    for (key,names) in candidates {
        for observation in public.iter().filter(|o| o["candidate"].as_str().is_some_and(|n| names.contains(n))) {
            let (Some(host),Some(organization)) = (observation["host"].as_str(),observation["organization"].as_str()) else {continue;};
            public_keys.insert(key.clone());
            result.entry(key.clone()).or_default().push(BoundWitness {
                host:host.into(),organization:organization.into(),kind:"public_ct_subject".into(),asset_id:0,
                wildcard:observation["wildcard"].as_bool().unwrap_or(false),
                certificate_sha256:observation["certificateSha256"].as_str().map(str::to_owned),
                target_match:target.names.contains(&organization_key(organization)),public_source:Some(observation.clone()),
            });
        }
    }
    // A new public proof must still see contradictory current local facts,
    // even when that item's previous model label was evidence/conflict. Read
    // just the public-backed keys so ordinary scope polling does not parse the
    // entire project a second time.
    if !public_keys.is_empty(){
        let mut local=c.prepare(&format!("SELECT refs.item_key,a.id,a.host,a.link,a.extra_json FROM json_each(?3) keys CROSS JOIN asset_seed_review_refs refs JOIN asset_seed_reviews review ON review.id=refs.run_id AND review.project_id=?2 JOIN asset_seed_review_items i ON i.run_id=refs.run_id AND i.item_key=refs.item_key JOIN assets a ON a.id=refs.asset_id JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?2 WHERE refs.run_id=?1 AND refs.item_key=keys.value AND pa.is_deleted=0 AND (review.include_isolated=1 OR NOT {ISOLATED}) AND i.decision!='rejected'")).map_err(err)?;
        for row in local.query_map(params![id,p,json!(public_keys).to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?))).map_err(err)?{
            let(key,asset,h,l,raw)=row.map_err(err)?;
            let witnesses=cached.entry(asset).or_insert_with(||{
                let v:Value=serde_json::from_str(&raw).unwrap_or(Value::Null);
                provenance::ownership_observations(&v,if l.is_empty(){&h}else{&l}).into_iter()
                    .filter(|o|matches!(o.source.as_str(),"leaf_subject"|"endpoint_icp"))
                    .map(|o|BoundWitness{target_match:target.names.contains(&organization_key(&o.organization)),host:o.host,organization:o.organization,kind:(if o.source=="leaf_subject"{"tls_subject"}else{"icp"}).into(),asset_id:asset,wildcard:o.wildcard,certificate_sha256:o.certificate_sha256,public_source:None}).collect()
            });
            let bound=result.entry(key).or_default();
            for witness in witnesses.iter(){if !bound.iter().any(|old|old.asset_id==witness.asset_id&&old.kind==witness.kind&&old.host==witness.host&&old.organization==witness.organization){bound.push(witness.clone());}}
        }
    }
    Ok(result)
}
// Read candidate references once, not once per proposed scope. Historical
// acceptance is retained, but a recycled/changed observation is not a current
// executable scope. Confirmation rechecks the same live facts in its transaction.
fn live_candidate_names(
    c: &Connection,
    p: i64,
    id: &str,
    keys: &BTreeSet<String>,
) -> Result<HashMap<String, BTreeSet<String>>, String> {
    let mut result = HashMap::<String, BTreeSet<String>>::new();
    if keys.is_empty() { return Ok(result); }
    let mut cached = HashMap::<i64, BTreeSet<String>>::new();
    let mut q = c.prepare(&format!("SELECT refs.item_key,a.id,a.host,a.link,a.extra_json FROM asset_seed_review_refs refs JOIN asset_seed_reviews review ON review.id=refs.run_id AND review.project_id=?2 JOIN asset_seed_review_items i ON i.run_id=refs.run_id AND i.item_key=refs.item_key JOIN project_assets pa ON pa.project_id=?2 AND pa.asset_id=refs.asset_id JOIN assets a ON a.id=refs.asset_id WHERE refs.run_id=?1 AND pa.is_deleted=0 AND (review.include_isolated=1 OR NOT {ISOLATED}) AND i.decision!='rejected' AND refs.item_key IN (SELECT value FROM json_each(?3))")).map_err(err)?;
    for row in q.query_map(params![id,p,json!(keys).to_string()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?))).map_err(err)? {
        let (key, asset, h, l, raw) = row.map_err(err)?;
        let names = cached.entry(asset).or_insert_with(|| observed_names(
            &serde_json::from_str(&raw).unwrap_or(Value::Null),
            if l.is_empty() { &h } else { &l },
        ));
        result.entry(key).or_default().extend(names.iter().cloned());
    }
    Ok(result)
}
pub(super) fn proposal_on(c: &Connection, i: &ProposalInput) -> Result<Value, String> {
    project(c, i.project_id)?;
    owned(c, i.project_id, &i.run_id)?;
    let (cursor, max, organization, include_isolated): (i64, i64, String, bool) = c
        .query_row(
            "SELECT cursor,max_id,organization,include_isolated FROM asset_seed_reviews WHERE id=?1",
            [&i.run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(err)?;
    if cursor < max {
        return Err("请先完成全量本地整理，再汇总项目范围".into());
    }
    let current = roots(c, i.project_id)?;
    let blocked = exclusions::rules(c, i.project_id)?;
    let configured = configured_shared(c, i.project_id)?;
    let rows: Vec<Row> = {
        let mut q=c.prepare("SELECT item_key,kind,evidence_json,state,COALESCE(json_extract(evidence_json,'$.currentAdvice'),advice),decision,confidence,revision,COALESCE(json_extract(evidence_json,'$.currentReason'),reason) FROM asset_seed_review_items WHERE run_id=?1 ORDER BY item_key").map_err(err)?;
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
    let target = target_context(c,i.project_id,&organization)?;
    let automatic_witnesses = automatic_witnesses(c,i.project_id,&i.run_id,&target,i.item_key.as_deref())?;
    let mut digest = Sha256::new();
    digest.update(json!([VERIFICATION_POLICY_VERSION,i.run_id,i.item_key,current,blocked,configured,organization,include_isolated,target.names,target.official_domains]).to_string());
    let mut unverified_scopes = BTreeSet::new();
    let mut held_families = BTreeSet::new();
    let mut verification_conflicts = BTreeSet::new();
    let mut families = BTreeMap::<String, Family>::new();
    let mut chosen_families = BTreeSet::new();
    let mut domains_by_key = HashMap::new();
    for r in &rows {
        let ns = names(&r.evidence);
        domains_by_key.insert(r.key.clone(), ns.clone());
        let complete =
            r.evidence["evidenceTruncated"] != true && r.evidence["correlationTruncated"] != true;
        // Legacy model output could mistake collection labels for independent
        // ownership evidence. Apply today's relevance gate to automatic
        // suggestions without changing original receipts or operator decisions.
        let mut current_evidence = r.evidence.clone();
        current_evidence["confirmedDomainMatches"] = json!(ns.iter().filter(|n| current.iter().any(|root| in_scope(n,root))).collect::<Vec<_>>());
        let meaningful = has_relevance_basis(&current_evidence) || automatic_witnesses.get(&r.key).is_some_and(|items| items.iter().any(|w| w.target_match));
        // A current public DER bound to the requested organization is already
        // independent evidence. It may form a suggestion while AI still says
        // evidence/review; the per-name gate below and human confirmation stay.
        let public_proof = automatic_witnesses.get(&r.key).is_some_and(|items|
            items.iter().any(|w| w.kind=="public_ct_subject" && w.target_match));
        let clear = complete
            && r.decision != "rejected"
            && (public_proof || r.decision == "accepted"
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
            {
                f.conflicts.insert(r.key.clone());
            }
            if !(explicit || clear) || exclusions::matches(&n,&blocked) || current.iter().any(|scope|in_scope(&n,scope)) { continue; }
            let directory = target.official_domains.iter().filter(|(domain,_)|in_scope(&n,domain)).max_by_key(|(domain,_)|domain.len());
            let observations = automatic_witnesses.get(&r.key).into_iter().flatten().filter(|w| w.host==n || w.wildcard && in_scope(&n,&w.host)).collect::<Vec<_>>();
            let witnesses = observations.iter().copied().filter(|w|w.target_match).collect::<Vec<_>>();
            let contradictory = (directory.is_some() || !witnesses.is_empty()) && observations.iter().any(|w|!w.target_match);
            // Even a manually accepted evidence group or an explicit item click
            // must pass this per-domain check. Recognition of one certificate
            // cannot convert every provider-associated domain into target scope.
            if directory.is_none() && witnesses.is_empty() || contradictory {
                if contradictory {
                    verification_conflicts.insert(root.clone());
                    f.conflicting_names.insert(n.clone());
                    f.conflicts.insert(r.key.clone());
                }
                held_families.insert(root.clone());
                unverified_scopes.insert(if shared(&root,&configured){n.clone()}else{root.clone()});
                f.pending.insert(r.key.clone());
                continue;
            }
            let mut candidate = directory.map(|(domain,_)|domain.clone()).unwrap_or_else(||n.clone());
            for witness in &witnesses {
                if witness.host==root { candidate=root.clone(); }
                let source=witness.source();
                f.verification_sources.insert(source.to_string(),source);
            }
            if let Some((domain,organization))=directory {
                let source=json!({"kind":"confirmed_directory","host":domain,"organization":organization});
                f.verification_sources.insert(source.to_string(),source);
            }
            // A target-owned service on another company's domain is evidence
            // only for that exact host. Promotion requires an observed root or
            // wildcard bound to a target leaf, or an already confirmed directory.
            if shared(&root,&configured) {
                if n==root { continue; }
                candidate=n;
            }
            if !current.iter().any(|s|in_scope(&candidate,s)) && !exclusions::matches(&candidate,&blocked) {
                f.scopes.insert(candidate);
                f.reasons.insert(clipped(&r.reason));
            }
        }
    }
    let candidate_keys = families.iter()
        .filter(|(root, f)| !f.scopes.is_empty() && (i.item_key.is_none() || chosen_families.contains(*root)))
        .flat_map(|(_, f)| f.keys.iter().cloned()).collect::<BTreeSet<_>>();
    let live_names = live_candidate_names(c, i.project_id, &i.run_id, &candidate_keys)?;
    let mut removed_no_live_evidence = 0usize;
    for (root, family) in &mut families {
        if i.item_key.is_some() && !chosen_families.contains(root) { continue; }
        family.scopes.retain(|scope| {
            // A root/wildcard grant cannot swallow a known conflicting child.
            if family.conflicting_names.iter().any(|name|in_scope(name,scope)||in_scope(scope,name)){return false;}
            let witnessed = family.keys.iter().any(|key| {
                domains_by_key.get(key).is_some_and(|names| names.iter().any(|name| in_scope(name, scope)))
                    && live_names.get(key).is_some_and(|names| names.iter().any(|name| in_scope(name, scope)))
            });
            if !witnessed { removed_no_live_evidence += 1; }
            witnessed
        });
        if !family.scopes.is_empty() && (i.item_key.is_none() || chosen_families.contains(root)) { digest.update(json!([root,family.scopes,family.verification_sources]).to_string()); }
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
    let mut refs=c.prepare(&format!("SELECT refs.item_key,refs.asset_id,pa.is_deleted FROM asset_seed_review_refs refs JOIN asset_seed_reviews review ON review.id=refs.run_id AND review.project_id=?2 JOIN project_assets pa ON pa.project_id=?2 AND pa.asset_id=refs.asset_id JOIN assets a ON a.id=refs.asset_id WHERE refs.run_id=?1 AND (review.include_isolated=1 OR NOT {ISOLATED}) ORDER BY refs.item_key,refs.asset_id")).map_err(err)?;
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
        groups.push(json!({"root":root,"shared":shared(&root,&configured),"scopes":f.scopes,"observedDomains":f.names.iter().take(12).collect::<Vec<_>>(),"observedCount":f.names.len(),"evidenceGroups":f.keys.len(),"certificateGroups":f.certs.len(),"pendingEvidenceGroups":f.pending.len(),"conflictGroups":f.conflicts.len(),"assetCount":f.assets.len(),"reasons":f.reasons.iter().take(3).collect::<Vec<_>>(),"verification":{"status":"bound_local_evidence","sources":f.verification_sources.values().take(6).collect::<Vec<_>>()}}));
    }
    Ok(
        json!({"runId":i.run_id,"token":format!("{:x}",digest.finalize()),"roots":current,"groups":groups,"totalEvidenceGroups":rows.len(),"pendingFamilyReferences":all_pending,"proposedPendingFamilyReferences":proposed_pending,"removedNoLiveEvidence":removed_no_live_evidence,"scopeExpanded":false,"hunterRequests":0,"verificationPolicyVersion":VERIFICATION_POLICY_VERSION,"heldForVerification":held_families.len(),"removedUnverified":unverified_scopes.len(),"verificationConflicts":verification_conflicts.len(),"networkVerified":false}),
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
        let mut q=tx.prepare(&format!("SELECT DISTINCT a.host,a.link,a.extra_json FROM asset_seed_review_refs refs JOIN asset_seed_reviews review ON review.id=refs.run_id AND review.project_id=?2 JOIN assets a ON a.id=refs.asset_id JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?2 WHERE refs.run_id=?1 AND pa.is_deleted=0 AND (review.include_isolated=1 OR NOT {ISOLATED}) AND refs.item_key IN (SELECT item_key FROM asset_seed_review_items WHERE run_id=?1 AND decision!='rejected' AND EXISTS(SELECT 1 FROM json_each(evidence_json,'$.domains') d WHERE d.value=?3 OR d.value LIKE '%.'||?3))")).map_err(err)?;
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
    tauri::async_runtime::spawn_blocking(move || confirm_and_launch(&path, &m, &input))
        .await
        .map_err(err)?
}

/// UI and chat use the same transaction, live evidence check and activation receipt.
pub(crate) fn confirm_and_launch(
    path: &std::path::Path,
    manager: &enrichment::Manager,
    input: &Selection,
) -> Result<Value, String> {
    let mut c = db::open(path)?;
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
    match manager.dispatch(
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
}


