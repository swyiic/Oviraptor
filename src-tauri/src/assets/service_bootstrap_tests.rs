use super::*;
use crate::native_app_sdk::HostProject;
struct Fixture {
    root: PathBuf,
    bootstrap: Bootstrap,
    app: Option<Application>,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-assets-app-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let data = root.join("data");
        let package = root.join("package");
        create_private(&data).unwrap();
        create_private(&package).unwrap();
        let bootstrap = Bootstrap {
            data_dir: data,
            package_dir: package,
            app_id: APP.into(),
            data_schema_version: 1,
            events: EventSink::silent(),
        };
        let app = Application::open_inner(&bootstrap, false).unwrap();
        Self {
            root,
            bootstrap,
            app: Some(app),
        }
    }
    fn app(&mut self) -> &mut Application {
        self.app.as_mut().unwrap()
    }
    fn reopen(&mut self) {
        drop(self.app.take());
        self.app = Some(Application::open_inner(&self.bootstrap, false).unwrap());
    }
    fn context(project: i64, permission: &str, operation: &str) -> NativeCallContext {
        NativeCallContext {
            host_bound_project_id: project,
            grant_revision: 1,
            operation_id: operation.into(),
            app_revision: 1,
            permissions: vec![
                "inventory.read".into(),
                "inventory.write".into(),
                "inventory.rules.manage".into(),
                "inventory.rules.global".into(),
                "inventory.export".into(),
                "inventory.jobs.run".into(),
                "inventory.configuration.manage".into(),
            ],
            project: HostProject {
                id: project,
                name: format!("Synthetic host {project}"),
                status: "active".into(),
            },
            permission: permission.into(),
        }
    }
    fn context_call(&mut self, project: i64) -> Value {
        self.app()
            .call(
                CallSurface::ProjectAction,
                "assets_context",
                json!({}),
                &Self::context(project, "inventory.read", "context"),
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.app.take());
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn assets_native_mapping_is_persistent_independent_and_scope_revision_cannot_rollback() {
    let mut f = Fixture::new();
    let a = f.context_call(55);
    let b = f.context_call(77);
    let native = a["nativeProjectId"].as_i64().unwrap();
    assert_ne!(native, 55);
    assert_ne!(a["nativeProjectId"], b["nativeProjectId"]);
    assert_eq!(a["name"], "Synthetic host 55");
    let mut context = Fixture::context(55, "inventory.read", "assets");
    assert!(f
        .app()
        .call(
            CallSurface::ProjectAction,
            "list_assets",
            json!({"query":{"projectId":55}}),
            &context
        )
        .is_err());
    assert_eq!(
        f.app()
            .call(
                CallSurface::ProjectAction,
                "list_assets",
                json!({"query":{"decisionView":"all","probeView":"all"}}),
                &context
            )
            .unwrap()["total"],
        0
    );
    context.grant_revision = 2;
    context.project.name = "Renamed through newer grant".into();
    f.app()
        .call(
            CallSurface::ProjectAction,
            "assets_context",
            json!({}),
            &context,
        )
        .unwrap();
    assert!(f
        .app()
        .call(
            CallSurface::ProjectAction,
            "assets_context",
            json!({}),
            &Fixture::context(55, "inventory.read", "old")
        )
        .is_err());
    context.project.name = "Changed without revision".into();
    assert!(f
        .app()
        .call(
            CallSurface::ProjectAction,
            "assets_context",
            json!({}),
            &context
        )
        .is_err());
    f.reopen();
    assert_eq!(f.app().data_schema_version(), 1);
    context.grant_revision = 3;
    assert_eq!(
        f.app()
            .call(
                CallSurface::ProjectAction,
                "assets_context",
                json!({}),
                &context
            )
            .unwrap()["nativeProjectId"],
        native
    );
}
#[test]
fn assets_native_surface_permissions_keep_raw_inventory_global_configuration_and_writes_off_model_tools(
) {
    let mut f = Fixture::new();
    let read = Fixture::context(55, "inventory.read", "read");
    assert!(f
        .app()
        .call(CallSurface::Tool, "list_assets", json!({"query":{}}), &read)
        .is_err());
    assert_eq!(
        f.app()
            .call(CallSurface::Tool, "saved_assets_stats", json!({}), &read)
            .unwrap()["projectTotal"],
        0
    );
    for name in [
        "add_content_rule",
        "list_config_profiles",
        "save_config_profile",
        "get_asset_rule_catalog",
        "start_job",
    ] {
        assert!(
            f.app()
                .call(CallSurface::Tool, name, json!({}), &read)
                .is_err(),
            "{name}"
        );
    }
    assert!(f
        .app()
        .call(
            CallSurface::ProjectAction,
            "get_asset_rule_catalog",
            json!({}),
            &Fixture::context(55, "inventory.rules.global", "global-as-project")
        )
        .is_err());
    let mut wrong = Fixture::context(55, "inventory.configuration.manage", "config");
    wrong
        .permissions
        .retain(|p| p != "inventory.configuration.manage");
    assert!(f
        .app()
        .call(
            CallSurface::ApplicationAction,
            "list_config_profiles",
            json!({}),
            &wrong
        )
        .is_err());
    let mut archived = read;
    archived.project.status = "archived".into();
    assert!(f
        .app()
        .call(
            CallSurface::Tool,
            "saved_assets_stats",
            json!({}),
            &archived
        )
        .is_err());
    assert!(f
        .app()
        .call(
            CallSurface::ProjectAction,
            "assets_context",
            json!({"dbPath":"/untrusted"}),
            &Fixture::context(55, "inventory.read", "extra")
        )
        .is_err());
}
#[test]
fn assets_native_mutation_ledger_replays_receipt_not_work_and_rejects_conflict_or_pending() {
    let mut f = Fixture::new();
    let p = f.context_call(55)["nativeProjectId"].as_i64().unwrap();
    let context = Fixture::context(55, "inventory.write", "import-1");
    let args =
        json!({"input":{"projectId":p,"targetType":"domain","values":["synthetic.example.test"]}});
    assert_eq!(
        f.app()
            .call(
                CallSurface::ProjectAction,
                "import_targets",
                args.clone(),
                &context
            )
            .unwrap(),
        1
    );
    assert_eq!(
        f.app()
            .call(
                CallSurface::ProjectAction,
                "import_targets",
                args.clone(),
                &context
            )
            .unwrap(),
        1
    );
    let mut changed = args.clone();
    changed["input"]["values"] = json!(["different.example.test"]);
    assert!(f
        .app()
        .call(
            CallSurface::ProjectAction,
            "import_targets",
            changed,
            &context
        )
        .is_err());
    assert_eq!(
        f.app()
            .database
            .query_row("SELECT COUNT(*) FROM targets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    f.reopen();
    assert_eq!(
        f.app()
            .call(CallSurface::ProjectAction, "import_targets", args, &context)
            .unwrap(),
        1
    );
    let app = f.app();
    let args = json!({"input":{"projectId":p,"targetType":"domain","values":["must-not-run.example.test"]}});
    let pending = Fixture::context(55, "inventory.write", "uncertain");
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                "project",
                "import_targets",
                &args,
                pending.host_bound_project_id,
                pending.grant_revision,
                pending.app_revision
            ))
            .unwrap()
        )
    );
    app.database
        .execute(
            "INSERT INTO asset_service_operations VALUES(?1,?2,?3,?4,'project','import_targets','pending',NULL)",
            params![55, pending.operation_id, digest, app.instance_id],
        )
        .unwrap();
    assert!(app
        .call(
            CallSurface::ProjectAction,
            "import_targets",
            args.clone(),
            &pending
        )
        .unwrap_err()
        .contains("uncertain"));
    f.reopen();
    assert!(f
        .app()
        .call(CallSurface::ProjectAction, "import_targets", args, &pending)
        .unwrap_err()
        .contains("uncertain"));
    assert_eq!(
        f.app()
            .database
            .query_row("SELECT COUNT(*) FROM targets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let status = f.app().status().unwrap();
    assert_eq!(status["unresolvedOperations"], 1);
    assert_eq!(status["operationResultsRequireReview"], true);
    assert_eq!(status["busy"], false);
    assert_eq!(status["uncertain"], false);
    assert_eq!(f.app().stop().unwrap()["stopped"], true);
}
#[test]
fn assets_native_application_ledger_binds_real_host_project_and_never_aliases_global_scope_to_host_zero(
) {
    let mut f = Fixture::new();
    let ctx = Fixture::context(55, "inventory.configuration.manage", "global-save");
    let args = json!({"input":{"name":"Synthetic profile","settings":{"modelProfiles":[],"runProbe":false}}});
    let profile = f
        .app()
        .call(
            CallSurface::ApplicationAction,
            "save_config_profile",
            args.clone(),
            &ctx,
        )
        .unwrap();
    assert_eq!(
        f.app()
            .call(
                CallSurface::ApplicationAction,
                "save_config_profile",
                args.clone(),
                &ctx
            )
            .unwrap(),
        profile
    );
    let changed = Fixture::context(77, "inventory.configuration.manage", "global-save");
    assert!(f
        .app()
        .call(
            CallSurface::ApplicationAction,
            "save_config_profile",
            args,
            &changed
        )
        .is_err());
    let mut zero = ctx;
    zero.host_bound_project_id = 0;
    zero.project.id = 0;
    assert!(f
        .app()
        .call(
            CallSurface::ApplicationAction,
            "list_config_profiles",
            json!({}),
            &zero
        )
        .is_err());
}
#[test]
fn assets_native_cold_start_recovers_only_same_instance_originated_jobs_and_polling_is_read_only() {
    let mut f = Fixture::new();
    let p = f.context_call(55)["nativeProjectId"].as_i64().unwrap();
    let app = f.app();
    for name in ["owned", "historical"] {
        app.database.execute("INSERT INTO runs(project_id,name,pipeline,status) VALUES(?1,?2,'collect','running')",params![p,name]).unwrap();
    }
    let owned = app
        .database
        .query_row("SELECT id FROM runs WHERE name='owned'", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    app.database
        .execute(
            "INSERT INTO asset_service_job_owners(run_id,instance_id) VALUES(?1,?2)",
            params![owned, app.instance_id],
        )
        .unwrap();
    let status = app.status().unwrap();
    assert_eq!(status["busy"], true);
    assert_eq!(status["uncertain"], true);
    assert_eq!(
        app.database
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE status='running'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    f.reopen();
    assert_eq!(
        f.app()
            .database
            .query_row("SELECT status FROM runs WHERE name='owned'", [], |r| r
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "interrupted"
    );
    assert_eq!(
        f.app()
            .database
            .query_row("SELECT status FROM runs WHERE name='historical'", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "running"
    );
}
#[test]
fn assets_native_existing_unmarked_store_unsupported_schema_and_changed_inode_fail_closed() {
    let mut f = Fixture::new();
    let mut unsupported = Bootstrap {
        data_dir: f.bootstrap.data_dir.clone(),
        package_dir: f.bootstrap.package_dir.clone(),
        app_id: APP.into(),
        data_schema_version: 999,
        events: EventSink::silent(),
    };
    assert!(Application::open_inner(&unsupported, false).is_err());
    unsupported.data_schema_version = 1;
    unsupported.app_id = "other.synthetic.app".into();
    assert!(Application::open_inner(&unsupported, false).is_err());
    let foreign = f.root.join("foreign");
    create_private(&foreign).unwrap();
    let store = foreign.join(STORE);
    create_private(&store).unwrap();
    db::initialize(&store).unwrap();
    let foreign = Bootstrap {
        data_dir: foreign,
        package_dir: f.bootstrap.package_dir.clone(),
        app_id: APP.into(),
        data_schema_version: 1,
        events: EventSink::silent(),
    };
    assert!(Application::open_inner(&foreign, false).is_err());
    let app = f.app();
    let original = app.db_path.with_extension("retained");
    fs::rename(&app.db_path, &original).unwrap();
    fs::copy(&original, &app.db_path).unwrap();
    assert!(app.status().is_err());
}

#[test]
fn assets_native_node_v1_inventory_is_never_ignored_or_replaced_by_an_empty_engine() {
    let f = Fixture::new();
    let data = f.root.join("node-v1");
    create_private(&data).unwrap();
    let file = data.join("inventory.sqlite3");
    let legacy = Connection::open(&file).unwrap();
    legacy.execute_batch("CREATE TABLE inventory(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL,observation_json TEXT NOT NULL);INSERT INTO inventory VALUES(1,55,'{\"source\":\"synthetic\"}');").unwrap();
    drop(legacy);
    let bootstrap = Bootstrap {
        data_dir: data.clone(),
        package_dir: f.bootstrap.package_dir.clone(),
        app_id: APP.into(),
        data_schema_version: 1,
        events: EventSink::silent(),
    };
    let error = match Application::open_inner(&bootstrap, false) {
        Ok(_) => panic!("must not create empty native engine beside old inventory"),
        Err(error) => error,
    };
    assert_eq!(error, "asset_service_store_migration_required");
    assert!(!data.join(STORE).exists());
    let legacy = Connection::open(&file).unwrap();
    assert_eq!(
        legacy
            .query_row("SELECT COUNT(*) FROM inventory", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn assets_native_user_configuration_import_binds_private_file_and_never_returns_key_or_local_path()
{
    let mut f = Fixture::new();
    let imported = f
        .app()
        .call(
            CallSurface::ApplicationAction,
            "import_asset_collection_configuration",
            json!({"content":"[fofa]\nkey=synthetic-import-fixture\n"}),
            &Fixture::context(55, "inventory.configuration.manage", "import-private"),
        )
        .unwrap();
    assert_eq!(imported["fofaKeyConfigured"], true);
    assert!(!imported.to_string().contains("synthetic-import-fixture"));
    let id=f.app().call(CallSurface::ApplicationAction,"save_config_profile",json!({"input":{"name":"Imported private configuration","settings":{"modelProfiles":[],"collectionConfigurationId":imported["configurationId"],"runProbe":false}}}),&Fixture::context(55,"inventory.configuration.manage","bind-private")).unwrap();
    let views = f
        .app()
        .call(
            CallSurface::ApplicationAction,
            "list_config_profiles",
            json!({}),
            &Fixture::context(55, "inventory.configuration.manage", "read-private"),
        )
        .unwrap();
    let profile = views
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap();
    assert_eq!(profile["settings"]["configPath"], "");
    assert_eq!(profile["settings"]["fofaKey"], "");
    assert_eq!(profile["settings"]["fofaKeyConfigured"], true);
    assert_eq!(
        profile["settings"]["collectionConfigurationId"],
        imported["configurationId"]
    );
    assert!(!views.to_string().contains("synthetic-import-fixture"));
    let native = f.context_call(55)["nativeProjectId"].as_i64().unwrap();
    let profiles = f
        .app()
        .call(
            CallSurface::ProjectAction,
            "list_asset_job_profiles",
            json!({}),
            &Fixture::context(55, "inventory.read", "safe-profiles"),
        )
        .unwrap();
    assert_eq!(
        profiles
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()["hasFofaKey"],
        true
    );
    assert!(f
        .app()
        .call(
            CallSurface::Tool,
            "import_asset_collection_configuration",
            json!({"content":"[fofa]\nkey=synthetic\n"}),
            &Fixture::context(55, "inventory.configuration.manage", "tool-import")
        )
        .is_err());
    assert!(native > 0);
}

#[test]
fn assets_native_unknown_background_operation_keeps_owner_while_synchronous_history_is_only_review_metadata(
) {
    let mut f = Fixture::new();
    f.context_call(55);
    // The original holder is a real OS file lock; a second controller cannot
    // enter. Cold reopening below drops that holder and obtains the same lock.
    assert!(Application::open_inner(&f.bootstrap, false).is_err());
    let context = Fixture::context(55, "inventory.jobs.run", "pending-start");
    let app = f.app();
    app.database.execute("INSERT INTO asset_service_operations VALUES(55,?1,'synthetic-unresolved-hash',?2,'project','start_job','pending',NULL)",params![context.operation_id,app.instance_id]).unwrap();
    f.reopen();
    let status = f.app().status().unwrap();
    assert_eq!(status["activeJobs"], 0);
    assert_eq!(status["unresolvedOperations"], 1);
    assert_eq!(status["busy"], true);
    assert_eq!(status["uncertain"], true);
    let stopped = f.app().stop().unwrap();
    assert_eq!(stopped["busy"], true);
    assert_eq!(stopped["stopped"], false);
    assert!(!synchronous_inventory_operation("project", "start_job"));
    assert!(!synchronous_inventory_operation(
        "project",
        "ownership_analysis_start"
    ));
    assert!(!synchronous_inventory_operation(
        "project",
        "exposure_task_start"
    ));
    assert!(!synchronous_inventory_operation(
        "project",
        "future_unknown_method"
    ));
    assert!(!synchronous_inventory_operation(
        "application",
        "configure_financial_lease"
    ));
}

#[test]
fn assets_native_upload_actions_are_operator_only_closed_and_scope_bound() {
 let mut f=Fixture::new();let mut c=Fixture::context(55,"inventory.exposure.manage","stage-begin-0001");c.permissions.push("inventory.exposure.manage".into());
 let begin=json!({"request":{"operationId":"native-stage-0001","channel":"document_import","operator":"Synthetic","sourceDeclaration":"Synthetic no-network document"},"files":[{"name":"empty.txt","format":"text","bytes":0,"sha256":format!("{:x}",Sha256::digest([]))}]});
 assert!(f.app().call(CallSurface::Tool,"begin_exposure_import_upload",begin.clone(),&c).is_err());
 let u=f.app().call(CallSurface::ProjectAction,"begin_exposure_import_upload",begin,&c).unwrap();
 c.operation_id="stage-status-0001".into();assert_eq!(f.app().call(CallSurface::ProjectAction,"exposure_import_upload_status",json!({"uploadId":u["uploadId"]}),&c).unwrap()["phase"],"receiving");
 let mut newer=c.clone();newer.grant_revision+=1;assert!(f.app().call(CallSurface::ProjectAction,"exposure_import_upload_status",json!({"uploadId":u["uploadId"]}),&newer).is_err());
 assert!(f.app().call(CallSurface::ProjectAction,"exposure_import_upload_status",json!({"uploadId":u["uploadId"],"path":"/tmp/forbidden"}),&newer).is_err());
}
#[test]
fn assets_native_upload_commit_runs_original_import_and_returns_its_report() {
 let mut f=Fixture::new();let mut c=Fixture::context(55,"inventory.exposure.manage","native-upload-begin");c.permissions.push("inventory.exposure.manage".into());
 let u=f.app().call(CallSurface::ProjectAction,"begin_exposure_import_upload",json!({"request":{"operationId":"native-upload-import-0001","channel":"document_import","operator":"Synthetic","sourceDeclaration":"Synthetic no-network document"},"files":[{"name":"empty.txt","format":"text","bytes":0,"sha256":format!("{:x}",Sha256::digest([]))}]}),&c).unwrap();
 c.operation_id="native-upload-commit".into();let result=f.app().call(CallSurface::ProjectAction,"commit_exposure_import_upload",json!({"uploadId":u["uploadId"],"expectedManifestDigest":u["manifestDigest"]}),&c).unwrap();
 assert_eq!(result["operationId"],"native-upload-import-0001");assert_eq!(result["declaredUrlVerified"],false);assert!(result["runId"].as_i64().unwrap()>0);
 assert_eq!(f.app().status().unwrap()["busy"],false);
}

#[test]
fn assets_native_selection_is_operator_only_and_revalidates_current_members_before_cached_output() {
 let mut f=Fixture::new();let project=f.context_call(55)["nativeProjectId"].as_i64().unwrap();
 {let a=f.app();a.database.execute("INSERT INTO assets(id,asset_key,link)VALUES(64001,'synthetic-source-selection','https://synthetic-selection.test/path')",[]).unwrap();a.database.execute("INSERT INTO project_assets(project_id,asset_id)VALUES(?1,64001)",[project]).unwrap();}
 let c=Fixture::context(55,"inventory.read","native-source-selection-0001");let args=json!({"assetIds":[64001],"mode":"standard"});
 assert!(f.app().call(CallSurface::Tool,"prepare_scan_selection",args.clone(),&c).is_err());
 let value=f.app().call(CallSurface::ProjectAction,"prepare_scan_selection",args.clone(),&c).unwrap();
 assert_eq!(value["kind"],"scan.targets.v1");assert_eq!(value["source"]["assetCount"],1);assert_eq!(value["targets"],json!(["https://synthetic-selection.test/path"]));
 assert_eq!(f.app().database.query_row("SELECT COUNT(*) FROM sentinel_scans",[],|r|r.get::<_,i64>(0)).unwrap(),0);
 f.app().database.execute("UPDATE project_assets SET is_deleted=1 WHERE project_id=?1 AND asset_id=64001",[project]).unwrap();
 assert!(f.app().call(CallSurface::ProjectAction,"prepare_scan_selection",args,&c).is_err());
 assert_eq!(f.app().database.query_row("SELECT COUNT(*) FROM asset_scan_selections",[],|r|r.get::<_,i64>(0)).unwrap(),1);
 assert!(f.app().call(CallSurface::ProjectAction,"prepare_scan_selection",json!({"assetIds":[64001],"mode":"standard","projectId":project}),&c).is_err());
}
