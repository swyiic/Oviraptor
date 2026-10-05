#[test]
fn ordinary_scan_deletion_preserves_retired_results_until_confirmed_cleanup() {
    for sql in [
        "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) VALUES('start-test','target','strix','vulnerability','old','old')",
        "INSERT INTO sentinel_checkpoints(scan_id,url,stage) VALUES('start-test','target','strix_events:old')",
        "INSERT INTO app_settings(key,value) VALUES('strix-result-signature:start-test:source','old')",
        "INSERT INTO agent_runs(id,scan_id,status,backend) VALUES('old','start-test','completed','strix')",
        "INSERT INTO agent_runs(id,scan_id,status) VALUES('old','start-test','legacy_backend_removed')",
    ] {
        let (root,path,connection)=deletion_fixture();connection.execute_batch(sql).unwrap();
        let before=deletion_snapshot(&connection);
        assert_eq!(delete_sentinel_scan_inner(&path,"start-test").unwrap_err(),"retired_result_data_requires_confirmed_cleanup","{sql}");
        assert_eq!(deletion_snapshot(&connection),before,"{sql}");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn ordinary_profile_deletion_preserves_inert_fields_until_confirmed_cleanup() {
    let root=std::env::temp_dir().join(format!("oviraptor-settings-delete-{}",Uuid::new_v4()));
    let path=db::initialize(&root).unwrap();let connection=db::open(&path).unwrap();
    connection.execute("INSERT INTO config_profiles(id,name,is_default,settings_json) VALUES(99,'old',0,?1)",
        [serde_json::json!({"strixApiKey":"inert-key","agentBackendPolicy":"strix"}).to_string()]).unwrap();
    let before=deletion_snapshot(&connection);
    assert_eq!(delete_config_profile_inner(&connection,99).unwrap_err(),"retired_settings_require_confirmed_cleanup");
    assert_eq!(deletion_snapshot(&connection),before);
    drop(connection);fs::remove_dir_all(root).unwrap();
}
