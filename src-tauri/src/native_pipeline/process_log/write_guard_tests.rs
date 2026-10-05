use super::*;
use rusqlite::OpenFlags;
use std::{fs, path::PathBuf};
struct Fixture {
    root: PathBuf,
    db: PathBuf,
    scope: Scope,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-native-write-guard-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let db = root.join("fixture.sqlite3");
        let c = Connection::open(&db).unwrap();
        c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA recursive_triggers=OFF;
   CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY,attempt_count INTEGER,status TEXT);
   CREATE TABLE sentinel_scan_attempts(scan_id TEXT,attempt_number INTEGER);
   CREATE TABLE sentinel_deleted_scans(scan_id TEXT);
   CREATE TABLE native_scan_branches(scan_id TEXT,attempt_number INTEGER,branch TEXT,status TEXT);
   CREATE TABLE native_branch_dispatches(scan_id TEXT,attempt_number INTEGER,branch TEXT,claim_id TEXT,claimed_at TEXT);
   INSERT INTO sentinel_scans VALUES('scan-a',1,'scanning');INSERT INTO sentinel_scan_attempts VALUES('scan-a',1);
   INSERT INTO native_scan_branches VALUES('scan-a',1,'source','pending');
   INSERT INTO native_branch_dispatches VALUES('scan-a',1,'source','11111111-1111-4111-8111-111111111111','claimed');
   CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT);INSERT INTO projects VALUES(1,'assets-preserved');
   CREATE TABLE agent_runs(id TEXT PRIMARY KEY,plan_json TEXT);INSERT INTO agent_runs VALUES('other-root','{\"native\":{\"bytes\":\"complete-original\"}}');
   CREATE TABLE asset_business(id TEXT PRIMARY KEY,payload BLOB);INSERT INTO asset_business VALUES('protected',x'0011ff');").unwrap();
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
                execution_id: uuid::Uuid::new_v4().to_string(),
                stage: "semgrep:step-0".into(),
            },
        }
    }
    fn snapshot(&self) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        let c = Connection::open(&self.db).unwrap();
        let tables: Vec<String> = c
            .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        tables
            .into_iter()
            .map(|table| {
                let mut s = c
                    .prepare(&format!(
                        "SELECT * FROM \"{}\" ORDER BY rowid",
                        table.replace('"', "\"\"")
                    ))
                    .unwrap();
                let columns = s.column_count();
                let rows = s
                    .query_map([], |r| (0..columns).map(|i| r.get(i)).collect())
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                (table, rows)
            })
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn event(n: i64) -> Event {
    Event {
        stream: "stderr",
        stream_sequence: n,
        message: "ordinary complete text".into(),
        gap: false,
    }
}

#[test]
fn native_process_write_guard_begin_blocks_unrelated_business_writes_and_ignored_binding() {
    for sql in [
  "CREATE TRIGGER damage AFTER INSERT ON native_process_log_executions BEGIN UPDATE projects SET name='damaged'; END;",
  "CREATE TRIGGER damage AFTER INSERT ON native_process_log_executions BEGIN UPDATE agent_runs SET plan_json='{}' WHERE id='other-root'; END;",
  "CREATE TRIGGER damage AFTER INSERT ON native_process_log_executions BEGIN DELETE FROM asset_business; END;",
  "CREATE TRIGGER damage BEFORE INSERT ON native_process_log_executions BEGIN SELECT RAISE(IGNORE); END;",
  "CREATE TRIGGER damage AFTER INSERT ON native_process_log_executions BEGIN UPDATE native_process_log_executions SET row_count=90 WHERE execution_id=NEW.execution_id; END;",
 ] {let f=Fixture::new();let c=Connection::open(&f.db).unwrap();c.execute_batch(sql).unwrap();let before=f.snapshot();
  assert!(Journal::begin(&f.db,f.scope.clone()).is_err(),"begin silently granted log scope: {sql}");
  assert_eq!(f.snapshot(),before,"begin failed to roll back all table rows: {sql}");
 }
}

#[test]
fn native_process_write_guard_append_and_finish_do_not_notify_phantom_or_side_effect_rows() {
    for sql in [
  "CREATE TRIGGER damage AFTER INSERT ON native_process_log_rows BEGIN UPDATE projects SET name='damaged'; END;",
  "CREATE TRIGGER damage AFTER INSERT ON native_process_log_rows BEGIN UPDATE agent_runs SET plan_json='{}' WHERE id='other-root'; END;",
  "CREATE TRIGGER damage AFTER INSERT ON native_process_log_rows BEGIN DELETE FROM asset_business; END;",
  "CREATE TRIGGER damage BEFORE INSERT ON native_process_log_rows BEGIN SELECT RAISE(IGNORE); END;",
  "CREATE TRIGGER damage BEFORE UPDATE OF row_count ON native_process_log_executions BEGIN SELECT RAISE(IGNORE); END;",
  "CREATE TRIGGER damage AFTER INSERT ON native_process_log_rows BEGIN DELETE FROM native_process_log_rows WHERE sequence=NEW.sequence; END;",
  "CREATE TRIGGER damage AFTER INSERT ON native_process_log_rows BEGIN INSERT INTO native_process_log_rows(execution_id,stream,stream_sequence,message) VALUES(NEW.execution_id,'stdout',99,'extra phantom row'); END;",
 ] {for terminal in [false,true] {
  let f=Fixture::new();let mut j=Journal::begin(&f.db,f.scope.clone()).unwrap();let c=Connection::open(&f.db).unwrap();
  c.execute_batch(sql).unwrap();let before=f.snapshot();let mut notified=0;
  let mut notify=|_:&Hint|{notified+=1;Ok(())};
  let outcome=if terminal {j.finish("cancelled",&mut notify)}else{j.append(&event(1),&mut notify)};
  assert!(outcome.is_err(),"{terminal}/{sql} claimed commit");assert_eq!(notified,0,"phantom commit notification");
  assert_eq!(f.snapshot(),before,"{terminal}/{sql} changed protected rows or counters");
  c.execute_batch("DROP TRIGGER damage;").unwrap();
  let seq=j.append(&event(1),&mut |_|Ok(())).unwrap();assert!(seq>0,"failure leaked authorizer/transaction state");
 }}
}

#[test]
fn native_process_write_guard_replace_cannot_implicitly_delete_any_unique_identity() {
    let f = Fixture::new();
    let mut j = Journal::begin(&f.db, f.scope.clone()).unwrap();
    let sequence = j.append(&event(1), &mut |_| Ok(())).unwrap();
    let c = Connection::open(&f.db).unwrap();
    c.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
    let before = f.snapshot();
    for sql in [
  format!("INSERT OR REPLACE INTO native_process_log_rows(sequence,execution_id,stream,stream_sequence,message) VALUES({sequence},'{}','stdout',77,'new sequence collision')",f.scope.execution_id),
  format!("INSERT OR REPLACE INTO native_process_log_rows(execution_id,stream,stream_sequence,message) VALUES('{}','stderr',1,'changed unique stream')",f.scope.execution_id),
  format!("INSERT OR REPLACE INTO native_process_log_executions(execution_id,scan_id,attempt_number,branch,dispatch_claim_id,invocation_key,stage) VALUES('{}','scan-a',1,'source','{}','{}','changed-stage')",f.scope.execution_id,f.scope.dispatch_claim_id,f.scope.invocation_key),
 ] {assert!(c.execute_batch(&sql).is_err(),"REPLACE bypassed immutable journal: {sql}");assert_eq!(f.snapshot(),before);}
}

#[test]
fn native_process_write_guard_late_cancelled_diagnostics_keep_original_scope_without_new_rights() {
    let f = Fixture::new();
    let mut j = Journal::begin(&f.db, f.scope.clone()).unwrap();
    let c = Connection::open(&f.db).unwrap();
    c.execute_batch(
        "UPDATE sentinel_scans SET status='paused',attempt_count=2;
  UPDATE native_scan_branches SET status='cancelled';",
    )
    .unwrap();
    assert!(Journal::begin(
        &f.db,
        Scope {
            execution_id: uuid::Uuid::new_v4().to_string(),
            ..f.scope.clone()
        }
    )
    .is_err());
    let seq = j
        .append(&event(1), &mut |hint| {
            let read =
                Connection::open_with_flags(&f.db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let p = page(&read, "scan-a", 1, None, 0, 300).unwrap();
            assert!(p.rows.iter().any(|r| r.hint.sequence == hint.sequence));
            assert_eq!(hint.scope, f.scope);
            Ok(())
        })
        .unwrap();
    assert!(seq > 0);
    j.finish("cancelled", &mut |_| Err("disconnected".into()))
        .unwrap();
    let native: String = c
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id='other-root'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(native, "{\"native\":{\"bytes\":\"complete-original\"}}");
    assert_eq!(
        c.query_row("SELECT state FROM native_process_log_executions", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "cancelled"
    );
    assert_eq!(c.query_row("SELECT COUNT(*) FROM native_branch_dispatches WHERE claim_id='11111111-1111-4111-8111-111111111111'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}
