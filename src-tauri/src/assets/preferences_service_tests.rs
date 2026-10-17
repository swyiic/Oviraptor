use super::super::owned_work::test_support::Fixture;
use super::*;
use crate::db;
use serde_json::json;

fn initial(f: &Fixture) -> Connection {
    let c = db::open(&f.path).unwrap();
    initialize_on(&c).unwrap();
    c
}
fn valid() -> Preferences {
    Preferences {
        columns: Some(vec!["host".into(), "title".into()]),
        column_widths: Some(vec![ColumnWidth {
            column: "host".into(),
            width: 178.5,
        }]),
        portal_columns_v1: Some(true),
        agent_scan_mode: Some("deep".into()),
    }
}

#[test]
fn reads_require_explicit_original_store_initialization_not_fake_empty_preferences() {
    let f = Fixture::new();
    let c = db::open(&f.path).unwrap();
    assert!(read_on(&c).is_err());
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_service_preferences')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!exists);
}
#[test]
fn fresh_nullable_layout_has_exactly_four_fixed_fields_and_read_is_not_a_write() {
    let f = Fixture::new();
    let c = initial(&f);
    let first = read_on(&c).unwrap();
    assert_eq!(first.revision, 0);
    assert_eq!(first.preferences, Preferences::default());
    let value = serde_json::to_value(first).unwrap();
    assert_eq!(value["preferences"].as_object().unwrap().len(), 4);
    assert!(value["preferences"]
        .as_object()
        .unwrap()
        .values()
        .all(Value::is_null));
    let changes = c.total_changes();
    read_on(&c).unwrap();
    assert_eq!(c.total_changes(), changes);
}
#[test]
fn original_instance_bound_layout_roundtrips_order_fractional_width_and_scan_preference_only() {
    let f = Fixture::new();
    let mut c = initial(&f);
    let saved = save_on(&mut c, 0, valid()).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.preferences, valid());
    drop(c);
    let c = db::open(&f.path).unwrap();
    assert_eq!(read_on(&c).unwrap().preferences, valid());
    let projects: i64 = c
        .query_row("SELECT COUNT(*) FROM exposure_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(projects, 0);
}
#[test]
fn stale_layout_cas_preserves_the_first_saved_snapshot() {
    let f = Fixture::new();
    let mut c = initial(&f);
    save_on(&mut c, 0, valid()).unwrap();
    assert_eq!(
        save_on(&mut c, 0, Preferences::default()).unwrap_err(),
        "asset_preferences_revision_conflict"
    );
    let current = read_on(&c).unwrap();
    assert_eq!(current.revision, 1);
    assert_eq!(current.preferences, valid());
}
#[test]
fn concurrent_same_revision_has_exactly_one_committed_winner() {
    use std::sync::{Arc, Barrier};
    let f = Fixture::new();
    initial(&f);
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for width in [200.0, 300.0] {
        let path = f.path.clone();
        let gate = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let mut c = db::open(&path).unwrap();
            let mut p = valid();
            p.column_widths.as_mut().unwrap()[0].width = width;
            gate.wait();
            save_on(&mut c, 0, p)
        }));
    }
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| r
                .as_ref()
                .is_err_and(|e| e == "asset_preferences_revision_conflict"))
            .count(),
        1
    );
    assert_eq!(read_on(&db::open(&f.path).unwrap()).unwrap().revision, 1);
}
#[test]
fn unknown_columns_duplicate_columns_and_paths_are_not_preference_authority() {
    let f = Fixture::new();
    let mut c = initial(&f);
    for columns in [
        vec!["/private/path".into()],
        vec!["__proto__".into()],
        vec!["host".into(), "host".into()],
    ] {
        let mut p = valid();
        p.columns = Some(columns);
        assert!(save_on(&mut c, 0, p).is_err());
    }
    assert_eq!(read_on(&c).unwrap().revision, 0);
}
#[test]
fn width_bounds_nonfinite_values_duplicates_and_unknown_keys_are_rejected() {
    let f = Fixture::new();
    let mut c = initial(&f);
    for width in [67.9, 720.1, f64::NAN, f64::INFINITY] {
        let mut p = valid();
        p.column_widths.as_mut().unwrap()[0].width = width;
        assert!(save_on(&mut c, 0, p).is_err());
    }
    let mut duplicate = valid();
    duplicate.column_widths.as_mut().unwrap().push(ColumnWidth {
        column: "host".into(),
        width: 100.0,
    });
    assert!(save_on(&mut c, 0, duplicate).is_err());
    assert_eq!(read_on(&c).unwrap().revision, 0);
}
#[test]
fn nullable_snapshot_clear_is_explicit_and_invalid_modes_or_revision_never_write() {
    let f = Fixture::new();
    let mut c = initial(&f);
    save_on(&mut c, 0, valid()).unwrap();
    save_on(&mut c, 1, Preferences::default()).unwrap();
    assert_eq!(read_on(&c).unwrap().preferences, Preferences::default());
    let mut invalid = valid();
    invalid.agent_scan_mode = Some("automatic-paid".into());
    assert!(save_on(&mut c, 2, invalid).is_err());
    assert!(save_on(&mut c, MAX_REVISION, valid()).is_err());
    assert!(save_on(&mut c, -1, valid()).is_err());
    assert_eq!(read_on(&c).unwrap().revision, 2);
}
#[test]
fn explicit_false_portal_marker_roundtrips_as_not_yet_migrated() {
    let f = Fixture::new();
    let mut c = initial(&f);
    let mut preferences = valid();
    preferences.portal_columns_v1 = Some(false);
    let saved = save_on(&mut c, 0, preferences.clone()).unwrap();
    assert_eq!(saved.preferences, preferences);
    assert_eq!(
        read_on(&c).unwrap().preferences.portal_columns_v1,
        Some(false)
    );
}
#[test]
fn application_arguments_are_closed_and_missing_snapshot_fields_are_not_silent_clears() {
    let f = Fixture::new();
    let mut c = initial(&f);
    for args in [json!({"dbPath":"/other"}), json!({"projectId":9001})] {
        assert!(dispatch_application(&mut c, "get_asset_preferences", args).is_err());
    }
    assert!(dispatch_application(
        &mut c,
        "save_asset_preferences",
        json!({"expectedRevision":0,"preferences":{"columns":null}})
    )
    .is_err());
    assert!(dispatch_application(&mut c,"save_asset_preferences",json!({"expectedRevision":0,"preferences":{"columns":null,"columnWidths":null,"portalColumnsV1":null,"agentScanMode":null,"credentials":"unacceptable"}})).is_err());
    assert_eq!(read_on(&c).unwrap().revision, 0);
}
#[test]
fn corrupt_existing_preferences_or_missing_identity_guards_are_never_reset() {
    let f = Fixture::new();
    let c = initial(&f);
    c.execute("UPDATE asset_service_preferences SET revision=revision+1,preferences_json=?1",[json!({"columns":["unknown"],"columnWidths":null,"portalColumnsV1":null,"agentScanMode":null}).to_string()]).unwrap();
    assert_eq!(
        read_on(&c).unwrap_err(),
        "asset_preferences_stored_value_invalid"
    );
    assert!(initialize_on(&c).is_err());
    assert_eq!(
        c.query_row("SELECT revision FROM asset_service_preferences", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    c.execute_batch("DROP TRIGGER asset_service_preferences_delete")
        .unwrap();
    assert_eq!(
        read_on(&c).unwrap_err(),
        "asset_preferences_schema_unavailable"
    );
}
#[test]
fn existing_store_cannot_change_preference_identity_or_delete_the_singleton() {
    let f = Fixture::new();
    let c = initial(&f);
    assert!(c
        .execute("DELETE FROM asset_service_preferences", [])
        .is_err());
    assert!(c
        .execute(
            "UPDATE asset_service_preferences SET revision=revision+2",
            []
        )
        .is_err());
    assert!(c.execute("UPDATE asset_service_preferences SET instance_id='not-an-instance',revision=revision+1",[]).is_err());
    assert_eq!(read_on(&c).unwrap().revision, 0);
}
#[test]
fn finite_column_contract_matches_the_actual_shared_assets_view() {
    let source = include_str!("../../../src/features/assets/components/AssetWorkspace.vue");
    let block = source
        .split("const allColumns = [")
        .nth(1)
        .unwrap()
        .split("] as const;")
        .next()
        .unwrap();
    let columns = block
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("[\"")
                .and_then(|s| s.split('"').next())
        })
        .collect::<Vec<_>>();
    assert_eq!(columns, COLUMNS);
    assert_eq!(columns.iter().collect::<HashSet<_>>().len(), COLUMNS.len());
}
