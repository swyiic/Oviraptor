use super::*;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
    directory: PathBuf,
    connection: rusqlite::Connection,
    settings: Value,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ovi-terminal-recovery-{}", Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        let connection = db::open(&path).unwrap();
        let directory = root.join("jobs").join(Uuid::new_v4().to_string());
        fs::create_dir_all(&directory).unwrap();
        let mut settings = json!({"_configurationSource":"legacy-profile","workers":4});
        begin_attempt(&mut settings);
        connection
            .execute(
                "INSERT INTO projects(id,name) VALUES(9101,'Terminal recovery fixture')",
                [],
            )
            .unwrap();
        connection.execute("INSERT INTO runs(id,project_id,profile_id,name,pipeline,status,stage,config_snapshot,output_dir) VALUES(9101,9101,1,'Fixture','reprobe','running','probe',?1,?2)",params![settings.to_string(),directory.to_string_lossy()]).unwrap();
        Self {
            root,
            path,
            directory,
            connection,
            settings,
        }
    }
    fn state(&self) -> (String, String) {
        self.connection
            .query_row("SELECT status,error FROM runs WHERE id=9101", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn failed_terminal_storage_recovers_online_and_can_resume_with_new_attempt_without_replaying_old_result(
) {
    let mut f = Fixture::new();
    f.connection.execute_batch("CREATE TRIGGER deny_terminal BEFORE UPDATE OF status ON runs WHEN NEW.status IN ('completed','failed','cancelled') BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END").unwrap();
    let result = Err("fixture collection stopped before any request".to_string());
    record(&f.directory, 9101, &f.settings, &result, false).unwrap();
    let previous = fs::read(f.directory.join(RECEIPT)).unwrap();
    assert!(job_lifecycle::finish_with_fallback(&mut f.connection, 9101, result, false).is_err());
    assert!(reconcile_on(&mut f.connection, &f.root, &HashSet::new()).is_err());
    assert_eq!(f.state().0, "running");
    f.connection
        .execute_batch("DROP TRIGGER deny_terminal")
        .unwrap();
    assert_eq!(
        reconcile_on(&mut f.connection, &f.root, &HashSet::new()).unwrap(),
        1
    );
    assert_eq!(
        f.state(),
        (
            "failed".into(),
            "fixture collection stopped before any request".into()
        )
    );
    job_resume::require_idle_project(&f.connection, 9101, None).unwrap();
    let resumed =
        job_resume::prepare_resume_with_credentials(&mut f.connection, 9101, &f.root, |_| {
            Ok(String::new())
        })
        .unwrap();
    assert_ne!(attempt(&resumed.settings), attempt(&f.settings));
    assert_eq!(resumed.settings["workers"], 4);
    // Even a cleanup failure/stale file cannot complete the resumed execution.
    fs::write(f.directory.join(RECEIPT), previous).unwrap();
    assert_eq!(
        reconcile_on(&mut f.connection, &f.root, &HashSet::new()).unwrap(),
        0
    );
    assert_eq!(f.state().0, "queued");
}

#[test]
fn bootstrap_restores_saved_completion_before_marking_only_unfinished_work_interrupted() {
    let f = Fixture::new();
    record(&f.directory, 9101, &f.settings, &Ok(()), false).unwrap();
    f.connection.execute("INSERT INTO runs(id,project_id,name,pipeline,status) VALUES(9102,9101,'Unknown unfinished','reprobe','running')",[]).unwrap();
    assert_eq!(
        crate::commands::assets::recover_interrupted_runs_at(&f.path).unwrap(),
        1
    );
    assert_eq!(f.state().0, "completed");
    let status: String = f
        .connection
        .query_row("SELECT status FROM runs WHERE id=9102", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "interrupted");
    assert!(!f.directory.join(RECEIPT).exists());
}

#[test]
fn pending_receipt_does_not_override_live_owner_or_any_existing_terminal_state() {
    let mut f = Fixture::new();
    record(&f.directory, 9101, &f.settings, &Ok(()), false).unwrap();
    assert_eq!(
        reconcile_on(&mut f.connection, &f.root, &HashSet::from([9101])).unwrap(),
        0
    );
    assert_eq!(f.state().0, "running");
    assert!(f.directory.join(RECEIPT).exists());
    for status in [
        "failed",
        "cancelled",
        "completed",
        "interrupted",
        "restarted",
    ] {
        f.connection
            .execute(
                "UPDATE runs SET status=?1,error='original preserved' WHERE id=9101",
                [status],
            )
            .unwrap();
        record(&f.directory, 9101, &f.settings, &Ok(()), false).unwrap();
        assert_eq!(
            reconcile_on(&mut f.connection, &f.root, &HashSet::new()).unwrap(),
            0
        );
        assert_eq!(f.state(), (status.into(), "original preserved".into()));
    }
}

#[test]
fn cancellation_wins_when_it_arrived_before_pending_terminal_commit() {
    let mut f = Fixture::new();
    record(&f.directory, 9101, &f.settings, &Ok(()), false).unwrap();
    job_lifecycle::request_cancel_on(&f.connection, 9101).unwrap();
    assert_eq!(
        reconcile_on(&mut f.connection, &f.root, &HashSet::new()).unwrap(),
        1
    );
    assert_eq!(f.state().0, "cancelled");
}

#[test]
fn malformed_or_untrusted_pending_receipts_never_update_run_state() {
    let mut f = Fixture::new();
    fs::write(f.directory.join(RECEIPT), b"{broken").unwrap();
    assert!(reconcile_on(&mut f.connection, &f.root, &HashSet::new()).is_err());
    assert_eq!(f.state().0, "running");
    let other = f.root.join("external");
    fs::create_dir(&other).unwrap();
    record(&other, 9101, &f.settings, &Ok(()), false).unwrap();
    f.connection
        .execute(
            "UPDATE runs SET output_dir=?1 WHERE id=9101",
            [other.to_string_lossy().to_string()],
        )
        .unwrap();
    assert!(reconcile_on(&mut f.connection, &f.root, &HashSet::new()).is_err());
    assert_eq!(f.state().0, "running");
}

#[test]
fn pending_receipts_have_private_permissions_and_contain_only_terminal_metadata() {
    let f = Fixture::new();
    let mut settings = f.settings.clone();
    settings["fofaKey"] = json!("fixture-private-key");
    record(&f.directory, 9101, &settings, &Ok(()), false).unwrap();
    let bytes = fs::read(f.directory.join(RECEIPT)).unwrap();
    assert!(!String::from_utf8(bytes)
        .unwrap()
        .contains("fixture-private-key"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(f.directory.join(RECEIPT))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn service_recovery_only_commits_receipts_born_in_its_immutable_instance() {
    let mut f = Fixture::new();
    f.connection.execute_batch("CREATE TABLE asset_service_job_owners(run_id INTEGER PRIMARY KEY,instance_id TEXT NOT NULL); INSERT INTO asset_service_job_owners VALUES(9101,'another-instance')").unwrap();
    record(&f.directory, 9101, &f.settings, &Ok(()), false).unwrap();
    assert_eq!(
        reconcile_owned_on(
            &mut f.connection,
            &f.root,
            &HashSet::new(),
            Some("current-instance")
        )
        .unwrap(),
        0
    );
    assert_eq!(f.state().0, "running");
    assert!(f.directory.join(RECEIPT).exists());
    assert_eq!(
        reconcile_owned_on(
            &mut f.connection,
            &f.root,
            &HashSet::new(),
            Some("another-instance")
        )
        .unwrap(),
        1
    );
    assert_eq!(f.state().0, "completed");
}

#[test]
fn one_corrupt_receipt_cannot_prevent_app_boot_or_recovery_of_other_valid_results() {
    let f = Fixture::new();
    fs::write(f.directory.join(RECEIPT), b"{broken").unwrap();
    let healthy = f.root.join("jobs").join(Uuid::new_v4().to_string());
    let mut settings = json!({});
    begin_attempt(&mut settings);
    f.connection.execute("INSERT INTO runs(id,project_id,profile_id,name,pipeline,status,config_snapshot,output_dir) VALUES(9102,9101,1,'Healthy pending result','reprobe','running',?1,?2)",params![settings.to_string(),healthy.to_string_lossy()]).unwrap();
    record(&healthy, 9102, &settings, &Ok(()), false).unwrap();
    assert_eq!(
        crate::commands::assets::recover_interrupted_runs_at(&f.path).unwrap(),
        1
    );
    assert_eq!(f.state().0, "interrupted");
    assert_eq!(fs::read(f.directory.join(RECEIPT)).unwrap(), b"{broken");
    let healthy_status: String = f
        .connection
        .query_row("SELECT status FROM runs WHERE id=9102", [], |r| r.get(0))
        .unwrap();
    assert_eq!(healthy_status, "completed");
    let diagnostics: i64 = f
        .connection
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE run_id=9101 AND stage='recovery'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(diagnostics, 1);
}
