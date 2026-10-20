use super::*;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,name TEXT,description TEXT,is_default INTEGER,settings_json TEXT);").unwrap();
    db.execute("INSERT INTO config_profiles VALUES(1,'legacy','',1,?1)",[json!({"workers":17,"pageSize":61,"modelProfiles":[],"proxyUrl":"http://legacy-user:legacy-secret@127.0.0.1:9","historical":{"kept":true}}).to_string()]).unwrap();
    db
}
fn input(db: &Connection, kind: &str) -> SaveCategoryInput {
    let current = read_category(db, kind, None).unwrap();
    SaveCategoryInput {
        request_id: uuid::Uuid::new_v4().to_string(),
        section: kind.into(),
        mode: SaveMode::Create,
        name: format!("New {kind}"),
        settings: current.settings.clone(),
        expected_settings: current.settings,
        expected_reference: None,
        expected_default: current.default_selection,
        make_default: true,
    }
}
fn raw(db: &Connection) -> String {
    db.query_row(
        "SELECT settings_json FROM config_profiles WHERE id=1",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
fn tables(db: &Connection) -> i64 {
    db.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type='table'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn category_reads_are_lazy_and_report_actual_legacy_fallback_without_migration() {
    let db = fixture();
    let before = raw(&db);
    let count = tables(&db);
    let changes = db.total_changes();
    let probe = read_category(&db, "probe", None).unwrap();
    assert_eq!(probe.source, "legacy-fallback");
    assert_eq!(probe.legacy_profile_id, Some(1));
    assert_eq!(probe.settings["workers"], 17);
    assert_eq!(probe.builtin_settings["workers"], 96);
    assert!(probe.entries.is_empty());
    assert!(probe.default_selection.is_none());
    assert!(read_receipt(&db, &uuid::Uuid::new_v4().to_string())
        .unwrap()
        .is_none());
    let fallback = policies::read_profile(&db, 1).unwrap();
    assert_eq!(read_defaults(&db, &fallback).unwrap()["workers"], 17);
    assert_eq!(tables(&db), count);
    assert_eq!(db.total_changes(), changes);
    assert_eq!(raw(&db), before);
}

#[test]
fn each_category_creates_only_one_owner_and_other_defaults_remain_unchanged() {
    let mut db = fixture();
    let before = raw(&db);
    let mut selected: BTreeMap<String, Option<DefaultSelection>> = BTreeMap::new();
    for kind in SECTIONS {
        let mut request = input(&db, kind);
        if kind == "network" {
            request.settings = json!({"proxyUrl":"http://new-user:new-secret@127.0.0.1:8080","noProxy":"localhost","agentProxyEnabled":true,"authorizedProxyPool":["EU|socks5h://pool-user:pool-secret@[::1]:1080"]});
        }
        if kind == "probe" {
            request.settings["workers"] = json!(4);
        }
        let ack = save(&mut db, request).unwrap();
        let reference = ack.receipt.reference.clone().unwrap();
        assert_eq!(reference.kind, kind);
        assert_eq!(reference.revision, 1);
        for (previous, selection) in &selected {
            assert_eq!(
                &read_category(&db, previous, None)
                    .unwrap()
                    .default_selection,
                selection
            );
        }
        selected.insert(kind.to_string(), ack.current_default);
    }
    assert_eq!(selected.len(), 8);
    assert_eq!(raw(&db), before);
    let defaults=read_defaults(&db,&json!({"workers":250,"pageSize":12,"proxyUrl":"http://ignored-user:ignored-secret@127.0.0.1:2"})).unwrap();
    assert_eq!(defaults["workers"], 4);
    assert_eq!(defaults["pageSize"], 61);
    assert_eq!(
        defaults["proxyUrl"],
        "http://new-user:new-secret@127.0.0.1:8080"
    );
    let rows: Vec<String> = db
        .prepare("SELECT receipt_json FROM configuration_category_receipts")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for text in rows {
        for secret in ["new-secret", "pool-secret", "legacy-secret"] {
            assert!(!text.contains(secret));
        }
    }
    let entries =
        serde_json::to_string(&read_category(&db, "network", None).unwrap().entries).unwrap();
    assert!(!entries.contains("new-user"));
    assert!(!entries.contains("new-secret"));
}

#[test]
fn revise_is_cas_guarded_and_fork_creates_a_fresh_owner_without_rebinding_profiles() {
    let mut db = fixture();
    let first = save(&mut db, input(&fixture(), "probe")).unwrap();
    let view = read_category(&db, "probe", None).unwrap();
    let mut request = SaveCategoryInput {
        request_id: uuid::Uuid::new_v4().to_string(),
        section: "probe".into(),
        mode: SaveMode::Revise,
        name: view.entries[0].name.clone(),
        settings: view.settings.clone(),
        expected_settings: view.settings.clone(),
        expected_reference: view.reference.clone(),
        expected_default: view.default_selection.clone(),
        make_default: true,
    };
    request.settings["workers"] = json!(5);
    let second = save(&mut db, request.clone()).unwrap();
    assert_eq!(
        second.receipt.reference.as_ref().unwrap().policy_id,
        first.receipt.reference.as_ref().unwrap().policy_id
    );
    assert_eq!(second.receipt.reference.as_ref().unwrap().revision, 2);
    request.request_id = uuid::Uuid::new_v4().to_string();
    assert!(save(&mut db, request.clone())
        .unwrap_err()
        .contains("default_conflict"));
    let old = first.receipt.reference.clone().unwrap();
    let base = read_reference(&db, &old).unwrap();
    request.expected_default = second.current_default.clone();
    request.expected_reference = Some(old.clone());
    request.expected_settings = base.clone();
    assert!(save(&mut db, request.clone())
        .unwrap_err()
        .contains("head_conflict"));
    request.request_id = uuid::Uuid::new_v4().to_string();
    request.mode = SaveMode::Fork;
    request.name = "Fork only probe".into();
    request.settings = base;
    request.settings["workers"] = json!(6);
    request.make_default = false;
    let fork = save(&mut db, request).unwrap();
    assert_ne!(
        fork.receipt.reference.as_ref().unwrap().policy_id,
        old.policy_id
    );
    assert_eq!(fork.receipt.reference.as_ref().unwrap().revision, 1);
    assert_eq!(fork.current_default, second.current_default);
    assert_eq!(policies::read_profile(&db, 1).unwrap()["workers"], 17);
}

#[test]
fn selection_cas_prevents_aba_and_explicit_builtin_clears_legacy_reference_and_credentials() {
    let mut db = fixture();
    let a = save(&mut db, input(&fixture(), "network")).unwrap();
    let view = read_category(&db, "network", None).unwrap();
    let mut b = input(&db, "network");
    b.name = "Other".into();
    b.make_default = false;
    b.settings["proxyUrl"] = json!("http://different-user:different-secret@127.0.0.1:90");
    let b = save(&mut db, b).unwrap();
    let switch = SelectCategoryInput {
        request_id: uuid::Uuid::new_v4().to_string(),
        section: "network".into(),
        reference: b.receipt.reference.clone(),
        expected_default: view.default_selection,
    };
    let b_selected = select(&mut db, switch).unwrap();
    let back = select(
        &mut db,
        SelectCategoryInput {
            request_id: uuid::Uuid::new_v4().to_string(),
            section: "network".into(),
            reference: a.receipt.reference.clone(),
            expected_default: b_selected.current_default,
        },
    )
    .unwrap();
    assert!(select(
        &mut db,
        SelectCategoryInput {
            request_id: uuid::Uuid::new_v4().to_string(),
            section: "network".into(),
            reference: b.receipt.reference,
            expected_default: a.current_default
        }
    )
    .unwrap_err()
    .contains("default_conflict"));
    select(
        &mut db,
        SelectCategoryInput {
            request_id: uuid::Uuid::new_v4().to_string(),
            section: "network".into(),
            reference: None,
            expected_default: back.current_default,
        },
    )
    .unwrap();
    let value=read_defaults(&db,&json!({"proxyUrl":"http://legacy-user:legacy-secret@127.0.0.1:9","authorizedProxyPool":["ALL|http://pool:secret@127.0.0.1:8"],"_configurationRefs":{"network":{"policyId":"old","revision":1,"kind":"network"}},"workers":21})).unwrap();
    assert_eq!(value["proxyUrl"], "");
    assert_eq!(value["authorizedProxyPool"], json!([]));
    assert!(value
        .get("_configurationRefs")
        .and_then(|v| v.get("network"))
        .is_none());
    assert_eq!(value["workers"], 21);
    assert_eq!(
        read_category(&db, "network", None).unwrap().source,
        "builtin"
    );
}

#[test]
fn request_replay_is_exact_and_reports_current_default_separately() {
    let mut db = fixture();
    let request = input(&db, "collection");
    let first = save(&mut db, request.clone()).unwrap();
    assert_eq!(
        save(&mut db, request.clone()).unwrap().receipt,
        first.receipt
    );
    let mut changed = request.clone();
    changed.settings["pageSize"] = json!(88);
    assert!(save(&mut db, changed)
        .unwrap_err()
        .contains("request_conflict"));
    select(
        &mut db,
        SelectCategoryInput {
            request_id: uuid::Uuid::new_v4().to_string(),
            section: "collection".into(),
            reference: None,
            expected_default: first.current_default.clone(),
        },
    )
    .unwrap();
    let retry = save(&mut db, request).unwrap();
    assert_eq!(retry.receipt, first.receipt);
    assert_eq!(retry.current_state, "modified");
    assert_ne!(retry.current_default, retry.receipt.default_selection);
    assert_eq!(
        read_receipt(&db, &first.receipt.request_id)
            .unwrap()
            .unwrap()
            .receipt,
        first.receipt
    );
}

#[test]
fn late_receipt_failure_rolls_back_owner_credentials_default_and_schema() {
    let mut db = fixture();
    let before = raw(&db);
    let setup = save(&mut db, input(&fixture(), "probe")).unwrap();
    db.execute_batch("CREATE TRIGGER synthetic_receipt_failure BEFORE INSERT ON configuration_category_receipts BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    let mut request = input(&db, "network");
    request.settings["proxyUrl"] = json!("http://rollback-user:rollback-secret@127.0.0.1:9");
    let count = tables(&db);
    assert!(save(&mut db, request)
        .unwrap_err()
        .contains("receipt_write_failed"));
    assert_eq!(tables(&db), count);
    assert_eq!(
        read_category(&db, "network", None)
            .unwrap()
            .default_selection,
        None
    );
    assert_eq!(
        read_category(&db, "probe", None).unwrap().default_selection,
        setup.current_default
    );
    assert_eq!(raw(&db), before);
}

#[test]
fn defaults_work_without_any_legacy_profile_and_do_not_create_profile_bindings() {
    let mut db = Connection::open_in_memory().unwrap();
    let view = read_category(&db, "models", None).unwrap();
    assert_eq!(view.source, "builtin");
    assert!(!view.legacy_available);
    let request = SaveCategoryInput {
        request_id: uuid::Uuid::new_v4().to_string(),
        section: "models".into(),
        mode: SaveMode::Create,
        name: "Independent".into(),
        settings: json!({"modelProfiles":[{"id":"one","name":"Only model","llm":"fixture-model","deployment":"cloud","apiBase":"https://fixture.invalid/v1","apiKey":"fixture-only-secret","localApiKey":""}],"activeModelProfileId":"one"}),
        expected_settings: view.settings,
        expected_reference: None,
        expected_default: None,
        make_default: true,
    };
    let result = save(&mut db, request).unwrap();
    let defaults = read_defaults(&db, &json!({})).unwrap();
    assert_eq!(
        defaults["modelProfiles"][0]["apiKey"],
        "fixture-only-secret"
    );
    assert_eq!(
        policies::reference(&defaults, "models").unwrap(),
        result.receipt.reference
    );
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM configuration_connection_bindings",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn fofa_account_identity_is_versioned_and_old_descriptors_without_email_still_read() {
    let mut db = fixture();
    let mut request = input(&db, "assetApis");
    request.settings = json!({"fofaEmail":"fixture-account@example.invalid","fofaKey":"fixture-fofa-secret","configPath":""});
    let first = save(&mut db, request).unwrap();
    let first_ref = first.receipt.reference.unwrap();
    let current = read_category(&db, "assetApis", None).unwrap();
    assert_eq!(
        current.settings["fofaEmail"],
        "fixture-account@example.invalid"
    );
    let mut changed = current.settings.clone();
    changed["fofaEmail"] = json!("second-account@example.invalid");
    let second = save(
        &mut db,
        SaveCategoryInput {
            request_id: uuid::Uuid::new_v4().to_string(),
            section: "assetApis".into(),
            mode: SaveMode::Revise,
            name: current.entries[0].name.clone(),
            settings: changed,
            expected_settings: current.settings,
            expected_reference: Some(first_ref.clone()),
            expected_default: current.default_selection,
            make_default: true,
        },
    )
    .unwrap();
    assert_eq!(
        read_reference(&db, &first_ref).unwrap()["fofaEmail"],
        "fixture-account@example.invalid"
    );
    assert_eq!(
        read_reference(&db, second.receipt.reference.as_ref().unwrap()).unwrap()["fofaEmail"],
        "second-account@example.invalid"
    );
    let text:String=db.query_row("SELECT descriptor_json FROM configuration_connection_versions WHERE owner_id=?1 AND revision=1",[&first_ref.policy_id],|r|r.get(0)).unwrap();
    assert!(!text.contains("fixture-fofa-secret"));
    let mut old: Value = serde_json::from_str(&text).unwrap();
    old.as_object_mut().unwrap().remove("fofaEmail");
    let text = old.to_string();
    let id = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO configuration_connection_versions(owner_id,revision,kind,descriptor_json,descriptor_digest,source_profile_id,source_digest) VALUES(?1,1,'assetApis',?2,?3,1,'fixture')",params![id,text,format!("{:x}",Sha256::digest(text.as_bytes()))]).unwrap();
    let old_ref = PolicyReference {
        policy_id: id.clone(),
        revision: 1,
        kind: "assetApis".into(),
    };
    assert_eq!(read_reference(&db, &old_ref).unwrap()["fofaEmail"], "");
    db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.fofaEmail','legacy-account@example.invalid') WHERE id=1",[]).unwrap();
    db.execute(
        "INSERT INTO configuration_connection_bindings VALUES(1,'assetApis',?1,1)",
        [id],
    )
    .unwrap();
    assert_eq!(
        policies::read_profile(&db, 1).unwrap()["fofaEmail"],
        "legacy-account@example.invalid"
    );
}

#[test]
fn two_connections_to_private_temporary_database_reject_stale_category_selection() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-category-cas-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let path = root.join("synthetic.sqlite3");
    let mut first = Connection::open(&path).unwrap();
    let mut second = Connection::open(&path).unwrap();
    let mut a = input(&first, "probe");
    let mut b = input(&second, "probe");
    a.settings["workers"] = json!(4);
    b.settings["workers"] = json!(5);
    let selected = save(&mut first, a).unwrap();
    assert!(save(&mut second, b)
        .unwrap_err()
        .contains("default_conflict"));
    assert_eq!(
        read_category(&second, "probe", None)
            .unwrap()
            .default_selection,
        selected.current_default
    );
    assert_eq!(
        read_category(&second, "probe", None).unwrap().settings["workers"],
        4
    );
    first.close().unwrap();
    second.close().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deferred_commit_failure_does_not_acknowledge_a_partial_category_or_secret() {
    let mut db = fixture();
    save(&mut db, input(&fixture(), "probe")).unwrap();
    db.execute_batch("CREATE TABLE synthetic_parent(id INTEGER PRIMARY KEY);CREATE TABLE synthetic_deferred(id INTEGER REFERENCES synthetic_parent(id) DEFERRABLE INITIALLY DEFERRED);CREATE TRIGGER synthetic_deferred_failure AFTER INSERT ON configuration_category_receipts BEGIN INSERT INTO synthetic_deferred VALUES(999); END;").unwrap();
    let request = input(&db, "network");
    let id = request.request_id.clone();
    let count = tables(&db);
    assert_eq!(
        save(&mut db, request).unwrap_err(),
        "configuration_category_commit_unconfirmed_retry_same_request"
    );
    assert!(read_receipt(&db, &id).unwrap().is_none());
    assert_eq!(
        read_category(&db, "network", None)
            .unwrap()
            .default_selection,
        None
    );
    assert_eq!(tables(&db), count);
}

#[test]
fn explicit_builtin_model_selection_removes_legacy_connection_aliases_and_references() {
    let mut db = fixture();
    select(
        &mut db,
        SelectCategoryInput {
            request_id: uuid::Uuid::new_v4().to_string(),
            section: "models".into(),
            reference: None,
            expected_default: None,
        },
    )
    .unwrap();
    let legacy = json!({"modelProfiles":[{"id":"old","apiKey":"old-cloud-secret","localApiKey":"old-local-secret"}],"activeModelProfileId":"old","modelName":"old","modelDeployment":"local","modelApiBase":"http://old.invalid","modelApiKey":"old-cloud-secret","localApiKey":"old-local-secret","_configurationRefs":{"models":{"policyId":"old","revision":1,"kind":"models"}},"workers":7});
    let actual = read_defaults(&db, &legacy).unwrap();
    assert_eq!(actual["modelProfiles"], json!([]));
    assert_eq!(actual["activeModelProfileId"], "");
    assert_eq!(actual["modelApiKey"], "");
    assert_eq!(actual["localApiKey"], "");
    assert_eq!(actual["modelApiBase"], "");
    assert!(actual
        .get("_configurationRefs")
        .and_then(|r| r.get("models"))
        .is_none());
    assert_eq!(actual["workers"], 7);
    assert!(!actual.to_string().contains("old-cloud-secret"));
}

#[test]
fn selecting_or_forking_another_category_reference_is_rejected_without_writes() {
    let mut db = fixture();
    let probe = save(&mut db, input(&fixture(), "probe")).unwrap();
    let before = db.total_changes();
    let bad = select(
        &mut db,
        SelectCategoryInput {
            request_id: uuid::Uuid::new_v4().to_string(),
            section: "network".into(),
            reference: probe.receipt.reference.clone(),
            expected_default: None,
        },
    )
    .unwrap_err();
    assert!(bad.contains("reference_invalid"));
    assert_eq!(db.total_changes(), before);
    let mut request = input(&db, "network");
    request.mode = SaveMode::Fork;
    request.expected_reference = probe.receipt.reference;
    assert!(save(&mut db, request)
        .unwrap_err()
        .contains("reference_invalid"));
    assert_eq!(db.total_changes(), before);
}

#[test]
fn current_consumers_ignore_broken_unrelated_defaults_and_do_not_import_unknown_legacy_fields() {
    let mut db = fixture();
    let model = save(&mut db, input(&fixture(), "models")).unwrap();
    let assets = read_current_defaults_for(
        &db,
        &["assetApis", "collection", "probe", "rules", "network"],
    )
    .unwrap();
    assert_eq!(assets["workers"], 17);
    assert!(assets.get("historical").is_none());
    assert!(assets.get("modelProfiles").is_none());
    let mut broken = model.receipt.reference.unwrap();
    broken.revision = 999;
    db.execute(
        "UPDATE configuration_category_defaults SET reference_json=?1 WHERE kind='models'",
        [serde_json::to_string(&Some(broken)).unwrap()],
    )
    .unwrap();
    assert!(read_current_defaults_for(&db, &["models"]).is_err());
    assert_eq!(
        read_current_defaults_for(
            &db,
            &["assetApis", "collection", "probe", "rules", "network"]
        )
        .unwrap(),
        assets
    );
    assert_eq!(
        read_current_defaults_for(&db, &["agent"]).unwrap()["agentQuickRequestLimit"],
        8
    );
}

#[test]
fn independent_model_read_skips_broken_legacy_model_binding_and_unrelated_asset_account() {
    let mut db = fixture();
    let model = save(&mut db, input(&fixture(), "models")).unwrap();
    let reference = model.receipt.reference.unwrap();
    // Deliberately construct a damaged historical reference in this in-memory
    // fixture; normal owner writes keep foreign keys enabled.
    db.pragma_update(None, "foreign_keys", "OFF").unwrap();
    db.execute(
        "INSERT INTO configuration_connection_bindings VALUES(1,'models',?1,999)",
        [&reference.policy_id],
    )
    .unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    let api = save(&mut db, input(&fixture(), "assetApis")).unwrap();
    let mut broken = api.receipt.reference.unwrap();
    broken.revision = 999;
    db.execute(
        "UPDATE configuration_category_defaults SET reference_json=?1 WHERE kind='assetApis'",
        [serde_json::to_string(&Some(broken)).unwrap()],
    )
    .unwrap();
    assert!(policies::read_profile(&db, 1).is_err());
    assert!(read_current_defaults_for(&db, &["assetApis"]).is_err());
    let settings = read_current_defaults_for(&db, &["models"]).unwrap();
    assert_eq!(
        policies::reference(&settings, "models").unwrap(),
        Some(reference)
    );
    assert!(settings.get("fofaKey").is_none());
    assert_eq!(
        read_current_defaults_for(&db, &["probe"]).unwrap()["workers"],
        17
    );
}

#[test]
fn fully_independent_defaults_do_not_parse_malformed_historical_json() {
    let mut db = fixture();
    for kind in SECTIONS {
        let request = input(&db, kind);
        save(&mut db, request).unwrap();
    }
    db.execute(
        "UPDATE config_profiles SET settings_json='not-json' WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(read_current_defaults(&db).unwrap()["workers"], 17);
    assert_eq!(
        read_current_defaults_for(&db, &["probe"]).unwrap()["workers"],
        17
    );
}
