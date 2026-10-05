//! The import pipeline of §9.3: discover → snapshot bytes → manifest → parse every
//! adapter independently → canonical records → reconcile → one atomic commit per
//! bundle. A bundle that fails is reported and skipped; it never blocks its
//! neighbours (§COR-001) and never leaves half a projection behind (§IDM-009).

use super::adapters::sentinel_bundle::ScopedRecords;
use super::adapters::{self, ParseContext};
use super::canonical::CanonicalRecord;
use super::diagnostics::{sorted, Diagnostic, Severity};
use super::discovery::{discover, Bundle};
use super::limits::Limits;
use super::manifest::{build, Manifest};
use super::projection::{shadow_compare, ShadowReport};
use super::reconcile;
use super::scope;
use super::sealing;
use super::store;
use crate::artifact_import::canonical::sha256_hex;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ImportContext<'a> {
    pub connection: &'a Connection,
    pub cas_dir: &'a Path,
    /// Key file for sealed originals (§9.4). Created on first use, never before a
    /// credential actually has to be stored.
    pub key_path: &'a Path,
    pub roots: &'a [PathBuf],
    pub limits: &'a Limits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleStatus {
    Imported,
    Unchanged,
    Failed,
}

impl BundleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::Unchanged => "unchanged",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BundleOutcome {
    pub source_path: String,
    pub bundle_id: String,
    pub status: BundleStatus,
    pub records: usize,
    // Per-pass diagnostics for fixture assertions. Production consumers use
    // committed_records and the persisted ledger, not the retired import IPC.
    #[cfg(test)]
    pub revisions: usize,
    #[cfg(test)]
    pub revoked: usize,
    pub diagnostics: Vec<Diagnostic>,
    /// Exact revisions accepted by THIS commit, not the mutable current projection.
    /// One receipt per revision/artifact pair; merging keeps all contributing
    /// artifacts without turning each origin into another semantic record.
    /// None means no new commit receipt (including signature-only reuse).
    pub committed_records: Option<Vec<ImportedRecordReceipt>>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportedRecordReceipt {
    pub revision_id: i64,
    pub logical_key: String,
    pub revision_hash: String,
    pub record_kind: String,
    pub envelope_sha256: String,
    /// From this parsed artifact, even when an older identical revision was reused.
    pub source_artifact_id: String,
}

impl ImportedRecordReceipt {
    pub fn read(&self, connection: &Connection) -> Result<serde_json::Value, String> {
        let text: String = connection.query_row(
            "SELECT envelope_json FROM import_record_revisions WHERE id=?1 AND logical_key=?2 AND revision_hash=?3 AND record_kind=?4",
            rusqlite::params![self.revision_id,self.logical_key,self.revision_hash,self.record_kind],
            |row|row.get(0)).map_err(|error|format!("accepted_result_revision_unavailable:{error}"))?;
        if super::canonical::sha256_hex(text.as_bytes()) != self.envelope_sha256 {
            return Err("accepted_result_envelope_changed".into());
        }
        serde_json::from_str(&text)
            .map_err(|error| format!("accepted_result_envelope_invalid:{error}"))
    }
}

type BundleCommit = (
    (usize, usize),
    Vec<Diagnostic>,
    Option<Vec<ImportedRecordReceipt>>,
);

#[derive(Clone, Debug, Default)]
pub struct ImportSummary {
    pub outcomes: Vec<BundleOutcome>,
    pub discovery_diagnostics: Vec<Diagnostic>,
    pub shadow: ShadowReport,
}

const BUSY_RETRIES: usize = 5;
const BUSY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(20);

/// One original that is already in the object store but has no database row yet:
/// the row is written inside the commit transaction, so a failed import cannot leave
/// an object visible to any query surface (§IDM-009, 缺口 7).
#[derive(Clone, Debug)]
pub struct StagedObject {
    pub relative_path: String,
    pub content_hash: String,
    pub bytes: u64,
    pub storage_path: PathBuf,
    pub display_text: String,
    pub secret_material: bool,
}

type CasRows = Vec<StagedObject>;

/// §IDM-010: a write that hits `SQLITE_BUSY` may be attempted again, but only a
/// bounded number of times, and each attempt is a whole transaction so a retry can
/// never project the same record twice.
fn within_busy_bound<T>(
    label: &str,
    mut work: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    let mut attempt = 0usize;
    loop {
        attempt += 1;
        match work() {
            Ok(value) => return Ok(value),
            Err(error) if error.contains("database is locked") && attempt < BUSY_RETRIES => {
                std::thread::sleep(BUSY_BACKOFF);
            }
            Err(error) => return Err(format!("{label} 第 {attempt} 次尝试失败：{error}")),
        }
    }
}

pub fn import_roots(context: &ImportContext<'_>) -> ImportSummary {
    let mut summary = ImportSummary::default();
    for root in context.roots {
        let (bundles, notes) = discover(&[root.as_path()], 8);
        summary.discovery_diagnostics.extend(notes);
        if let Err(error) = within_busy_bound("登记来源", || {
            store::upsert_source(context.connection, root)
        }) {
            summary.discovery_diagnostics.push(Diagnostic::error(
                "source_unrecorded",
                root.display().to_string(),
                error,
            ));
            continue;
        }
        for bundle in bundles {
            summary.outcomes.push(import_bundle(context, root, &bundle));
        }
    }
    summary.shadow = shadow_compare(context.connection).unwrap_or_default();
    summary
}

/// Import bytes supplied by the explicit JSON commands. No temporary plaintext
/// source is written, no path inside the document is opened, and all task scopes
/// share one database commit. File and pasted-content imports use identical IDs.
pub fn import_sentinel_snapshot(
    context: &ImportContext<'_>,
    bytes: &[u8],
    source: Option<&Path>,
) -> Result<BundleOutcome, String> {
    use super::manifest::ManifestFile;
    if bytes.len() as u64 > context.limits.file_bytes.min(context.limits.bundle_bytes) {
        return Err("sentinel_bundle_bytes_limit".into());
    }
    if !context.limits.json_within_depth(bytes) {
        return Err("sentinel_bundle_depth_limit".into());
    }
    let value = serde_json::from_slice(bytes).map_err(|_| "invalid_sentinel_bundle_json")?;
    let hash = sha256_hex(bytes);
    let root = context
        .cas_dir
        .parent()
        .unwrap_or(context.cas_dir)
        .join("manual-json-imports");
    let source_path = source
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(&hash));
    let canonical = format!("sentinel-bundle.json\t{hash}\t{}\n", bytes.len());
    let manifest = Manifest {
        bundle_id: format!("sha256:{}", sha256_hex(canonical.as_bytes())),
        canonical,
        files: vec![ManifestFile {
            relative_path: "sentinel-bundle.json".into(),
            content_hash: hash,
            bytes: bytes.len() as u64,
            source_path: source_path.clone(),
        }],
    };
    let payloads = vec![("sentinel-bundle.json".into(), bytes.to_vec())];
    let parse = ParseContext {
        bundle_id: &manifest.bundle_id,
        manifest: &manifest,
        payloads: &payloads,
        limits: context.limits,
    };
    let groups = adapters::sentinel_bundle::parse(&parse, &value)?;
    let records = groups.iter().map(|group| group.records.len()).sum();
    let cas_files = write_objects(context, &manifest, &payloads).map_err(|notes| {
        format!(
            "sentinel_bundle_store_failed:{}",
            notes
                .iter()
                .map(|n| n.code.as_str())
                .collect::<Vec<_>>()
                .join(",")
        )
    })?;
    let bundle = Bundle {
        root: root.clone(),
        source_dir: source_path.clone(),
        attempt_key: String::new(),
        files: Vec::new(),
        discovery_errors: Vec::new(),
    };
    let signature = format!(
        "{}\u{1}sentinel-snapshot-v1\u{1}adapter={}",
        manifest.canonical,
        super::canonical::ADAPTER_VERSION
    );
    let ((revisions, revoked), diagnostics, committed_records) =
        within_busy_bound("提交历史 JSON", || {
            commit_bundle(
                context,
                &root,
                &bundle,
                &manifest,
                &signature,
                &cas_files,
                &groups,
                &[],
            )
        })?;
    Ok(BundleOutcome {
        source_path: source_path.to_string_lossy().into(),
        bundle_id: manifest.bundle_id,
        status: status_of(&[], &diagnostics, revisions, revoked),
        records,
        #[cfg(test)]
        revisions,
        #[cfg(test)]
        revoked,
        diagnostics,
        committed_records,
    })
}

fn envelope_text(record: &CanonicalRecord) -> String {
    serde_json::to_string(record).unwrap_or_else(|_| "{}".to_string())
}

#[path = "service/bundle.rs"]
mod bundle;
#[path = "service/commit.rs"]
mod commit;
#[path = "service/objects.rs"]
mod objects;

use bundle::import_bundle;
#[cfg(test)]
pub(super) use commit::begin_bundle_transaction;
use commit::commit_bundle;
use objects::write_objects;

/// One failed check makes the whole bundle fail; with nothing new projected it is
/// simply unchanged; otherwise this pass added something.
fn status_of(
    diagnostics: &[Diagnostic],
    extra: &[Diagnostic],
    revisions: usize,
    revoked: usize,
) -> BundleStatus {
    if diagnostics
        .iter()
        .chain(extra.iter())
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        BundleStatus::Failed
    } else if revisions + revoked == 0 {
        BundleStatus::Unchanged
    } else {
        BundleStatus::Imported
    }
}
