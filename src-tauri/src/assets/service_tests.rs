use super::*;
use rusqlite::params;
use std::fs;
use uuid::Uuid;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
    service: AssetService,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-assets-service-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let exports = root.join("exports");
        fs::create_dir_all(&exports).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for p in [&root, &exports] {
                fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let path = db::initialize(&root).unwrap();
        let connection = db::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE asset_service_store(singleton INTEGER PRIMARY KEY CHECK(singleton=1),instance_id TEXT NOT NULL UNIQUE,schema_version INTEGER NOT NULL CHECK(schema_version=1));").unwrap();
        connection
            .execute(
                "INSERT INTO asset_service_store VALUES(1,?1,1)",
                [uuid::Uuid::new_v4().to_string()],
            )
            .unwrap();
        connection.execute("INSERT INTO projects(id,name) VALUES(9001,'Synthetic asset service'),(9002,'Other private project')", []).unwrap();
        for (id, project) in [(1, 9001), (2, 9001), (3, 9002)] {
            connection.execute("INSERT INTO assets(id,asset_key,host,link,title,probe_outcome,extra_json) VALUES(?1,?2,?3,?3,'Synthetic matched page','web_alive',?4)",
                params![id,format!("synthetic-{id}"),format!("https://synthetic-{id}.example.test"),json!({"probe_checked_at":"2026-10-15T10:00:00Z","probe_history_reason":"collection_after_probe","collection_received_at":"2026-10-15T11:00:00Z","discovery_sources":[{"source":"synthetic","observedAt":"unknown"}]}).to_string()]).unwrap();
            connection
                .execute(
                    "INSERT INTO project_assets(project_id,asset_id) VALUES(?1,?2)",
                    [project, id],
                )
                .unwrap();
        }
        let environment = AssetServiceEnvironment::from_paths(&root, &path, &exports).unwrap();
        Self {
            root,
            path,
            service: AssetService::new(environment),
        }
    }
    fn call(&self, command: &str, arguments: Value) -> Value {
        self.service
            .dispatch_project(9001, command, arguments)
            .unwrap()
    }
    fn rule(&self) -> Value {
        json!({"projectId":9001,"sourceAssetId":1,"kind":"title_contains","value":"Synthetic matched"})
    }
    fn project_state(&self, id: i64) -> (String, String, bool) {
        db::open(&self.path).unwrap().query_row("SELECT decision,note,is_deleted FROM project_assets WHERE project_id=9001 AND asset_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn service_inventory_keeps_real_full_dto_filter_totals_and_source_observation_fields() {
    let f = Fixture::new();
    let raw = json!({"decisionView":"all","probeView":"all","pageSize":10,"sortBy":"host","sortDirection":"asc"});
    let query:AssetQuery=serde_json::from_value(json!({"projectId":9001,"decisionView":"all","probeView":"all","pageSize":10,"sortBy":"host","sortDirection":"asc"})).unwrap();
    let expected = serde_json::to_value(assets::list_assets_at(&f.path, &query).unwrap()).unwrap();
    let actual = f.call("list_assets", json!({"query":raw}));
    assert_eq!(actual, expected);
    assert_eq!(actual["memberTotal"], 2);
    assert_eq!(actual["total"], 2);
    assert_eq!(
        actual["items"][0]["extra"]["probe_history_reason"],
        "collection_after_probe"
    );
    assert_eq!(actual["items"][0]["projectId"], 9001);
    assert!(f
        .service
        .dispatch_project(9001, "list_assets", json!({"query":{"projectId":9002}}))
        .is_err());
    assert!(f
        .service
        .dispatch_project(
            9001,
            "list_assets",
            json!({"query":{},"dbPath":"/untrusted.sqlite"})
        )
        .is_err());
    let stats = f.call("saved_assets_stats", json!({}));
    assert_eq!(stats["activeTotal"], 2);
    let search = f.call("saved_assets_search", json!({}));
    assert_eq!(search["total"], 2);
    assert!(f
        .service
        .dispatch_project(
            9001,
            "saved_assets_search",
            json!({"filter":{"deletedView":"trash"}})
        )
        .is_err());
}

#[test]
fn service_targets_use_original_auto_detection_dedup_and_scope_guard_before_delete() {
    let f = Fixture::new();
    let args = json!({"input":{"projectId":9001,"targetType":"auto","values":["example.test","192.0.2.1","example.test","","# comment"]}});
    assert_eq!(f.call("import_targets", args.clone()), 2);
    assert_eq!(f.call("import_targets", args), 0);
    let targets = f.call("list_targets", json!({}));
    assert_eq!(targets.as_array().unwrap().len(), 2);
    let c = db::open(&f.path).unwrap();
    c.execute("INSERT INTO targets(project_id,target_type,value,normalized_value) VALUES(9002,'domain','outside.test','outside.test')",[]).unwrap();
    let outside = c.last_insert_rowid();
    assert!(f
        .service
        .dispatch_project(9001, "remove_target", json!({"targetId":outside}))
        .is_err());
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM targets WHERE id=?1", [outside], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    f.call("remove_target", json!({"targetId":targets[0]["id"]}));
    assert_eq!(
        f.call("list_targets", json!({})).as_array().unwrap().len(),
        1
    );
}

#[test]
fn service_manual_decisions_archive_and_restore_remain_project_scoped_and_record_real_events() {
    let f = Fixture::new();
    let mixed = json!({"input":{"selections":[{"projectId":9001,"assetId":1},{"projectId":9002,"assetId":3}],"decision":"confirmed","note":"Must not partially write"}});
    assert!(f
        .service
        .dispatch_project(9001, "update_asset_decisions", mixed)
        .is_err());
    assert_eq!(f.project_state(1).0, "pending");
    assert_eq!(f.call("update_asset_decisions",json!({"input":{"selections":[{"projectId":9001,"assetId":1},{"projectId":9001,"assetId":1}],"decision":"confirmed","note":"Human reviewed"}})),1);
    assert_eq!(
        f.project_state(1),
        ("confirmed".into(), "Human reviewed".into(), false)
    );
    f.call("soft_delete_assets", json!({"assetIds":[1],"deleted":true}));
    assert!(f.project_state(1).2);
    f.call(
        "soft_delete_asset_selections",
        json!({"input":{"selections":[{"projectId":9001,"assetId":1}],"deleted":false}}),
    );
    assert_eq!(
        f.project_state(1),
        ("confirmed".into(), "Human reviewed".into(), false)
    );
    let events = f.call("list_asset_events", json!({}));
    assert_eq!(events.as_array().unwrap().len(), 3);
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["projectId"] == 9001));
}

#[test]
fn service_page_rule_preview_stale_apply_and_disable_use_original_inventory_cas_and_backups() {
    let f = Fixture::new();
    let rule = f.rule();
    let stale = f.call("preview_page_rule", json!({"input":rule}));
    assert_eq!(stale["eligible"], 2);
    f.call("update_decision",json!({"input":{"projectId":9001,"assetIds":[1],"decision":"confirmed","note":"Preserve manual choice"}}));
    assert!(f
        .service
        .dispatch_project(
            9001,
            "apply_page_rule",
            json!({"input":{"rule":rule,"previewToken":stale["previewToken"]}})
        )
        .is_err());
    assert!(f
        .call("list_page_rules", json!({}))
        .as_array()
        .unwrap()
        .is_empty());
    let preview = f.call("preview_page_rule", json!({"input":rule}));
    assert_eq!(preview["eligible"], 1);
    assert_eq!(preview["protected"], 1);
    let applied = f.call(
        "apply_page_rule",
        json!({"input":{"rule":rule,"previewToken":preview["previewToken"]}}),
    );
    assert_eq!(applied["isolated"], 1);
    assert_eq!(applied["protected"], 1);
    assert!(Path::new(applied["backupPath"].as_str().unwrap()).starts_with(&f.root));
    assert!(f.project_state(2).2);
    assert_eq!(f.project_state(1).0, "confirmed");
    let disabled = f.call("disable_page_rule", json!({"ruleId":applied["ruleId"]}));
    assert_eq!(disabled["restored"], 1);
    assert!(!f.project_state(2).2);
    assert_eq!(f.project_state(1).1, "Preserve manual choice");
    assert_eq!(f.call("list_page_rules", json!({}))[0]["enabled"], false);
}

#[test]
fn service_export_writes_full_original_csv_for_bound_project_and_exact_filters_only() {
    let f = Fixture::new();
    let result=f.call("export_assets",json!({"request":{"query":{"search":"synthetic-1.example.test","decisionView":"all","probeView":"all"},"fields":["host","decision","projectName"],"chineseHeaders":false}}));
    assert_eq!(result["rows"], 1);
    let reference = result["path"].as_str().unwrap();
    assert!(reference.starts_with("oviraptor-asset-export:"));
    let resolved = export_artifacts::resolve(
        &f.path,
        f.service.environment().export_dir(),
        9001,
        reference,
    )
    .unwrap();
    assert!(resolved
        .path
        .starts_with(f.service.environment().export_dir().join("project-9001")));
    let landed = fs::read(&resolved.path).unwrap();
    let page = f.call(
        "read_asset_artifact",
        json!({"artifactId":result["artifactId"],"maxBytes":196608}),
    );
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(page["bytesBase64"].as_str().unwrap())
        .unwrap();
    assert_eq!(page["done"], true);
    assert_eq!(bytes, landed);
    assert!(f
        .service
        .dispatch_project(
            9002,
            "read_asset_artifact",
            json!({"artifactId":result["artifactId"]})
        )
        .is_err());
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("synthetic-1.example.test"));
    assert!(!text.contains("synthetic-2.example.test"));
    assert!(!text.contains("synthetic-3.example.test"));
}

#[test]
fn service_application_admin_is_separate_from_project_tools_and_safe_job_profiles_do_not_echo_credentials(
) {
    let f = Fixture::new();
    assert!(f
        .service
        .dispatch_project(
            9001,
            "add_content_rule",
            json!({"input":{"keyword":"Synthetic matched"}})
        )
        .is_err());
    assert!(f
        .service
        .dispatch_project(9001, "list_config_profiles", json!({}))
        .is_err());
    let profile=f.service.dispatch_application("save_config_profile",json!({"input":{"name":"Synthetic private profile","settings":{"fofaKey":"SYNTHETIC-NONSECRET-FIXTURE","runProbe":false,"runRefine":true,"modelProfiles":[]}}})).unwrap();
    let safe = f.call("list_asset_job_profiles", json!({}));
    let chosen = safe
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == profile)
        .unwrap();
    assert_eq!(chosen["hasFofaKey"], true);
    assert_eq!(chosen["runProbe"], false);
    assert_eq!(chosen["runRefine"], true);
    assert!(!safe.to_string().contains("SYNTHETIC-NONSECRET-FIXTURE"));
    assert!(!safe.to_string().contains("settings"));
    let rule = f
        .service
        .dispatch_application(
            "add_content_rule",
            json!({"input":{"keyword":"Synthetic matched","sourceAssetId":1}}),
        )
        .unwrap();
    assert_eq!(rule["matchedProjectAssets"], 3);
    let catalog = f
        .service
        .dispatch_application("get_asset_rule_catalog", json!({}))
        .unwrap();
    assert_eq!(catalog["legacyRules"].as_array().unwrap().len(), 1);
    assert_eq!(f.project_state(1).0, "rejected");
}

#[test]
fn service_history_scopes_runs_logs_events_and_does_not_mark_running_jobs_interrupted_on_reads() {
    let f = Fixture::new();
    let c = db::open(&f.path).unwrap();
    c.execute("INSERT INTO runs(id,project_id,profile_id,name,pipeline,status) SELECT 8001,9001,id,'Synthetic live','collect','running' FROM config_profiles LIMIT 1",[]).unwrap();
    c.execute("INSERT INTO runs(id,project_id,profile_id,name,pipeline,status) SELECT 8002,9002,id,'Other live','collect','running' FROM config_profiles LIMIT 1",[]).unwrap();
    let runs = f.call("list_runs", json!({}));
    assert_eq!(runs.as_array().unwrap().len(), 1);
    assert_eq!(runs[0]["id"], 8001);
    assert_eq!(runs[0]["status"], "running");
    assert!(f
        .service
        .dispatch_project(9001, "list_logs", json!({"runId":8002}))
        .is_err());
    assert!(f
        .service
        .dispatch_project(9001, "acknowledge_interrupted_run", json!({"runId":8002}))
        .is_err());
    assert!(f
        .service
        .dispatch_project(9001, "startup_status", json!({}))
        .is_err());
    assert_eq!(
        c.query_row("SELECT status FROM runs WHERE id=8001", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "running"
    );
}

#[test]
fn service_archived_projects_allow_reads_but_reject_mutation_and_wrong_context() {
    let f = Fixture::new();
    db::open(&f.path)
        .unwrap()
        .execute("UPDATE projects SET status='archived' WHERE id=9001", [])
        .unwrap();
    assert_eq!(
        f.call(
            "list_assets",
            json!({"query":{"decisionView":"all","probeView":"all"}})
        )["total"],
        2
    );
    assert!(f
        .service
        .dispatch_project(
            9001,
            "soft_delete_assets",
            json!({"assetIds":[1],"deleted":true})
        )
        .is_err());
    assert!(f
        .service
        .dispatch_project(0, "list_targets", json!({}))
        .is_err());
    assert!(f
        .service
        .dispatch_project(9999, "list_targets", json!({}))
        .is_err());
    assert!(f
        .service
        .dispatch_project(9001, "execute_sql", json!({"sql":"SELECT 1"}))
        .is_err());
}

#[cfg(unix)]
#[test]
fn service_environment_rejects_symlinks_outside_paths_and_replaced_database_identity() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let link = f.root.join("linked.sqlite3");
    symlink(&f.path, &link).unwrap();
    assert!(AssetServiceEnvironment::from_paths(
        &f.root,
        &link,
        f.service.environment().export_dir()
    )
    .is_err());
    assert!(
        AssetServiceEnvironment::from_paths(Path::new("/tmp"), &f.path, Path::new("/tmp")).is_err()
    );
    let copy = f.root.join("replacement.sqlite3");
    fs::copy(&f.path, &copy).unwrap();
    fs::rename(&copy, &f.path).unwrap();
    assert_eq!(
        f.service
            .dispatch_project(9001, "list_targets", json!({}))
            .err()
            .unwrap(),
        "asset_service_path_identity_changed"
    );
}

#[test]
fn service_configuration_is_write_only_and_blank_updates_retain_keys_without_recipient_reuse() {
    let f = Fixture::new();
    let settings = json!({"fofaKey":"synthetic-fofa-fixture","modelName":"synthetic-model","modelApiBase":"https://model.example.test/v1","modelDeployment":"cloud","modelApiKey":"synthetic-model-fixture","modelApiKeyConfigured":false,"localApiKey":"","modelProfiles":[{"id":"synthetic","name":"Synthetic model","llm":"synthetic-model","apiBase":"https://model.example.test/v1","deployment":"cloud","apiKey":"synthetic-model-fixture","apiKeyConfigured":false,"localApiKey":""}],"activeModelProfileId":"synthetic"});
    let id = f
        .service
        .dispatch_application(
            "save_config_profile",
            json!({"input":{"name":"Write-only synthetic profile","settings":settings}}),
        )
        .unwrap()
        .as_i64()
        .unwrap();
    let views = f
        .service
        .dispatch_application("list_config_profiles", json!({}))
        .unwrap();
    let profile = views
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap();
    let view = profile["settings"].clone();
    assert_eq!(view["fofaKey"], "");
    assert_eq!(view["fofaKeyConfigured"], true);
    assert_eq!(view["modelApiKey"], "");
    assert_eq!(view["modelApiKeyConfigured"], true);
    assert_eq!(view["modelProfiles"][0]["apiKey"], "");
    assert_eq!(view["modelProfiles"][0]["apiKeyConfigured"], true);
    assert!(!views.to_string().contains("synthetic-fofa-fixture"));
    assert!(!views.to_string().contains("synthetic-model-fixture"));
    f.service
        .dispatch_application(
            "save_config_profile",
            json!({"input":{"id":id,"name":"Write-only synthetic profile","settings":view}}),
        )
        .unwrap();
    let stored = || {
        let raw: String = db::open(&f.path)
            .unwrap()
            .query_row(
                "SELECT settings_json FROM config_profiles WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str::<Value>(&raw).unwrap()
    };
    assert_eq!(stored()["fofaKey"], "synthetic-fofa-fixture");
    assert_eq!(stored()["modelApiKey"], "synthetic-model-fixture");
    assert_eq!(
        stored()["modelProfiles"][0]["apiKey"],
        "synthetic-model-fixture"
    );
    let before = stored();
    let mut changed = view.clone();
    changed["modelApiBase"] = json!("https://other-model.example.test/v1");
    changed["modelProfiles"][0]["apiBase"] = json!("https://other-model.example.test/v1");
    assert!(f
        .service
        .dispatch_application(
            "save_config_profile",
            json!({"input":{"id":id,"name":"Must not inherit","settings":changed}})
        )
        .unwrap_err()
        .contains("recipient_requires_new_credential"));
    assert_eq!(stored(), before);
    changed["modelApiKey"] = json!("synthetic-new-model-fixture");
    changed["modelProfiles"][0]["apiKey"] = json!("synthetic-new-model-fixture");
    f.service
        .dispatch_application(
            "save_config_profile",
            json!({"input":{"id":id,"name":"Explicit new recipient/key","settings":changed}}),
        )
        .unwrap();
    assert_eq!(
        stored()["modelProfiles"][0]["apiBase"],
        "https://other-model.example.test/v1"
    );
    assert_eq!(
        stored()["modelProfiles"][0]["apiKey"],
        "synthetic-new-model-fixture"
    );
    assert_eq!(stored()["fofaKey"], "synthetic-fofa-fixture");
}
#[test]
fn service_configuration_cannot_import_model_paths_or_read_external_config_files() {
    let f = Fixture::new();
    assert!(f
        .service
        .dispatch_project(
            9001,
            "import_asset_collection_configuration",
            json!({"content":"[fofa]\nkey=synthetic\n"})
        )
        .is_err());
    assert!(f
        .service
        .dispatch_application(
            "import_asset_collection_configuration",
            json!({"path":"/untrusted/config.ini"})
        )
        .is_err());
    assert!(f.service.dispatch_application("save_config_profile",json!({"input":{"name":"Untrusted file","settings":{"modelProfiles":[],"configPath":"/untrusted/config.ini"}}})).is_err());
    assert!(!f
        .service
        .dispatch_application("list_config_profiles", json!({}))
        .unwrap()
        .to_string()
        .contains("/untrusted/config.ini"));
}

#[test]
fn service_export_asset_sources_unions_provider_receipts_and_keeps_validity_separate() {
    let f=Fixture::new();
    let c=db::open(&f.path).unwrap();
    c.execute("UPDATE assets SET extra_json=?1 WHERE id=1",[json!({"collection_provider":"hunter","discovery_sources_json":json!([{"provider":"fofa"},{"source":"hunter"}]).to_string()}).to_string()]).unwrap();
    let export=|fields:Value| {
        let result=f.call("export_assets",json!({"request":{"query":{"decisionView":"all","probeView":"all"},"fields":fields,"chineseHeaders":true}}));
        let page=f.call("read_asset_artifact",json!({"artifactId":result["artifactId"],"maxBytes":196608}));
        use base64::Engine;
        String::from_utf8(base64::engine::general_purpose::STANDARD.decode(page["bytesBase64"].as_str().unwrap()).unwrap()).unwrap()
    };
    let csv=export(json!(["assetSources","host"]));
    assert!(csv.contains("资产来源"));assert!(csv.contains("FOFA + Hunter · 待复核"));assert!(csv.contains("历史来源未记录"));assert!(!csv.contains("所属项目"));
    c.execute("UPDATE project_assets SET decision='confirmed' WHERE project_id=9001 AND asset_id=1",[]).unwrap();
    assert!(export(json!(["assetSources"])).contains("FOFA + Hunter · 已确认有效"));
}
