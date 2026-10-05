//! What a projection belongs to. §IDM-007/§IDM-011 reconcile *within* one scan and
//! one attempt, so the scope must name a run/scan/target/attempt — never the
//! directory the user happened to point the importer at, or two sibling runs under
//! one search root would revoke each other's findings.

use super::discovery::Bundle;
use super::manifest::Payloads;
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};

/// Same marker files the legacy importer uses to bind a run directory to an
/// Oviraptor scan (`commands/result_ingestion_runs.rs`). A drift lock over in
/// `commands/tests_results.rs` keeps the two lists equal.
const SCAN_ID_MARKERS: &[&str] = &[".oviraptor-scan-id", ".asset-atlas-scan-id"];
const MARKER_PARENTS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub scan_id: String,
    pub attempt_number: i64,
}

impl Scope {
    /// Reconciling never crosses a scope, so this string is the whole difference
    /// between "this run replaced that finding" and "a different run deleted it".
    /// The attempt is deliberately *not* part of it: attempts of one scan share a
    /// projection and are ordered against each other by `attempt_number`.
    pub fn reconcile_key(&self) -> String {
        format!("scan={}", self.scan_id)
    }
}

/// Directory that identifies the run: an `attempt-000N` component and everything
/// below it belong to one attempt of the *parent* run, so they are cut off.
fn base_dir(bundle: &Bundle, attempt_number: i64) -> PathBuf {
    if attempt_number <= 0 {
        return bundle.source_dir.clone();
    }
    let mut base = PathBuf::new();
    for component in bundle.source_dir.components() {
        if component
            .as_os_str()
            .to_string_lossy()
            .starts_with("attempt-")
        {
            break;
        }
        base.push(component.as_os_str());
    }
    base
}

fn marker_scan_id(root: &Path, dir: &Path) -> Option<String> {
    let mut cursor = Some(dir);
    let mut depth = 0usize;
    while let Some(current) = cursor {
        if !current.starts_with(root) {
            break;
        }
        for marker in SCAN_ID_MARKERS {
            let path = current.join(marker);
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > 4096 {
                continue;
            }
            let Ok(text) = std::fs::read(path) else {
                continue;
            };
            let value = String::from_utf8_lossy(&text).trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
        cursor = current.parent();
        depth += 1;
        if depth > MARKER_PARENTS {
            break;
        }
    }
    None
}

/// Current bundle metadata may identify a scan; retired run files never do.
fn metadata_identity(prefix: &str, payloads: &Payloads) -> Option<String> {
    let wanted = if prefix.is_empty() {
        "meta.json".to_string()
    } else {
        format!("{prefix}/meta.json")
    };
    let (_, bytes) = payloads.iter().find(|(path, _)| *path == wanted)?;
    let value = serde_json::from_slice::<JsonValue>(bytes).ok()?;
    for key in ["run_id", "run_name", "scanId", "scan_id"] {
        if let Some(text) = value.get(key).and_then(JsonValue::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(sanitize(trimmed));
            }
        }
    }
    None
}

fn sanitize(text: &str) -> String {
    text.replace(['/', '\\'], "_").replace("..", "_")
}

fn relative_identity(root: &Path, dir: &Path) -> String {
    let identity = super::manifest::normalize_relative(root, dir).unwrap_or_default();
    if identity.is_empty() {
        "run-root".to_string()
    } else {
        identity
    }
}

pub fn resolve(root: &Path, bundle: &Bundle, payloads: &Payloads) -> Scope {
    let attempt_number = super::reconcile::attempt_number_of(&bundle.attempt_key);
    let base = base_dir(bundle, attempt_number);
    let prefix = super::manifest::normalize_relative(root, &base).unwrap_or_default();
    let scan_id = marker_scan_id(root, &bundle.source_dir)
        .or_else(|| metadata_identity(&prefix, payloads))
        .unwrap_or_else(|| relative_identity(root, &base));
    Scope {
        scan_id,
        attempt_number,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_search_skips_empty_files_and_stays_inside_root_and_depth_limit() {
        let outer = std::env::temp_dir().join(format!("marker-{}", uuid::Uuid::new_v4()));
        let root = outer.join("allowed");
        let nested = root.join("1/2/3/4/5/6");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(outer.join(SCAN_ID_MARKERS[0]), "outside").unwrap();
        assert_eq!(marker_scan_id(&root, &root), None);
        std::fs::write(root.join(SCAN_ID_MARKERS[0]), "inside").unwrap();
        assert_eq!(
            marker_scan_id(&root, &nested),
            None,
            "six parents exceeds the limit"
        );
        assert_eq!(
            marker_scan_id(&root, nested.parent().unwrap()),
            Some("inside".into())
        );
        std::fs::write(nested.join(SCAN_ID_MARKERS[0]), " \n").unwrap();
        std::fs::write(nested.join(SCAN_ID_MARKERS[1]), "alias").unwrap();
        assert_eq!(marker_scan_id(&root, &nested), Some("alias".into()));
        #[cfg(unix)]
        {
            std::fs::remove_file(nested.join(SCAN_ID_MARKERS[1])).unwrap();
            std::os::unix::fs::symlink(
                outer.join(SCAN_ID_MARKERS[0]),
                nested.join(SCAN_ID_MARKERS[1]),
            )
            .unwrap();
            assert_eq!(
                marker_scan_id(&root, &nested),
                None,
                "do not follow marker symlinks"
            );
        }
        std::fs::remove_dir_all(outer).unwrap();
    }
}
