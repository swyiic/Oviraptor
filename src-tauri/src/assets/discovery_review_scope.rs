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
    conflicting_names: BTreeSet<String>,
    reasons: BTreeSet<String>,
    assets: BTreeSet<i64>,
    verification_sources: BTreeMap<String, Value>,
}
// Scope proposals use independent, domain-bound facts. A model label and a
// historical "accepted" click only express relevance, never ownership of all
// co-located hostnames or SAN association lists.
#[derive(Default)]
struct TargetContext {
    names: BTreeSet<String>,
    official_domains: BTreeMap<String, String>,
}
fn organization_key(value: &str) -> String {
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
fn target_context(c: &Connection, p: i64, organization: &str) -> Result<TargetContext, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Connection, String) {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(r#"CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT);INSERT INTO projects VALUES(1,'移动','active'),(2,'其他','active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,legal_name TEXT,aliases_json TEXT,approved_domains_json TEXT,shared_domains_json TEXT);INSERT INTO asset_ownership_profiles VALUES(1,'中国移动集团有限公司','["中国移动集团"]','[]','[]');CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,company TEXT,extra_json TEXT,content_category TEXT,probe_outcome TEXT DEFAULT '');CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,decision TEXT DEFAULT 'pending');"#).unwrap();
        ensure(&c).unwrap();
        c.execute_batch(TABLES).unwrap();
        c.execute_batch("CREATE TABLE org_entities(id INTEGER PRIMARY KEY,group_id INTEGER,parent_id INTEGER,entity_kind TEXT,legal_name TEXT,region_code TEXT,unified_code TEXT,source TEXT,source_ref TEXT,directory_version INTEGER,normalized_name TEXT,status TEXT);CREATE TABLE org_entity_identifiers(id INTEGER PRIMARY KEY,entity_id INTEGER,kind TEXT,value TEXT,source TEXT,source_ref TEXT);CREATE TABLE project_org_bindings(project_id INTEGER,project_kind TEXT,group_entity_id INTEGER,unit_entity_id INTEGER,binding_version INTEGER,updated_at TEXT);INSERT INTO org_entities VALUES(1,NULL,NULL,'group','中国移动集团有限公司','','','manual','fixture',1,'中国移动集团有限公司','confirmed');INSERT INTO project_org_bindings VALUES(1,'group_ledger',1,NULL,1,'now');INSERT INTO org_entity_identifiers VALUES(1,1,'official_domain','10086.cn','manual','fixture'),(2,1,'official_domain','eos.tenant89.cmecloud.cn','manual','fixture'),(3,1,'official_domain','eos.tenant90.cmecloud.cn','manual','fixture');").unwrap();
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
                    json!({"cert.domain":name,"cert.subject.org":"目标单位","icp.name":"中国移动集团有限公司"}).to_string()
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
    fn verification_fixture(target: &str, assets: Vec<(&str, Value)>) -> (Connection, String) {
        verification_fixture_with_isolated(target,assets,true)
    }
    fn verification_fixture_with_isolated(target: &str, assets: Vec<(&str, Value)>, include_isolated: bool) -> (Connection, String) {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT);INSERT INTO projects VALUES(1,'项目','active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,legal_name TEXT,aliases_json TEXT,approved_domains_json TEXT,shared_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,company TEXT,extra_json TEXT,content_category TEXT,probe_outcome TEXT DEFAULT '');CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,decision TEXT DEFAULT 'pending');").unwrap();
        ensure(&c).unwrap(); c.execute_batch(TABLES).unwrap();
        for (index,(endpoint,extra)) in assets.into_iter().enumerate() {
            let asset=index as i64+1;
            c.execute("INSERT INTO assets VALUES(?1,?2,?2,'',?3,'','')",params![asset,endpoint,extra.to_string()]).unwrap();
            c.execute("INSERT INTO project_assets VALUES(1,?1,0,'pending')",[asset]).unwrap();
        }
        let id=prepare_on(&mut c,1,target,include_isolated,&HashSet::new()).unwrap();
        while !harvest_page(&mut c,1,&id).unwrap() {}
        c.execute("UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=.95,reason='模型声称与目标有关' WHERE run_id=?1",[&id]).unwrap();
        (c,id)
    }
    #[test]
    fn accepted_association_lists_and_explicit_item_clicks_do_not_authorize_third_parties() {
        let (mut c,id)=verification_fixture("中国移动集团及其下属单位",vec![("https://82.156.210.64:9009",json!({"cert.subject.cn":"*.10086.cn","cert.subject.org":"中国移动通信有限公司","cert.domain":["10086.cn","essences.com.cn","grcbank.com"]})),("https://gaxcy.cn",json!({"cert.domain":"gaxcy.cn","cert.subject.org":"中移（苏州）软件技术有限公司"}))]);
        c.execute("UPDATE asset_seed_review_items SET decision='accepted' WHERE kind='certificate_metadata'",[]).unwrap();
        let before:Vec<(String,String,String,String)>=c.prepare("SELECT item_key,decision,advice,evidence_json FROM asset_seed_review_items ORDER BY item_key").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().map(Result::unwrap).collect();
        let v=proposal_on(&c,&input(&id)).unwrap();
        assert!(v["groups"].as_array().unwrap().is_empty());
        assert!(v["heldForVerification"].as_u64().unwrap()>=3);
        assert!(v["removedUnverified"].as_u64().unwrap()>=3);
        assert_eq!(v["networkVerified"],false);
        let key:String=c.query_row("SELECT item_key FROM asset_seed_review_items WHERE kind='certificate_metadata' AND EXISTS(SELECT 1 FROM json_each(evidence_json,'$.domains') WHERE value='grcbank.com')",[],|r|r.get(0)).unwrap();
        let mut selected=input(&id);selected.item_key=Some(key);
        let explicit=proposal_on(&c,&selected).unwrap();
        assert!(explicit["groups"].as_array().unwrap().is_empty());
        assert!(confirm_on(&mut c,&selection(&id,&v,vec!["grcbank.com".into()])).is_err());
        let after:Vec<(String,String,String,String)>=c.prepare("SELECT item_key,decision,advice,evidence_json FROM asset_seed_review_items ORDER BY item_key").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(after,before);assert!(roots(&c,1).unwrap().is_empty());
    }
    #[test]
    fn endpoint_registration_verifies_only_the_named_service_not_its_parent_or_colo_names() {
        let (mut c,id)=verification_fixture("目标单位",vec![("https://mobile.other-company.cn",json!({"cert.domain":["mobile.other-company.cn","bank.example.com"],"icp.name":"目标单位有限公司"}))]);
        let v=proposal_on(&c,&input(&id)).unwrap();
        assert_eq!(v["groups"].as_array().unwrap().len(),1);
        assert_eq!(v["groups"][0]["root"],"other-company.cn");
        assert_eq!(v["groups"][0]["scopes"],json!(["mobile.other-company.cn"]));
        assert_eq!(v["groups"][0]["verification"]["status"],"bound_local_evidence");
        assert_eq!(v["groups"][0]["verification"]["sources"][0]["kind"],"icp");
        assert!(confirm_on(&mut c,&selection(&id,&v,vec!["other-company.cn".into()])).is_err());
        confirm_on(&mut c,&selection(&id,&v,vec!["mobile.other-company.cn".into()])).unwrap();
        assert_eq!(roots(&c,1).unwrap(),vec!["mobile.other-company.cn"]);
    }
    #[test]
    fn parsed_leaf_organization_binds_the_exact_dns_identity_without_provider_metadata_pollution() {
        use base64::Engine;
        let leaf=base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../test-fixtures/tls/cn-only.der"));
        let (c,id)=verification_fixture("AuditFixture",vec![("https://203.0.113.20",json!({"probe_cert_der_base64":leaf,"cert.domain":"foreign.example.com","cert.subject.org":"Unrelated Company"}))]);
        let v=proposal_on(&c,&input(&id)).unwrap();
        assert_eq!(v["groups"].as_array().unwrap().len(),1);
        assert_eq!(v["groups"][0]["scopes"],json!(["fixture.example.test"]));
        assert_eq!(v["groups"][0]["verification"]["sources"][0]["kind"],"tls_subject");
        assert_eq!(v["groups"][0]["verification"]["sources"][0]["organization"],"AuditFixture");
    }
    #[test]
    fn changing_bound_organization_invalidates_confirmation_even_without_reharvesting_review() {
        let (mut c,id)=verification_fixture("目标单位",vec![("https://service.example.com",json!({"icp.name":"目标单位有限公司"}))]);
        let v=proposal_on(&c,&input(&id)).unwrap();
        assert_eq!(v["groups"].as_array().unwrap().len(),1);
        c.execute("UPDATE assets SET extra_json=?1",[json!({"icp.name":"另一家企业有限公司"}).to_string()]).unwrap();
        assert!(confirm_on(&mut c,&selection(&id,&v,vec!["service.example.com".into()])).unwrap_err().contains("已变化"));
        assert!(roots(&c,1).unwrap().is_empty());
    }
    #[test]
    fn unrelated_target_does_not_inherit_profile_domains_or_organization_aliases() {
        let (c,id)=fixture();
        c.execute("UPDATE asset_seed_reviews SET organization='另外一个集团' WHERE id=?1",[&id]).unwrap();
        let v=proposal_on(&c,&input(&id)).unwrap();
        assert!(v["groups"].as_array().unwrap().is_empty());
        assert!(v["heldForVerification"].as_u64().unwrap()>0);
    }
    #[test]
    fn suppliers_in_the_confirmed_directory_are_not_implicitly_group_subsidiaries() {
        let (mut c,_) = fixture();
        c.execute_batch("INSERT INTO org_entities VALUES(2,1,1,'supplier','Supplier Incorporated','','','manual','fixture',1,'supplier','confirmed');INSERT INTO org_entity_identifiers VALUES(4,2,'official_domain','supplier.example.com','manual','fixture');").unwrap();
        c.execute("INSERT INTO assets VALUES(91,'https://supplier.example.com','https://supplier.example.com','',?1,'','')",[json!({"icp.name":"Supplier Incorporated","cert.domain":"supplier.example.com"}).to_string()]).unwrap();
        c.execute("INSERT INTO project_assets VALUES(1,91,0,'pending')",[]).unwrap();
        let id=prepare_on(&mut c,1,"中国移动集团",true,&HashSet::new()).unwrap();
        while !harvest_page(&mut c,1,&id).unwrap() {}
        c.execute("UPDATE asset_seed_review_items SET state='done',advice='retain',confidence=.95 WHERE run_id=?1",[&id]).unwrap();
        let v=proposal_on(&c,&input(&id)).unwrap();
        assert!(!v["groups"].as_array().unwrap().iter().any(|g|g["root"]=="example.com"));
        assert!(v["heldForVerification"].as_u64().unwrap()>0);
    }
    #[test]
    fn target_leaf_and_third_party_registration_are_a_conflict_not_an_automatic_grant() {
        use base64::Engine;
        let leaf=base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../test-fixtures/tls/cn-only.der"));
        let (c,id)=verification_fixture("AuditFixture",vec![("https://fixture.example.test",json!({"probe_cert_der_base64":leaf,"icp.name":"Independent Bank Limited"}))]);
        let v=proposal_on(&c,&input(&id)).unwrap();
        assert!(v["groups"].as_array().unwrap().is_empty());
        assert_eq!(v["verificationConflicts"],1);
        assert_eq!(v["heldForVerification"],1);
    }
    #[test]
    fn current_isolation_changes_remove_sources_from_a_run_that_excludes_isolated_assets() {
        for update in [
            "UPDATE assets SET content_category='custom_rule'",
            "UPDATE assets SET probe_outcome='blocked_content'",
            "UPDATE project_assets SET decision='rejected'",
        ] {
            let (mut c,id)=verification_fixture_with_isolated("目标单位",vec![("https://service.example.com",json!({"icp.name":"目标单位有限公司"}))],false);
            let before=proposal_on(&c,&input(&id)).unwrap();
            assert_eq!(before["groups"].as_array().unwrap().len(),1);
            c.execute(update,[]).unwrap();
            let after=proposal_on(&c,&input(&id)).unwrap();
            assert!(after["groups"].as_array().unwrap().is_empty(),"isolated asset supplied scope after {update}");
            assert!(confirm_on(&mut c,&selection(&id,&before,vec!["service.example.com".into()])).is_err());
            assert!(roots(&c,1).unwrap().is_empty());
            assert!(c.query_row("SELECT COUNT(*) FROM asset_seed_review_refs",[],|r|r.get::<_,i64>(0)).unwrap()>0);
        }
    }
    #[test]
    fn explicitly_including_isolated_records_uses_their_evidence_without_restoring_assets() {
        let (mut c,id)=verification_fixture_with_isolated("目标单位",vec![("https://service.example.com",json!({"icp.name":"目标单位有限公司"}))],true);
        c.execute("UPDATE assets SET content_category='custom_rule',probe_outcome='blocked_content'",[]).unwrap();
        c.execute("UPDATE project_assets SET decision='rejected'",[]).unwrap();
        let v=proposal_on(&c,&input(&id)).unwrap();
        assert_eq!(v["groups"].as_array().unwrap().len(),1);
        confirm_on(&mut c,&selection(&id,&v,vec!["service.example.com".into()])).unwrap();
        let unchanged:(String,String,String,i64)=c.query_row("SELECT a.content_category,a.probe_outcome,pa.decision,pa.is_deleted FROM assets a JOIN project_assets pa ON pa.asset_id=a.id",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(unchanged,("custom_rule".into(),"blocked_content".into(),"rejected".into(),0));
    }
    #[test]
    fn current_advice_projection_can_withhold_old_model_retain_without_rewriting_it() {
        let (c,id)=verification_fixture("目标单位",vec![("https://service.example.com",json!({"icp.name":"目标单位有限公司"}))]);
        assert_eq!(proposal_on(&c,&input(&id)).unwrap()["groups"].as_array().unwrap().len(),1);
        c.execute("UPDATE asset_seed_review_items SET evidence_json=json_set(evidence_json,'$.currentAdvice','evidence','$.currentReason','当前证据不足') WHERE run_id=?1",[&id]).unwrap();
        assert!(proposal_on(&c,&input(&id)).unwrap()["groups"].as_array().unwrap().is_empty());
        assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_seed_review_items WHERE advice!='retain'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
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
    fn accepted_recycled_families_are_omitted_without_blocking_live_batch_or_rewriting_history() {
        let (mut c, id) = fixture();
        c.execute("UPDATE asset_seed_review_items SET decision='accepted' WHERE run_id=?1 AND kind='certificate_metadata'", [&id]).unwrap();
        c.execute("UPDATE project_assets SET is_deleted=1 WHERE asset_id!=90", []).unwrap();
        let before: Vec<(String,String,String,i64)> = c.prepare("SELECT item_key,decision,evidence_json,revision FROM asset_seed_review_items WHERE run_id=?1 ORDER BY item_key").unwrap()
            .query_map([&id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().map(Result::unwrap).collect();
        let v = proposal_on(&c, &input(&id)).unwrap();
        assert_eq!(v["removedNoLiveEvidence"], 2);
        assert_eq!(v["groups"].as_array().unwrap().len(), 1);
        assert_eq!(v["groups"][0]["scopes"], json!(["eos.tenant90.cmecloud.cn"]));
        let hosts = v["groups"].as_array().unwrap().iter().flat_map(|g| g["scopes"].as_array().unwrap()).map(|n| n.as_str().unwrap().to_string()).collect();
        confirm_on(&mut c, &selection(&id, &v, hosts)).unwrap();
        assert_eq!(roots(&c, 1).unwrap(), vec!["eos.tenant90.cmecloud.cn"]);
        let after: Vec<(String,String,String,i64)> = c.prepare("SELECT item_key,decision,evidence_json,revision FROM asset_seed_review_items WHERE run_id=?1 ORDER BY item_key").unwrap()
            .query_map([&id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(after, before);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM project_assets WHERE is_deleted=1", [], |r| r.get::<_,i64>(0)).unwrap(), 89);
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
        assert!(v["totalEvidenceGroups"].as_u64().unwrap() > 10000);
        assert_eq!(v["verificationPolicyVersion"], VERIFICATION_POLICY_VERSION);
        assert_eq!(v["networkVerified"], false);
        for excluded in ["essences.com.cn", "gaxcy.cn", "grcbank.com"] {
            assert!(!v["groups"].as_array().unwrap().iter().any(|g|g["root"]==excluded), "unverified third-party family suggested: {excluded}");
        }
        assert_eq!(roots(&c, 3).unwrap(), before);
        for group in v["groups"].as_array().unwrap() {
            assert_eq!(group["verification"]["status"], "bound_local_evidence");
            assert!(!group["verification"]["sources"].as_array().unwrap().is_empty());
            if group["shared"] == true {
                assert!(!group["scopes"].as_array().unwrap().contains(&group["root"]));
            }
        }
        std::fs::write(output, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    }
}

#[cfg(test)]
mod passive_tests {
    use super::*;
    #[test]
    fn public_current_leaf_can_propose_only_its_target_names_not_banks_or_other_runs() {
        let mut c=Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT);INSERT INTO projects VALUES(1,'项目','active'),(2,'别的项目','active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,legal_name TEXT,aliases_json TEXT,approved_domains_json TEXT,shared_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,company TEXT,extra_json TEXT,content_category TEXT,probe_outcome TEXT DEFAULT '');CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,decision TEXT DEFAULT 'pending');").unwrap();
        ensure(&c).unwrap();c.execute_batch(TABLES).unwrap();
        c.execute("INSERT INTO assets VALUES(1,'https://mail.example.test','https://mail.example.test','',?1,'','')",[json!({"cert.domain":"mail.example.test bank.test","cert.subject.cn":"*.example.test","cert.subject.org":"Target Group"}).to_string()]).unwrap();
        c.execute("INSERT INTO project_assets VALUES(1,1,0,'pending')",[]).unwrap();
        let id=prepare_on(&mut c,1,"Target Group",true,&HashSet::new()).unwrap();while !harvest_page(&mut c,1,&id).unwrap(){}
        c.execute("UPDATE asset_seed_review_items SET state='deferred',advice='evidence',confidence=0,reason='缺少独立证据' WHERE run_id=?1",[&id]).unwrap();
        let input=ProposalInput{project_id:1,run_id:id.clone(),item_key:None};
        assert!(proposal_on(&c,&input).unwrap()["groups"].as_array().unwrap().is_empty());
        // The complete cold-start path does not manufacture a model retain:
        // evidence -> public queue -> exact DER persistence -> triage -> proposal.
        use super::super::super::passive_evidence as passive;
        assert_eq!(passive::enqueue_review_candidates(&c,1,&id).unwrap(),1);
        let search=passive::claim(&mut c,1,&id).unwrap().unwrap();
        assert_eq!(search.target,"example.test");
        assert!(passive::finish(&mut c,1,&id,&search,Ok(json!({"certificateIds":[19]}))).unwrap());
        c.execute("UPDATE asset_passive_tasks SET updated_at=datetime('now','-1 hour')",[]).unwrap();
        let download=passive::claim(&mut c,1,&id).unwrap().unwrap();
        assert_eq!(download.kind,"ct_certificate");
        let cert=crate::certificates::from_text(include_str!("../../test-fixtures/ct/public-leaf.pem")).unwrap();
        assert!(passive::finish(&mut c,1,&id,&download,Ok(json!({"derBase64":cert.der_base64,"organizations":["Bank Inc"],"names":["bank.test"]}))).unwrap());
        triage::run(&mut c,1,&id).unwrap();
        assert_eq!(c.query_row("SELECT requests FROM asset_seed_reviews WHERE id=?1",[&id],|r|r.get::<_,i64>(0)).unwrap(),0);
        let model_retains:i64=c.query_row("SELECT count(*) FROM asset_seed_review_items WHERE run_id=?1 AND state='done' AND advice='retain'",[&id],|r|r.get(0)).unwrap();
        assert_eq!(model_retains,0);
        // Derived cache corruption also cannot replace Subject/SAN from DER.
        c.execute("UPDATE asset_passive_certificates SET names_json='[\"bank.test\"]',organizations_json='[\"Another Group\"]'",[]).unwrap();
        let v=proposal_on(&c,&input).unwrap();
        let text=v["groups"].to_string();assert!(text.contains("example.test"));assert!(!text.contains("bank.test"));assert!(text.contains("public_ct_subject"));assert!(text.contains("https://crt.sh/?d=19"));
        assert!(roots(&c,1).unwrap().is_empty());
        let mut conflict:Value=c.query_row("SELECT extra_json FROM assets WHERE id=1",[],|r|r.get::<_,String>(0)).map(|raw|serde_json::from_str(&raw).unwrap()).unwrap();
        conflict["icp.name"]=json!("Another Bank");
        c.execute("UPDATE assets SET extra_json=?1 WHERE id=1",[conflict.to_string()]).unwrap();
        let conflicting=proposal_on(&c,&input).unwrap();
        assert_eq!(conflicting["verificationConflicts"],1);
        assert!(conflicting["groups"].as_array().unwrap().is_empty());
        c.execute("UPDATE assets SET extra_json='{}' WHERE id=1",[]).unwrap();
        c.execute("UPDATE asset_seed_review_items SET decision='rejected' WHERE run_id=?1",[&id]).unwrap();
        assert!(proposal_on(&c,&input).unwrap()["groups"].as_array().unwrap().is_empty());
        c.execute("UPDATE asset_seed_review_items SET decision='pending' WHERE run_id=?1",[&id]).unwrap();
        c.execute("UPDATE project_assets SET is_deleted=1",[]).unwrap();
        assert!(proposal_on(&c,&input).unwrap()["groups"].as_array().unwrap().is_empty());
        c.execute("UPDATE project_assets SET is_deleted=0",[]).unwrap();
        c.execute("UPDATE asset_seed_reviews SET organization='Another Group' WHERE id=?1",[&id]).unwrap();
        assert!(proposal_on(&c,&input).unwrap()["groups"].as_array().unwrap().is_empty());
        c.execute("UPDATE asset_seed_reviews SET organization='Target Group' WHERE id=?1",[&id]).unwrap();
        c.execute("UPDATE asset_passive_certificates SET not_after=1",[]).unwrap();
        assert!(proposal_on(&c,&input).unwrap()["groups"].as_array().unwrap().is_empty());
    }
}
