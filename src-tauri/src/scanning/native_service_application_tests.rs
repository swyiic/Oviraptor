use super::*;

struct Fixture {
    root: PathBuf,
    app: Application,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-native-app-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        for path in [
            &root,
            &root.join("data"),
            &root.join("configuration"),
            &root.join("resources"),
        ] {
            if !path.exists() {
                fs::create_dir(path).unwrap();
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let environment = ScanServiceEnvironment::from_roots(
            &root.join("data"),
            &root.join("configuration"),
            &root.join("resources"),
        )
        .unwrap();
        let service = Service::open(environment).unwrap();
        execution::enable(&service).unwrap();
        // Each unit fixture owns a genuine original store. Permanent process
        // registration and the real factory run only in separate CLI processes.
        let app = Application {
            service,
            app_revision: None,
        };
        Self { root, app }
    }
    fn context(permission: &str, op: &str) -> NativeCallContext {
        NativeCallContext {
            host_bound_project_id: 71,
            grant_revision: 1,
            operation_id: op.into(),
            app_revision: 1,
            permission: permission.into(),
            permissions: [
                "investigation.read",
                "investigation.draft.write",
                "investigation.execute",
                "investigation.control",
                "configuration.read",
                "configuration.write",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            project: crate::native_app_sdk::HostProject {
                id: 71,
                name: "Synthetic host project".into(),
                status: "active".into(),
            },
        }
    }
    fn draft(&mut self) -> Value {
        self.app
            .call(
                CallSurface::Tool,
                "scan_draft_create",
                json!({"title":"SDK native draft","targets":["https://fixture.invalid/Exact"]}),
                &Self::context("investigation.draft.write", "draft-once"),
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
#[test]
fn native_adapter_rejects_project_and_permission_claim_mismatch_before_any_domain_write() {
    let mut f = Fixture::new();
    let changes = f.app.service.database.total_changes();
    let mut context = Fixture::context("investigation.draft.write", "invalid-project");
    context.project.id = 72;
    assert_eq!(
        f.app
            .call(
                CallSurface::Tool,
                "scan_draft_create",
                json!({"title":"Invalid","targets":["https://fixture.invalid/"]}),
                &context
            )
            .unwrap_err(),
        "scan_host_scope_permission_denied"
    );
    let mut context = Fixture::context("investigation.read", "wrong-permission");
    context.permissions.push("investigation.execute".into());
    assert_eq!(
        f.app
            .call(
                CallSurface::Tool,
                "scan_start",
                json!({"taskId":"task"}),
                &context
            )
            .unwrap_err(),
        "scan_host_scope_permission_denied"
    );
    let context = Fixture::context("investigation.draft.write", "invalid-args");
    assert!(f.app.call(CallSurface::Tool,"scan_draft_create",json!({"title":"Invalid","targets":["https://fixture.invalid/;https://u:p@other.invalid/"]}),&context).is_err());
    let mut duplicated = context.clone();
    duplicated
        .permissions
        .push(duplicated.permissions[0].clone());
    assert_eq!(
        f.app
            .call(
                CallSurface::Tool,
                "scan_draft_create",
                json!({"title":"Invalid","targets":["https://fixture.invalid/"]}),
                &duplicated
            )
            .unwrap_err(),
        "scan_host_scope_permission_denied"
    );
    assert_eq!(f.app.service.database.total_changes(), changes);

    // The SDK permits revision zero and a 256-byte project name. Its exact
    // validator, rather than a second near-copy, governs this public adapter.
    let mut compatible = Fixture::new();
    let mut context = Fixture::context("investigation.draft.write", "revision-zero");
    context.app_revision = 0;
    context.project.name = "p".repeat(256);
    let created = compatible
        .app
        .call(
            CallSurface::Tool,
            "scan_draft_create",
            json!({"title":"Compatible","targets":["https://fixture.invalid/"]}),
            &context,
        )
        .unwrap();
    assert_eq!(created["executionAvailable"], true);
}

#[cfg(unix)]
#[test]
fn native_operator_model_configuration_is_not_a_tool_and_uses_the_existing_complete_profile_resolver(
) {
    let mut f = Fixture::new();
    let context = Fixture::context("configuration.write", "configure-local");
    let local = json!({"llm":"fixture-local","deployment":"local","apiBase":"http://127.0.0.1:1/v1","apiKey":"synthetic-local-token"});
    let changes = f.app.service.database.total_changes();
    assert_eq!(
        f.app
            .call(
                CallSurface::Tool,
                "configure_model",
                local.clone(),
                &context
            )
            .unwrap_err(),
        "scan_surface_permission_denied"
    );
    assert_eq!(f.app.service.database.total_changes(), changes);
    let result = f
        .app
        .call(
            CallSurface::ApplicationAction,
            "configure_model",
            local,
            &context,
        )
        .unwrap();
    assert_eq!(result["configured"], true);
    assert_eq!(result["providerRequestSent"], false);
    assert!(!result.to_string().contains("synthetic-local-token"));
    let selected = crate::model_configuration::default_settings(&f.app.service.database);
    let connection = crate::model_configuration::model_connection(
        &crate::model_configuration::NormalizedModelSettings::new(&selected),
    )
    .unwrap();
    assert_eq!(
        (
            connection.llm.as_str(),
            connection.deployment.as_str(),
            connection.api_base.as_str(),
            connection.api_key.as_str()
        ),
        (
            "fixture-local",
            "local",
            "http://127.0.0.1:1/v1",
            "synthetic-local-token"
        )
    );
    let cloud = json!({"llm":"fixture-cloud","deployment":"cloud","apiBase":"https://provider.invalid/v1","apiKey":"synthetic-cloud-token"});
    let result = f
        .app
        .call(
            CallSurface::ApplicationAction,
            "configure_model",
            cloud,
            &Fixture::context("configuration.write", "configure-cloud"),
        )
        .unwrap();
    assert!(!result.to_string().contains("synthetic-cloud-token"));
    let selected = crate::model_configuration::default_settings(&f.app.service.database);
    let connection = crate::model_configuration::model_connection(
        &crate::model_configuration::NormalizedModelSettings::new(&selected),
    )
    .unwrap();
    assert_eq!(
        (
            connection.llm.as_str(),
            connection.deployment.as_str(),
            connection.api_base.as_str(),
            connection.api_key.as_str()
        ),
        (
            "fixture-cloud",
            "cloud",
            "https://provider.invalid/v1",
            "synthetic-cloud-token"
        )
    );
    assert_eq!(
        f.app
            .service
            .database
            .query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[cfg(unix)]
#[test]
fn native_sdk_task_context_stays_bound_to_project_grant_and_installed_application_revision() {
    let mut f = Fixture::new();
    let created = f.draft();
    let task = created["taskId"].as_str().unwrap();
    assert_eq!(f.app.data_schema_version(), 2);
    assert_eq!(created["creationReceipt"]["liveState"], false);
    assert_eq!(created["executionAvailable"], true);
    let read = f
        .app
        .call(
            CallSurface::Tool,
            "scan_status",
            json!({"taskId":task}),
            &Fixture::context("investigation.read", "read-one"),
        )
        .unwrap();
    assert_eq!(read["status"], "draft");
    let mut foreign = Fixture::context("investigation.read", "read-other");
    foreign.host_bound_project_id = 72;
    foreign.project.id = 72;
    assert_eq!(
        f.app
            .call(
                CallSurface::Tool,
                "scan_status",
                json!({"taskId":task}),
                &foreign
            )
            .unwrap_err(),
        "scan_task_scope_unavailable"
    );
    let mut changed = Fixture::context("investigation.read", "changed-app");
    changed.app_revision = 2;
    assert_eq!(
        f.app
            .call(
                CallSurface::Tool,
                "scan_status",
                json!({"taskId":task}),
                &changed
            )
            .unwrap_err(),
        "scan_application_revision_changed"
    );
    let mut revoked = Fixture::context("investigation.read", "read-after-revoke");
    revoked.grant_revision = 2;
    revoked.permissions = vec!["investigation.read".into()];
    f.app
        .call(
            CallSurface::Tool,
            "scan_status",
            json!({"taskId":task}),
            &revoked,
        )
        .unwrap();
    assert_eq!(
        f.app
            .call(
                CallSurface::Tool,
                "scan_start",
                json!({"taskId":task}),
                &Fixture::context("investigation.execute", "stale-grant")
            )
            .unwrap_err(),
        "scan_host_scope_revision_changed"
    );
    assert_eq!(
        f.app
            .service
            .database
            .query_row("SELECT COUNT(*) FROM scan_service_actions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[cfg(unix)]
#[test]
fn unsupported_manifest_data_version_is_refused_before_creating_configuration_or_database() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-unsupported-native-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let result = Application::open(&Bootstrap {
        data_dir: root.clone(),
        package_dir: root.clone(),
        app_id: "org.oviraptor.scanning".into(),
        data_schema_version: 999,
        events: crate::native_app_sdk::EventSink::silent(),
    });
    assert!(matches!(result,Err(ref code) if code=="scan_execution_schema_unsupported"));
    assert!(fs::read_dir(&root).unwrap().next().is_none());
    fs::remove_dir(root).unwrap();
}
