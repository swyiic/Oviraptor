use super::*;
use std::sync::Mutex;

struct Roots {
    root: PathBuf,
    data: PathBuf,
    configuration: PathBuf,
    resources: PathBuf,
}

impl Roots {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-scan-env-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let value = Self {
            data: root.join("data"),
            configuration: root.join("configuration"),
            resources: root.join("package"),
            root,
        };
        for directory in [&value.data, &value.configuration, &value.resources] {
            fs::create_dir(directory).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        value
    }

    fn environment(&self) -> ScanServiceEnvironment {
        ScanServiceEnvironment::from_roots(&self.data, &self.configuration, &self.resources)
            .unwrap()
    }

    fn worker(&self, prefix: &str) -> PathBuf {
        let directory = self.resources.join(prefix);
        fs::create_dir_all(directory.join("workers")).unwrap();
        fs::create_dir_all(directory.join("config")).unwrap();
        let worker = directory.join("workers/9_frontend_runtime_probe.cjs");
        fs::write(
            &worker,
            include_bytes!("../../resources/workers/9_frontend_runtime_probe.cjs"),
        )
        .unwrap();
        fs::write(
            directory.join("config/browser-locations.json"),
            include_bytes!("../../resources/config/browser-locations.json"),
        )
        .unwrap();
        worker
    }
}

impl Drop for Roots {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn explicit_service_roots_resolve_the_real_pinned_worker_and_keep_inventory_empty() {
    let roots = Roots::new();
    let expected = roots.worker("resources");
    let environment = roots.environment();
    assert_eq!(environment.data_root(), roots.data);
    assert_eq!(environment.frontend_worker().unwrap(), expected);
    assert!(fs::read_dir(&roots.data).unwrap().next().is_none());
    assert!(fs::read_dir(&roots.configuration).unwrap().next().is_none());
    #[cfg(unix)]
    {
        let _configuration = environment.tool_configuration().unwrap();
        assert!(fs::read_dir(&roots.configuration).unwrap().next().is_none());
    }
}

#[test]
fn explicit_service_resources_never_fall_back_to_the_desktop_repository() {
    let roots = Roots::new();
    let environment = roots.environment();
    assert_eq!(
        environment.frontend_worker().unwrap_err(),
        "应用内置 CDP 运行时探测器 9_frontend_runtime_probe.cjs 缺失"
    );
    let secondary = roots.worker("");
    assert_eq!(environment.frontend_worker().unwrap(), secondary);
    let primary = roots.worker("resources");
    assert_eq!(environment.frontend_worker().unwrap(), primary);
    fs::write(
        &primary,
        b"untrusted candidate before a valid second candidate",
    )
    .unwrap();
    assert_eq!(
        environment.frontend_worker().unwrap_err(),
        "bundled_worker_digest_mismatch"
    );
}

#[test]
fn source_only_workbench_does_not_resolve_browser_or_administrator_configuration() {
    struct SourceOnly;
    impl ScanEnvironment for SourceOnly {
        fn frontend_worker(&self) -> Result<PathBuf, String> {
            panic!("source-only browser resolution")
        }
        fn tool_configuration(&self) -> Result<ToolAdministratorConfiguration, String> {
            panic!("source-only tool configuration")
        }
    }
    assert_eq!(
        super::super::workbench_frontend_worker(&SourceOnly, &[]).unwrap(),
        PathBuf::new()
    );
    let roots = Roots::new();
    let environment = roots.environment();
    assert!(super::super::workbench_frontend_worker(
        &environment,
        &["https://fixture.invalid/".into()]
    )
    .is_err());
    assert!(fs::read_dir(&roots.data).unwrap().next().is_none());
}

#[test]
fn explicit_service_resources_preserve_config_digest_refusal() {
    let roots = Roots::new();
    roots.worker("resources");
    fs::write(
        roots
            .resources
            .join("resources/config/browser-locations.json"),
        b"{}",
    )
    .unwrap();
    assert_eq!(
        roots.environment().frontend_worker().unwrap_err(),
        "bundled_worker_browser_locations_mismatch"
    );
}

#[cfg(unix)]
#[test]
fn service_bootstrap_rejects_symlink_noncanonical_and_public_private_roots() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let roots = Roots::new();
    let link = roots.root.join("data-link");
    symlink(&roots.data, &link).unwrap();
    assert!(
        ScanServiceEnvironment::from_roots(&link, &roots.configuration, &roots.resources).is_err()
    );
    assert!(ScanServiceEnvironment::from_roots(
        &roots.data.join(".."),
        &roots.configuration,
        &roots.resources
    )
    .is_err());
    fs::set_permissions(&roots.data, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(ScanServiceEnvironment::from_roots(&roots.data, &roots.configuration, &roots.resources), Err(error) if error == "scan_service_root_permissions")
    );
    fs::set_permissions(&roots.data, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&roots.configuration, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(ToolAdministratorConfiguration::from_service_root(&roots.configuration).is_err());
    fs::set_permissions(&roots.configuration, fs::Permissions::from_mode(0o700)).unwrap();
    let environment = roots.environment();
    fs::remove_dir(&roots.configuration).unwrap();
    symlink(&roots.data, &roots.configuration).unwrap();
    assert!(environment.tool_configuration().is_err());
    fs::remove_file(&roots.configuration).unwrap();
    fs::create_dir(&roots.configuration).unwrap();
}

struct RecordingSink(Mutex<Vec<(String, serde_json::Value)>>);

impl ScanEventSink for RecordingSink {
    fn emit(&self, event: ScanEvent<'_>) -> Result<(), String> {
        let payload = match event {
            ScanEvent::RunnerLog(value) => value.clone(),
            ScanEvent::SdkLog(hint) => serde_json::to_value(hint).unwrap(),
            ScanEvent::ProcessLog(hint) => serde_json::to_value(hint).unwrap(),
        };
        self.0.lock().unwrap().push((event.name().into(), payload));
        Ok(())
    }
}

fn process_hint() -> ProcessHint {
    ProcessHint {
        scope: crate::native_pipeline::process::log::Scope {
            scan_id: "existing-scan".into(),
            attempt: 3,
            branch: "web".into(),
            dispatch_claim_id: "scope-claim".into(),
            invocation_key: "a".repeat(64),
            execution_id: "existing-execution".into(),
            stage: "frontend".into(),
        },
        sequence: 12,
        stream: "stderr".into(),
        stream_sequence: 5,
    }
}

#[test]
fn injected_event_sink_preserves_existing_runner_and_process_names_and_payloads() {
    let sink = RecordingSink(Mutex::new(Vec::new()));
    let runner = super::super::runner_log_notification("existing-scan", 3, "safe display line");
    let process = process_hint();
    emit_to(Some(&sink), ScanEvent::RunnerLog(&runner)).unwrap();
    emit_to(Some(&sink), ScanEvent::ProcessLog(&process)).unwrap();
    let recorded = sink.0.lock().unwrap();
    assert_eq!(recorded[0], ("nest-runner-log".into(), runner));
    assert_eq!(
        recorded[1],
        (
            "nest-native-process-log".into(),
            serde_json::to_value(&process).unwrap()
        )
    );
    assert!(recorded[1].1.get("message").is_none());
    assert_eq!(recorded[1].1["attempt"], 3);
    assert_eq!(recorded[1].1["sequence"], 12);
}

#[test]
fn injected_event_sink_preserves_sdk_identity_payload_without_provider_content() {
    let sink = RecordingSink(Mutex::new(Vec::new()));
    let hint = SdkHint {
        scan_id: "existing-scan".into(),
        attempt: 3,
        owner_id: "existing-owner".into(),
        domain: serde_json::from_value(serde_json::json!("root")).unwrap(),
        dispatch_key: "existing-dispatch".into(),
        run_id: "existing-run".into(),
        worker_id: None,
        sequence: 11,
        round: 2,
    };
    emit_to(Some(&sink), ScanEvent::SdkLog(&hint)).unwrap();
    let recorded = sink.0.lock().unwrap();
    assert_eq!(
        recorded.as_slice(),
        &[(
            "nest-native-sdk-log".into(),
            serde_json::to_value(&hint).unwrap()
        )]
    );
    assert_eq!(recorded[0].1["domain"], "root");
    assert_eq!(recorded[0].1["round"], 2);
    for private in ["content", "messages", "apiKey", "endpoint", "model"] {
        assert!(recorded[0].1.get(private).is_none());
    }
}

#[test]
fn event_observer_absence_and_failure_never_create_authority_or_leak_errors() {
    struct Failed;
    impl ScanEventSink for Failed {
        fn emit(&self, _: ScanEvent<'_>) -> Result<(), String> {
            Err("untrusted observer error text".into())
        }
    }
    let process = process_hint();
    emit_to(None, ScanEvent::ProcessLog(&process)).unwrap();
    assert_eq!(
        emit_to(Some(&Failed), ScanEvent::ProcessLog(&process)).unwrap_err(),
        "native_process_log_notification_unavailable"
    );
    let runner = serde_json::json!({"scanId":"existing-scan","attempt":3,"line":"safe"});
    assert_eq!(
        emit_to(Some(&Failed), ScanEvent::RunnerLog(&runner)).unwrap_err(),
        "runner_log_notification_unavailable"
    );
    assert_eq!(process.sequence, 12);
    assert_eq!(process.scope.attempt, 3);
}
