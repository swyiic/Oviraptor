#[test]
fn editing_current_config_preserves_existing_inert_fields_without_activating_them() {
    let root=std::env::temp_dir().join(format!("oviraptor-settings-edit-{}",Uuid::new_v4()));
    let path=db::initialize(&root).unwrap();let mut connection=db::open(&path).unwrap();
    let original=serde_json::json!({"strixApiKey":"old-key","STRIXRunsDirectory":"/old",
        "agentBackendPolicy":"strix","custom":{"strixExample":"user document"}});
    connection.execute("UPDATE config_profiles SET settings_json=?1 WHERE id=1",[original.to_string()]).unwrap();
    let current=read_config_profiles(&connection).unwrap().into_iter().find(|p|p.id==1).unwrap();
    assert!(current.settings.get("strixApiKey").is_none());
    let mut settings=current.settings;settings["agentDeepTokenLimit"]=serde_json::json!(123);
    settings["strixApiKey"]=serde_json::json!("replacement-old-key");
    persist_config_profile(&mut connection,ConfigProfileInput{id:Some(1),name:"edited".into(),description:"".into(),is_default:true,settings}).unwrap();
    let raw:String=connection.query_row("SELECT settings_json FROM config_profiles WHERE id=1",[],|r|r.get(0)).unwrap();
    let stored:JsonValue=serde_json::from_str(&raw).unwrap();
    for (key,value) in original.as_object().unwrap(){assert_eq!(stored.get(key),Some(value),"save modified {key}");}
    assert_eq!(stored["agentDeepTokenLimit"],123);
    assert!(model_runtime_env(&stored).err().unwrap().contains("模型未配置"));
    let current=read_config_profiles(&connection).unwrap().into_iter().find(|p|p.id==1).unwrap();
    assert!(current.settings.get("strixApiKey").is_none());
    assert!(current.settings.get("agentBackendPolicy").is_none());
    drop(connection);fs::remove_dir_all(root).unwrap();
}
