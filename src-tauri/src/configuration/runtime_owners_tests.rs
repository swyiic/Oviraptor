use super::*;
fn db(settings: Value) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE config_profiles(id INTEGER PRIMARY KEY,name TEXT,description TEXT,is_default INTEGER,settings_json TEXT);").unwrap();
    db.execute(
        "INSERT INTO config_profiles VALUES(1,'fixture','',0,?1)",
        [settings.to_string()],
    )
    .unwrap();
    db
}
fn transport() -> Value {
    json!({"proxyUrl":"http://fixture-user:p%40ss%3A%23@[::1]:8123","noProxy":"localhost,.example.test,[::1]","agentProxyEnabled":false,"authorizedProxyPool":["EU|socks5h://user:p%40ss@[::1]:1080","https://user:pw@127.0.0.1:8443"]})
}
fn analyzers() -> Value {
    json!({"nativeAnalyzers":{"semgrep":{"image":format!("registry.example.test/security/semgrep@sha256:{}","a".repeat(64))},"codeql":{"image":""}}})
}
#[test]
fn legacy_read_has_no_ddl_or_credentials_and_explicit_adoption_preserves_consumed_transport() {
    let raw = transport();
    let mut db = db(raw.clone());
    assert_eq!(policies::read_profile(&db, 1).unwrap(), raw);
    assert!(!exists(&db).unwrap());
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let reference = save(&tx, 1, "network", &raw, &raw, None, true).unwrap();
    tx.commit().unwrap();
    let resolved = policies::read_profile(&db, 1).unwrap();
    assert_eq!(resolved["proxyUrl"], raw["proxyUrl"]);
    assert_eq!(resolved["noProxy"], raw["noProxy"]);
    assert_eq!(resolved["agentProxyEnabled"], false);
    assert_eq!(
        resolved["authorizedProxyPool"][0],
        raw["authorizedProxyPool"][0]
    );
    assert_eq!(
        resolved["authorizedProxyPool"][1],
        "ALL|https://user:pw@127.0.0.1:8443"
    );
    assert_eq!(
        policies::reference(&resolved, "network").unwrap(),
        Some(reference)
    );
    let descriptor: String = db
        .query_row(
            "SELECT descriptor_json FROM configuration_runtime_versions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for secret in ["fixture-user", "p%40ss", "user:pw"] {
        assert!(!descriptor.contains(secret));
    }
    assert!(descriptor.contains("socks5h://[::1]:1080"));
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT settings_json FROM config_profiles WHERE id=1",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        raw.to_string()
    );
}
#[test]
fn revisions_are_cas_guarded_and_new_proxy_credentials_rollback_with_binding_failure() {
    let original = transport();
    let mut db = db(original.clone());
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let reference = save(&tx, 1, "network", &original, &original, None, true).unwrap();
    tx.commit().unwrap();
    let current = project(&policies::read_profile(&db, 1).unwrap(), "network").unwrap();
    let mut changed = current.clone();
    changed["proxyUrl"] = json!("http://changed:secret@127.0.0.1:8888");
    changed["agentProxyEnabled"] = json!(true);
    db.execute_batch("CREATE TRIGGER fixture_binding_failure BEFORE UPDATE ON configuration_runtime_bindings BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM configuration_proxy_credentials",
            [],
            |r| r.get(0),
        )
        .unwrap();
    {
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        assert!(save(
            &tx,
            1,
            "network",
            &changed,
            &current,
            Some(&reference),
            false
        )
        .is_err());
        tx.rollback().unwrap();
    }
    assert_eq!(
        db.query_row::<i64, _, _>(
            "SELECT count(*) FROM configuration_proxy_credentials",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        count
    );
    db.execute_batch("DROP TRIGGER fixture_binding_failure;")
        .unwrap();
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let next = save(
        &tx,
        1,
        "network",
        &changed,
        &current,
        Some(&reference),
        false,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(next.revision, 2);
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(
        save(
            &tx,
            1,
            "network",
            &changed,
            &current,
            Some(&reference),
            false
        )
        .unwrap_err(),
        "configuration_runtime_conflict"
    );
    tx.rollback().unwrap();
    assert_eq!(
        policies::read_profile(&db, 1).unwrap()["agentProxyEnabled"],
        true
    );
}
#[test]
fn analyzers_require_supported_engines_and_fixed_digest_but_empty_is_unconfigured() {
    let original = analyzers();
    let mut db = db(original.clone());
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    save(&tx, 1, "securityRules", &original, &original, None, true).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        project(&policies::read_profile(&db, 1).unwrap(), "securityRules").unwrap(),
        original
    );
    for invalid in [
        json!({"nativeAnalyzers":{"semgrep":{"image":"registry.example.test/semgrep:latest"},"codeql":{"image":""}}}),
        json!({"nativeAnalyzers":{"semgrep":{"image":""},"codeql":{"image":""},"unknown":{"image":""}}}),
    ] {
        let current = policies::read_profile(&db, 1).unwrap();
        let reference = policies::reference(&current, "securityRules")
            .unwrap()
            .unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        assert!(save(
            &tx,
            1,
            "securityRules",
            &invalid,
            &project(&current, "securityRules").unwrap(),
            Some(&reference),
            false
        )
        .is_err());
        tx.rollback().unwrap();
    }
    assert_eq!(
        db.query_row::<i64, _, _>(
            "SELECT count(*) FROM configuration_runtime_versions",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn broken_proxy_owner_reference_never_falls_back_to_legacy_raw_credential() {
    let original = transport();
    let mut db = db(original.clone());
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    save(&tx, 1, "network", &original, &original, None, true).unwrap();
    tx.commit().unwrap();
    db.execute_batch("DROP TRIGGER configuration_proxy_credentials_no_update;UPDATE configuration_proxy_credentials SET owner='business-transport',endpoint='http://wrong.example.test/';").unwrap();
    db.execute_batch("CREATE TRIGGER configuration_proxy_credentials_no_update BEFORE UPDATE ON configuration_proxy_credentials BEGIN SELECT RAISE(ABORT,'configuration_proxy_credential_immutable'); END;").unwrap();
    assert_eq!(
        policies::read_profile(&db, 1).unwrap_err(),
        "configuration_proxy_scope_mismatch"
    );
}

#[test]
fn asset_snapshot_keeps_only_transport_refs_and_materializes_exact_immutable_version() {
    let original=transport(); let mut db=db(original.clone());
    let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let first=save(&tx,1,"network",&original,&original,None,true).unwrap();tx.commit().unwrap();
    let effective=policies::read_profile(&db,1).unwrap();let mut snapshot=effective.clone();
    redact_asset_snapshot(&db,&mut snapshot).unwrap();
    let text=snapshot.to_string();
    for secret in ["fixture-user","p%40ss","user:pw"] {assert!(!text.contains(secret),"snapshot must not contain a proxy credential");}
    assert_eq!(snapshot["_businessTransport"]["reference"]["revision"],1);
    let current=project(&effective,"network").unwrap();let mut updated=current.clone();updated["proxyUrl"]=json!("http://next-user:next-secret@127.0.0.1:8888");
    let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let second=save(&tx,1,"network",&updated,&current,Some(&first),false).unwrap();tx.commit().unwrap();
    assert_eq!(second.revision,2);
    release(&db,1).unwrap();
    materialize_asset_transport(&db,&mut snapshot).unwrap();
    assert_eq!(snapshot["proxyUrl"],original["proxyUrl"]);
    assert_eq!(snapshot["_configurationRefs"]["network"]["revision"],1);
    assert!(!snapshot.to_string().contains("next-secret"));
}

#[test]
fn legacy_snapshot_strips_userinfo_without_creating_owner_or_credentials() {
    let mut settings=transport(); let db=db(settings.clone());let changes=db.total_changes();
    settings["authorizedProxyPool"].as_array_mut().unwrap().push(json!("ALL|not-a-url-secret@invalid"));
    redact_asset_snapshot(&db,&mut settings).unwrap();
    assert!(!exists(&db).unwrap());assert_eq!(db.total_changes(),changes);
    let encoded=settings.to_string();
    for secret in ["fixture-user","p%40ss","user:pw","not-a-url-secret"] {assert!(!encoded.contains(secret));}
    assert!(materialize_asset_transport(&db,&mut settings).unwrap_err().contains("requires_current_profile"));
}

#[test]
fn resume_transport_repair_records_current_owner_without_changing_frozen_probe_policy() {
    let original=transport();let mut db=db(original.clone());
    let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    save(&tx,1,"network",&original,&original,None,true).unwrap();tx.commit().unwrap();
    let current=policies::read_profile(&db,1).unwrap();
    let mut resumed=json!({"workers":17,"pageSize":99,"_assetTitleRules":["kept"],"proxyUrl":"http://old:secret@127.0.0.1:9"});
    repair_asset_transport(&mut resumed,&current).unwrap();
    assert_eq!(resumed["workers"],17);assert_eq!(resumed["pageSize"],99);assert_eq!(resumed["_assetTitleRules"],json!(["kept"]));
    assert_eq!(resumed["proxyUrl"],current["proxyUrl"]);
    let mut persisted=resumed.clone();redact_asset_snapshot(&db,&mut persisted).unwrap();
    assert_eq!(persisted["_businessTransport"]["reference"],current["_configurationRefs"]["network"]);
    assert!(!persisted.to_string().contains("fixture-user"));assert!(!persisted.to_string().contains("old:secret"));
    materialize_asset_transport(&db,&mut persisted).unwrap();assert_eq!(persisted["proxyUrl"],resumed["proxyUrl"]);
}

#[test]
fn tampered_transport_snapshots_and_replace_operations_fail_closed() {
    let original=transport();let mut db=db(original.clone());
    let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    save(&tx,1,"network",&original,&original,None,true).unwrap();tx.commit().unwrap();
    let mut snapshot=policies::read_profile(&db,1).unwrap();redact_asset_snapshot(&db,&mut snapshot).unwrap();
    snapshot["_businessTransport"]["descriptor"]["proxy"]["endpoint"]=json!("http://foreign.invalid/");
    assert!(materialize_asset_transport(&db,&mut snapshot).is_err());
    assert!(db.execute("INSERT OR REPLACE INTO configuration_runtime_versions SELECT * FROM configuration_runtime_versions",[]).is_err());
    assert!(db.execute("INSERT OR REPLACE INTO configuration_proxy_credentials SELECT * FROM configuration_proxy_credentials",[]).is_err());
}
