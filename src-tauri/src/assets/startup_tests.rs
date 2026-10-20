use super::*;

struct Fixture {
    root: std::path::PathBuf,
    path: std::path::PathBuf,
    connection: rusqlite::Connection,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ovi-startup-regression-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("synthetic.sqlite3");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("
            PRAGMA journal_mode=WAL;
            CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT);
            CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT);
            CREATE TABLE targets(project_id INTEGER,enabled INTEGER);
            CREATE TABLE project_assets(project_id INTEGER,is_deleted INTEGER);
            CREATE TABLE runs(id INTEGER PRIMARY KEY,project_id INTEGER,profile_id INTEGER,name TEXT,pipeline TEXT,status TEXT,stage TEXT,error TEXT,created_at TEXT,finished_at TEXT);
            INSERT INTO projects VALUES(1,'Synthetic startup','active');
            INSERT INTO targets VALUES(1,1);
        ").unwrap();
        for (index, status) in ["queued", "running", "cancel_requested", "interrupted", "completed", "failed", "cancelled"].iter().enumerate() {
            connection.execute("INSERT INTO runs VALUES(?1,1,NULL,'Synthetic job','collect',?2,?2,'original detail','2026-10-18 00:00:00',NULL)", rusqlite::params![index as i64 + 1, status]).unwrap();
        }
        Self { root, path, connection }
    }
    fn snapshot(&self) -> String {
        self.connection.query_row("SELECT json_group_array(json_array(id,status,stage,error,finished_at)) FROM (SELECT * FROM runs ORDER BY id)", [], |row| row.get(0)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

#[test]
fn repeated_recovery_status_reads_preserve_running_and_terminal_jobs() {
    let f = Fixture::new();
    let before = f.snapshot();
    for _ in 0..3 {
        let status = startup_status_at(&f.path).unwrap();
        assert_eq!(status.interrupted_jobs.len(), 1);
        assert_eq!(status.interrupted_jobs[0].run_id, 4);
        assert_eq!(f.snapshot(), before);
    }
    // A precheck failure must still be able to finish the original live run.
    assert_eq!(f.connection.execute("UPDATE runs SET status='failed',stage='failed',error='synthetic precheck failure' WHERE id=2 AND status='running'", []).unwrap(), 1);
    startup_status_at(&f.path).unwrap();
    let error: String = f.connection.query_row("SELECT error FROM runs WHERE id=2", [], |row| row.get(0)).unwrap();
    assert_eq!(error, "synthetic precheck failure");
}

#[test]
fn recovery_status_can_be_read_while_another_connection_holds_the_writer_lock() {
    let f = Fixture::new();
    f.connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let before = f.snapshot();
    let status = startup_status_at(&f.path).unwrap();
    assert_eq!(status.interrupted_jobs.len(), 1);
    assert_eq!(f.snapshot(), before);
    f.connection.execute_batch("ROLLBACK;").unwrap();
}

#[test]
fn bootstrap_recovers_old_jobs_but_later_status_reads_preserve_resumed_jobs() {
    let f = Fixture::new();
    assert_eq!(recover_interrupted_runs_at(&f.path).unwrap(), 3);
    assert_eq!(startup_status_at(&f.path).unwrap().interrupted_jobs.len(), 4);
    // The existing resume path makes one interrupted job live again.
    f.connection.execute("UPDATE runs SET status='running',stage='collect',error='',finished_at=NULL WHERE id=2", []).unwrap();
    let before = f.snapshot();
    for _ in 0..3 {
        let status = startup_status_at(&f.path).unwrap();
        assert_eq!(status.interrupted_jobs.len(), 3);
        assert!(!status.interrupted_jobs.iter().any(|job| job.run_id == 2));
        assert_eq!(f.snapshot(), before);
    }
    for (id, expected) in [(5, "completed"), (6, "failed"), (7, "cancelled")] {
        let actual: (String, String) = f.connection.query_row("SELECT status,error FROM runs WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(actual, (expected.into(), "original detail".into()));
    }
}
