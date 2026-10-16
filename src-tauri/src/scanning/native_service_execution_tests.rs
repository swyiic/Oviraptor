use super::*;

struct Fixture {
    root: PathBuf,
    service: Service,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-service-execution-{}",
            uuid::Uuid::new_v4()
        ));
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
        let mut service = Service::open(environment).unwrap();
        service
            .set_execution_project(HostProject {
                project_id: 71,
                name: "Local fixture".into(),
                status: "active".into(),
                grant_revision: 1,
                permissions: ["draft.read", "draft.write", "scan.execute", "scan.control"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            })
            .unwrap();
        Self { root, service }
    }
    fn context(&self, operation: &str) -> Context {
        Context {
            host_bound_project_id: 71,
            grant_revision: 1,
            operation_id: operation.into(),
        }
    }
    fn draft(&self) -> Value {
        self.service
            .create(
                &self.context("create-once"),
                NewDraft {
                    title: "Real original engine draft".into(),
                    targets: vec!["https://fixture.invalid/Exact".into()],
                    scan_mode: Some("quick".into()),
                },
            )
            .unwrap()
    }
    fn count(&self, table: &str) -> i64 {
        self.service
            .database
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
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
fn explicit_upgrade_preserves_original_draft_receipt_bytes_foreign_keys_and_idempotency() {
    let f = Fixture::new();
    let receipt = f.draft();
    let before = operations_snapshot(&f.service.database).unwrap();
    enable(&f.service).unwrap();
    assert_eq!(operations_snapshot(&f.service.database).unwrap(), before);
    let changes = f.service.database.total_changes();
    enable(&f.service).unwrap();
    assert_eq!(f.service.database.total_changes(), changes);
    assert_eq!(
        f.service
            .database
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let references: Vec<String> = f
        .service
        .database
        .prepare("PRAGMA foreign_key_list(scan_service_operations)")
        .unwrap()
        .query_map([], |r| r.get(2))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(references.len(), 2);
    assert!(references.contains(&"projects".into()));
    assert!(references.contains(&"scan_service_store".into()));
    assert_eq!(f.draft(), receipt);
    assert_eq!(f.count("sentinel_scans"), 1);
}

#[cfg(unix)]
#[test]
fn original_draft_cancel_commits_but_its_historical_creation_receipt_never_recreates_it() {
    let f = Fixture::new();
    let receipt = f.draft();
    let task = receipt["taskId"].as_str().unwrap();
    enable(&f.service).unwrap();
    let cancel = perform(
        &f.service,
        &f.context("cancel-once"),
        Action::CancelDraft,
        task,
    )
    .unwrap();
    assert_eq!(cancel["outcome"]["accepted"], true);
    assert_eq!(cancel["outcome"]["task"]["deletedDraft"], true);
    assert_eq!(f.count("sentinel_scans"), 0);
    assert_eq!(f.count("scan_service_operations"), 1);
    assert_eq!(f.draft(), receipt);
    assert_eq!(f.count("sentinel_scans"), 0);
    assert_eq!(
        perform(
            &f.service,
            &f.context("cancel-once"),
            Action::CancelDraft,
            task
        )
        .unwrap(),
        cancel
    );
    assert!(!busy(&f.service).unwrap());
}

#[cfg(unix)]
#[test]
fn partial_execution_schema_is_not_repaired_and_migration_never_adopts_an_attempt() {
    for partial in [false, true] {
        let f = Fixture::new();
        let receipt = f.draft();
        if partial {
            f.service
                .database
                .execute("CREATE TABLE scan_service_actions(partial TEXT)", [])
                .unwrap();
        } else {
            f.service
                .database
                .execute(
                    "UPDATE sentinel_scans SET status='paused',attempt_count=1 WHERE id=?1",
                    [receipt["taskId"].as_str().unwrap()],
                )
                .unwrap();
        }
        let before = operations_snapshot(&f.service.database).unwrap();
        let changes = f.service.database.total_changes();
        assert_eq!(
            enable(&f.service).unwrap_err(),
            "scan_execution_schema_requires_independent_new_store"
        );
        assert_eq!(operations_snapshot(&f.service.database).unwrap(), before);
        assert_eq!(f.service.database.total_changes(), changes);
        assert_eq!(
            f.service
                .database
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name='scan_service_execution_schema'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn real_original_start_failure_rolls_back_the_attempt_and_operation_retry_never_starts_again() {
    let f = Fixture::new();
    let receipt = f.draft();
    let task = receipt["taskId"].as_str().unwrap();
    enable(&f.service).unwrap();
    let before = startup_snapshot(&f.service.database, task).unwrap();
    // The real owner resolves an absent pinned worker/configuration before any
    // dispatch. No fake engine, target or provider transport is substituted.
    let result = perform(&f.service, &f.context("start-once"), Action::Start, task).unwrap();
    assert_eq!(result["outcome"]["accepted"], false);
    assert_eq!(result["outcome"]["disposition"], "rejected");
    assert_eq!(startup_snapshot(&f.service.database, task).unwrap(), before);
    for table in [
        "sentinel_scan_attempts",
        "agent_runs",
        "agent_budget_entries",
        "agent_http_request_claims",
        "agent_model_cost_facts",
    ] {
        assert_eq!(f.count(table), 0, "{table}");
    }
    assert_eq!(
        perform(&f.service, &f.context("start-once"), Action::Start, task).unwrap(),
        result
    );
    assert_eq!(
        perform(
            &f.service,
            &f.context("start-once"),
            Action::CancelDraft,
            task
        )
        .unwrap_err(),
        "scan_operation_conflict"
    );
    assert_eq!(f.count("scan_service_actions"), 1);
    assert!(!busy(&f.service).unwrap());
}

#[cfg(unix)]
#[test]
fn ignored_action_claim_rolls_back_its_service_birth_before_the_mature_owner_can_run() {
    let f = Fixture::new();
    let receipt = f.draft();
    let task = receipt["taskId"].as_str().unwrap();
    enable(&f.service).unwrap();
    f.service.database.execute_batch("CREATE TRIGGER fixture_ignore_action BEFORE INSERT ON scan_service_actions BEGIN SELECT RAISE(IGNORE);END;").unwrap();
    let before = startup_snapshot(&f.service.database, task).unwrap();
    assert_eq!(
        perform(&f.service, &f.context("ignored"), Action::Start, task).unwrap_err(),
        "scan_action_claim_failed"
    );
    assert_eq!(f.count("scan_service_actions"), 0);
    assert_eq!(f.count("scan_service_execution_births"), 0);
    assert_eq!(startup_snapshot(&f.service.database, task).unwrap(), before);
    assert_eq!(f.count("agent_runs"), 0);
}

#[cfg(unix)]
#[test]
fn original_cancel_commit_followed_by_ignored_receipt_remains_uncertain_and_cannot_replay() {
    let f = Fixture::new();
    let receipt = f.draft();
    let task = receipt["taskId"].as_str().unwrap();
    enable(&f.service).unwrap();
    f.service.database.execute_batch("CREATE TRIGGER fixture_ignore_outcome BEFORE UPDATE ON scan_service_actions BEGIN SELECT RAISE(IGNORE);END;").unwrap();
    assert_eq!(
        perform(
            &f.service,
            &f.context("uncertain"),
            Action::CancelDraft,
            task
        )
        .unwrap_err(),
        "scan_action_outcome_uncertain_no_automatic_replay"
    );
    assert_eq!(f.count("sentinel_scans"), 0);
    assert_eq!(f.count("scan_service_operations"), 1);
    assert!(busy(&f.service).unwrap());
    f.service
        .database
        .execute_batch("DROP TRIGGER fixture_ignore_outcome;")
        .unwrap();
    assert_eq!(
        perform(
            &f.service,
            &f.context("uncertain"),
            Action::CancelDraft,
            task
        )
        .unwrap_err(),
        "scan_action_outcome_uncertain_no_automatic_replay"
    );
    assert_eq!(f.count("scan_service_actions"), 1);
    assert_eq!(f.count("sentinel_scans"), 0);
}

#[cfg(unix)]
#[test]
fn host_revocation_and_cross_project_calls_do_not_replay_a_saved_action_or_disclose_a_task() {
    let mut f = Fixture::new();
    let receipt = f.draft();
    let task = receipt["taskId"].as_str().unwrap();
    enable(&f.service).unwrap();
    perform(&f.service, &f.context("start-once"), Action::Start, task).unwrap();
    f.service
        .set_execution_project(HostProject {
            project_id: 72,
            name: "Other".into(),
            status: "active".into(),
            grant_revision: 1,
            permissions: ["draft.read", "draft.write", "scan.execute", "scan.control"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        })
        .unwrap();
    assert_eq!(
        perform(
            &f.service,
            &Context {
                host_bound_project_id: 72,
                grant_revision: 1,
                operation_id: "foreign".into()
            },
            Action::Start,
            task
        )
        .unwrap_err(),
        "scan_task_scope_unavailable"
    );
    f.service
        .set_execution_project(HostProject {
            project_id: 71,
            name: "Local fixture".into(),
            status: "active".into(),
            grant_revision: 2,
            permissions: ["draft.read"].into_iter().map(str::to_owned).collect(),
        })
        .unwrap();
    let before = startup_snapshot(&f.service.database, task).unwrap();
    assert_eq!(
        perform(&f.service, &f.context("start-once"), Action::Start, task).unwrap_err(),
        "scan_host_scope_permission_denied"
    );
    assert_eq!(f.count("scan_service_actions"), 1);
    assert_eq!(startup_snapshot(&f.service.database, task).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn another_executable_birth_is_refused_without_resigning_or_allocating_an_attempt() {
    let f = Fixture::new();
    let receipt = f.draft();
    let task = receipt["taskId"].as_str().unwrap();
    enable(&f.service).unwrap();
    f.service
        .database
        .execute(
            "INSERT INTO scan_service_execution_births VALUES(?1,?2,?3,?4)",
            params![
                task,
                f.service.instance_id,
                "0".repeat(64),
                f.service.db_path.to_str().unwrap()
            ],
        )
        .unwrap();
    let before = startup_snapshot(&f.service.database, task).unwrap();
    let changes = f.service.database.total_changes();
    assert_eq!(
        perform(&f.service, &f.context("new-binary"), Action::Start, task).unwrap_err(),
        "scan_execution_build_or_store_changed_requires_new_task"
    );
    assert_eq!(f.service.database.total_changes(), changes);
    assert_eq!(startup_snapshot(&f.service.database, task).unwrap(), before);
    assert_eq!(f.count("scan_service_actions"), 0);
    assert_eq!(f.count("sentinel_scan_attempts"), 0);
}

#[cfg(unix)]
#[test]
fn service_stop_uses_real_physical_ownership_even_when_the_task_label_says_draft() {
    let f = Fixture::new();
    let receipt = f.draft();
    let task = receipt["taskId"].as_str().unwrap();
    enable(&f.service).unwrap();
    assert!(!busy(&f.service).unwrap());
    let physical =
        owner::claim_native_invocation(&f.service.db_path, task, 0, "frontend_producer", "web")
            .unwrap();
    assert!(busy(&f.service).unwrap());
    drop(physical);
    assert!(!busy(&f.service).unwrap());
}

#[cfg(unix)]
#[test]
fn saved_task_list_reuses_original_mapper_excludes_foreign_rows_and_binds_pagination_to_project() {
    let mut f = Fixture::new();
    enable(&f.service).unwrap();
    for index in 0..3 {
        f.service
            .create(
                &f.context(&format!("create-{index}")),
                NewDraft {
                    title: format!("Original {index}"),
                    targets: vec![format!("https://fixture.invalid/{index}")],
                    scan_mode: None,
                },
            )
            .unwrap();
    }
    // This is a genuine original-owner draft but has no service operation
    // receipt. Listing must not turn it into an adopted application task.
    owner::create_sentinel_url_scan_with_mode_in(
        &f.service.database,
        71,
        "Foreign original draft".into(),
        vec!["https://foreign.invalid/".into()],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    f.service
        .set_execution_project(HostProject {
            project_id: 72,
            name: "Other".into(),
            status: "active".into(),
            grant_revision: 1,
            permissions: ["draft.read", "draft.write"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        })
        .unwrap();
    let first = list(
        &f.service,
        &f.context("list"),
        serde_json::from_value(json!({"limit":2})).unwrap(),
    )
    .unwrap();
    assert_eq!(first["total"], 3);
    assert_eq!(first["returned"], 2);
    assert_eq!(first["hasMore"], true);
    assert!(!first.to_string().contains("taskPath"));
    assert!(!first.to_string().contains("Foreign original draft"));
    let cursor = first["nextCursor"].as_str().unwrap();
    let second = list(
        &f.service,
        &f.context("list-next"),
        serde_json::from_value(json!({"limit":2,"cursor":cursor})).unwrap(),
    )
    .unwrap();
    assert_eq!(second["total"], 3);
    assert_eq!(second["returned"], 1);
    assert_eq!(second["hasMore"], false);
    let ids = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|item| item["taskId"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 3);
    let context = Context {
        host_bound_project_id: 72,
        grant_revision: 1,
        operation_id: "foreign-cursor".into(),
    };
    assert_eq!(
        list(
            &f.service,
            &context,
            serde_json::from_value(json!({"cursor":cursor})).unwrap()
        )
        .unwrap_err(),
        "scan_cursor_scope_changed"
    );
}
