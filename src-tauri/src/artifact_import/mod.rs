//! §9 — canonical, read-only import of historical scan artifacts.
//!
//! This module is the only place that reads old result formats into new rows. It
//! never launches a process, never touches the network, never writes `sentinel_*`
//! tables and never changes an active attempt (§9.2). The former in-place
//! historical writer has been removed; historical claims remain in `import_*`.

pub mod canonical;
pub mod diagnostics;
pub mod discovery;
pub mod limits;
pub mod manifest;

mod adapters;
mod atomic_file;
mod projection;
mod reconcile;
pub(crate) mod report_bundle;
mod scope;
mod sealing;
mod service;
mod store;

#[cfg(test)]
pub(crate) use sealing::{open as open_sealed_original, Key as OriginalKey};

pub use limits::Limits;
pub use projection::ShadowReport;
pub use service::{
    import_roots, import_sentinel_snapshot, BundleStatus, ImportContext, ImportSummary,
    ImportedRecordReceipt,
};

/// §12 Stage 2 缺口 6: the read-only half of the shadow comparison, exposed so a
/// status query can report it without running an import.
pub fn shadow_report(connection: &rusqlite::Connection) -> Result<ShadowReport, String> {
    projection::shadow_compare(connection)
}

#[cfg(test)]
mod tests;
