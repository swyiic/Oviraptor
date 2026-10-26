//! Project-scoped, append-preserving seed provenance. Only the execution owner
//! initializes/synchronizes this index; read/detail never create tables or run
//! discovery. A model label is not ownership verification or an accuracy score.
use super::*;
use std::collections::{BTreeMap, HashMap, HashSet};

#[cfg(test)]
thread_local! {static GATE_CALLS:std::cell::RefCell<HashMap<String,usize>>=std::cell::RefCell::new(HashMap::new());}
const BATCH: usize = 500;
const PROOF_TTL_SECONDS: i64 = 15 * 60;
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS asset_project_seed_state(project_id INTEGER PRIMARY KEY REFERENCES projects(id),initialized INTEGER NOT NULL DEFAULT 0,phase INTEGER NOT NULL DEFAULT 0,cursor INTEGER NOT NULL DEFAULT 0,cutoffs_json TEXT NOT NULL,change_cursor INTEGER NOT NULL DEFAULT 0,policy_cursor INTEGER NOT NULL DEFAULT -1,proof_revision INTEGER NOT NULL DEFAULT 0,proof_cache_revision INTEGER NOT NULL DEFAULT -1,proof_cache_at INTEGER NOT NULL DEFAULT 0,proofs_json TEXT NOT NULL DEFAULT '{}',created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));
CREATE TABLE IF NOT EXISTS asset_project_seeds(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),kind TEXT NOT NULL,value TEXT NOT NULL,wildcard INTEGER NOT NULL DEFAULT 0,first_round INTEGER,last_round INTEGER,first_seen_at TEXT,last_seen_at TEXT NOT NULL,historical_baseline INTEGER NOT NULL,trust_status TEXT NOT NULL DEFAULT 'needs_evidence',trust_reason TEXT NOT NULL DEFAULT '尚无与当前目标绑定的独立核验证据',hunter_eligible INTEGER NOT NULL DEFAULT 0,verified_evidence INTEGER NOT NULL DEFAULT 0,total_evidence INTEGER NOT NULL DEFAULT 0,trust_checked_at TEXT,gate_revision INTEGER NOT NULL DEFAULT -1,UNIQUE(project_id,kind,value,wildcard));
CREATE INDEX IF NOT EXISTS asset_project_seed_list ON asset_project_seeds(project_id,trust_status,kind,id);
CREATE TABLE IF NOT EXISTS asset_project_seed_observations(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL,seed_id INTEGER NOT NULL REFERENCES asset_project_seeds(id),event_key TEXT NOT NULL,source_type TEXT NOT NULL,source_ref TEXT NOT NULL,operation_id TEXT,round INTEGER,seen_at TEXT NOT NULL,historical_baseline INTEGER NOT NULL,asset_id INTEGER,query_id TEXT,query_kind TEXT,query_value TEXT,page INTEGER,spent_points INTEGER,cost_unknown INTEGER NOT NULL DEFAULT 0,asset_count INTEGER,evidence_json TEXT NOT NULL,UNIQUE(project_id,event_key));
CREATE INDEX IF NOT EXISTS asset_project_seed_history ON asset_project_seed_observations(project_id,seed_id,id);
CREATE INDEX IF NOT EXISTS asset_project_seed_asset ON asset_project_seed_observations(project_id,asset_id,seed_id);
CREATE TABLE IF NOT EXISTS asset_project_seed_changes(id INTEGER PRIMARY KEY AUTOINCREMENT,project_id INTEGER NOT NULL,source_table TEXT NOT NULL,source_key TEXT NOT NULL,action TEXT NOT NULL,payload_json TEXT NOT NULL,seen_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')));
CREATE INDEX IF NOT EXISTS asset_project_seed_changes_project ON asset_project_seed_changes(project_id,id);
";

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Query {
    pub project_id: i64,
    #[serde(default)]
    pub search: String,
    #[serde(default = "all")]
    pub kind: String,
    #[serde(default = "all")]
    pub trust: String,
    #[serde(default = "first")]
    pub page: i64,
    #[serde(default = "list_size")]
    pub page_size: i64,
}
fn all() -> String {
    "all".into()
}
fn first() -> i64 {
    1
}
fn list_size() -> i64 {
    50
}
fn has(c: &Connection, t: &str) -> Result<bool, String> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [t],
        |r| r.get(0),
    )
    .map_err(err)
}
fn current_time(c: &Connection) -> Result<String, String> {
    c.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
        r.get(0)
    })
    .map_err(err)
}
fn project_read(c: &Connection, p: i64) -> Result<(), String> {
    if p < 1
        || !c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
                [p],
                |r| r.get::<_, bool>(0),
            )
            .map_err(err)?
    {
        return Err("seed_ledger_project_unavailable".into());
    }
    Ok(())
}
fn clipped(v: &str, n: usize) -> String {
    let mut out = String::new();
    for ch in v.chars().filter(|c| !c.is_control()) {
        if out.len() + ch.len_utf8() > n {
            break;
        }
        out.push(ch)
    }
    out
}
fn normalized(kind: &str, value: &str) -> Option<(String, String)> {
    if kind == "fingerprint" || kind == "certificate_sha256" {
        hunter_expansion::fingerprint(value).map(|s| ("fingerprint".into(), s))
    } else {
        host(value).map(|h| {
            (
                if kind == "cname" || kind == "dns_cname" {
                    "cname"
                } else {
                    "domain"
                }
                .into(),
                h,
            )
        })
    }
}

// Each SELECT yields (stable source rowid, small metadata JSON). No response
// bodies, account records, credentials or certificate DER enter this journal.
const SOURCES: &[&str] = &[
    "asset_discovery_seeds",
    "asset_discovery_seed_evidence",
    "asset_passive_certificate_refs",
    "hunter_expansions",
    "hunter_expansion_pages",
    "hunter_expansion_links",
    "asset_enrichment_observations",
];
fn source_sql(table: &str) -> Option<String> {
    let sql=match table{
        "asset_discovery_seeds"|"asset_discovery_seed_evidence"=>format!("SELECT s.rowid,json_object('value',s.host,'source',s.source,'reference',s.source_ref,'wildcard',s.wildcard) FROM {table} s WHERE s.project_id=?1 AND s.rowid>?2 AND s.rowid<=?3 ORDER BY s.rowid LIMIT ?4"),
        "asset_passive_certificate_refs"=>"SELECT s.rowid,json_object('value',s.host,'source','public_ct','reference','ct:'||s.certificate_id||':'||s.host,'certificateId',s.certificate_id,'certificateSha256',cert.sha256,'sourceUrl',cert.source_url,'certificateAvailable',cert.certificate_id IS NOT NULL) FROM asset_passive_certificate_refs s LEFT JOIN asset_passive_certificates cert ON cert.certificate_id=s.certificate_id WHERE s.project_id=?1 AND s.rowid>?2 AND s.rowid<=?3 ORDER BY s.rowid LIMIT ?4".into(),
        "hunter_expansions"=>"SELECT s.rowid,json_object('value',s.target,'kind',s.kind,'source','hunter_query','reference','hunter:'||s.id,'queryId',s.id,'queryKind',s.kind,'queryValue',s.target,'queryState',s.status,'seenAt',s.created_at,'spentPoints',s.spent,'costUnknown',s.cost_unknown,'assetCount',s.unique_rows) FROM hunter_expansions s WHERE s.project_id=?1 AND s.rowid>?2 AND s.rowid<=?3 ORDER BY s.rowid LIMIT ?4".into(),
        "hunter_expansion_pages"=>"SELECT s.rowid,json_object('value',h.target,'kind',h.kind,'source','hunter_page','reference','hunter:'||h.id||':page:'||s.page,'queryId',h.id,'queryKind',h.kind,'queryValue',h.target,'queryState',s.state,'page',s.page,'spentPoints',s.cost,'costUnknown',s.cost IS NULL OR s.accounted=0,'assetCount',s.accepted,'returned',s.raw_count,'accepted',s.accepted,'rejected',s.rejected,'inserted',json_extract(s.import_json,'$.result.inserted'),'updated',json_extract(s.import_json,'$.result.updated')) FROM hunter_expansion_pages s JOIN hunter_expansions h ON h.id=s.query_id WHERE h.project_id=?1 AND s.rowid>?2 AND s.rowid<=?3 ORDER BY s.rowid LIMIT ?4".into(),
        "hunter_expansion_links"=>"SELECT s.rowid,json_object('value',s.endpoint,'kind','domain','domain',json_extract(s.evidence_json,'$.domain'),'certificateSha256',json_extract(s.evidence_json,'$.cert_sha256'),'source','hunter_result','reference','hunter:'||h.id||':asset:'||s.endpoint,'queryId',h.id,'queryKind',h.kind,'queryValue',h.target,'page',s.page,'assetCount',1) FROM hunter_expansion_links s JOIN hunter_expansions h ON h.id=s.query_id AND h.project_id=s.project_id WHERE s.project_id=?1 AND s.rowid>?2 AND s.rowid<=?3 ORDER BY s.rowid LIMIT ?4".into(),
        "asset_enrichment_observations"=>"SELECT s.rowid,json_object('value',s.target,'kind','domain','certificateSha256',json_extract(s.payload_json,'$.sha256'),'source','free_'||s.kind,'reference','free:'||s.id,'seenAt',strftime('%Y-%m-%dT%H:%M:%SZ',s.last_seen,'unixepoch'),'evidenceKind',s.kind) FROM asset_enrichment_observations s WHERE s.project_id=?1 AND s.rowid>?2 AND s.rowid<=?3 ORDER BY s.rowid LIMIT ?4".into(),
        _=>return None,
    };
    Some(sql)
}
fn source_ready(c: &Connection, t: &str) -> Result<bool, String> {
    Ok(has(c, t)?
        && (!matches!(t, "hunter_expansion_pages" | "hunter_expansion_links")
            || has(c, "hunter_expansions")?)
        && (t != "asset_passive_certificate_refs" || has(c, "asset_passive_certificates")?))
}
fn install_triggers(c: &Connection, table: &str) -> Result<(), String> {
    if !source_ready(c, table)? {
        return Ok(());
    }
    // Trigger expressions use only names from this constant source allowlist.
    let select = source_sql(table).ok_or("seed_ledger_source_invalid")?;
    let projection = select
        .split(" FROM ")
        .next()
        .unwrap()
        .trim_start_matches("SELECT s.rowid,")
        .to_owned();
    for (action, newold) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")] {
        let name = format!("project_seed_{}_{}", table, action.to_ascii_lowercase());
        let present: Option<String> = c
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [&name],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if let Some(sql) = present {
            let needs_upgrade = (table == "hunter_expansion_links"
                && !sql.contains(&format!("h.project_id={newold}.project_id")))
                || (table == "hunter_expansion_pages"
                    && action == "UPDATE"
                    && !sql.contains("OLD.import_json"));
            if !needs_upgrade {
                continue;
            }
            c.execute_batch(&format!("DROP TRIGGER {name}"))
                .map_err(err)?;
        }
        let payload = projection.replace("s.", &format!("{newold}."));
        let (project,from)=match table{
            "hunter_expansion_pages"=>("h.project_id".into(),format!(" FROM hunter_expansions h WHERE h.id={newold}.query_id")),
            "hunter_expansion_links"=>("h.project_id".into(),format!(" FROM hunter_expansions h WHERE h.id={newold}.query_id AND h.project_id={newold}.project_id")),
            "asset_passive_certificate_refs"=>(format!("{newold}.project_id"),format!(" FROM (SELECT 1) LEFT JOIN asset_passive_certificates cert ON cert.certificate_id={newold}.certificate_id")),
            _=>(format!("{newold}.project_id"),String::new()),
        };
        let when = if action == "UPDATE" {
            match table{
                "asset_discovery_seeds"|"asset_discovery_seed_evidence"=>" WHEN OLD.host IS NOT NEW.host OR OLD.source IS NOT NEW.source OR OLD.source_ref IS NOT NEW.source_ref OR OLD.wildcard IS NOT NEW.wildcard",
                "hunter_expansions"=>" WHEN OLD.status IS NOT NEW.status OR OLD.spent IS NOT NEW.spent OR OLD.cost_unknown IS NOT NEW.cost_unknown OR OLD.received IS NOT NEW.received OR OLD.unique_rows IS NOT NEW.unique_rows",
                "hunter_expansion_pages"=>" WHEN OLD.state IS NOT NEW.state OR OLD.cost IS NOT NEW.cost OR OLD.accounted IS NOT NEW.accounted OR OLD.raw_count IS NOT NEW.raw_count OR OLD.accepted IS NOT NEW.accepted OR OLD.rejected IS NOT NEW.rejected OR OLD.import_json IS NOT NEW.import_json",
                "asset_enrichment_observations"=>" WHEN OLD.last_seen IS NOT NEW.last_seen OR OLD.content_hash IS NOT NEW.content_hash",
                _=>"",
            }
        } else {
            ""
        };
        c.execute_batch(&format!("CREATE TRIGGER {name} AFTER {action} ON {table}{when} BEGIN INSERT INTO asset_project_seed_changes(project_id,source_table,source_key,action,payload_json) SELECT {project},'{table}',CAST({newold}.rowid AS TEXT),'{action}',{payload}{from}; END;")).map_err(err)?;
    }
    Ok(())
}
pub(crate) fn ensure(c: &Connection) -> Result<(), String> {
    if !has(c, "asset_project_seeds")? {
        c.execute_batch(SCHEMA).map_err(err)?;
    }
    for (table, column, definition) in [
        ("asset_project_seeds", "trust_checked_at", "TEXT"),
        (
            "asset_project_seeds",
            "gate_revision",
            "INTEGER NOT NULL DEFAULT -1",
        ),
        ("asset_project_seed_observations", "operation_id", "TEXT"),
        (
            "asset_project_seed_state",
            "proof_revision",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "asset_project_seed_state",
            "proof_cache_revision",
            "INTEGER NOT NULL DEFAULT -1",
        ),
        (
            "asset_project_seed_state",
            "proof_cache_at",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "asset_project_seed_state",
            "proofs_json",
            "TEXT NOT NULL DEFAULT '{}'",
        ),
    ] {
        let cols: Vec<String> = c
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(err)?
            .query_map([], |r| r.get(1))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        if !cols.iter().any(|n| n == column) {
            c.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))
            .map_err(err)?;
        }
    }
    for table in SOURCES {
        install_triggers(c, table)?;
    }
    // A newly downloaded DER completes refs that predated the cache entry.
    if has(c, "asset_passive_certificates")? && has(c, "asset_passive_certificate_refs")? {
        let name = "project_seed_public_certificate";
        if !c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='trigger' AND name=?1)",
                [name],
                |r| r.get::<_, bool>(0),
            )
            .map_err(err)?
        {
            c.execute_batch("CREATE TRIGGER project_seed_public_certificate AFTER INSERT ON asset_passive_certificates BEGIN INSERT INTO asset_project_seed_changes(project_id,source_table,source_key,action,payload_json) SELECT r.project_id,'asset_passive_certificate_refs',CAST(r.rowid AS TEXT),'UPDATE',json_object('value',r.host,'source','public_ct','reference','ct:'||r.certificate_id||':'||r.host,'certificateId',r.certificate_id,'certificateSha256',NEW.sha256,'sourceUrl',NEW.source_url,'certificateAvailable',1) FROM asset_passive_certificate_refs r WHERE r.certificate_id=NEW.certificate_id; END;").map_err(err)?;
        }
    }
    // Raw evidence/membership changes invalidate the local verification cache,
    // without manufacturing a new discovery or discarding any old receipt.
    if has(c, "assets")? && has(c, "project_assets")? {
        let cols: HashSet<String> = c
            .prepare("PRAGMA table_info(assets)")
            .map_err(err)?
            .query_map([], |r| r.get(1))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        let changed = [
            "extra_json",
            "host",
            "link",
            "company",
            "content_category",
            "probe_outcome",
        ]
        .into_iter()
        .filter(|n| cols.contains(*n))
        .map(|n| format!("OLD.{n} IS NOT NEW.{n}"))
        .collect::<Vec<_>>()
        .join(" OR ");
        if !changed.is_empty() {
            c.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS project_seed_asset_evidence_v2 AFTER UPDATE ON assets WHEN {changed} BEGIN UPDATE asset_project_seed_state SET policy_cursor=0,proof_revision=proof_revision+1 WHERE project_id IN (SELECT project_id FROM project_assets WHERE asset_id=NEW.id); END;")).map_err(err)?;
        }
        c.execute_batch("CREATE TRIGGER IF NOT EXISTS project_seed_asset_deleted_v2 AFTER DELETE ON assets BEGIN UPDATE asset_project_seed_state SET policy_cursor=0,proof_revision=proof_revision+1 WHERE project_id IN (SELECT project_id FROM project_assets WHERE asset_id=OLD.id); END;").map_err(err)?;
        for action in ["INSERT", "UPDATE", "DELETE"] {
            let alias = if action == "DELETE" { "OLD" } else { "NEW" };
            c.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS project_seed_membership_v2_{} AFTER {action} ON project_assets BEGIN UPDATE asset_project_seed_state SET policy_cursor=0,proof_revision=proof_revision+1 WHERE project_id={alias}.project_id; END;",action.to_ascii_lowercase())).map_err(err)?;
        }
    }
    if has(c, "project_assets")? && has(c, "asset_discovery_seed_evidence")? {
        let cols: Vec<String> = c
            .prepare("PRAGMA table_info(project_assets)")
            .map_err(err)?
            .query_map([], |r| r.get(1))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        if cols.iter().any(|n| n == "last_run_id") {
            c.execute_batch("CREATE TRIGGER IF NOT EXISTS project_seed_collection_receipt AFTER UPDATE ON project_assets WHEN NEW.last_run_id IS NOT NULL AND OLD.last_run_id IS NOT NEW.last_run_id BEGIN INSERT INTO asset_project_seed_changes(project_id,source_table,source_key,action,payload_json) SELECT NEW.project_id,'project_assets',CAST(NEW.asset_id AS TEXT),'UPDATE',json_object('value',e.host,'source',e.source,'reference',e.source_ref,'wildcard',e.wildcard,'collectionRunId',NEW.last_run_id,'queryId','collection-run:'||NEW.last_run_id,'queryKind','collection_run','assetCount',1) FROM asset_discovery_seed_evidence e WHERE e.project_id=NEW.project_id AND e.source_ref='asset:'||NEW.asset_id; END;").map_err(err)?;
        }
    }
    if has(c, "asset_seed_review_items")? && has(c, "asset_seed_assists")? {
        c.execute_batch("CREATE TRIGGER IF NOT EXISTS project_seed_review_evidence AFTER UPDATE ON asset_seed_review_items WHEN OLD.evidence_json IS NOT NEW.evidence_json OR OLD.state IS NOT NEW.state OR OLD.advice IS NOT NEW.advice OR OLD.decision IS NOT NEW.decision OR OLD.revision IS NOT NEW.revision BEGIN UPDATE asset_project_seed_state SET policy_cursor=0,proof_revision=proof_revision+1 WHERE project_id IN (SELECT project_id FROM asset_seed_assists WHERE run_id=NEW.run_id); END;").map_err(err)?;
    }
    if has(c, "asset_passive_certificates")? && has(c, "asset_passive_certificate_refs")? {
        c.execute_batch("CREATE TRIGGER IF NOT EXISTS project_seed_public_proof AFTER INSERT ON asset_passive_certificates BEGIN UPDATE asset_project_seed_state SET policy_cursor=0,proof_revision=proof_revision+1 WHERE project_id IN (SELECT project_id FROM asset_passive_certificate_refs WHERE certificate_id=NEW.certificate_id); END;").map_err(err)?;
    }
    // Policy changes invalidate derived trust, without inventing a discovery.
    for table in [
        "asset_discovery_scopes",
        "asset_discovery_domain_exclusions",
        "asset_seed_assists",
        "asset_passive_certificate_refs",
    ] {
        if !has(c, table)? {
            continue;
        }
        for action in ["INSERT", "UPDATE", "DELETE"] {
            let name = format!(
                "project_seed_policy_v2_{}_{}",
                table,
                action.to_ascii_lowercase()
            );
            if c.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='trigger' AND name=?1)",
                [&name],
                |r| r.get::<_, bool>(0),
            )
            .map_err(err)?
            {
                continue;
            }
            let alias = if action == "DELETE" { "OLD" } else { "NEW" };
            let when = if action == "UPDATE" && table == "asset_seed_assists" {
                " WHEN OLD.proposal_json IS NOT NEW.proposal_json OR OLD.organization IS NOT NEW.organization"
            } else {
                ""
            };
            c.execute_batch(&format!("CREATE TRIGGER {name} AFTER {action} ON {table}{when} BEGIN UPDATE asset_project_seed_state SET policy_cursor=0,proof_revision=proof_revision+1 WHERE project_id={alias}.project_id; END;")).map_err(err)?;
        }
    }
    c.execute_batch("CREATE INDEX IF NOT EXISTS asset_project_seed_local_asset ON asset_project_seed_observations(project_id,seed_id,asset_id)").map_err(err)?;
    install_context_triggers(c)?;
    Ok(())
}

// Organization context changes invalidate only derived trust. Project-bound
// context is isolated; shared directory tables invalidate ledger states only,
// never source assets, observations, network scope or authorization.
fn install_context_triggers(c: &Connection) -> Result<(), String> {
    for table in [
        "project_org_bindings",
        "asset_ownership_profiles",
        "asset_discovery_organization_evidence",
        "org_entities",
        "org_entity_identifiers",
    ] {
        if !has(c, table)? {
            continue;
        }
        let columns: Vec<String> = c
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(err)?
            .query_map([], |r| r.get(1))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        let per_project = columns.iter().any(|n| n == "project_id");
        for action in ["INSERT", "UPDATE", "DELETE"] {
            let name = format!(
                "project_seed_context_{}_{}",
                table,
                action.to_ascii_lowercase()
            );
            let filter = if !per_project {
                String::new()
            } else {
                match action {
                    "INSERT" => " WHERE project_id=NEW.project_id",
                    "DELETE" => " WHERE project_id=OLD.project_id",
                    _ => " WHERE project_id=OLD.project_id OR project_id=NEW.project_id",
                }
                .into()
            };
            c.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS {name} AFTER {action} ON {table} BEGIN UPDATE asset_project_seed_state SET policy_cursor=0,proof_revision=proof_revision+1{filter}; END;")).map_err(err)?;
        }
    }
    Ok(())
}
fn expire_proofs(c: &Connection, p: i64) -> Result<(), String> {
    // One expiry transition schedules one bounded pass. Its first batch builds
    // a current local projection and advances proof_cache_at; later batches
    // reuse that snapshot instead of projecting a large project repeatedly.
    c.execute("UPDATE asset_project_seed_state SET policy_cursor=0,proof_revision=proof_revision+1 WHERE project_id=?1 AND proof_cache_at<=CAST(strftime('%s','now') AS INTEGER)-?2 AND EXISTS(SELECT 1 FROM asset_project_seeds WHERE project_id=?1)",params![p,PROOF_TTL_SECONDS]).map_err(err)?;
    Ok(())
}

fn create_project(c: &Connection, p: i64) -> Result<(), String> {
    project_read(c, p)?;
    ensure(c)?;
    if c.query_row(
        "SELECT EXISTS(SELECT 1 FROM asset_project_seed_state WHERE project_id=?1)",
        [p],
        |r| r.get::<_, bool>(0),
    )
    .map_err(err)?
    {
        return Ok(());
    }
    let tx = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let mut limits = BTreeMap::<String, i64>::new();
    for table in SOURCES {
        let max = if source_ready(&tx, table)? {
            let sql = source_sql(table).unwrap();
            let from = sql
                .split(" FROM ")
                .nth(1)
                .ok_or("seed_ledger_source_invalid")?
                .split(" AND s.rowid>")
                .next()
                .unwrap();
            tx.query_row(
                &format!("SELECT COALESCE(MAX(s.rowid),0) FROM {from}"),
                [p],
                |r| r.get(0),
            )
            .map_err(err)?
        } else {
            0
        };
        limits.insert((*table).into(), max);
    }
    let watermark: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(id),0) FROM asset_project_seed_changes WHERE project_id=?1",
            [p],
            |r| r.get(0),
        )
        .map_err(err)?;
    tx.execute("INSERT OR IGNORE INTO asset_project_seed_state(project_id,cutoffs_json,change_cursor) VALUES(?1,?2,?3)",params![p,json!(limits).to_string(),watermark]).map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(())
}

#[derive(Default)]
struct Added {
    seeds: usize,
    observations: usize,
    touched: HashSet<i64>,
    providers: HashMap<(String, String), (Vec<String>, Option<i64>)>,
}
fn provider_sources(
    c: &Connection,
    p: i64,
    reference: &str,
    source: &str,
) -> Result<(Vec<String>, Option<i64>), String> {
    let asset = reference
        .strip_prefix("asset:")
        .and_then(|s| s.parse::<i64>().ok());
    let mut providers = BTreeSet::new();
    if let Some(a) = asset {
        let fields:Option<(Option<String>,Option<String>)>=c.prepare_cached("SELECT CAST(CASE WHEN json_valid(a.extra_json) THEN json_extract(a.extra_json,'$.collection_provider') END AS TEXT),CAST(CASE WHEN json_valid(a.extra_json) THEN json_extract(a.extra_json,'$.discovery_sources_json') END AS TEXT) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?1 WHERE a.id=?2").map_err(err)?.query_row(params![p,a],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
        if let Some((provider, sources)) = fields {
            if let Some(s) = provider.filter(|s| matches!(s.as_str(), "fofa" | "hunter")) {
                providers.insert(s);
            }
            if let Some(value) = sources.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()) {
                for row in value.as_array().into_iter().flatten() {
                    if let Some(s) = row["source"]
                        .as_str()
                        .filter(|s| matches!(*s, "fofa" | "hunter"))
                    {
                        providers.insert(s.into());
                    }
                }
            }
        }
    }
    if providers.is_empty() {
        providers.insert(
            match source {
                "public_ct" => "public_ct",
                "certificate_transparency" => "free_ct",
                "dns_cname" | "cname" => "free_dns",
                s if s.starts_with("hunter_") || s.starts_with("free_") => s,
                _ => "history",
            }
            .into(),
        );
    }
    Ok((providers.into_iter().collect(), asset))
}
fn summary_evidence(raw: &Value, action: &str) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "source",
        "queryState",
        "evidenceKind",
        "certificateSha256",
        "certificateId",
        "certificateAvailable",
        "collectionRunId",
        "returned",
        "accepted",
        "rejected",
        "inserted",
        "updated",
    ] {
        if let Some(v) = raw.get(key) {
            if v.is_number() || v.is_boolean() || v.is_null() {
                out.insert(key.into(), v.clone());
            } else if let Some(s) = v.as_str() {
                out.insert(key.into(), json!(clipped(s, 96)));
            }
        }
    }
    if let Some(url) = raw["sourceUrl"]
        .as_str()
        .filter(|s| s.starts_with("https://crt.sh/"))
    {
        out.insert("sourceUrl".into(), json!(clipped(url, 160)));
    }
    out.insert("action".into(), json!(action));
    Value::Object(out)
}
fn apply(
    c: &Connection,
    p: i64,
    raw: &Value,
    seen: &str,
    round: Option<i64>,
    operation: &str,
    baseline: bool,
    action: &str,
    added: &mut Added,
) -> Result<(), String> {
    let original_lineage: Option<(String, Option<i64>)> = if raw["queryId"].is_string()
        && has(c, "asset_discovery_pipeline_queries")?
    {
        let cols: Vec<String> = c
            .prepare("PRAGMA table_info(asset_discovery_pipeline_queries)")
            .map_err(err)?
            .query_map([], |r| r.get(1))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        if cols.iter().any(|n| n == "round_number") {
            c.query_row("SELECT operation_id,round_number FROM asset_discovery_pipeline_queries WHERE project_id=?1 AND query_id=?2 ORDER BY CASE WHEN round_number IS NULL THEN 1 ELSE 0 END,round_number LIMIT 1",params![p,raw["queryId"].as_str()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?
        } else {
            None
        }
    } else {
        None
    };
    // Baseline history remains unknown even if a legacy query happens to have
    // been linked later. New late receipts belong to their original wave.
    let round = if baseline {
        None
    } else {
        original_lineage.as_ref().map(|(_, r)| *r).unwrap_or(round)
    };
    let operation = original_lineage
        .as_ref()
        .map(|(op, _)| op.as_str())
        .unwrap_or(operation);
    let source = raw["source"].as_str().unwrap_or("history");
    let reference = raw["reference"].as_str().unwrap_or("");
    let requested = raw["kind"].as_str().unwrap_or(source);
    let value = raw["value"].as_str().unwrap_or("");
    let mut values = BTreeSet::new();
    if let Some(v) = normalized(requested, value) {
        values.insert((v.0, v.1, raw["wildcard"] == true || raw["wildcard"] == 1));
    }
    if let Some(h) = raw["domain"].as_str().and_then(host) {
        values.insert(("domain".into(), h, false));
    }
    if let Some(sha) = raw["certificateSha256"]
        .as_str()
        .and_then(hunter_expansion::fingerprint)
    {
        values.insert(("fingerprint".into(), sha, false));
    }
    let cache_key = (reference.to_owned(), source.to_owned());
    let (sources, asset) = if let Some(value) = added.providers.get(&cache_key) {
        value.clone()
    } else {
        let value = provider_sources(c, p, reference, source)?;
        added.providers.insert(cache_key, value.clone());
        value
    };
    for (kind, value, wildcard) in values {
        let existing:Option<i64>=c.query_row("SELECT id FROM asset_project_seeds WHERE project_id=?1 AND kind=?2 AND value=?3 AND wildcard=?4",params![p,kind,value,wildcard],|r|r.get(0)).optional().map_err(err)?;
        c.execute("INSERT INTO asset_project_seeds(project_id,kind,value,wildcard,first_round,last_round,first_seen_at,last_seen_at,historical_baseline) VALUES(?1,?2,?3,?4,?5,?5,?6,?7,?8) ON CONFLICT(project_id,kind,value,wildcard) DO UPDATE SET last_round=CASE WHEN excluded.last_round IS NULL THEN last_round ELSE MAX(COALESCE(last_round,0),excluded.last_round) END,last_seen_at=MAX(last_seen_at,excluded.last_seen_at)",params![p,kind,value,wildcard,round,if baseline{None}else{Some(seen)},seen,baseline]).map_err(err)?;
        let id:i64=c.query_row("SELECT id FROM asset_project_seeds WHERE project_id=?1 AND kind=?2 AND value=?3 AND wildcard=?4",params![p,kind,value,wildcard],|r|r.get(0)).map_err(err)?;
        if existing.is_none() {
            added.seeds += 1;
        }
        added.touched.insert(id);
        for provider in &sources {
            let tag = round
                .map(|r| format!("round:{r}"))
                .unwrap_or_else(|| "historical".into());
            // A source identity is idempotent for identical facts in one global
            // round. Material changes (unknown -> receipt -> imported) append;
            // no previously received billing or failure receipt is overwritten.
            let fact_hash = format!("{:x}", Sha256::digest(raw.to_string().as_bytes()));
            let action_tag = if action == "DELETE" {
                "deleted"
            } else {
                "observed"
            };
            let key = format!(
                "{:x}",
                Sha256::digest(
                    json!([
                        kind, value, wildcard, provider, source, reference, tag, fact_hash,
                        action_tag
                    ])
                    .to_string()
                    .as_bytes()
                )
            );
            let evidence = summary_evidence(raw, action);
            let inserted=c.execute("INSERT OR IGNORE INTO asset_project_seed_observations(project_id,seed_id,event_key,source_type,source_ref,operation_id,round,seen_at,historical_baseline,asset_id,query_id,query_kind,query_value,page,spent_points,cost_unknown,asset_count,evidence_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",params![p,id,key,provider,clipped(reference,240),if operation.is_empty(){None}else{Some(operation)},round,seen,baseline,asset,raw["queryId"].as_str().map(|s|clipped(s,128)),raw["queryKind"].as_str().map(|s|clipped(s,32)),raw["queryValue"].as_str().map(|s|clipped(s,253)),raw["page"].as_i64(),raw["spentPoints"].as_i64(),raw["costUnknown"]==true||raw["costUnknown"]==1,raw["assetCount"].as_i64(),evidence.to_string()]).map_err(err)?;
            added.observations += inserted;
        }
    }
    Ok(())
}

fn proof_map(c: &Connection, p: i64) -> Result<HashMap<String, usize>, String> {
    let (revision,cached,raw,cache_at,now):(i64,i64,String,i64,i64)=c.query_row("SELECT proof_revision,proof_cache_revision,proofs_json,proof_cache_at,CAST(strftime('%s','now') AS INTEGER) FROM asset_project_seed_state WHERE project_id=?1",[p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(err)?;
    if revision == cached && cache_at > now - PROOF_TTL_SECONDS {
        return serde_json::from_str(&raw).map_err(err);
    }
    let mut map = HashMap::new();
    if !has(c, "asset_seed_assists")? {
        c.execute("UPDATE asset_project_seed_state SET proof_cache_revision=?2,proof_cache_at=CAST(strftime('%s','now') AS INTEGER),proofs_json='{}' WHERE project_id=?1",params![p,revision]).map_err(err)?;
        return Ok(map);
    }
    let v = review::assist::seed_verification_snapshot(c, p)?;
    if !v.is_null() {
        for group in v["groups"].as_array().into_iter().flatten() {
            if group["verification"]["status"] != "bound_local_evidence" {
                continue;
            }
            for source in group["verification"]["sources"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if !matches!(
                    source["kind"].as_str(),
                    Some("tls_subject" | "icp" | "public_ct_subject" | "official_domain")
                ) {
                    continue;
                }
                if let Some(h) = source["host"].as_str().and_then(host) {
                    *map.entry(format!("domain:{h}")).or_insert(0) += 1;
                }
                if let Some(h) = source["certificateSha256"]
                    .as_str()
                    .and_then(hunter_expansion::fingerprint)
                {
                    *map.entry(format!("fingerprint:{h}")).or_insert(0) += 1;
                }
            }
        }
    }
    c.execute("UPDATE asset_project_seed_state SET proof_cache_revision=?2,proof_cache_at=CAST(strftime('%s','now') AS INTEGER),proofs_json=?3 WHERE project_id=?1",params![p,revision,serde_json::to_string(&map).map_err(err)?]).map_err(err)?;
    Ok(map)
}
fn refresh_trust(
    c: &Connection,
    p: i64,
    ids: &HashSet<i64>,
    reuse_gates: bool,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let proofs = proof_map(c, p)?;
    let excluded = exclusions::rules(c, p)?;
    let confirmed = roots(c, p)?;
    let mut conflicts = HashSet::<String>::new();
    if has(c, "asset_enrichment_conflicts")? {
        let mut q=c.prepare("SELECT identity,hosts_json FROM asset_enrichment_conflicts WHERE project_id=?1 AND decision='pending'").map_err(err)?;
        for row in q
            .query_map([p], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(err)?
        {
            let (identity, raw) = row.map_err(err)?;
            conflicts.insert(identity);
            let values = match serde_json::from_str::<Value>(&raw).map_err(err)? {
                Value::Array(values) => values,
                Value::Object(values) => values.into_iter().map(|(_, value)| value).collect(),
                value => vec![value],
            };
            conflicts.extend(
                values
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned)),
            );
        }
    }
    let revision: i64 = c
        .query_row(
            "SELECT proof_revision FROM asset_project_seed_state WHERE project_id=?1",
            [p],
            |r| r.get(0),
        )
        .map_err(err)?;
    for id in ids {
        let (kind, value,gate_revision,previous_gate): (String, String,i64,bool) = c
            .query_row(
                "SELECT kind,value,gate_revision,hunter_eligible FROM asset_project_seeds WHERE project_id=?1 AND id=?2",
                params![p, id],
                |r| Ok((r.get(0)?, r.get(1)?,r.get(2)?,r.get(3)?)),
            )
            .map_err(err)?;
        let total:i64=c.query_row("SELECT COUNT(*) FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id=?2",params![p,id],|r|r.get(0)).map_err(err)?;
        // During historical initialization the source facts already exist.
        // Reuse both admitted and refused paid gates only at the same current
        // proof revision. Live delta/policy passes always revalidate the gate.
        let paid = if reuse_gates && gate_revision == revision {
            previous_gate
        } else {
            #[cfg(test)]
            GATE_CALLS.with(|calls| {
                *calls
                    .borrow_mut()
                    .entry(format!("{p}:{kind}:{value}"))
                    .or_insert(0) += 1
            });
            let started = std::time::Instant::now();
            // These are mandatory rejection conditions of evidence_anchor,
            // evaluated from the same transaction once per batch. An admitted
            // seed still passes the complete existing provenance gate.
            let rejected = confirmed.is_empty()
                || kind != "fingerprint"
                    && (exclusions::matches(&value, &excluded)
                        || !confirmed.iter().any(|root| in_scope(&value, root)));
            let admitted = !rejected
                && hunter_expansion::evidence_anchor(
                    c,
                    p,
                    &value,
                    if kind == "fingerprint" {
                        "fingerprint"
                    } else if kind == "cname" {
                        "cname"
                    } else {
                        "domain"
                    },
                )
                .is_ok();
            #[cfg(test)]
            if kind == "fingerprint" && std::env::var_os("OVIRAPTOR_SEED_LEDGER_SNAPSHOT").is_some()
            {
                eprintln!(
                    "SEED_LEDGER_PAID_GATE {}",
                    json!({"kind":kind,"elapsedMs":started.elapsed().as_millis()})
                );
            }
            let _ = started;
            admitted
        };
        let mut verified = proofs
            .get(&format!(
                "{}:{value}",
                if kind == "fingerprint" {
                    "fingerprint"
                } else {
                    "domain"
                }
            ))
            .copied()
            .unwrap_or(0)
            .min(total as usize) as i64;
        if kind == "fingerprint" && !paid {
            verified = 0;
        }
        let blocked = kind != "fingerprint" && exclusions::matches(&value, &excluded);
        let conflict = conflicts.contains(&value);
        let (status, reason) = if blocked {
            ("excluded", "项目已排除此域名；历史发现和查询回执保留")
        } else if conflict {
            ("conflict", "存在独立证书身份冲突，需要核对证据")
        } else if verified > 0 && (kind != "fingerprint" || paid) {
            (
                "verified",
                "有与当前目标和具体名称绑定的独立核验证据；不代表资产归属百分百或自动授权",
            )
        } else {
            (
                "needs_evidence",
                "尚无完整的目标绑定核验证据；AI建议和出现次数不等于可信归属",
            )
        };
        c.execute("UPDATE asset_project_seeds SET trust_status=?3,trust_reason=?4,hunter_eligible=?5,verified_evidence=?6,total_evidence=?7,trust_checked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),gate_revision=?8 WHERE project_id=?1 AND id=?2",params![p,id,status,reason,paid,verified,total,revision]).map_err(err)?;
    }
    Ok(())
}

/// Call before owner execution. Initial high-water snapshots are imported in
/// bounded batches and always remain historical with unknown first round.
pub(crate) fn initialize(c: &Connection, p: i64) -> Result<Value, String> {
    create_project(c, p)?;
    if ready(c, p)? {
        return Ok(
            json!({"projectId":p,"initialized":true,"hasMore":false,"processed":0,"newSeeds":0,"newObservations":0}),
        );
    }
    sync_internal(c, p, "", None)
}
pub(crate) fn sync(
    c: &Connection,
    p: i64,
    operation_id: &str,
    round: i64,
) -> Result<Value, String> {
    if operation_id.is_empty()
        || operation_id.len() > 160
        || operation_id.chars().any(char::is_control)
        || round < 1
    {
        return Err("seed_ledger_context_invalid".into());
    }
    create_project(c, p)?;
    sync_internal(c, p, operation_id, Some(round))
}
fn sync_internal(
    c: &Connection,
    p: i64,
    operation: &str,
    round: Option<i64>,
) -> Result<Value, String> {
    let tx = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)
        .map_err(err)?;
    let (ready,phase_num,mut cursor,cutoffs,mut change_cursor):(bool,i64,i64,String,i64)=tx.query_row("SELECT initialized,phase,cursor,cutoffs_json,change_cursor FROM asset_project_seed_state WHERE project_id=?1",[p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(err)?;
    // Preparation has no execution-round context. Once baseline is initialized
    // it must never drain real changes ahead of the owner's numbered sync.
    if ready && round.is_none() {
        tx.commit().map_err(err)?;
        return Ok(
            json!({"projectId":p,"initialized":true,"hasMore":false,"processed":0,"newSeeds":0,"newObservations":0}),
        );
    }
    expire_proofs(&tx, p)?;
    let mut phase = usize::try_from(phase_num).map_err(err)?;
    let limits: HashMap<String, i64> = serde_json::from_str(&cutoffs).map_err(err)?;
    let mut added = Added::default();
    let mut processed = 0usize;
    let mut initialized = ready;
    let seen = current_time(&tx)?;
    if !ready {
        while phase < SOURCES.len() && processed < BATCH {
            let table = SOURCES[phase];
            let limit = *limits.get(table).unwrap_or(&0);
            let rows: Vec<(i64, String)> = if limit > 0 && source_ready(&tx, table)? {
                tx.prepare(&source_sql(table).unwrap())
                    .map_err(err)?
                    .query_map(params![p, cursor, limit, (BATCH - processed) as i64], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .map_err(err)?
                    .collect::<Result<_, _>>()
                    .map_err(err)?
            } else {
                vec![]
            };
            for (id, raw) in &rows {
                apply(
                    &tx,
                    p,
                    &serde_json::from_str::<Value>(raw).map_err(err)?,
                    &seen,
                    None,
                    "",
                    true,
                    "BASELINE",
                    &mut added,
                )?;
                cursor = *id;
                processed += 1;
            }
            if rows.is_empty() || cursor >= limit {
                phase += 1;
                cursor = 0;
            } else {
                break;
            }
        }
        initialized = phase >= SOURCES.len();
        tx.execute("UPDATE asset_project_seed_state SET initialized=?2,phase=?3,cursor=?4 WHERE project_id=?1",params![p,initialized,phase as i64,cursor]).map_err(err)?;
    } else {
        let rows:Vec<(i64,String,String,String)>=tx.prepare("SELECT id,action,payload_json,seen_at FROM asset_project_seed_changes WHERE project_id=?1 AND id>?2 ORDER BY id LIMIT ?3").map_err(err)?.query_map(params![p,change_cursor,BATCH as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
        for (id, action, raw, event_seen) in rows {
            apply(
                &tx,
                p,
                &serde_json::from_str::<Value>(&raw).map_err(err)?,
                &event_seen,
                round,
                operation,
                false,
                &action,
                &mut added,
            )?;
            change_cursor = id;
            processed += 1;
        }
        tx.execute(
            "UPDATE asset_project_seed_state SET change_cursor=?2 WHERE project_id=?1",
            params![p, change_cursor],
        )
        .map_err(err)?;
    }
    let policy: i64 = tx
        .query_row(
            "SELECT policy_cursor FROM asset_project_seed_state WHERE project_id=?1",
            [p],
            |r| r.get(0),
        )
        .map_err(err)?;
    if policy >= 0 && processed < BATCH {
        let available = BATCH - processed;
        let ids:Vec<i64>=tx.prepare("SELECT id FROM asset_project_seeds WHERE project_id=?1 AND id>?2 ORDER BY id LIMIT ?3").map_err(err)?.query_map(params![p,policy,available as i64],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
        added.touched.extend(ids.iter().copied());
        processed += ids.len();
        tx.execute(
            "UPDATE asset_project_seed_state SET policy_cursor=?2 WHERE project_id=?1",
            params![
                p,
                if ids.len() < available {
                    -1
                } else {
                    *ids.last().unwrap_or(&policy)
                }
            ],
        )
        .map_err(err)?;
    }
    refresh_trust(&tx, p, &added.touched, !ready)?;
    tx.commit().map_err(err)?;
    let pending:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM asset_project_seed_changes WHERE project_id=?1 AND id>?2) OR (SELECT policy_cursor>=0 FROM asset_project_seed_state WHERE project_id=?1)",params![p,change_cursor],|r|r.get(0)).map_err(err)?;
    Ok(
        json!({"projectId":p,"initialized":initialized,"hasMore":!initialized||(round.is_some()&&pending),"processed":processed,"newSeeds":added.seeds,"newObservations":added.observations}),
    )
}

fn pending(p: i64) -> Value {
    json!({"projectId":p,"ready":false,"state":"pending_initialization","items":[],"total":null,"summary":null,"reason":"项目种子台账待整理；尚未统计不表示没有线索。只读查看不会启动发现或消耗积分"})
}
fn ready(c: &Connection, p: i64) -> Result<bool, String> {
    Ok(has(c, "asset_project_seed_state")?
        && c.query_row(
            "SELECT initialized FROM asset_project_seed_state WHERE project_id=?1",
            [p],
            |r| r.get::<_, bool>(0),
        )
        .optional()
        .map_err(err)?
        .unwrap_or(false))
}
fn samples(c: &Connection, p: i64, id: i64) -> Result<Value, String> {
    let total:i64=c.query_row("SELECT COUNT(DISTINCT asset_id) FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id=?2 AND asset_id IS NOT NULL",params![p,id],|r|r.get(0)).map_err(err)?;
    let (mut yes, mut no) = (0_i64, 0_i64);
    if has(c, "asset_events")? {
        (yes,no)=c.query_row("SELECT COALESCE(SUM(e.summary LIKE 'confirmed:%'),0),COALESCE(SUM(e.summary LIKE 'rejected:%'),0) FROM (SELECT DISTINCT asset_id FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id=?2 AND asset_id IS NOT NULL) a JOIN asset_events e ON e.project_id=?1 AND e.asset_id=a.asset_id AND e.id=(SELECT MAX(x.id) FROM asset_events x WHERE x.project_id=?1 AND x.asset_id=a.asset_id AND x.event_type='decision')",params![p,id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
    }
    Ok(
        json!({"accepted":yes,"rejected":no,"reviewed":yes+no,"total":total,"acceptanceRate":if yes+no>0{json!(yes as f64/(yes+no)as f64)}else{Value::Null},"meaning":"人工已审样本"}),
    )
}
#[derive(Default)]
struct PageFacts {
    rows: HashMap<i64,(Value,String,bool,bool)>,
    subjects: HashMap<i64,Value>,
    sources: HashMap<i64, Vec<String>>,
    counts: HashMap<i64, i64>,
    samples: HashMap<i64, (i64, i64, i64)>,
}
fn page_facts(c: &Connection, p: i64, ids: &[i64], projection:&str) -> Result<PageFacts, String> {
    let mut facts = PageFacts::default();
    if ids.is_empty() {
        return Ok(facts);
    }
    let keys = json!(ids).to_string();
    let mut rows=c.prepare(&format!("{projection} SELECT id,kind,value,wildcard,first_round,last_round,first_seen_at,last_seen_at,historical_baseline,trust_status,trust_reason,hunter_eligible,verified_evidence,total_evidence,trust_checked_at,current_trust,current_stale,rule_excluded FROM current_seeds WHERE id IN (SELECT value FROM json_each(?2)) ")).map_err(err)?;
    for row in rows.query_map(params![p,keys],decoded_seed).map_err(err)?{let row=row.map_err(err)?;facts.rows.insert(row.0["id"].as_i64().ok_or("seed_id_missing")?,row);}

    let mut q=c.prepare("SELECT seed_id,source_type FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id IN (SELECT value FROM json_each(?2)) GROUP BY seed_id,source_type ORDER BY seed_id,source_type").map_err(err)?;
    for row in q
        .query_map(params![p, keys], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(err)?
    {
        let (id, source) = row.map_err(err)?;
        facts.sources.entry(id).or_default().push(source);
    }
    let mut q=c.prepare("SELECT seed_id,COUNT(*) FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id IN (SELECT value FROM json_each(?2)) GROUP BY seed_id").map_err(err)?;
    for row in q
        .query_map(params![p, keys], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(err)?
    {
        let (id, count) = row.map_err(err)?;
        facts.counts.insert(id, count);
    }
    let linked="WITH linked AS (SELECT seed_id,asset_id FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id IN (SELECT value FROM json_each(?2)) AND asset_id IS NOT NULL GROUP BY seed_id,asset_id) ";
    let sql = if has(c, "asset_events")? {
        format!("{linked} SELECT a.seed_id,COUNT(*),COALESCE(SUM(e.summary LIKE 'confirmed:%'),0),COALESCE(SUM(e.summary LIKE 'rejected:%'),0) FROM linked a LEFT JOIN asset_events e ON e.project_id=?1 AND e.asset_id=a.asset_id AND e.id=(SELECT MAX(x.id) FROM asset_events x WHERE x.project_id=?1 AND x.asset_id=a.asset_id AND x.event_type='decision') GROUP BY a.seed_id")
    } else {
        format!("{linked} SELECT seed_id,COUNT(*),0,0 FROM linked GROUP BY seed_id")
    };
    let mut q = c.prepare(&sql).map_err(err)?;
    for row in q
        .query_map(params![p, keys], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(err)?
    {
        let (id, total, yes, no) = row.map_err(err)?;
        facts.samples.insert(id, (total, yes, no));
    }
    let seeds:Vec<Value>=facts.rows.values().filter(|row|!row.3).map(|row|row.0.clone()).collect();
    let groups=subject_groups(c,p,&seeds)?;
    for seed in seeds{facts.subjects.insert(seed["id"].as_i64().ok_or("seed_id_missing")?,subjects_from_groups(&groups,&seed));}
    Ok(facts)
}
fn decoded_seed(r:&rusqlite::Row<'_>)->rusqlite::Result<(Value,String,bool,bool)>{Ok((json!({"id":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"value":r.get::<_,String>(2)?,"wildcard":r.get::<_,bool>(3)?,"firstSeenRound":r.get::<_,Option<i64>>(4)?,"firstSeenAt":r.get::<_,Option<String>>(6)?,"lastSeenRound":r.get::<_,Option<i64>>(5)?,"lastSeenAt":r.get::<_,String>(7)?,"historicalBaseline":r.get::<_,bool>(8)?,"trustStatus":r.get::<_,String>(15)?,"trustReason":clipped(&r.get::<_,String>(10)?,108),"hunterEligible":r.get::<_,bool>(11)?,"trustCheckedAt":r.get::<_,Option<String>>(14)?,"verificationCoverage":{"verifiedEvidence":r.get::<_,i64>(12)?,"totalEvidence":r.get::<_,i64>(13)?}}),r.get(9)?,r.get(16)?,r.get(17)?))}
fn seed(c: &Connection, p: i64, id: i64) -> Result<Value, String> {
    seed_with_facts(c, p, id, None, &current(c)?)
}
fn seed_with_facts(
    c: &Connection,
    p: i64,
    id: i64,
    facts: Option<&PageFacts>,
    projection: &str,
) -> Result<Value, String> {
    let (mut value,stored,stale,excluded)=if let Some(row)=facts.and_then(|f|f.rows.get(&id)){row.clone()}else{
        c.query_row(&format!("{projection} SELECT id,kind,value,wildcard,first_round,last_round,first_seen_at,last_seen_at,historical_baseline,trust_status,trust_reason,hunter_eligible,verified_evidence,total_evidence,trust_checked_at,current_trust,current_stale,rule_excluded FROM current_seeds WHERE id=?2"),params![p,id],decoded_seed).optional().map_err(err)?.ok_or("seed_ledger_seed_unavailable")?
    };
    value["trustPending"] = json!(stale || stored == "excluded" && !excluded);
    if excluded || stored == "excluded" || stale || value["trustStatus"]=="conflict" {
        value["hunterEligible"] = json!(false);
    }
    if excluded {
        value["trustReason"] = json!("项目已排除此域名；历史发现和查询回执保留");
        value["verificationCoverage"]["verifiedEvidence"] = json!(0);
    } else if stored == "excluded" {
        value["trustReason"] = json!("项目排除规则已撤销；等待本地核验证据复核，历史保留");
        value["verificationCoverage"]["verifiedEvidence"] = json!(0);
    } else if stale && stored == "verified" {
        value["trustReason"] = json!("证据或核验快照已失效；等待本地复核，历史保留");
        value["verificationCoverage"]["verifiedEvidence"] = json!(0);
    }
    let (sources, observation_count, manual) = if let Some(facts) = facts {
        let (total, yes, no) = facts.samples.get(&id).copied().unwrap_or((0, 0, 0));
        (
            facts.sources.get(&id).cloned().unwrap_or_default(),
            *facts.counts.get(&id).unwrap_or(&0),
            json!({"accepted":yes,"rejected":no,"reviewed":yes+no,"total":total,"acceptanceRate":if yes+no>0{json!(yes as f64/(yes+no)as f64)}else{Value::Null},"meaning":"人工已审样本"}),
        )
    } else {
        let sources:Vec<String>=c.prepare("SELECT DISTINCT source_type FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id=?2 ORDER BY source_type").map_err(err)?.query_map(params![p,id],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
        let count:i64=c.query_row("SELECT COUNT(*) FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id=?2",params![p,id],|r|r.get(0)).map_err(err)?;
        (sources, count, samples(c, p, id)?)
    };
    if value["trustStatus"]=="accepted" {value["trustReason"]=json!("已有人工认可或确认范围，复用原结论；Hunter 仍按实际来源与范围核验，不重复要求确认");}
    if value["trustStatus"]=="conflict" {value["trustReason"]=json!("存在待核验的独立证据冲突；该种子暂停 Hunter 查询，原证据与人工决定保留");}
    if value["trustStatus"]=="ready" {value["trustReason"]=json!("已有真实发现与确认范围，满足精确查询条件；主体独立证据尚未齐全，不等于归属确认");}
    if value["trustStatus"]=="accepted" && !excluded {
        value["hunterEligible"]=json!(hunter_expansion::evidence_anchor(c,p,value["value"].as_str().unwrap_or(""),value["kind"].as_str().unwrap_or("")).is_ok());
    }
    value["subjects"]=if excluded {json!([])}else if let Some(cached)=facts.and_then(|f|f.subjects.get(&id)){cached.clone()}else{subjects_from_groups(&subject_groups(c,p,&[value.clone()])?,&value)};
    value["sourceTypes"] = json!(sources);
    value["observationCount"] = json!(observation_count);
    value["assetCount"] = manual["total"].clone();
    value["manualConclusion"] = manual;
    Ok(value)
}
// Exclusion rules are live policy, not cached ownership evidence. Every read
// projection (items, detail, filter and summary) uses the same relation without
// waiting for an execution owner to rewrite its derived trust cache. A shared
// fingerprint is never excluded merely because one associated domain is ruled
// out; the existing independent-reference/paid admission gate remains in charge.
pub(super) fn current(c: &Connection) -> Result<String, String> {
    let rules = if has(c, "asset_discovery_domain_exclusions")? {
        "SELECT domain FROM asset_discovery_domain_exclusions WHERE project_id=?1 AND enabled=1"
    } else {
        // New projects may have a ready ledger before their first exclusion.
        // A pure read must not call ensure() to manufacture a policy table.
        "SELECT NULL AS domain WHERE 0"
    };
    let exact=seed_policy::exact_sql(c,"s")?;
    let scopes=if has(c,"asset_discovery_scopes")? {"SELECT value root FROM asset_discovery_scopes,json_each(roots_json) WHERE project_id=?1"}else{"SELECT NULL root WHERE 0"};
    let manual=seed_policy::manual_sql(c)?;
    let conflicts=seed_policy::conflict_sql(c)?;
    let cached_conflict=if has(c,"asset_enrichment_conflicts")?{"0"}else{"trust_status='conflict'"};
    Ok(format!("WITH live_rules AS ({rules}), confirmed_roots AS ({scopes}), manual_decisions AS ({manual}), pending_conflicts AS ({conflicts}), policy_seeds AS (
        SELECT s.*,
          ((st.policy_cursor>=0 AND s.id>st.policy_cursor) OR st.proof_cache_at<=CAST(strftime('%s','now') AS INTEGER)-{PROOF_TTL_SECONDS}) AS current_stale,
          ({exact} OR (s.kind='domain' AND EXISTS(SELECT 1 FROM manual_decisions m WHERE m.value=s.value AND m.rejected=1 AND m.accepted=0)) OR (s.kind!='fingerprint' AND EXISTS(SELECT 1 FROM live_rules d WHERE s.value=d.domain OR substr(s.value,-(length(d.domain)+1))='.'||d.domain))) AS rule_excluded,
          (s.kind!='fingerprint' AND (EXISTS(SELECT 1 FROM confirmed_roots r WHERE s.value=r.root OR substr(s.value,-(length(r.root)+1))='.'||r.root) OR EXISTS(SELECT 1 FROM manual_decisions m WHERE m.value=s.value AND m.accepted=1 AND m.rejected=0))) AS human_accepted
        FROM asset_project_seeds s JOIN asset_project_seed_state st ON st.project_id=s.project_id WHERE s.project_id=?1
      ), current_seeds AS (
        SELECT policy_seeds.*,CASE
          WHEN rule_excluded THEN 'excluded'
          WHEN {cached_conflict} OR EXISTS(SELECT 1 FROM pending_conflicts c WHERE c.value=policy_seeds.value) THEN 'conflict'
          WHEN human_accepted THEN 'accepted'
          WHEN hunter_eligible=1 AND NOT current_stale THEN CASE WHEN trust_status='verified' THEN 'verified' ELSE 'ready' END
          WHEN trust_status IN ('excluded','conflict') OR (trust_status='verified' AND current_stale) THEN 'needs_evidence'
          ELSE trust_status END AS current_trust
        FROM policy_seeds
      ) "))
}
pub(crate) fn read(c: &Connection, q: &Query) -> Result<Value, String> {
    project_read(c, q.project_id)?;
    if !matches!(q.kind.as_str(), "all" | "domain" | "fingerprint" | "cname")
        || !matches!(
            q.trust.as_str(),
            "all" | "verified" | "accepted" | "ready" | "needs_evidence" | "conflict" | "excluded"
        )
        || q.page < 1
        || !(1..=50).contains(&q.page_size)
        || q.search.chars().count() > 240
    {
        return Err("seed_ledger_query_invalid".into());
    }
    if !ready(c, q.project_id)? {
        return Ok(pending(q.project_id));
    }
    let projection = current(c)?;
    let search = q.search.trim().to_lowercase();
    let total:i64=c.query_row(&format!("{projection} SELECT COUNT(*) FROM current_seeds WHERE (?2='all' OR kind=?2) AND (?3='all' OR current_trust=?3) AND (?4='' OR instr(lower(value),?4)>0)" ),params![q.project_id,q.kind,q.trust,search],|r|r.get(0)).map_err(err)?;
    let offset = (q.page - 1)
        .checked_mul(q.page_size)
        .ok_or("seed_ledger_query_invalid")?;
    let ids:Vec<i64>=c.prepare(&format!("{projection} SELECT id FROM current_seeds WHERE (?2='all' OR kind=?2) AND (?3='all' OR current_trust=?3) AND (?4='' OR instr(lower(value),?4)>0) ORDER BY id DESC LIMIT ?5 OFFSET ?6")).map_err(err)?.query_map(params![q.project_id,q.kind,q.trust,search,q.page_size,offset],|r|r.get(0)).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    let facts = page_facts(c, q.project_id, &ids,&projection)?;
    let items = ids
        .iter()
        .map(|id| seed_with_facts(c, q.project_id, *id, Some(&facts), &projection))
        .collect::<Result<Vec<_>, _>>()?;
    let aggregate_projection=projection.replace("policy_seeds AS (","policy_seeds AS MATERIALIZED (").replace("current_seeds AS (","current_seeds AS MATERIALIZED (");
    let summary=c.query_row(&format!("{aggregate_projection} SELECT COUNT(*),COALESCE(SUM(current_trust='verified'),0),COALESCE(SUM(current_trust='needs_evidence'),0),COALESCE(SUM(current_trust='conflict'),0),COALESCE(SUM(current_trust='excluded'),0),COALESCE(SUM(historical_baseline=1),0),COALESCE(SUM(current_trust='accepted'),0),COALESCE(SUM(current_trust='ready'),0) FROM current_seeds"),[q.project_id],|r|Ok(json!({"total":r.get::<_,i64>(0)?,"verified":r.get::<_,i64>(1)?,"needsEvidence":r.get::<_,i64>(2)?,"conflicts":r.get::<_,i64>(3)?,"excluded":r.get::<_,i64>(4)?,"historicalBaseline":r.get::<_,i64>(5)?,"accepted":r.get::<_,i64>(6)?,"ready":r.get::<_,i64>(7)?}))).map_err(err)?;
    Ok(
        json!({"projectId":q.project_id,"ready":true,"state":"ready","items":items,"page":q.page,"pageSize":q.page_size,"total":total,"summary":summary,"exclusionBatches":seed_policy::batches(c,q.project_id)?}),
    )
}
pub(crate) fn detail(
    c: &Connection,
    p: i64,
    id: i64,
    page: i64,
    page_size: i64,
) -> Result<Value, String> {
    project_read(c, p)?;
    if page < 1 || !(1..=20).contains(&page_size) || id < 1 {
        return Err("seed_ledger_query_invalid".into());
    }
    if !ready(c, p)? {
        return Err("seed_ledger_not_initialized".into());
    }
    let item = seed(c, p, id)?;
    let total:i64=c.query_row("SELECT COUNT(*) FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id=?2",params![p,id],|r|r.get(0)).map_err(err)?;
    let offset = (page - 1)
        .checked_mul(page_size)
        .ok_or("seed_ledger_query_invalid")?;
    let rows:Vec<Value>=c.prepare("SELECT id,source_type,source_ref,round,seen_at,historical_baseline,asset_id,query_id,query_kind,query_value,page,spent_points,cost_unknown,asset_count,evidence_json,operation_id FROM asset_project_seed_observations WHERE project_id=?1 AND seed_id=?2 ORDER BY id DESC LIMIT ?3 OFFSET ?4").map_err(err)?.query_map(params![p,id,page_size,offset],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"sourceType":r.get::<_,String>(1)?,"sourceRef":r.get::<_,String>(2)?,"operationId":r.get::<_,Option<String>>(15)?,"round":r.get::<_,Option<i64>>(3)?,"seenAt":r.get::<_,String>(4)?,"historicalBaseline":r.get::<_,bool>(5)?,"assetId":r.get::<_,Option<i64>>(6)?,"queryId":r.get::<_,Option<String>>(7)?,"queryKind":r.get::<_,Option<String>>(8)?,"queryValue":r.get::<_,Option<String>>(9)?,"page":r.get::<_,Option<i64>>(10)?,"spentPoints":r.get::<_,Option<i64>>(11)?,"costUnknown":r.get::<_,bool>(12)?,"assetCount":r.get::<_,Option<i64>>(13)?,"evidence":serde_json::from_str::<Value>(&r.get::<_,String>(14)?).unwrap_or(Value::Null)}))).map_err(err)?.collect::<Result<_,_>>().map_err(err)?;
    Ok(
        json!({"projectId":p,"ready":true,"seed":item,"observations":rows,"observationsTotal":total,"page":page,"pageSize":page_size}),
    )
}

#[cfg(test)]
#[path = "discovery_seed_ledger_tests.rs"]
mod tests;

// One bounded set query for full filtered selection; never loops UI pages.
pub(super) fn selected_ids(c:&Connection,p:i64,ids:&[i64],query:Option<&Query>)->Result<Vec<i64>,String>{
 project_read(c,p)?;
 let mut out=if let Some(q)=query{
  if q.project_id!=p{return Err("seed_selection_project_mismatch".into())}
  // Validate the same filters as list without loading every detail.
  if !matches!(q.kind.as_str(),"all"|"domain"|"cname"|"fingerprint") || !matches!(q.trust.as_str(),"all"|"verified"|"accepted"|"ready"|"needs_evidence"|"conflict"|"excluded") || q.search.len()>960{return Err("seed_selection_invalid".into())}
  c.prepare(&format!("{} SELECT id FROM current_seeds WHERE current_trust!='excluded' AND (?2='all' OR kind=?2) AND (?3='all' OR current_trust=?3) AND (?4='' OR instr(lower(value),?4)>0) ORDER BY id LIMIT 100001",current(c)?)).map_err(err)?.query_map(params![p,q.kind,q.trust,q.search.trim().to_lowercase()],|r|r.get(0)).map_err(err)?.collect::<Result<Vec<i64>,_>>().map_err(err)?
 }else{
  let unique:BTreeSet<i64>=ids.iter().copied().collect();
  if unique.is_empty(){return Err("seed_selection_empty".into())}
  let found:i64=c.query_row("SELECT count(*) FROM asset_project_seeds WHERE project_id=?1 AND id IN (SELECT value FROM json_each(?2))",params![p,json!(unique).to_string()],|r|r.get(0)).map_err(err)?;
  if found!=unique.len() as i64{return Err("seed_selection_project_mismatch".into())}
  c.prepare(&format!("{} SELECT id FROM current_seeds WHERE id IN (SELECT value FROM json_each(?2)) AND current_trust!='excluded' ORDER BY id",current(c)?)).map_err(err)?.query_map(params![p,json!(unique).to_string()],|r|r.get(0)).map_err(err)?.collect::<Result<Vec<i64>,_>>().map_err(err)?
 };
 if out.len()>100000{return Err("seed_selection_limit".into())}out.sort_unstable();Ok(out)
}
// Fetch all subject witnesses for a page once. This avoids parsing the entire
// project evidence JSON once for every fingerprint or repeated domain family.
fn subject_groups(c:&Connection,p:i64,seeds:&[Value])->Result<Vec<Value>,String>{
 if seeds.is_empty()||!has(c,"asset_cleanup_groups")?{return Ok(vec![])}
 let run:Option<String>=c.query_row("SELECT id FROM asset_cleanup_runs WHERE project_id=?1 ORDER BY created_at DESC,id DESC LIMIT 1",[p],|r|r.get(0)).optional().map_err(err)?;let Some(run)=run else{return Ok(vec![])};
 let mut keys=BTreeSet::new();let mut hashes=BTreeSet::new();
 for seed in seeds{let value=seed["value"].as_str().unwrap_or("");if seed["kind"]=="fingerprint"{hashes.insert(value.to_owned());}else{let family=crate::commands::ownership::ownership_domain_parts(value).and_then(|p|p.registrable).unwrap_or_else(||value.into());keys.insert(format!("domain:{family}"));}}
 let mut raw=BTreeSet::new();
 if !keys.is_empty(){let mut q=c.prepare("SELECT evidence_json FROM asset_cleanup_groups WHERE run_id=?1 AND group_key IN (SELECT value FROM json_each(?2))").map_err(err)?;for row in q.query_map(params![run,json!(keys).to_string()],|r|r.get::<_,String>(0)).map_err(err)?{raw.insert(row.map_err(err)?);}}
 if !hashes.is_empty(){let mut q=c.prepare("SELECT evidence_json FROM asset_cleanup_groups WHERE run_id=?1 AND EXISTS(SELECT 1 FROM json_each(evidence_json,'$.domainEvidence') e WHERE json_extract(e.value,'$.certificateSha256') IN (SELECT value FROM json_each(?2))) LIMIT 500").map_err(err)?;for row in q.query_map(params![run,json!(hashes).to_string()],|r|r.get::<_,String>(0)).map_err(err)?{raw.insert(row.map_err(err)?);}}
 Ok(raw.into_iter().filter_map(|s|serde_json::from_str(&s).ok()).collect())
}
fn subjects_from_groups(groups:&[Value],seed:&Value)->Value{
 let value=seed["value"].as_str().unwrap_or("");let mut facts=Vec::<Value>::new();
 for evidence in groups{
  for fact in evidence["domainEvidence"].as_array().into_iter().flatten(){
   let h=fact["host"].as_str().unwrap_or("");let source=fact["source"].as_str().unwrap_or("");let name=fact["organization"].as_str().unwrap_or("");
   let bound=if seed["kind"]=="fingerprint"{fact["certificateSha256"]==value}else{h==value || (fact["wildcard"]==true && enrichment::certificate_covers(value,&format!("*.{h}")))};
   if !bound||name.is_empty(){continue}
   let field=match source{"endpoint_icp"=>"备案主体", "official_site_operator"=>"官网运营主体", "website_icp_declared"=>"网页备案声明", "official_unit_relation"=>"官网单位关系", "leaf_subject"|"public_ct_subject"|"public_site_certificate"=>"证书 Subject.Organization (O)","provider_subject_cn"|"provider_subject_san"=>"测绘证书主体（关联 CN/SAN）",_=>continue};
   let item=json!({"name":clipped(name,240),"field":field,"source":source,"host":h,"certificateSha256":fact["certificateSha256"]});
   if !facts.iter().any(|v|v["name"]==item["name"]&&v["field"]==item["field"]&&v["source"]==item["source"]){facts.push(item)}if facts.len()>=6{return json!(facts)}
  }
 }
 json!(facts)
}
