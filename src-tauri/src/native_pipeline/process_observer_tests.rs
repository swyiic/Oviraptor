//! Contract tests use real local children and the production observer/journal.
//! No browser, remote traffic, CAS or project database is used.
use super::*;
use rusqlite::{Connection, OpenFlags};
use std::{fs, path::PathBuf};

struct Fixture {
    root: PathBuf,
    db: PathBuf,
    scope: log::Scope,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-live-process-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let db = root.join("fixture.sqlite3");
        let connection = Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
            CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY,attempt_count INTEGER,status TEXT);
            CREATE TABLE sentinel_scan_attempts(scan_id TEXT,attempt_number INTEGER);
            CREATE TABLE sentinel_deleted_scans(scan_id TEXT);
            CREATE TABLE native_scan_branches(scan_id TEXT,attempt_number INTEGER,branch TEXT,status TEXT);
            CREATE TABLE native_branch_dispatches(scan_id TEXT,attempt_number INTEGER,branch TEXT,claim_id TEXT,claimed_at TEXT);
            INSERT INTO native_scan_branches VALUES('scan-a',1,'source','pending');
            INSERT INTO native_branch_dispatches VALUES('scan-a',1,'source','11111111-1111-4111-8111-111111111111','fixture-claimed');
            INSERT INTO sentinel_scans VALUES('scan-a',1,'scanning'),('scan-b',1,'scanning');
            INSERT INTO sentinel_scan_attempts VALUES('scan-a',1),('scan-b',1);",
            )
            .unwrap();
        connection.execute_batch(log::SCHEMA).unwrap();
        Self {
            root,
            db,
            scope: log::Scope {
                scan_id: "scan-a".into(),
                attempt: 1,
                branch: "source".into(),
                dispatch_claim_id: "11111111-1111-4111-8111-111111111111".into(),
                invocation_key: "a".repeat(64),
                execution_id: uuid::Uuid::new_v4().to_string(),
                stage: "semgrep:step-0".into(),
            },
        }
    }
    fn read(&self, after: i64, limit: i64) -> log::Page {
        let reader =
            Connection::open_with_flags(&self.db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        log::page(
            &reader,
            &self.scope.scan_id,
            self.scope.attempt,
            Some(&self.scope.execution_id),
            after,
            limit,
        )
        .unwrap()
    }
    fn run(
        &self,
        script: &str,
        cancel: &dyn Fn() -> bool,
        observe: &mut dyn FnMut(&log::Event) -> Result<(), String>,
    ) -> ObservedRun {
        run_observed(
            Path::new("/bin/sh"),
            &["-c".into(), script.into()],
            Some(&self.root),
            &ProcessLimits {
                timeout: Duration::from_millis(900),
                poll: Duration::from_millis(2),
                ..Default::default()
            },
            cancel,
            observe,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn native_process_observer_persists_both_streams_before_child_can_exit_and_before_notify() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let mut before_exit = false;
    let mut notifications = 0;
    let mut notify = |hint: &log::Hint| {
        // Separate read-only connection sees the commit before notification.
        let page = f.read(0, 300);
        assert!(page
            .rows
            .iter()
            .any(|row| row.hint.sequence == hint.sequence));
        assert_eq!(hint.scope, f.scope);
        notifications += 1;
        let has_out = page
            .rows
            .iter()
            .any(|row| row.hint.stream == "stdout" && row.message == "first");
        let has_err = page
            .rows
            .iter()
            .any(|row| row.hint.stream == "stderr" && row.message == "warning");
        if has_out && has_err && !f.root.join("ended").exists() {
            before_exit = true;
            fs::write(f.root.join("gate"), b"release").unwrap();
        }
        Ok(())
    };
    let result = f.run(
        "printf 'first\\n'; printf 'warning\\n' >&2;
        while [ ! -f gate ]; do sleep 0.01; done; printf 'last\\n'; printf ended > ended",
        &|| false,
        &mut |event| journal.append(event, &mut notify).map(|_| ()),
    );
    assert!(
        before_exit,
        "logs were not committed while the child waited for its gate"
    );
    assert!(notifications >= 3);
    assert!(result.run.succeeded());
    assert!(result.log_error.is_none());
    assert_eq!(result.run.stdout, b"first\nlast\n");
    assert_eq!(result.run.stderr, b"warning\n");
}

#[test]
fn native_process_observer_redacts_cross_chunk_secrets_and_preserves_utf8_records() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let result = f.run("printf 'Authoriz' >&2; sleep 0.03;
        printf 'ation: Be' >&2; sleep 0.03; printf 'arer bearer-synthetic-value\\n' >&2;
        printf 'password=pass' >&2; sleep 0.03; printf 'word-synthetic-value\\n' >&2;
        printf 'api_key=api-' >&2; sleep 0.03; printf 'synthetic-value\\n' >&2;
        printf '\\345\\256'; sleep 0.03; printf '\\214\\346\\225\\264\\346\\226\\207\\346\\234\\254\\n'",
        &|| false,&mut |event| journal.append(event,&mut |_| Ok(())).map(|_| ()));
    assert!(result.run.succeeded());
    let page = f.read(0, 300);
    assert!(page.rows.iter().any(|row| row.message == "完整文本"));
    let text = page
        .rows
        .iter()
        .map(|row| row.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for secret in [
        "bearer-synthetic-value",
        "password-synthetic-value",
        "api-synthetic-value",
    ] {
        assert!(
            !text.contains(secret),
            "a cross-chunk secret reached persisted rows"
        );
    }
    assert_eq!(text.matches("<redacted:").count(), 3);
    // The capture remains raw bytes; only the diagnostic copy is redacted.
    assert!(result.run.stderr_text().contains("bearer-synthetic-value"));
}

#[test]
fn native_process_observer_retains_byte_exact_json_stdout_and_stderr_eof_tail() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let result = f.run(
        "printf '%s' '{\"schemaVersion\":'; sleep 0.03;
        printf '%s' '1,\"password\":\"json-synthetic-secret\",\"routes\":[\"/catalog\"]}';
        printf 'password=stderr-synthetic-secret' >&2",
        &|| false,
        &mut |event| journal.append(event, &mut |_| Ok(())).map(|_| ()),
    );
    let expected =
        br#"{"schemaVersion":1,"password":"json-synthetic-secret","routes":["/catalog"]}"#;
    assert_eq!(result.run.stdout, expected);
    let parsed: serde_json::Value = serde_json::from_slice(&result.run.stdout).unwrap();
    assert_eq!(parsed["password"], "json-synthetic-secret");
    assert!(result.log_error.is_none());
    let page = f.read(0, 300);
    assert!(page.rows.iter().any(|row| row.hint.stream == "stderr"));
    let text = format!("{:?}", page.rows);
    assert!(!text.contains("stderr-synthetic-secret"));
    assert!(!text.contains("json-synthetic-secret"));
}

#[test]
fn native_process_observer_disconnected_notifications_replay_identical_rows_without_scope_bleed() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let result = f.run(
        "printf 'same\\nsame\\n'; printf 'error-context\\n' >&2",
        &|| false,
        &mut |event| {
            journal
                .append(event, &mut |_| Err("fixture transport disconnected".into()))
                .map(|_| ())
        },
    );
    assert!(result.run.succeeded());
    assert!(
        result.log_error.is_none(),
        "notification loss is not a persistence failure"
    );
    let first = f.read(0, 1);
    assert!(first.more);
    let mut rows = first.rows;
    let rest = f.read(first.after_sequence, 300);
    rows.extend(rest.rows);
    assert_eq!(rows.iter().filter(|row| row.message == "same").count(), 2);
    assert!(rows
        .windows(2)
        .all(|pair| pair[0].hint.sequence < pair[1].hint.sequence));
    let reader = Connection::open(&f.db).unwrap();
    for (scan, attempt, execution) in [
        ("scan-b", 1, f.scope.execution_id.as_str()),
        ("scan-a", 2, f.scope.execution_id.as_str()),
        ("scan-a", 1, "different-execution"),
    ] {
        assert!(log::page(&reader, scan, attempt, Some(execution), 0, 300)
            .unwrap()
            .rows
            .is_empty());
    }
    assert!(rows.iter().all(|row| row.hint.scope == f.scope));
    let after_restart = log::page(&reader, "scan-a", 1, None, 0, 300).unwrap();
    assert!(after_restart.available);
    assert_eq!(
        after_restart.rows.len(),
        rows.len(),
        "recovery cannot depend on retained execution IDs"
    );
    assert!(log::page(
        &reader,
        "scan-a",
        1,
        Some(&f.scope.execution_id),
        i64::MAX,
        300
    )
    .is_err());
    assert!(log::page(&reader, "scan-a", 0, Some(&f.scope.execution_id), 0, 300).is_err());
}

mod negative_contracts {
    use super::*;
    include!("process_observer_negative_tests.rs");
}
mod source_entry_contract {
    use super::*;
    include!("process_observer_source_tests.rs");
}
