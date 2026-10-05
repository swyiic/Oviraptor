use crate::native_pipeline::process::log::{Event, Journal, Scope, SCHEMA};
use rusqlite::{Connection, OpenFlags};

struct ReplayFixture {
    root: PathBuf,
    db: PathBuf,
    scope: Scope,
}
impl ReplayFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("oviraptor-native-replay-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let db = root.join("fixture.sqlite3");
        let c = Connection::open(&db).unwrap();
        c.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY,attempt_count INTEGER,status TEXT);
            CREATE TABLE sentinel_scan_attempts(scan_id TEXT,attempt_number INTEGER);
            CREATE TABLE sentinel_deleted_scans(scan_id TEXT);
            CREATE TABLE native_scan_branches(scan_id TEXT,attempt_number INTEGER,branch TEXT,status TEXT);
            CREATE TABLE native_branch_dispatches(scan_id TEXT,attempt_number INTEGER,branch TEXT,claim_id TEXT,claimed_at TEXT);
            INSERT INTO sentinel_scans VALUES('scan-a',1,'scanning'),('scan-b',1,'scanning');
            INSERT INTO sentinel_scan_attempts VALUES('scan-a',1),('scan-a',2),('scan-b',1);
            INSERT INTO native_scan_branches VALUES('scan-a',1,'source','pending'),('scan-a',2,'source','pending');
            INSERT INTO native_branch_dispatches VALUES('scan-a',1,'source','11111111-1111-4111-8111-111111111111','claimed'),
            ('scan-a',2,'source','22222222-2222-4222-8222-222222222222','claimed');").unwrap();
        c.execute_batch(SCHEMA).unwrap();
        Self {
            root,
            db,
            scope: Scope {
                scan_id: "scan-a".into(),
                attempt: 1,
                branch: "source".into(),
                dispatch_claim_id: "11111111-1111-4111-8111-111111111111".into(),
                invocation_key: "a".repeat(64),
                execution_id: Uuid::new_v4().to_string(),
                stage: "semgrep:step-0".into(),
            },
        }
    }
    fn journal(&self) -> Journal {
        Journal::begin(&self.db, self.scope.clone()).unwrap()
    }
    fn read(
        &self,
        scan: &str,
        attempt: i64,
        cursor: Option<i64>,
        execution: Option<&str>,
        after: i64,
        limit: i64,
    ) -> Result<NativeProcessLogSnapshot, String> {
        read_native_process_log_snapshot(&self.db, scan, attempt, cursor, execution, after, limit)
    }
}
impl Drop for ReplayFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn event(n: i64, message: &str) -> Event {
    Event {
        stream: "stderr",
        stream_sequence: n,
        message: message.into(),
        gap: false,
    }
}

#[test]
fn native_process_replay_real_commits_restore_after_lost_hints_without_content_dedup() {
    let f = ReplayFixture::new();
    let mut journal = f.journal();
    let first = journal
        .append(&event(1, "same"), &mut |_| Err("disconnected".into()))
        .unwrap();
    let second = journal
        .append(&event(2, "same"), &mut |_| Err("disconnected".into()))
        .unwrap();
    journal
        .append(
            &Event {
                stream: "gap",
                stream_sequence: 1,
                message: "2 records dropped".into(),
                gap: true,
            },
            &mut |_| Ok(()),
        )
        .unwrap();
    let p = f.read("scan-a", 1, None, None, 0, 1).unwrap();
    assert_eq!(p.schema_version, 1);
    assert_eq!(p.attempt, 1);
    assert!(p.page.more);
    assert_eq!(p.page.after_sequence, first);
    assert_eq!(p.page.rows[0].message, "same");
    let p = f.read("scan-a", 1, Some(1), None, first, 1).unwrap();
    assert_eq!(p.page.after_sequence, second);
    assert_eq!(p.page.rows[0].message, "same");
    let p = f.read("scan-a", 1, Some(1), None, second, 300).unwrap();
    assert_eq!(p.page.rows.len(), 1);
    assert!(p.page.rows[0].gap);
    assert!(!p.page.more);
    assert_eq!(p.page.rows[0].hint.scope, f.scope);
}

#[test]
fn native_process_replay_scope_deleted_foreign_execution_future_cursor_fail_closed() {
    let f = ReplayFixture::new();
    let mut journal = f.journal();
    journal
        .append(&event(1, "only scan-a"), &mut |_| Ok(()))
        .unwrap();
    for (scan, attempt, cursor, execution, after, limit) in [
        ("missing", 1, None, None, 0, 300),
        (
            "scan-b",
            1,
            None,
            Some(f.scope.execution_id.as_str()),
            0,
            300,
        ),
        ("scan-a", 99, None, None, 0, 300),
        ("scan-a", 1, Some(2), None, 0, 300),
        ("scan-a", 1, None, Some("not-an-id"), 0, 300),
        ("scan-a", 1, Some(1), None, 999, 300),
        ("scan-a", 1, None, None, 1, 300),
        ("scan-a", -1, None, None, 0, 300),
        ("scan-a", 1, None, None, 0, 301),
        ("scan-a", 1, None, None, 0, 0),
    ] {
        assert!(
            f.read(scan, attempt, cursor, execution, after, limit)
                .is_err(),
            "{scan} {attempt} {cursor:?} {execution:?} {after} {limit}"
        );
    }
    let c = Connection::open(&f.db).unwrap();
    c.execute("INSERT INTO sentinel_deleted_scans VALUES('scan-a')", [])
        .unwrap();
    assert!(f.read("scan-a", 1, None, None, 0, 300).is_err());
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM native_process_log_rows", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn native_process_replay_latest_attempt_switch_resets_cursor_and_keeps_history_pinned() {
    let f = ReplayFixture::new();
    let mut old = f.journal();
    let cursor = old.append(&event(1, "old"), &mut |_| Ok(())).unwrap();
    old.finish("completed", &mut |_| Ok(())).unwrap();
    let c = Connection::open(&f.db).unwrap();
    c.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id='scan-a'",
        [],
    )
    .unwrap();
    let mut scope = f.scope.clone();
    scope.attempt = 2;
    scope.execution_id = Uuid::new_v4().to_string();
    scope.dispatch_claim_id = "22222222-2222-4222-8222-222222222222".into();
    let mut current = Journal::begin(&f.db, scope).unwrap();
    current.append(&event(1, "new"), &mut |_| Ok(())).unwrap();
    let page = f.read("scan-a", 0, Some(1), None, cursor, 300).unwrap();
    assert_eq!(page.attempt, 2);
    assert!(page.reset_cursor);
    assert_eq!(page.requested_after_sequence, cursor);
    assert!(page.page.rows.iter().all(|r| r.hint.scope.attempt == 2));
    assert_eq!(page.page.rows[0].message, "new");
    let historic = f.read("scan-a", 1, None, None, 0, 300).unwrap();
    assert_eq!(historic.attempt, 1);
    assert_eq!(historic.page.rows[0].message, "old");
    assert!(f
        .read(
            "scan-a",
            0,
            Some(1),
            Some(&f.scope.execution_id),
            cursor,
            300
        )
        .is_err());
}

#[test]
fn native_process_replay_actual_sqlite_race_keeps_high_water_and_rows_in_one_snapshot() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let f = ReplayFixture::new();
    let mut journal = f.journal();
    journal
        .append(&event(1, "before"), &mut |_| Ok(()))
        .unwrap();
    let path = f.db.clone();
    let scope = f.scope.clone();
    let fired = Arc::new(AtomicBool::new(false));
    let observed = fired.clone();
    let mut reader = Connection::open_with_flags(&f.db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    reader
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "native_process_log_rows",
                    column_name: "message"
                }
            ) && !observed.swap(true, Ordering::SeqCst)
            {
                // Real WAL writer commits between the max(sequence) and row SELECTs.
                // No replay implementation or observer is replaced by this hook.
                let mut j = Journal::begin(
                    &path,
                    Scope {
                        execution_id: Uuid::new_v4().to_string(),
                        ..scope.clone()
                    },
                )
                .unwrap();
                j.append(&event(1, "after"), &mut |_| Ok(())).unwrap();
            }
            Authorization::Allow
        }))
        .unwrap();
    let page =
        read_native_process_log_snapshot_in(&mut reader, "scan-a", 1, None, None, 0, 300).unwrap();
    assert!(fired.load(Ordering::SeqCst));
    assert!(page
        .page
        .rows
        .iter()
        .all(|row| row.hint.sequence <= page.page.latest_sequence));
    assert_eq!(
        page.page.rows.len(),
        1,
        "a later commit leaked into an earlier high-water snapshot"
    );
    let later = f
        .read("scan-a", 1, Some(1), None, page.page.after_sequence, 300)
        .unwrap();
    assert_eq!(later.page.rows[0].message, "after");
}

#[test]
fn native_process_replay_missing_database_does_not_create_or_migrate_any_file() {
    let f = ReplayFixture::new();
    let missing = f.root.join("never-created.sqlite3");
    assert!(read_native_process_log_snapshot(&missing, "scan-a", 1, None, None, 0, 300).is_err());
    assert!(!missing.exists());
    let c = Connection::open(&f.db).unwrap();
    c.execute_batch("CREATE TABLE protected_business(id INTEGER,value TEXT); INSERT INTO protected_business VALUES(1,'asset-native-json');").unwrap();
    let before: Vec<String> = c
        .prepare("SELECT name FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    f.read("scan-a", 1, None, None, 0, 300).unwrap();
    let after: Vec<String> = c
        .prepare("SELECT name FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        c.query_row("SELECT value FROM protected_business WHERE id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "asset-native-json"
    );
}
