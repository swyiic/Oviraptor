use super::{test_support::Fixture, *};

#[test]
fn unowned_nonterminal_rows_block_recovery_and_report_unknown_instead_of_adoption() {
    let f = Fixture::new();
    let run = f.exposure_run(9001, "running");
    assert_eq!(
        guard_recovery(&f.path, Family::Exposure).unwrap_err(),
        "asset_service_foreign_work_not_adopted"
    );
    let status = status_on(&f.path, Family::Exposure, 0).unwrap();
    assert_eq!(status["busy"], true);
    assert_eq!(status["uncertain"], true);
    assert_eq!(status["unownedRows"], 1);
    assert!(require_existing_owned(&f.path, Kind::ExposureRun, &run.to_string()).is_err());
    assert!(!directory(&f.path, &f.instance).unwrap().exists());
}

#[test]
fn own_birth_receipt_does_not_prove_idle_until_terminal_and_worker_exit() {
    let f = Fixture::new();
    let run = f.ownership_run(9001, "running");
    record(&f.path, Kind::OwnershipAnalysis, 9001, &run.to_string()).unwrap();
    guard_recovery(&f.path, Family::Ownership).unwrap();
    assert_eq!(
        status_on(&f.path, Family::Ownership, 0).unwrap()["uncertain"],
        true
    );
    assert_eq!(
        status_on(&f.path, Family::Ownership, 1).unwrap()["uncertain"],
        false
    );
    db::open(&f.path)
        .unwrap()
        .execute(
            "UPDATE ownership_analysis_runs SET status='completed' WHERE id=?1",
            [run],
        )
        .unwrap();
    assert_eq!(
        status_on(&f.path, Family::Ownership, 1).unwrap()["busy"],
        true
    );
    assert_eq!(
        status_on(&f.path, Family::Ownership, 0).unwrap()["busy"],
        false
    );
}

#[test]
fn changed_birth_or_instance_cannot_reuse_a_durable_receipt() {
    let f = Fixture::new();
    let run = f.exposure_run(9001, "interrupted");
    record(&f.path, Kind::ExposureRun, 9001, &run.to_string()).unwrap();
    require_existing_owned(&f.path, Kind::ExposureRun, &run.to_string()).unwrap();
    db::open(&f.path)
        .unwrap()
        .execute(
            "UPDATE exposure_runs SET created_at='2000-01-01 00:00:00' WHERE id=?1",
            [run],
        )
        .unwrap();
    assert!(require_existing_owned(&f.path, Kind::ExposureRun, &run.to_string()).is_err());
    let g = Fixture::new();
    assert!(require_existing_owned(&g.path, Kind::ExposureRun, &run.to_string()).is_err());
}

#[test]
fn receipt_io_failure_is_sticky_unknown_for_only_that_database() {
    let f = Fixture::new();
    let g = Fixture::new();
    let run = f.exposure_run(9001, "completed");
    fs::write(directory(&f.path, &f.instance).unwrap(), b"not a directory").unwrap();
    assert_eq!(
        record(&f.path, Kind::ExposureRun, 9001, &run.to_string()).unwrap_err(),
        "asset_service_work_receipt_outcome_unknown"
    );
    assert_eq!(
        status_on(&f.path, Family::Exposure, 0).unwrap()["uncertain"],
        true
    );
    assert_eq!(
        status_on(&g.path, Family::Exposure, 0).unwrap()["busy"],
        false
    );
}

#[test]
fn recovery_checks_all_projects_because_legacy_startup_recovery_is_database_wide() {
    let f = Fixture::new();
    let own = f.exposure_run(9001, "running");
    record(&f.path, Kind::ExposureRun, 9001, &own.to_string()).unwrap();
    f.exposure_run(9002, "running");
    assert!(guard_recovery(&f.path, Family::Exposure).is_err());
    assert_eq!(
        status_on(&f.path, Family::Exposure, 1).unwrap()["unownedRows"],
        1
    );
}
