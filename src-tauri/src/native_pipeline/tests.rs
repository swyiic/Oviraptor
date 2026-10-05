//! §13.5 — Stage 4 acceptance tests for the Native code, greybox and CI surfaces. One
//! file per contract area, included here so the repository fixtures stay shared.

use crate::artifact_import::limits::Limits;
use crate::artifact_import::{import_roots, ImportContext};
use crate::native_pipeline::process::ProcessLimits;
use crate::native_pipeline::snapshot::RepositorySnapshot;
use rusqlite::Connection;
use serde_json::{json, Value as JsonValue};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

#[path = "tests_analyzer_regressions.rs"]
mod tests_analyzer_regressions;
#[path = "tests_native_ci.rs"]
mod tests_native_ci;
#[path = "tests_native_code.rs"]
mod tests_native_code;
#[path = "tests_native_code_broker.rs"]
mod tests_native_code_broker;
#[path = "tests_native_code_results.rs"]
mod tests_native_code_results;
#[path = "tests_native_greybox.rs"]
mod tests_native_greybox;
#[cfg(unix)]
#[path = "tests_process_lifecycle.rs"]
mod tests_process_lifecycle;
#[path = "tests_snapshot_diff.rs"]
mod tests_snapshot_diff;
#[path = "tests_snapshot_materialization.rs"]
mod tests_snapshot_materialization;

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("oviraptor-native-{tag}-{}", Uuid::new_v4()))
}

fn write_file(dir: &Path, relative: &str, content: &str) {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_json(dir: &Path, relative: &str, value: &JsonValue) {
    write_file(dir, relative, &serde_json::to_string_pretty(value).unwrap());
}

fn initialize_db(root: &Path) -> PathBuf {
    crate::db::initialize(&root.join("app")).unwrap()
}

fn open_connection(db_path: &Path) -> Connection {
    crate::db::open(db_path).unwrap()
}

/// A small monorepo: two languages, an excluded build directory, a vendor tree and a
/// symlink that points outside the repository.
fn fixture_repository(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    write_file(
        &repo,
        "package.json",
        "{\"dependencies\":{\"express\":\"4.18.2\"}}",
    );
    write_file(
        &repo,
        "src/server.js",
        "const express = require('express');\napp.get('/user/:id', handler);\n",
    );
    write_file(
        &repo,
        "python/requirements.txt",
        "requests==2.31.0\nflask>=2.0\n",
    );
    write_file(
        &repo,
        "python/app.py",
        "def load(cur, pid):\n    cur.execute('SELECT * FROM t WHERE id=' + pid)\n",
    );
    write_file(
        &repo,
        "node_modules/left-pad/index.js",
        "module.exports = 1;\n",
    );
    write_file(&repo, "target/generated.rs", "fn generated() {}\n");
    write_file(&repo, "dist/app.min.js", "console.log(1);\n");
    write_file(&repo, "vendor/dep/lib.go", "package dep\n");
    write_file(&repo, ".venv/lib/site.py", "x = 1\n");
    write_file(&repo, ".git/config", "[core]\n");
    fs::create_dir_all(repo.join("outside")).unwrap();
    write_file(root, "outside/secret.env", "TOKEN=do-not-read\n");
    write_file(&repo, "README.md", "# fixture repository\n");
    #[cfg(unix)]
    {
        // AlreadyExists on a repeated fixture build is fine; the links are the point.
        let _ = std::os::unix::fs::symlink(root.join("outside"), repo.join("escape"));
        let _ = std::os::unix::fs::symlink(repo.join("src"), repo.join("link-to-src"));
        let _ = std::os::unix::fs::symlink(repo.join("README.md"), repo.join("link-to-readme"));
    }
    repo
}

fn scratch_dir(root: &Path) -> PathBuf {
    let path = root.join("scratch");
    fs::create_dir_all(&path).unwrap();
    path
}

fn capture(root: &Path, base: Option<&str>) -> RepositorySnapshot {
    capture_at(&fixture_repository(root), &scratch_dir(root), base)
}

fn capture_at(repo: &Path, scratch: &Path, base: Option<&str>) -> RepositorySnapshot {
    RepositorySnapshot::capture(repo, scratch, base).unwrap()
}

/// `agent_runs.scan_id` is a foreign key, so the scan has to exist first.
fn make_scan(connection: &Connection, scan_id: &str, scan_type: &str) {
    connection
        .execute(
            "INSERT OR IGNORE INTO sentinel_scans(id,project_name,status,scan_type) VALUES(?1,'native-fixture','completed',?2)",
            rusqlite::params![scan_id, scan_type],
        )
        .unwrap();
}

/// Register the run a graph fact must be attributed to.
fn make_agent_run(connection: &Connection, id: &str, scan_id: &str, role: &str) {
    make_scan(connection, scan_id, "code");
    connection
        .execute(
            "INSERT OR IGNORE INTO agent_runs(id,scan_id,attempt_number,role,root_run_id,lane,backend,status)
             VALUES(?1,?2,1,?3,?1,'read_only_analysis','native','running')",
            rusqlite::params![id, scan_id, role],
        )
        .unwrap();
}

/// A reviewer run belongs to the root whose candidates it reviews.
fn make_reviewer_run(connection: &Connection, id: &str, root_run_id: &str, scan_id: &str) {
    make_scan(connection, scan_id, "code");
    connection
        .execute(
            "INSERT OR IGNORE INTO agent_runs(id,scan_id,attempt_number,role,root_run_id,lane,backend,status)
             VALUES(?1,?3,1,'evidence_reviewer',?2,'review','native','running')",
            rusqlite::params![id, root_run_id, scan_id],
        )
        .unwrap();
}

fn sarif_document(results: Vec<JsonValue>, tool: &str) -> JsonValue {
    json!({
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {"name": tool, "version": "1.2.3", "rules": []}},
            "results": results,
        }]
    })
}

fn sarif_result(rule: &str, level: &str, message: &str, path: &str, line: u64) -> JsonValue {
    json!({
        "ruleId": rule,
        "level": level,
        "message": {"text": message},
        "locations": [{
            "physicalLocation": {
                "artifactLocation": {"uri": path},
                "region": {"startLine": line}
            }
        }]
    })
}

/// Import one directory through the canonical importer, the same way a live scan would.
fn import_directory(connection: &Connection, root: &Path, directory: &Path) {
    let cas = root.join("cas");
    let key_path = root.join("artifact-import.key");
    let roots = vec![directory.to_path_buf()];
    let limits = Limits::default();
    let context = ImportContext {
        connection,
        cas_dir: &cas,
        key_path: &key_path,
        roots: &roots,
        limits: &limits,
    };
    import_roots(&context);
}

/// The current finding candidates of one scan scope, as stored envelopes.
fn current_candidates(connection: &Connection, scan_id: &str) -> Vec<JsonValue> {
    let mut statement = connection
        .prepare(
            "SELECT r.envelope_json FROM import_projection_memberships m
             JOIN import_record_revisions r ON r.id=m.revision_id
             WHERE m.scope_key=?1 AND m.current=1 AND m.tombstone=0
               AND r.record_kind='finding_candidate'
             ORDER BY r.logical_key",
        )
        .unwrap();
    statement
        .query_map([format!("scan={scan_id}")], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(|row| row.ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

/// The current candidate logical keys of one scan scope, with the rule that produced them.
fn current_logical_keys(connection: &Connection, scan_id: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT r.logical_key || '|' || COALESCE(json_extract(r.envelope_json,'$.payload.rule_id'),'')
             FROM import_projection_memberships m
             JOIN import_record_revisions r ON r.id=m.revision_id
             WHERE m.scope_key=?1 AND m.current=1 AND m.tombstone=0
               AND r.record_kind='finding_candidate'
             ORDER BY r.logical_key",
        )
        .unwrap();
    statement
        .query_map([format!("scan={scan_id}")], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(|row| row.ok())
        .collect()
}

fn payload_text(envelope: &JsonValue, key: &str) -> String {
    envelope
        .pointer(&format!("/payload/{key}"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A value that the canonical record kept whether the format named it as a known field
/// or as an extension.
fn field_text(envelope: &JsonValue, key: &str) -> String {
    let from_payload = payload_text(envelope, key);
    if !from_payload.is_empty() {
        return from_payload;
    }
    envelope
        .pointer(&format!("/extensions/{key}"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string()
}

/// `table_count(&connection, "analyzer_runs")` or a fragment after the table name.
fn table_count(connection: &Connection, from: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {from}"), [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|error| panic!("{from} 不可读：{error}"))
}

fn process_limits(timeout_secs: u64, stdout_bytes: usize) -> ProcessLimits {
    ProcessLimits {
        timeout: Duration::from_secs(timeout_secs),
        stdout_bytes,
        stderr_bytes: 1024,
        poll: Duration::from_millis(5),
    }
}

fn cancelled_once() -> bool {
    true
}

fn never_cancelled() -> Box<dyn Fn() -> bool> {
    Box::new(|| false)
}
