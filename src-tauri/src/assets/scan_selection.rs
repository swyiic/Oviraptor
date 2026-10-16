//! Current-project saved-asset selection evidence for a manual cross-App draft.
//! A receipt is provenance, never authorization, liveness, or a scan execution.
use crate::native_app_sdk::NativeCallContext;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
const MAX_MEMBERS: usize = 1000;
const MAX_TARGETS: usize = 200;
const MAX_URL_BYTES: usize = 2048;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS asset_scan_selections(
 selection_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL,project_id INTEGER NOT NULL,host_project_id INTEGER NOT NULL,
 grant_revision INTEGER NOT NULL,app_revision INTEGER NOT NULL,operation_id TEXT NOT NULL,
 input_sha256 TEXT NOT NULL,source_facts_sha256 TEXT NOT NULL,selection_digest TEXT NOT NULL,
 member_count INTEGER NOT NULL,payload_json TEXT NOT NULL,created_at TEXT NOT NULL,
 UNIQUE(instance_id,project_id,operation_id));
CREATE TRIGGER IF NOT EXISTS asset_scan_selection_no_update BEFORE UPDATE ON asset_scan_selections BEGIN SELECT RAISE(ABORT,'asset selection immutable');END;
CREATE TRIGGER IF NOT EXISTS asset_scan_selection_no_delete BEFORE DELETE ON asset_scan_selections BEGIN SELECT RAISE(ABORT,'asset selection immutable');END;
"#;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    asset_ids: Vec<i64>,
    mode: String,
}
fn error(name: &str) -> String {
    format!("asset_scan_selection_{name}")
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(super) fn initialize_on(db: &Connection) -> Result<(), String> {
    db.execute_batch(SCHEMA).map_err(|_| error("schema"))
}
fn canonical(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty()
        || raw.len() > MAX_URL_BYTES
        || !raw.to_ascii_lowercase().starts_with("http://")
            && !raw.to_ascii_lowercase().starts_with("https://")
        || raw.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '\\' | ',' | '，' | ';' | '；')
        })
        || raw.trim_matches(|c| matches!(c, '"' | '\'' | '<' | '>' | '，' | '。')) != raw
    {
        return Err(error("asset_url_unavailable"));
    }
    let mut url = reqwest::Url::parse(raw).map_err(|_| error("asset_url_unavailable"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port() == Some(0)
    {
        return Err(error("asset_url_unavailable"));
    }
    // Fragment never reaches an HTTP request; the immutable source hash still
    // binds the exact saved original, including its fragment and surrounding trim.
    url.set_fragment(None);
    let result = url.to_string();
    if result.len() > MAX_URL_BYTES {
        return Err(error("asset_url_unavailable"));
    }
    Ok(result)
}
fn receipt_payload(
    instance: &str,
    project: i64,
    c: &NativeCallContext,
    input_hash: &str,
    facts: &str,
    selection: &str,
    mode: &str,
    targets: &BTreeSet<String>,
    members: usize,
) -> Result<(Value, String), String> {
    if !uuid::Uuid::parse_str(selection)
        .is_ok_and(|id| id.get_version_num() == 4 && id.to_string() == selection)
    {
        return Err(error("receipt_invalid"));
    }
    let digest = hash(
        &serde_json::to_vec(&(
            "scan.targets.v1",
            instance,
            project,
            c.host_bound_project_id,
            &c.operation_id,
            input_hash,
            facts,
            selection,
            mode,
            targets,
        ))
        .map_err(|_| error("receipt_invalid"))?,
    );
    Ok((
        json!({"kind":"scan.targets.v1","targets":targets,"mode":mode,"source":{"kind":"asset-selection","applicationInstanceId":instance,"selectionId":selection,"selectionDigest":digest,"assetCount":members}}),
        digest,
    ))
}
/// Own transactional receipt ledger intentionally bypasses the generic cached
/// action ledger: every replay rechecks current membership and original URL
/// facts, so a later archived/replaced asset cannot leak via an old response.
pub(super) fn prepare_on(
    db: &Connection,
    project: i64,
    c: &NativeCallContext,
    args: Value,
) -> Result<Value, String> {
    c.validate("inventory.read")
        .map_err(|_| error("permission_denied"))?;
    if !db.is_autocommit() {
        return Err(error("requires_current_connection"));
    }
    let a: Input = serde_json::from_value(args).map_err(|_| error("arguments_invalid"))?;
    if project <= 0
        || a.asset_ids.is_empty()
        || a.asset_ids.len() > MAX_MEMBERS
        || !matches!(a.mode.as_str(), "quick" | "standard" | "deep")
        || a.asset_ids
            .iter()
            .any(|id| !(1..=MAX_SAFE_INTEGER).contains(id))
    {
        return Err(error("arguments_invalid"));
    }
    let ids: BTreeSet<i64> = a.asset_ids.iter().copied().collect();
    if ids.len() != a.asset_ids.len() {
        return Err(error("duplicate_asset_ids"));
    }
    let instance = super::service_instance_on(db)?;
    let mut permissions = c.permissions.clone();
    permissions.sort();
    let input_hash = hash(
        &serde_json::to_vec(&(
            &instance,
            project,
            c.host_bound_project_id,
            c.grant_revision,
            c.app_revision,
            &c.project.name,
            &c.project.status,
            permissions,
            &c.operation_id,
            &a.mode,
            &ids,
        ))
        .map_err(|_| error("arguments_invalid"))?,
    );
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|_| error("storage"))?;
    let active: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1 AND status='active')",
            [project],
            |r| r.get(0),
        )
        .map_err(|_| error("project_unavailable"))?;
    if !active {
        return Err(error("project_unavailable"));
    }
    let mut source = Sha256::new();
    source.update(b"oviraptor.asset-selection.source.v1\0");
    let mut targets = BTreeSet::new();
    for id in &ids {
        // No LIMIT sample, implicit other-project join, or recycled member content.
        // SQL byte-length guards bound legacy malformed metadata before materializing it.
        let row:Option<(String,String,String)>=tx.query_row("SELECT a.asset_key,a.link,a.host FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND a.id=?2 AND pa.is_deleted=0 AND length(CAST(a.asset_key AS BLOB))<=16384 AND length(CAST(a.link AS BLOB))<=16384 AND length(CAST(a.host AS BLOB))<=16384",params![project,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|error("read_failed"))?;
        let (key, link, host) = row.ok_or_else(|| error("member_unavailable"))?;
        let original = if link.trim().is_empty() { &host } else { &link };
        let target = canonical(original)?;
        let fact = serde_json::to_vec(&(id, key, original)).map_err(|_| error("read_failed"))?;
        source.update((fact.len() as u64).to_be_bytes());
        source.update(fact);
        targets.insert(target);
    }
    if targets.is_empty() || targets.len() > MAX_TARGETS {
        return Err(error("split_batch_required"));
    }
    let facts = format!("{:x}", source.finalize());
    let old:Option<(String,String,String,String,String,i64)>=tx.query_row("SELECT input_sha256,source_facts_sha256,payload_json,selection_id,selection_digest,member_count FROM asset_scan_selections WHERE instance_id=?1 AND project_id=?2 AND operation_id=?3",params![instance,project,c.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_|error("storage"))?;
    if let Some((old_input, old_facts, payload, selection, digest, members)) = old {
        if old_input != input_hash {
            return Err(error("operation_conflict"));
        }
        if old_facts != facts {
            return Err(error("source_changed_prepare_new_selection"));
        }
        let v: Value = serde_json::from_str(&payload).map_err(|_| error("receipt_invalid"))?;
        let (expected, expected_digest) = receipt_payload(
            &instance,
            project,
            c,
            &input_hash,
            &facts,
            &selection,
            &a.mode,
            &targets,
            ids.len(),
        )?;
        if v != expected || digest != expected_digest || members != ids.len() as i64 {
            return Err(error("receipt_invalid"));
        }
        if serde_json::to_vec(&v)
            .map_err(|_| error("receipt_invalid"))?
            .len()
            > MAX_OUTPUT_BYTES
        {
            return Err(error("split_batch_required"));
        }
        tx.commit().map_err(|_| error("commit_unconfirmed"))?;
        return Ok(v);
    }
    let selection = uuid::Uuid::new_v4().to_string();
    let (output, digest) = receipt_payload(
        &instance,
        project,
        c,
        &input_hash,
        &facts,
        &selection,
        &a.mode,
        &targets,
        ids.len(),
    )?;
    let payload = serde_json::to_string(&output).map_err(|_| error("receipt_invalid"))?;
    if payload.len() > MAX_OUTPUT_BYTES {
        return Err(error("split_batch_required"));
    }
    tx.execute(
        "INSERT INTO asset_scan_selections VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![
            selection,
            instance,
            project,
            c.host_bound_project_id,
            c.grant_revision,
            c.app_revision,
            c.operation_id,
            input_hash,
            facts,
            digest,
            ids.len() as i64,
            payload,
            chrono::Utc::now().to_rfc3339()
        ],
    )
    .map_err(|_| error("receipt_unconfirmed"))?;
    if super::service_instance_on(&tx)? != instance {
        return Err(error("instance_changed"));
    }
    tx.commit().map_err(|_| error("commit_unconfirmed"))?;
    Ok(output)
}
#[cfg(test)]
#[path = "scan_selection_tests.rs"]
mod tests;
