//! §13.2 — Stage 2 acceptance tests for the canonical historical importer. One file
//! per contract area, included here so the fixtures stay shared without a copy.

use super::*;
use crate::artifact_import::canonical::RecordKind;
use crate::artifact_import::diagnostics::Severity;
use crate::artifact_import::limits::Limits;
use crate::artifact_import::service::{import_roots, BundleStatus, ImportContext};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Marker string that must never appear outside the byte-identical original.
const SECRET_MARKER: &str = "do-not-store";

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("oviraptor-import-{tag}-{}", Uuid::new_v4()))
}

fn write_file(dir: &Path, relative: &str, content: &str) {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_json(dir: &Path, relative: &str, value: &serde_json::Value) {
    write_file(dir, relative, &serde_json::to_string_pretty(value).unwrap());
}

fn initialize_db(root: &Path) -> PathBuf {
    crate::db::initialize(&root.join("app")).unwrap()
}

fn open_connection(db_path: &Path) -> Connection {
    crate::db::open(db_path).unwrap()
}

/// Everything a test imports lives under `<sandbox>/source`; the object store
/// stays outside it, so a write into the source is visible.
fn source_dir(root: &Path) -> PathBuf {
    let path = root.join("source");
    fs::create_dir_all(&path).unwrap();
    path
}

fn import_dir(connection: &Connection, root: &Path, limits: &Limits) -> ImportSummary {
    let cas = root.join("cas");
    let key_path = root.join("artifact-import.key");
    let roots = vec![source_dir(root)];
    let context = ImportContext {
        connection,
        cas_dir: &cas,
        key_path: &key_path,
        roots: &roots,
        limits,
    };
    import_roots(&context)
}

/// `root` is the sandbox: the imported tree is `<root>/source`.
fn import_at(connection: &Connection, root: &Path) -> ImportSummary {
    import_dir(connection, root, &Limits::default())
}

fn table_count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|error| panic!("{table} 不可读：{error}"))
}

/// `(relative path -> (content hash, bytes, mtime nanos, mode))` for a tree.
fn fingerprint_tree(root: &Path) -> BTreeMap<String, (String, u64, String, String)> {
    let mut rows = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(&path).unwrap_or_default();
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                format!("{:o}", metadata.permissions().mode() & 0o7777)
            };
            #[cfg(not(unix))]
            let mode = String::new();
            rows.insert(
                relative,
                (
                    crate::artifact_import::canonical::sha256_hex(&bytes),
                    bytes.len() as u64,
                    metadata
                        .modified()
                        .ok()
                        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|value| value.as_nanos().to_string())
                        .unwrap_or_default(),
                    mode,
                ),
            );
        }
    }
    rows
}

/// A SARIF document with one run and the given results.
fn sarif_document(results: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {"name": "fixture-sarif", "version": "1", "rules": []}},
            "results": results,
        }]
    })
}

/// One SARIF result; `paths` decides how many locations it carries.
fn sarif_result(
    rule_id: &str,
    level: &str,
    text: &str,
    paths: &[(&str, i64)],
    properties: Option<serde_json::Value>,
) -> serde_json::Value {
    let locations: Vec<serde_json::Value> = paths
        .iter()
        .map(|(uri, line)| {
            serde_json::json!({
                "physicalLocation": {
                    "artifactLocation": {"uri": uri},
                    "region": {"startLine": line}
                }
            })
        })
        .collect();
    let mut row = serde_json::json!({
        "ruleId": rule_id,
        "level": level,
        "message": {"text": text},
        "locations": locations,
    });
    if let Some(properties) = properties {
        row["properties"] = properties;
    }
    row
}

/// Builds a real SQLite file from a frozen generation script.
fn build_sqlite(path: &Path, script: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _ = fs::remove_file(path);
    let connection = Connection::open(path).unwrap();
    connection.execute_batch(script).unwrap();
    drop(connection);
}

/// A clone of `base` with some fields replaced, to build look-alike findings.
fn with_override(base: &serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let mut row = base.as_object().cloned().unwrap_or_default();
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        row.insert(key, value);
    }
    serde_json::Value::Object(row)
}

fn copy_fixture_bundle(root: &Path, bundle: &str) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/legacy_strix/bundles")
        .join(bundle);
    let target = source_dir(root).join(bundle);
    copy_tree(&source, &target);
    target
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(&path, &target).unwrap();
        }
    }
}

fn codes(summary: &ImportSummary) -> Vec<String> {
    let mut codes: Vec<String> = summary
        .outcomes
        .iter()
        .flat_map(|outcome| outcome.diagnostics.iter())
        .chain(summary.discovery_diagnostics.iter())
        .map(|diagnostic| diagnostic.code.clone())
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

fn has_code(summary: &ImportSummary, code: &str) -> bool {
    codes(summary).iter().any(|found| found == code)
}

fn severity_of(summary: &ImportSummary, code: &str) -> Option<Severity> {
    summary
        .outcomes
        .iter()
        .flat_map(|outcome| outcome.diagnostics.iter())
        .chain(summary.discovery_diagnostics.iter())
        .find(|diagnostic| diagnostic.code == code)
        .map(|diagnostic| diagnostic.severity)
}

/// Envelopes currently projected in the scope, newest revision id first.
fn current_envelopes(connection: &Connection, kind: RecordKind) -> Vec<serde_json::Value> {
    let mut statement = connection
        .prepare(
            "SELECT r.envelope_json FROM import_projection_memberships m
                 JOIN import_record_revisions r ON r.id=m.revision_id
                 WHERE m.current=1 AND r.record_kind=?1 ORDER BY r.id",
        )
        .unwrap();
    statement
        .query_map([kind.as_str()], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

fn payload_of(envelope: &serde_json::Value) -> &serde_json::Value {
    envelope.get("payload").unwrap_or(&serde_json::Value::Null)
}

fn envelope_where<'a>(
    envelopes: &'a [serde_json::Value],
    key: &str,
    wanted: &str,
) -> Option<&'a serde_json::Value> {
    envelopes.iter().find(|envelope| {
        payload_of(envelope)
            .get(key)
            .and_then(serde_json::Value::as_str)
            == Some(wanted)
    })
}

fn scalar_text(envelope: &serde_json::Value, key: &str) -> String {
    payload_of(envelope)
        .get(key)
        .map(|value| match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

include!("tests_import_formats.rs");
include!("tests_import_record_limits.rs");
include!("tests_result_retirement.rs");
include!("tests_sqlite_retirement.rs");
include!("tests_import_merge.rs");
include!("tests_import_authority.rs");
include!("tests_import_sarif.rs");
include!("tests_sarif_retirement.rs");
include!("tests_import_metadata.rs");
include!("tests_import_corruption.rs");
include!("tests_import_idempotence.rs");
include!("tests_import_regressions.rs");
include!("tests_import_trace.rs");
include!("tests_event_retirement.rs");
include!("tests_coverage_retirement.rs");
include!("tests_run_retirement.rs");
include!("tests_import_source_reports.rs");
include!("tests_import_discovery.rs");
