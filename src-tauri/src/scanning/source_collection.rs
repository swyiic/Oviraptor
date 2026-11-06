//! Controlled OSS source collection for autonomous source audit.
//!
//! **This module is INERT in production.** No scan path passes `opt_in = true`
//! yet, and `plan_fetch` refuses every request without it, so no network fetch
//! can fire. The opt-in gate encodes the hard product requirement: source is
//! pulled ONLY when the task-creation UI explicitly selected "allow source
//! fetch" — never implicitly, never from a web page's suggestion, never
//! unattended. Wiring this into the scan pipeline (and adding that UI toggle) is
//! a separate, reviewed activation step.
//!
//! The collect→analyze→reproduce chain's other two stages already exist: the
//! analyzer sandbox (`tool_supply/sandbox/macos_static_parser.rs` for the
//! lightweight path, `native_pipeline/analyzer.rs` for containers) and greybox
//! correlation (`native_pipeline/greybox.rs`). This module only adds the
//! missing, most-sensitive first stage, behind its gate.
#![allow(dead_code)]

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

const BUILTIN_CATALOG: &str = include_str!("../../resources/rules/source_sources.json");

/// Hosts a fetch may clone from. An entry whose `host` is not here is rejected
/// at plan time, and a repo URL whose host disagrees with the entry is rejected
/// too, so a catalog row can never widen the reachable network surface.
pub(crate) const DEFAULT_FETCH_ALLOWLIST: &[&str] =
    &["github.com", "gitee.com", "gitlab.com"];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SourceEntry {
    /// Must match a fingerprint id so web identification resolves a source.
    pub product_id: String,
    pub name: String,
    pub repo_url: String,
    pub host: String,
    pub default_ref: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceCatalog {
    schema_version: u32,
    #[serde(default)]
    note: String,
    sources: Vec<SourceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FetchPlan {
    pub product_id: String,
    pub repo_url: String,
    pub git_ref: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FetchError {
    /// The task did not opt in to source fetch. This is the UI-gated guard.
    NotAuthorized,
    UnknownProduct(String),
    HostNotAllowed(String),
    BadUrl(String),
    BadRef(String),
    CatalogInvalid(String),
    SizeExceeded { bytes: u64, cap: u64 },
    Clone(String),
    Quarantine(String),
}
impl FetchError {
    pub(crate) fn code(&self) -> String {
        match self {
            Self::NotAuthorized => "source_fetch_not_authorized".into(),
            Self::UnknownProduct(p) => format!("source_fetch_unknown_product:{p}"),
            Self::HostNotAllowed(h) => format!("source_fetch_host_not_allowed:{h}"),
            Self::BadUrl(u) => format!("source_fetch_bad_url:{u}"),
            Self::BadRef(r) => format!("source_fetch_bad_ref:{r}"),
            Self::CatalogInvalid(e) => format!("source_catalog_invalid:{e}"),
            Self::SizeExceeded { bytes, cap } => {
                format!("source_fetch_too_large:{bytes}>{cap}")
            }
            Self::Clone(e) => format!("source_fetch_clone_failed:{e}"),
            Self::Quarantine(e) => format!("source_fetch_quarantine_failed:{e}"),
        }
    }
}

pub(crate) fn builtin_catalog() -> Result<Vec<SourceEntry>, FetchError> {
    parse_catalog(BUILTIN_CATALOG)
}

fn parse_catalog(text: &str) -> Result<Vec<SourceEntry>, FetchError> {
    let catalog: SourceCatalog =
        serde_json::from_str(text).map_err(|e| FetchError::CatalogInvalid(e.to_string()))?;
    if catalog.schema_version != 1 {
        return Err(FetchError::CatalogInvalid(format!(
            "schemaVersion={}",
            catalog.schema_version
        )));
    }
    let mut seen = std::collections::HashSet::new();
    for entry in &catalog.sources {
        if !seen.insert(entry.product_id.clone()) {
            return Err(FetchError::CatalogInvalid(format!(
                "duplicate productId {}",
                entry.product_id
            )));
        }
    }
    Ok(catalog.sources)
}

/// A git ref we are willing to pass to `--branch`: a tag or branch name, no
/// option injection, no traversal, bounded length.
fn valid_ref(git_ref: &str) -> bool {
    !git_ref.is_empty()
        && git_ref.len() <= 200
        && !git_ref.starts_with('-')
        && !git_ref.contains("..")
        && git_ref
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

/// `https://<host>/...` with no embedded credentials and a host that both sits
/// on the allowlist and matches the catalog entry's declared host.
fn validate_url(repo_url: &str, declared_host: &str, allowlist: &[&str]) -> Result<(), FetchError> {
    let rest = repo_url
        .strip_prefix("https://")
        .ok_or_else(|| FetchError::BadUrl(repo_url.to_string()))?;
    // No `user:pass@` and no `@` host override.
    let authority = rest.split('/').next().unwrap_or("");
    if authority.contains('@') || authority.is_empty() {
        return Err(FetchError::BadUrl(repo_url.to_string()));
    }
    let host = authority.split(':').next().unwrap_or("");
    if host != declared_host {
        return Err(FetchError::BadUrl(format!("{repo_url} (host!=declared)")));
    }
    if !allowlist.contains(&host) {
        return Err(FetchError::HostNotAllowed(host.to_string()));
    }
    if repo_url.contains(['"', '\'', ' ', '\n', '\r', '\\']) {
        return Err(FetchError::BadUrl(repo_url.to_string()));
    }
    Ok(())
}

/// Authorize + plan a fetch. **Refuses unless `opt_in` is true** (the UI gate),
/// then resolves the product in the catalog and validates host/url/ref. Returns
/// a plan only; it performs no network action. Separating planning from
/// `execute_fetch` keeps every guard unit-testable without a network.
pub(crate) fn plan_fetch(
    opt_in: bool,
    product_id: &str,
    version_ref: Option<&str>,
    catalog: &[SourceEntry],
    allowlist: &[&str],
) -> Result<FetchPlan, FetchError> {
    if !opt_in {
        return Err(FetchError::NotAuthorized);
    }
    let entry = catalog
        .iter()
        .find(|e| e.product_id == product_id)
        .ok_or_else(|| FetchError::UnknownProduct(product_id.to_string()))?;
    validate_url(&entry.repo_url, &entry.host, allowlist)?;
    let git_ref = version_ref.unwrap_or(&entry.default_ref);
    if !valid_ref(git_ref) {
        return Err(FetchError::BadRef(git_ref.to_string()));
    }
    Ok(FetchPlan {
        product_id: entry.product_id.clone(),
        repo_url: entry.repo_url.clone(),
        git_ref: git_ref.to_string(),
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FetchLimits {
    pub max_bytes: u64,
    pub timeout_secs: u64,
}
impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            max_bytes: 2 * 1024 * 1024 * 1024,
            timeout_secs: 300,
        }
    }
}

/// Execute a planned fetch: shallow single-branch clone into a FRESH quarantine
/// directory under `quarantine_parent`, with no credential prompts and a
/// timeout, then enforce the size cap. Only reachable with a `FetchPlan`, which
/// `plan_fetch` only returns when `opt_in` was true — so this cannot run from an
/// un-opted-in task. The checkout is untrusted data: callers must analyze it
/// only inside the sandbox, never execute it.
pub(crate) fn execute_fetch(
    plan: &FetchPlan,
    quarantine_parent: &Path,
    limits: &FetchLimits,
) -> Result<PathBuf, FetchError> {
    let dir = quarantine_parent.join(format!("src-{}-{}", plan.product_id, uuid::Uuid::new_v4()));
    if dir.exists() {
        return Err(FetchError::Quarantine("quarantine_dir_exists".into()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| FetchError::Quarantine(e.to_string()))?;

    let mut child = Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "clone",
            "--depth",
            "1",
            "--single-branch",
            "--no-tags",
            "--branch",
            &plan.git_ref,
            "--",
            &plan.repo_url,
        ])
        .arg(&dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GCM_INTERACTIVE", "never")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| FetchError::Clone(e.to_string()))?;

    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(limits.timeout_secs.max(1));
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let _ = std::fs::remove_dir_all(&dir);
                    return Err(FetchError::Clone(format!("git exit {:?}", status.code())));
                }
                break;
            }
            Ok(None) => {}
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(FetchError::Clone(e.to_string()));
            }
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&dir);
            return Err(FetchError::Clone("timeout".into()));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let bytes = dir_size(&dir, limits.max_bytes);
    if bytes > limits.max_bytes {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(FetchError::SizeExceeded {
            bytes,
            cap: limits.max_bytes,
        });
    }
    Ok(dir)
}

fn dir_size(root: &Path, cap: u64) -> u64 {
    fn walk(dir: &Path, acc: &mut u64, cap: u64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if *acc > cap {
                return;
            }
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            let kind = meta.file_type();
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                walk(&entry.path(), acc, cap);
            } else if kind.is_file() {
                *acc = acc.saturating_add(meta.len());
            }
        }
    }
    let mut total = 0;
    walk(root, &mut total, cap);
    total
}

// ---------------------------------------------------------------------------
// Per-scan source-fetch consent (the product gate the user requires:
// "在添加任务的地方选中拉源的按钮，否则不让拉").
//
// Consent is an explicit, auditable row keyed by scan_id — modelled like this
// codebase's other consent records, not a boolean smuggled through policy JSON.
// Default-deny: no row means no fetch. The orchestrator fetches source for a
// scan ONLY when `source_fetch_allowed_in` returns true, which is ONLY after the
// task-creation UI called `authorize_scan_source_fetch` for that scan. The
// backend never infers consent and never carries it over to another scan.
// ---------------------------------------------------------------------------

/// Idempotent, additive schema for the consent record. Wired into the DB
/// bootstrap next to `agent_stage_decisions::SCHEMA`.
pub(crate) const CONSENT_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS scan_source_fetch_consent(\
    scan_id TEXT PRIMARY KEY,\
    allowed INTEGER NOT NULL DEFAULT 0,\
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),\
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))\
);";

/// True only when an explicit, un-revoked consent row exists for this scan.
pub(crate) fn source_fetch_allowed_in(conn: &rusqlite::Connection, scan_id: &str) -> bool {
    conn.query_row(
        "SELECT allowed FROM scan_source_fetch_consent WHERE scan_id=?1",
        [scan_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|allowed| allowed == 1)
    .unwrap_or(false)
}

/// Record that the user selected "allow source fetch" for this scan at task
/// creation. Accepts only a web scan that exists and is not deleted, so consent
/// is bound to a real authorized target and never to an arbitrary id. Recording
/// consent is itself inert — it authorizes nothing until the orchestrator both
/// sees this row AND resolves an allowlisted upstream for an identified product.
#[tauri::command]
pub fn authorize_scan_source_fetch(
    state: tauri::State<crate::AppState>,
    scan_id: String,
) -> Result<(), String> {
    let conn = crate::db::open(&state.db_path)?;
    let is_web_scan: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND scan_type='web' \
             AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1))",
            [&scan_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !is_web_scan {
        return Err("source_fetch_consent_requires_live_web_scan".into());
    }
    conn.execute(
        "INSERT INTO scan_source_fetch_consent(scan_id,allowed) VALUES(?1,1) \
         ON CONFLICT(scan_id) DO UPDATE SET allowed=1,updated_at=datetime('now','localtime')",
        [&scan_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Withdraw source-fetch consent for a scan. Idempotent; leaves an `allowed=0`
/// row so the withdrawal itself is auditable.
#[tauri::command]
pub fn revoke_scan_source_fetch(
    state: tauri::State<crate::AppState>,
    scan_id: String,
) -> Result<(), String> {
    let conn = crate::db::open(&state.db_path)?;
    conn.execute(
        "INSERT INTO scan_source_fetch_consent(scan_id,allowed) VALUES(?1,0) \
         ON CONFLICT(scan_id) DO UPDATE SET allowed=0,updated_at=datetime('now','localtime')",
        [&scan_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Read the consent state for a scan (for the task-creation UI to reflect the
/// checkbox and for the overview to show whether the chain may run).
#[tauri::command]
pub fn scan_source_fetch_allowed(
    state: tauri::State<crate::AppState>,
    scan_id: String,
) -> Result<bool, String> {
    let conn = crate::db::open(&state.db_path)?;
    Ok(source_fetch_allowed_in(&conn, &scan_id))
}

// ---------------------------------------------------------------------------
// Gated commands (activation surface for testing the collect → sandbox-analyze
// chain). These are the ONLY callers that can set opt_in; they still refuse
// every un-opted-in request through `plan_fetch`. The UI must only pass
// opt_in=true when the task-creation "allow source fetch" box was selected —
// that checkbox is the product-level gate; these commands are its backend.
// ---------------------------------------------------------------------------

/// Controlled source collection. Network clone fires ONLY when `opt_in` is true
/// (the UI gate) and the product resolves to an allowlisted host in the catalog.
/// The checkout lands in a fresh quarantine dir under the app data dir and is
/// untrusted data — analyze it only via `scan_sandbox_analyze`, never execute it.
#[tauri::command]
pub fn scan_collect_source(
    state: tauri::State<crate::AppState>,
    opt_in: bool,
    product_id: String,
    version_ref: Option<String>,
) -> Result<serde_json::Value, String> {
    let catalog = builtin_catalog().map_err(|e| e.code())?;
    let plan = plan_fetch(
        opt_in,
        &product_id,
        version_ref.as_deref(),
        &catalog,
        DEFAULT_FETCH_ALLOWLIST,
    )
    .map_err(|e| e.code())?;
    let quarantine = state.app_data_dir.join("source-quarantine");
    std::fs::create_dir_all(&quarantine).map_err(|e| format!("source_fetch_quarantine_failed:{e}"))?;
    let dir = execute_fetch(&plan, &quarantine, &FetchLimits::default()).map_err(|e| e.code())?;
    Ok(serde_json::json!({
        "productId": plan.product_id,
        "repoUrl": plan.repo_url,
        "gitRef": plan.git_ref,
        "quarantinePath": dir.to_string_lossy(),
        "isolation": "untrusted:analyze-in-sandbox-only"
    }))
}

/// Run a non-executing static analyzer over `source_dir` inside the macOS
/// Seatbelt lightweight sandbox: network denied, source read-only, writes
/// confined to a scratch dir, CPU/time bounded. `program` must be a static,
/// non-executing analyzer (e.g. Semgrep); never a build-running engine, because
/// the lightweight sandbox has no hard memory boundary and the model only holds
/// when the target code is read, not executed.
#[tauri::command]
pub fn scan_sandbox_analyze(
    state: tauri::State<crate::AppState>,
    program: String,
    args: Vec<String>,
    source_dir: String,
) -> Result<serde_json::Value, String> {
    let scratch = state.app_data_dir.join("sandbox-scratch").join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&scratch).map_err(|e| format!("sandbox_scratch_failed:{e}"))?;
    let limits = crate::tool_supply::macos_static_parser::SandboxLimits::default();
    let result = crate::tool_supply::macos_static_parser::run_sandboxed(
        Path::new(&program),
        &args,
        Path::new(&source_dir),
        &scratch,
        &limits,
        &|| false,
    );
    // Best-effort cleanup of the scratch dir regardless of outcome.
    let summary = match result {
        Ok(run) => serde_json::json!({
            "ok": true,
            "exitCode": run.exit_code,
            "timedOut": run.timed_out,
            "networkDisabled": run.network_disabled,
            "sourceReadOnly": run.source_read_only,
            "truncated": run.truncated,
            "durationMs": run.duration.as_millis() as u64,
            "stdoutHead": String::from_utf8_lossy(&run.stdout).chars().take(2000).collect::<String>(),
            "stderrHead": String::from_utf8_lossy(&run.stderr).chars().take(2000).collect::<String>(),
        }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&scratch);
            return Err(e.describe());
        }
    };
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(summary)
}

#[cfg(test)]
mod source_collection_tests {
    use super::*;

    fn catalog() -> Vec<SourceEntry> {
        vec![
            SourceEntry {
                product_id: "ruoyi".into(),
                name: "RuoYi".into(),
                repo_url: "https://gitee.com/y_project/RuoYi.git".into(),
                host: "gitee.com".into(),
                default_ref: "master".into(),
                notes: String::new(),
            },
            SourceEntry {
                product_id: "evil".into(),
                name: "Evil".into(),
                repo_url: "https://attacker.example/x.git".into(),
                host: "attacker.example".into(),
                default_ref: "main".into(),
                notes: String::new(),
            },
            SourceEntry {
                product_id: "creds".into(),
                name: "Creds".into(),
                repo_url: "https://user:pass@github.com/x/y.git".into(),
                host: "github.com".into(),
                default_ref: "main".into(),
                notes: String::new(),
            },
        ]
    }

    #[test]
    fn builtin_catalog_parses_and_matches_fingerprint_id() {
        let c = builtin_catalog().expect("builtin source catalog must parse");
        assert!(c.iter().any(|e| e.product_id == "ruoyi"));
    }

    #[test]
    fn refuses_without_opt_in() {
        // The UI gate: no opt-in, no fetch, regardless of product validity.
        let err = plan_fetch(false, "ruoyi", None, &catalog(), DEFAULT_FETCH_ALLOWLIST);
        assert_eq!(err, Err(FetchError::NotAuthorized));
    }

    #[test]
    fn unknown_product_is_rejected() {
        let err = plan_fetch(true, "nope", None, &catalog(), DEFAULT_FETCH_ALLOWLIST);
        assert_eq!(err, Err(FetchError::UnknownProduct("nope".into())));
    }

    #[test]
    fn host_off_allowlist_is_rejected() {
        let err = plan_fetch(true, "evil", None, &catalog(), DEFAULT_FETCH_ALLOWLIST);
        assert_eq!(err, Err(FetchError::HostNotAllowed("attacker.example".into())));
    }

    #[test]
    fn embedded_credentials_url_is_rejected() {
        let err = plan_fetch(true, "creds", None, &catalog(), DEFAULT_FETCH_ALLOWLIST);
        assert!(matches!(err, Err(FetchError::BadUrl(_))));
    }

    #[test]
    fn option_injection_ref_is_rejected() {
        let err = plan_fetch(
            true,
            "ruoyi",
            Some("--upload-pack=evil"),
            &catalog(),
            DEFAULT_FETCH_ALLOWLIST,
        );
        assert!(matches!(err, Err(FetchError::BadRef(_))));
    }

    #[test]
    fn traversal_ref_is_rejected() {
        let err = plan_fetch(true, "ruoyi", Some("../../etc"), &catalog(), DEFAULT_FETCH_ALLOWLIST);
        assert!(matches!(err, Err(FetchError::BadRef(_))));
    }

    #[test]
    fn valid_opt_in_plan_is_returned_but_nothing_fetched() {
        let plan = plan_fetch(true, "ruoyi", Some("v4.7.8"), &catalog(), DEFAULT_FETCH_ALLOWLIST)
            .expect("a valid opted-in request plans");
        assert_eq!(plan.product_id, "ruoyi");
        assert_eq!(plan.git_ref, "v4.7.8");
        assert_eq!(plan.repo_url, "https://gitee.com/y_project/RuoYi.git");
        // Note: no execute_fetch here — planning performs no network action.
    }
}
