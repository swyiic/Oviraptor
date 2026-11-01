//! web → source → greybox orchestration (§3/§7 of
//! AUTONOMOUS_OSS_SOURCE_AUDIT_DESIGN).
//!
//! The chain is automatic up to — and only up to — the spend boundary:
//!   web scan identifies a product (fingerprint findings)
//!     → resolve an allowlisted upstream from the catalog
//!       → controlled fetch into an isolated quarantine dir
//!         → emit a greybox "handoff" (source_path + the scan's authorized URLs)
//!
//! The greybox RUN itself spends model budget, so it is NOT auto-started here.
//! It stays an explicit user action through the existing, already budget-gated
//! workbench flow — consistent with this codebase's financial-continuation
//! guardrails (`automaticResumeAllowed:false`, approval-gated continuation). The
//! orchestrator's job is to make that run one click away, never to spend behind
//! the user's back.
//!
//! Hard gate: fetch fires ONLY when `source_fetch_allowed_in` returns true for
//! this scan — i.e. the task-creation "allow source fetch" box was selected.
//! Without consent the orchestrator refuses before touching the network.
#![allow(dead_code)]

use super::agent_stage_decisions::{record_stage_decision, StageDecision, StageOutcome};
use super::source_collection::{
    builtin_catalog, execute_fetch, plan_fetch, source_fetch_allowed_in, FetchLimits, SourceEntry,
    DEFAULT_FETCH_ALLOWLIST,
};
use serde::Serialize;

/// A product the web scan identified that also resolves to an allowlisted
/// upstream source in the catalog.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchedSource {
    pub product_id: String,
    pub product_name: String,
    pub repo_url: String,
    pub host: String,
    pub default_ref: String,
}

/// A source checkout the orchestrator collected, plus the greybox handoff that
/// lets the user start reproduction against the same authorized URLs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectedSource {
    pub product_id: String,
    pub product_name: String,
    pub repo_url: String,
    pub git_ref: String,
    pub quarantine_path: String,
    pub target_urls: Vec<String>,
}

/// The full orchestration state for a scan. `source_audit_report` returns this
/// read-only; `orchestrate_source_audit` returns it after attempting a fetch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAuditReport {
    pub scan_id: String,
    /// Whether the task-creation gate ("allow source fetch") was selected.
    pub consent: bool,
    /// Product ids the web scan identified (fingerprint findings).
    pub identified: Vec<String>,
    /// The subset that resolves to an allowlisted upstream in the catalog.
    pub matched: Vec<MatchedSource>,
    /// Sources already collected into quarantine for this scan.
    pub collected: Vec<CollectedSource>,
    /// The scan's authorized target URLs (greybox reproduction runs against these).
    pub target_urls: Vec<String>,
    /// A human-readable note on what the orchestrator did or why it stopped.
    pub note: String,
}

/// Product ids the web scan identified, from the fingerprint findings.
fn identified_products(conn: &rusqlite::Connection, scan_id: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare(
            "SELECT DISTINCT record_key FROM sentinel_findings \
             WHERE scan_id=?1 AND stage='fingerprint' AND kind='tech_identified' \
             ORDER BY record_key",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([scan_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// The scan's authorized target URLs.
fn target_urls(conn: &rusqlite::Connection, scan_id: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1 ORDER BY url")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([scan_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Identified products that resolve to an allowlisted upstream, in catalog order.
fn match_catalog(identified: &[String], catalog: &[SourceEntry]) -> Vec<MatchedSource> {
    catalog
        .iter()
        .filter(|entry| identified.iter().any(|id| id == &entry.product_id))
        .map(|entry| MatchedSource {
            product_id: entry.product_id.clone(),
            product_name: entry.name.clone(),
            repo_url: entry.repo_url.clone(),
            host: entry.host.clone(),
            default_ref: entry.default_ref.clone(),
        })
        .collect()
}

/// Sources already collected for this scan, read back from the handoff findings.
fn collected_sources(
    conn: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Vec<CollectedSource>, String> {
    let mut statement = conn
        .prepare(
            "SELECT record_json FROM sentinel_findings \
             WHERE scan_id=?1 AND stage='source_audit' AND kind='source_collected' \
             ORDER BY record_key",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([scan_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let text = row.map_err(|e| e.to_string())?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            out.push(CollectedSource {
                product_id: value["productId"].as_str().unwrap_or_default().to_string(),
                product_name: value["productName"].as_str().unwrap_or_default().to_string(),
                repo_url: value["repoUrl"].as_str().unwrap_or_default().to_string(),
                git_ref: value["gitRef"].as_str().unwrap_or_default().to_string(),
                quarantine_path: value["quarantinePath"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                target_urls: value["targetUrls"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    Ok(out)
}

/// Build the read-only orchestration state from an open connection. Shared by
/// the command and the orchestrator so both report identically. Touches no
/// network and spends nothing.
fn report_for(
    conn: &rusqlite::Connection,
    scan_id: &str,
) -> Result<SourceAuditReport, String> {
    let consent = source_fetch_allowed_in(conn, scan_id);
    let identified = identified_products(conn, scan_id)?;
    let matched = match builtin_catalog() {
        Ok(catalog) => match_catalog(&identified, &catalog),
        Err(_) => Vec::new(),
    };
    let collected = collected_sources(conn, scan_id)?;
    let target_urls = target_urls(conn, scan_id)?;
    let note = if !consent {
        "未选择「允许拉源」：不会拉取任何源码。".to_string()
    } else if matched.is_empty() {
        "已授权拉源，但识别到的产品暂无可用上游映射。".to_string()
    } else if collected.is_empty() {
        "已授权且已匹配上游，尚未拉取。运行编排以收集源码。".to_string()
    } else {
        format!("已收集 {} 份源码,可创建灰盒复现任务。", collected.len())
    };
    Ok(SourceAuditReport {
        scan_id: scan_id.to_string(),
        consent,
        identified,
        matched,
        collected,
        target_urls,
        note,
    })
}

/// Read-only orchestration state for a scan. Touches no network and spends
/// nothing — the task-creation UI and the 执行概览 panel call this to show
/// whether the chain may run and what it has already collected.
#[tauri::command]
pub fn source_audit_report(
    state: tauri::State<crate::AppState>,
    scan_id: String,
) -> Result<SourceAuditReport, String> {
    let conn = crate::db::open(&state.db_path)?;
    report_for(&conn, &scan_id)
}

/// Run the chain for a scan: resolve an allowlisted upstream for an identified
/// product, fetch it into quarantine, and emit a greybox handoff. Refuses
/// without task-creation consent. Idempotent: a product already collected for
/// this scan is not re-fetched. Collects at most ONE product per call to keep
/// the network action predictable and reviewable; call again to collect the next.
///
/// This never starts a greybox scan — see the module docs on the spend boundary.
#[tauri::command]
pub fn orchestrate_source_audit(
    state: tauri::State<crate::AppState>,
    scan_id: String,
) -> Result<SourceAuditReport, String> {
    let conn = crate::db::open(&state.db_path)?;

    // Hard gate (§ the user's requirement): no consent, no fetch, full stop.
    if !source_fetch_allowed_in(&conn, &scan_id) {
        let _ = record_stage_decision(
            &conn,
            &scan_id,
            0,
            "source_orchestration",
            StageOutcome::Blocked,
            StageDecision::NeedsUser,
            3,
            "未授权拉源:任务创建时未选择「允许拉源」(§安全门禁)",
            "{}",
        );
        return Err("source_fetch_not_authorized_for_scan".into());
    }

    let identified = identified_products(&conn, &scan_id)?;
    let catalog = builtin_catalog().map_err(|e| e.code())?;
    let matched = match_catalog(&identified, &catalog);
    let targets = target_urls(&conn, &scan_id)?;
    let collected = collected_sources(&conn, &scan_id)?;

    if matched.is_empty() {
        let _ = record_stage_decision(
            &conn,
            &scan_id,
            0,
            "source_orchestration",
            StageOutcome::Gap,
            StageDecision::Skip,
            2,
            "已授权拉源,但识别到的产品暂无可用上游映射,跳过。",
            "{}",
        );
        return report_for(&conn, &scan_id);
    }

    // Collect the first matched product not already collected (bounded to one
    // network action per call).
    let next = matched
        .iter()
        .find(|m| !collected.iter().any(|c| c.product_id == m.product_id));
    let Some(target) = next else {
        // Everything matched is already collected: nothing to do.
        return report_for(&conn, &scan_id);
    };

    let plan = plan_fetch(
        true,
        &target.product_id,
        None,
        &catalog,
        DEFAULT_FETCH_ALLOWLIST,
    )
    .map_err(|e| e.code())?;
    let quarantine = state.app_data_dir.join("source-quarantine");
    std::fs::create_dir_all(&quarantine)
        .map_err(|e| format!("source_fetch_quarantine_failed:{e}"))?;

    let dir = match execute_fetch(&plan, &quarantine, &FetchLimits::default()) {
        Ok(dir) => dir,
        Err(error) => {
            let code = error.code();
            // Surface the failure as a finding + a tier-2 decision, but do not
            // crash the command — the operator can retry or pick another product.
            let record = serde_json::json!({
                "productId": target.product_id,
                "repoUrl": plan.repo_url,
                "error": code,
            })
            .to_string();
            let _ = conn.execute(
                "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
                 VALUES(?1,'',?2,?3,?4,?5,'info',?6) \
                 ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
                 title=excluded.title,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
                rusqlite::params![
                    scan_id,
                    "source_audit",
                    "source_fetch_failed",
                    target.product_id,
                    format!("源码拉取失败：{}", target.product_name),
                    record
                ],
            );
            let _ = record_stage_decision(
                &conn,
                &scan_id,
                0,
                "source_orchestration",
                StageOutcome::Blocked,
                StageDecision::Degrade,
                2,
                &format!("拉取 {} 失败:{}", target.product_id, code),
                "{}",
            );
            return report_for(&conn, &scan_id);
        }
    };

    let quarantine_path = dir.to_string_lossy().to_string();
    let record = serde_json::json!({
        "productId": target.product_id,
        "productName": target.product_name,
        "repoUrl": plan.repo_url,
        "gitRef": plan.git_ref,
        "quarantinePath": quarantine_path,
        "targetUrls": targets.clone(),
        "isolation": "untrusted:greybox-or-sandbox-only",
    })
    .to_string();
    conn.execute(
        "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
         VALUES(?1,'',?2,?3,?4,?5,'info',?6) \
         ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
         title=excluded.title,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
        rusqlite::params![
            scan_id,
            "source_audit",
            "source_collected",
            target.product_id,
            format!("源码已收集：{}（可创建灰盒复现）", target.product_name),
            record
        ],
    )
    .map_err(|e| e.to_string())?;

    let _ = record_stage_decision(
        &conn,
        &scan_id,
        0,
        "source_orchestration",
        StageOutcome::Ok,
        StageDecision::Continue,
        1,
        &format!(
            "已拉取 {} 源码至隔离区,待创建灰盒复现({} 个授权 URL)。",
            target.product_id,
            targets.len()
        ),
        "{}",
    );

    // Read back the full state (including the handoff just written) as the single
    // source of truth, rather than mutating the in-memory list.
    report_for(&conn, &scan_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(product_id: &str) -> SourceEntry {
        SourceEntry {
            product_id: product_id.to_string(),
            name: format!("{product_id} name"),
            repo_url: format!("https://gitee.com/x/{product_id}.git"),
            host: "gitee.com".to_string(),
            default_ref: "master".to_string(),
            notes: String::new(),
        }
    }

    #[test]
    fn match_catalog_keeps_only_identified_products_in_catalog_order() {
        let catalog = vec![entry("ruoyi"), entry("other")];
        let identified = vec!["other".to_string(), "unmapped".to_string()];
        let matched = match_catalog(&identified, &catalog);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].product_id, "other");
    }

    #[test]
    fn match_catalog_empty_when_nothing_identified_resolves() {
        let catalog = vec![entry("ruoyi")];
        let matched = match_catalog(&["nope".to_string()], &catalog);
        assert!(matched.is_empty());
    }

    #[test]
    fn builtin_catalog_matches_seeded_ruoyi() {
        // The shipped catalog must resolve the seeded product, so a web scan that
        // fingerprints RuoYi can find its upstream.
        let catalog = builtin_catalog().expect("builtin catalog parses");
        let matched = match_catalog(&["ruoyi".to_string()], &catalog);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].host, "gitee.com");
    }
}
