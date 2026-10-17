use super::super::owned_work::test_support::Fixture;
use super::*;
use crate::db;
use serde_json::json;
fn call(f: &Fixture, name: &str, args: Value) -> Result<Value, String> {
    dispatch_project(&f.path, &f.exports, 9001, name, args)
}

#[test]
fn findings_keep_original_dto_pagination_and_reject_cross_project_reads() {
    let f = Fixture::new();
    let own = f.finding(9001, "Own public finding");
    let foreign = f.finding(9002, "Foreign finding");
    let original = exposure::read_exposure_finding_at(&f.path, own, 9001).unwrap();
    assert_eq!(
        call(
            &f,
            "read_exposure_finding",
            json!({"findingId":own,"expectedProjectId":9001})
        )
        .unwrap(),
        serde_json::to_value(original).unwrap()
    );
    assert!(call(
        &f,
        "read_exposure_finding",
        json!({"findingId":foreign,"expectedProjectId":9001})
    )
    .is_err());
    assert!(call(&f,"page_exposure_findings",json!({"input":{"projectId":9002,"status":"","search":"","pageSize":10,"cursor":"","disposition":""}})).is_err());
    let page=call(&f,"page_exposure_findings",json!({"input":{"projectId":9001,"status":"","search":"","pageSize":10,"cursor":"","disposition":""}})).unwrap();
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["id"], own);
}

#[test]
fn settings_preserve_saved_cas_and_do_not_disguise_corruption_as_defaults() {
    let f = Fixture::new();
    let initial = call(&f, "get_exposure_settings", json!({"projectId":9001})).unwrap();
    assert_eq!(initial["saved"], false);
    let expected = initial["storedVersion"].as_i64().unwrap();
    let settings = json!({"exposureMaxRequests":77});
    assert_eq!(
        call(
            &f,
            "save_exposure_settings",
            json!({"projectId":9001,"expectedVersion":expected,"settings":settings})
        )
        .unwrap(),
        1
    );
    assert!(call(
        &f,
        "save_exposure_settings",
        json!({"projectId":9001,"expectedVersion":expected,"settings":settings})
    )
    .is_err());
    db::open(&f.path)
        .unwrap()
        .execute(
            "UPDATE exposure_settings SET settings_json=?1 WHERE project_id=9001",
            [json!({"unknownField":true}).to_string()],
        )
        .unwrap();
    assert!(call(&f, "get_exposure_settings", json!({"projectId":9001}))
        .unwrap_err()
        .contains("exposure_settings_corrupt"));
}

#[test]
fn foreign_interrupted_run_is_not_silently_signed_or_resumed() {
    let f = Fixture::new();
    let run = f.exposure_run(9001, "interrupted");
    assert_eq!(
        call(
            &f,
            "resume_exposure_scan",
            json!({"runId":run,"expectedProjectId":9001})
        )
        .unwrap_err(),
        "asset_service_foreign_work_not_adopted"
    );
    assert_eq!(
        db::open(&f.path)
            .unwrap()
            .query_row("SELECT status FROM exposure_runs WHERE id=?1", [run], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "interrupted"
    );
    assert_eq!(exposure::active_background_workers(&f.path).unwrap(), 0);
}

#[test]
fn export_is_opaque_project_owned_and_original_csv_verifier_is_preserved() {
    let f = Fixture::new();
    f.finding(9001, "Synthetic export row");
    f.finding(9002, "Foreign export row");
    let output = call(
        &f,
        "export_exposure_findings",
        json!({"projectId":9001,"kind":"findings"}),
    )
    .unwrap();
    let reference = output["path"].as_str().unwrap();
    assert!(reference.starts_with("oviraptor-asset-export:"));
    assert_eq!(output["rows"], 1);
    let verified = call(&f, "verify_exposure_export", json!({"path":reference})).unwrap();
    assert_eq!(verified["path"], reference);
    assert_eq!(verified["rows"], 1);
    assert!(dispatch_project(
        &f.path,
        &f.exports,
        9002,
        "verify_exposure_export",
        json!({"path":reference})
    )
    .is_err());
    assert!(call(
        &f,
        "verify_exposure_export",
        json!({"path":f.exports.join("arbitrary.csv").to_str().unwrap()})
    )
    .is_err());
}

#[test]
fn exposure_live_worker_is_busy_even_when_database_has_only_terminal_rows() {
    use std::sync::{Arc, Barrier};
    let f = Fixture::new();
    f.exposure_run(9001, "completed");
    let ready = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let a = ready.clone();
    let b = release.clone();
    let worker = exposure::with_background_scope(&f.path, || {
        exposure::spawn_background_for_test(move || {
            a.wait();
            b.wait();
        })
    });
    ready.wait();
    let status = status_on(&f.path).unwrap();
    assert_eq!(status["busy"], true);
    assert_eq!(status["liveWorkers"], 1);
    assert_eq!(status["activeRows"], 0);
    release.wait();
    worker.join().unwrap();
    assert_eq!(status_on(&f.path).unwrap()["busy"], false);
}
