use super::*;
use crate::native_app_sdk::HostProject;
struct Fixture {
    db: Connection,
    c: NativeCallContext,
}
impl Fixture {
    fn new() -> Self {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT NOT NULL DEFAULT 'active');CREATE TABLE assets(id INTEGER PRIMARY KEY,asset_key TEXT NOT NULL,link TEXT NOT NULL DEFAULT '',host TEXT NOT NULL DEFAULT '');CREATE TABLE project_assets(project_id INTEGER NOT NULL,asset_id INTEGER NOT NULL,is_deleted INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(project_id,asset_id));CREATE TABLE asset_service_store(singleton INTEGER PRIMARY KEY,instance_id TEXT NOT NULL,schema_version INTEGER NOT NULL);INSERT INTO projects(id,name)VALUES(7,'Synthetic native'),(8,'Other synthetic');").unwrap();
        db.execute(
            "INSERT INTO asset_service_store VALUES(1,?1,1)",
            [uuid::Uuid::new_v4().to_string()],
        )
        .unwrap();
        initialize_on(&db).unwrap();
        Self {
            db,
            c: NativeCallContext {
                host_bound_project_id: 55,
                grant_revision: 1,
                operation_id: "synthetic-selection-0001".into(),
                app_revision: 1,
                permission: "inventory.read".into(),
                permissions: vec!["inventory.read".into()],
                project: HostProject {
                    id: 55,
                    name: "Synthetic host project".into(),
                    status: "active".into(),
                },
            },
        }
    }
    fn asset(&self, id: i64, project: i64, url: &str) {
        self.db
            .execute(
                "INSERT INTO assets(id,asset_key,link)VALUES(?1,?2,?3)",
                params![id, format!("synthetic-{id}"), url],
            )
            .unwrap();
        self.db
            .execute(
                "INSERT INTO project_assets(project_id,asset_id)VALUES(?1,?2)",
                params![project, id],
            )
            .unwrap();
    }
    fn prepare(&self, ids: Vec<i64>) -> Result<Value, String> {
        prepare_on(
            &self.db,
            7,
            &self.c,
            json!({"assetIds":ids,"mode":"standard"}),
        )
    }
    fn receipts(&self) -> i64 {
        self.db
            .query_row("SELECT COUNT(*) FROM asset_scan_selections", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
}
#[test]
fn asset_scan_selection_resolves_all_current_owned_members_and_deduplicates_only_urls() {
    let f = Fixture::new();
    f.asset(1, 7, "HTTPS://Example.test:443/path#original-fragment");
    f.asset(2, 7, "https://example.test/path");
    let p = f.prepare(vec![2, 1]).unwrap();
    assert_eq!(p["kind"], "scan.targets.v1");
    assert_eq!(p["targets"], json!(["https://example.test/path"]));
    assert_eq!(p["mode"], "standard");
    assert_eq!(p["source"]["assetCount"], 2);
    assert_eq!(p["source"]["kind"], "asset-selection");
    assert!(uuid::Uuid::parse_str(p["source"]["selectionId"].as_str().unwrap()).is_ok());
    assert_eq!(p["source"]["selectionDigest"].as_str().unwrap().len(), 64);
    assert_eq!(f.receipts(), 1);
    assert_eq!(p.as_object().unwrap().len(), 4);
    assert_eq!(p["source"].as_object().unwrap().len(), 5);
    assert!(p.get("nativeProjectId").is_none());
    assert!(p.get("path").is_none());
}
#[test]
fn asset_scan_selection_rejects_cross_project_missing_or_recycled_members_without_partial_receipt()
{
    let f = Fixture::new();
    f.asset(1, 7, "https://owned.test/");
    f.asset(2, 8, "https://other.test/");
    f.asset(3, 7, "https://archived.test/");
    f.db.execute(
        "UPDATE project_assets SET is_deleted=1 WHERE asset_id=3",
        [],
    )
    .unwrap();
    for ids in [vec![1, 2], vec![1, 3], vec![1, 999]] {
        assert!(f.prepare(ids).unwrap_err().contains("member_unavailable"));
        assert_eq!(f.receipts(), 0);
    }
}
#[test]
fn asset_scan_selection_replay_rechecks_membership_and_exact_original_url_facts() {
    let f = Fixture::new();
    f.asset(1, 7, "https://saved.test/path#first");
    let first = f.prepare(vec![1]).unwrap();
    assert_eq!(f.prepare(vec![1]).unwrap(), first);
    assert_eq!(f.receipts(), 1);
    f.db.execute(
        "UPDATE assets SET link='https://saved.test/path#changed' WHERE id=1",
        [],
    )
    .unwrap();
    assert!(f.prepare(vec![1]).unwrap_err().contains("source_changed"));
    let mut fresh = f.c.clone();
    fresh.operation_id = "fresh-selection-0002".into();
    let second = prepare_on(&f.db, 7, &fresh, json!({"assetIds":[1],"mode":"standard"})).unwrap();
    assert_ne!(
        first["source"]["selectionId"],
        second["source"]["selectionId"]
    );
    assert_eq!(first["targets"], second["targets"]);
    f.db.execute(
        "UPDATE project_assets SET is_deleted=1 WHERE asset_id=1",
        [],
    )
    .unwrap();
    assert!(
        prepare_on(&f.db, 7, &fresh, json!({"assetIds":[1],"mode":"standard"}))
            .unwrap_err()
            .contains("member_unavailable")
    );
    assert_eq!(f.receipts(), 2);
}
#[test]
fn asset_scan_selection_operation_binds_mode_members_recipient_and_both_revisions() {
    let f = Fixture::new();
    f.asset(1, 7, "https://one.test/");
    f.asset(2, 7, "https://two.test/");
    f.prepare(vec![1]).unwrap();
    for args in [
        json!({"assetIds":[2],"mode":"standard"}),
        json!({"assetIds":[1],"mode":"deep"}),
    ] {
        assert!(prepare_on(&f.db, 7, &f.c, args)
            .unwrap_err()
            .contains("operation_conflict"));
    }
    for change in 0..3 {
        let mut c = f.c.clone();
        match change {
            0 => c.app_revision += 1,
            1 => c.grant_revision += 1,
            _ => {
                c.host_bound_project_id = 56;
                c.project.id = 56;
            }
        }
        assert!(
            prepare_on(&f.db, 7, &c, json!({"assetIds":[1],"mode":"standard"}))
                .unwrap_err()
                .contains("operation_conflict")
        );
    }
    assert_eq!(f.receipts(), 1);
}
#[test]
fn asset_scan_selection_thousand_members_can_map_to_two_hundred_targets_without_count_guessing() {
    let f = Fixture::new();
    for id in 1..=1000 {
        f.asset(
            id,
            7,
            &format!("https://saved.test/path-{}", (id - 1) % 200),
        );
    }
    let p = f.prepare((1..=1000).rev().collect()).unwrap();
    assert_eq!(p["source"]["assetCount"], 1000);
    assert_eq!(p["targets"].as_array().unwrap().len(), 200);
    assert_eq!(f.receipts(), 1);
}
#[test]
fn asset_scan_selection_two_hundred_one_unique_targets_requires_explicit_split_no_truncation() {
    let f = Fixture::new();
    for id in 1..=201 {
        f.asset(id, 7, &format!("https://saved.test/{id}"));
    }
    assert!(f
        .prepare((1..=201).collect())
        .unwrap_err()
        .contains("split_batch_required"));
    assert_eq!(f.receipts(), 0);
    let assets: i64 =
        f.db.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
            .unwrap();
    assert_eq!(assets, 201);
}
#[test]
fn asset_scan_selection_input_is_closed_positive_unique_integer_and_finite_mode() {
    let f = Fixture::new();
    f.asset(1, 7, "https://saved.test/");
    for args in [
        json!({"assetIds":[],"mode":"standard"}),
        json!({"assetIds":[0],"mode":"standard"}),
        json!({"assetIds":[-1],"mode":"standard"}),
        json!({"assetIds":[1,1],"mode":"standard"}),
        json!({"assetIds":[1.5],"mode":"standard"}),
        json!({"assetIds":[9_007_199_254_740_992_i64],"mode":"standard"}),
        json!({"assetIds":vec![1;1001],"mode":"standard"}),
        json!({"assetIds":[1],"mode":"shell"}),
        json!({"assetIds":[1],"mode":"standard","projectId":8}),
        json!({"assetIds":[1],"mode":"standard","targets":["https://forged.test/"]}),
    ] {
        assert!(prepare_on(&f.db, 7, &f.c, args).is_err());
    }
    assert_eq!(f.receipts(), 0);
}
#[test]
fn asset_scan_selection_urls_are_literal_http_candidates_not_inferred_services_or_credentials() {
    for raw in [
        "ssh://service.test:22",
        "javascript:alert(1)",
        "file:///tmp/synthetic",
        "plain-host.test",
        "https://synthetic:synthetic-password@example.test/",
        "https://saved.test/a,b",
        "https://saved.test/white space",
        "https://saved.test/\\path",
        "https://saved.test:0/",
    ] {
        let f = Fixture::new();
        f.asset(1, 7, raw);
        assert!(
            f.prepare(vec![1]).is_err(),
            "synthetic malformed URL unexpectedly accepted"
        );
        assert_eq!(f.receipts(), 0);
    }
    let f = Fixture::new();
    f.asset(1, 7, "");
    f.db.execute(
        "UPDATE assets SET host='https://explicit-host.test/path' WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(
        f.prepare(vec![1]).unwrap()["targets"],
        json!(["https://explicit-host.test/path"])
    );
}
#[test]
fn asset_scan_selection_utf8_canonicalization_and_final_byte_limits_are_exact() {
    let f = Fixture::new();
    f.asset(1, 7, "https://例子.test/路径");
    let p = f.prepare(vec![1]).unwrap();
    let target = p["targets"][0].as_str().unwrap();
    assert!(target.is_ascii());
    assert!(target.contains("xn--"));
    assert!(target.contains('%'));
    let f = Fixture::new();
    for id in 1..=200 {
        let base = format!("https://saved.test/{id}/");
        let url = format!("{base}{}", "a".repeat(MAX_URL_BYTES - base.len()));
        assert_eq!(url.len(), MAX_URL_BYTES);
        f.asset(id, 7, &url);
    }
    let p = f.prepare((1..=200).collect()).unwrap();
    assert_eq!(p["targets"].as_array().unwrap().len(), 200);
    assert!(serde_json::to_vec(&p).unwrap().len() <= MAX_OUTPUT_BYTES);
    assert!(serde_json::to_vec(&p).unwrap().len() > 64 * 1024);
}
#[test]
fn asset_scan_selection_never_borrows_transaction_or_grants_execution_permission() {
    let f = Fixture::new();
    f.asset(1, 7, "https://saved.test/");
    f.db.execute_batch("BEGIN").unwrap();
    assert!(f
        .prepare(vec![1])
        .unwrap_err()
        .contains("requires_current_connection"));
    f.db.execute_batch("ROLLBACK").unwrap();
    let mut c = f.c.clone();
    c.permissions.clear();
    assert!(prepare_on(&f.db, 7, &c, json!({"assetIds":[1],"mode":"standard"})).is_err());
    c = f.c.clone();
    c.project.status = "archived".into();
    assert!(prepare_on(&f.db, 7, &c, json!({"assetIds":[1],"mode":"standard"})).is_err());
    let p = f.prepare(vec![1]).unwrap();
    assert!(p.get("executionStarted").is_none());
    let native_tables:i64=f.db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('sentinel_scans','runs','native_branch_dispatches')",[],|r|r.get(0)).unwrap();
    assert_eq!(native_tables, 0);
}
#[test]
fn asset_scan_selection_immutable_receipts_retain_hashed_originals_not_extra_raw_url_copies() {
    let f = Fixture::new();
    f.asset(1, 7, "https://saved.test/path#original-private-fragment");
    let p = f.prepare(vec![1]).unwrap();
    let id = p["source"]["selectionId"].as_str().unwrap();
    let(source,payload):(String,String)=f.db.query_row("SELECT source_facts_sha256,payload_json FROM asset_scan_selections WHERE selection_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(source.len(), 64);
    assert!(!payload.contains("original-private-fragment"));
    assert!(f
        .db
        .execute(
            "UPDATE asset_scan_selections SET member_count=99 WHERE selection_id=?1",
            [id]
        )
        .is_err());
    assert!(f
        .db
        .execute(
            "DELETE FROM asset_scan_selections WHERE selection_id=?1",
            [id]
        )
        .is_err());
}
