use super::super::owned_work::test_support::Fixture;
use super::*;
use crate::db;
use serde_json::json;

fn call(f: &Fixture, name: &str, args: Value) -> Result<Value, String> {
    dispatch_project(&f.path, 9001, name, args)
}

#[test]
fn profile_adapter_preserves_original_full_dto_and_transactional_cas() {
    let f = Fixture::new();
    let read = call(&f, "get_asset_ownership_profile", json!({"projectId":9001})).unwrap();
    assert_eq!(
        read,
        serde_json::to_value(ownership::get_asset_ownership_profile_at(&f.path, 9001).unwrap())
            .unwrap()
    );
    let mut profile = read;
    profile["legalName"] = json!("Synthetic legal entity");
    profile["approvedDomains"] = json!(["example.test"]);
    let saved = call(
        &f,
        "save_asset_ownership_profile",
        json!({"input":profile,"expectedVersion":0}),
    )
    .unwrap();
    assert_eq!(saved["policyVersion"], 1);
    let mut stale = saved.clone();
    stale["legalName"] = json!("Stale must not write");
    assert!(call(
        &f,
        "save_asset_ownership_profile",
        json!({"input":stale,"expectedVersion":0})
    )
    .unwrap_err()
    .starts_with("OWNERSHIP_PROFILE_VERSION_CONFLICT:"));
    assert_eq!(
        call(&f, "get_asset_ownership_profile", json!({"projectId":9001})).unwrap(),
        saved
    );
}

#[test]
fn confirm_is_fact_only_and_authorization_requires_current_confirmed_expectation() {
    let f = Fixture::new();
    let selections = json!([{"projectId":9001,"assetId":1}]);
    let seen = json!([Fixture::expectation(1)]);
    assert!(call(&f,"authorize_asset_ownership",json!({"input":{"selections":selections,"note":"Synthetic"},"expectedProjectId":9001,"expectedDecisions":seen})).is_err());
    let confirmed=call(&f,"confirm_asset_ownership",json!({"input":{"selections":selections,"note":"Human fact","domainRuleScope":"none"},"expectedProjectId":9001,"expectedDecisions":seen})).unwrap();
    assert_eq!(confirmed["changed"], 1);
    assert_eq!(
        confirmed["expectations"][0]["authorizationStatus"],
        "unknown"
    );
    assert_eq!(confirmed["expectations"][0]["exposureEligible"], false);
    assert!(call(
        &f,
        "authorize_asset_ownership",
        json!({"input":{"selections":selections},"expectedProjectId":9001,"expectedDecisions":seen})
    )
    .is_err());
    assert_eq!(call(&f,"authorize_asset_ownership",json!({"input":{"selections":selections,"note":"Separate explicit authorization"},"expectedProjectId":9001,"expectedDecisions":confirmed["expectations"]})).unwrap(),1);
    let conn = db::open(&f.path).unwrap();
    let state:(String,bool)=conn.query_row("SELECT authorization_status,exposure_eligible FROM asset_ownership_decisions WHERE project_id=9001 AND asset_id=1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, ("allowed".into(), true));
}

#[test]
fn mixed_scope_bulk_is_rejected_before_any_ownership_write_and_outer_fields_are_closed() {
    let f = Fixture::new();
    assert!(call(&f,"confirm_asset_ownership",json!({"input":{"selections":[{"projectId":9001,"assetId":1},{"projectId":9002,"assetId":3}]},"expectedProjectId":9001,"expectedDecisions":[Fixture::expectation(1),Fixture::expectation(3)]})).is_err());
    let count: i64 = db::open(&f.path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM asset_ownership_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    assert!(call(&f, "get_asset_ownership_profile", json!({"projectId":9002})).is_err());
    assert!(call(
        &f,
        "get_asset_ownership_profile",
        json!({"projectId":9001,"dbPath":"/untrusted"})
    )
    .is_err());
}

#[test]
fn global_directory_is_application_only_and_never_uses_project_zero() {
    let f = Fixture::new();
    let args = json!({"input":{"projectId":9001,"expectedDirectoryVersion":0,"expectedBindingVersion":0,"entities":[{"draftKey":"synthetic-group","legalName":"合成测试集团有限公司","entityKind":"group","source":"manual","sourceRef":"synthetic adapter fixture"}],"identifiers":[],"binding":{"projectKind":"group_ledger","group":{"draftKey":"synthetic-group"}}}});
    assert!(!project_commands().contains(&"save_project_ownership_directory"));
    assert!(call(&f, "save_project_ownership_directory", args.clone()).is_err());
    let actual =
        dispatch_application(&f.path, 9001, "save_project_ownership_directory", args).unwrap();
    assert_eq!(actual["projectId"], 9001);
    assert_eq!(actual["directoryVersion"], 1);
    assert_eq!(actual["bindingVersion"], 1);
    assert_eq!(actual["createdEntities"].as_array().unwrap().len(), 1);
    assert_eq!(actual["createdEntities"][0]["draftKey"], "synthetic-group");
    let bootstrap = call(
        &f,
        "get_project_ownership_bootstrap",
        json!({"projectId":9001}),
    )
    .unwrap();
    assert_eq!(bootstrap["directoryVersion"], 1);
    assert_eq!(bootstrap["binding"]["bindingVersion"], 1);
    assert_eq!(
        bootstrap["binding"]["groupEntityId"],
        actual["createdEntities"][0]["entityId"]
    );
    assert_eq!(bootstrap["authorization"]["allowed"], 0);
    assert_eq!(bootstrap["authorization"]["exposureEligible"], 0);
    assert!(dispatch_application(
        &f.path,
        0,
        "save_project_ownership_directory",
        json!({"input":{"projectId":0,"expectedDirectoryVersion":0,"expectedBindingVersion":0}})
    )
    .is_err());
}

#[test]
fn old_nonterminal_analysis_cannot_be_adopted_by_read_startup_recovery() {
    let f = Fixture::new();
    let id = f.ownership_run(9001, "running");
    assert_eq!(
        call(&f, "read_ownership_analysis", json!({"projectId":9001})).unwrap_err(),
        "asset_service_foreign_work_not_adopted"
    );
    assert_eq!(
        db::open(&f.path)
            .unwrap()
            .query_row(
                "SELECT status FROM ownership_analysis_runs WHERE id=?1",
                [id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "running"
    );
    assert_eq!(status_on(&f.path).unwrap()["uncertain"], true);
}

#[test]
fn terminal_database_still_reports_busy_while_real_owner_thread_is_alive() {
    use std::sync::{Arc, Barrier};
    let f = Fixture::new();
    let ready = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let a = ready.clone();
    let b = release.clone();
    let worker = ownership::with_background_scope(&f.path, || {
        ownership::spawn_background_for_test(move || {
            a.wait();
            b.wait();
        })
    });
    ready.wait();
    let status = status_on(&f.path).unwrap();
    assert_eq!(status["busy"], true);
    assert_eq!(status["liveWorkers"], 1);
    assert_eq!(status["activeRows"], 0);
    // Exposure is a separate private module registry, even at the same DB path.
    assert_eq!(
        crate::commands::exposure::active_background_workers(&f.path).unwrap(),
        0
    );
    release.wait();
    worker.join().unwrap();
    assert_eq!(status_on(&f.path).unwrap()["busy"], false);
}
