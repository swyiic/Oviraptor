#[test]
fn startup_preserves_retired_config_fields_and_watermark_without_activating_them() {
    let root = std::env::temp_dir().join(format!("oviraptor-settings-preserve-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    connection.execute_batch("DELETE FROM app_settings WHERE key='builtin_src_assurance_version';").unwrap();
    let retired = serde_json::json!({"strixLlm":"old-model","strixApiKey":"old-key", "strixRunsDirectory":"/old",
        "strixOastEndpoint":"https://old.invalid", "strixRawHttpEnabled":true, "strixRaceEnabled":true,
        "strixMaxRaceConcurrency":99, "strixControlledWriteEnabled":true,"strixAttackChainEnabled":true,
        "agentBackendPolicy":"strix","custom":{"strixExample":"user-owned"}});
    connection.execute("UPDATE config_profiles SET settings_json=?1", [retired.to_string()]).unwrap();
    drop(connection);
    for _ in 0..2 {
        initialize(&root).unwrap();
        let connection = open(&path).unwrap();
        let raw: String = connection.query_row("SELECT settings_json FROM config_profiles LIMIT 1", [], |r| r.get(0)).unwrap();
        let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for (key,value) in retired.as_object().unwrap() {
            assert_eq!(stored.get(key), Some(value), "startup modified {key}");
        }
        let current = normalize_settings(&stored);
        assert!(current.get("modelProfiles").is_none());
        assert!(current.get("modelApiKey").is_none());
        assert!(current.get("agentBackendPolicy").is_none());
        let watermark: i64 = connection.query_row("SELECT COUNT(*) FROM app_settings WHERE key='builtin_src_assurance_version'", [], |r| r.get(0)).unwrap();
        assert_eq!(watermark, 0, "startup must not advance a retired cleanup watermark");
    }
    fs::remove_dir_all(root).unwrap();
}
