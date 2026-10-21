use super::*;

struct Fixture {
    root: PathBuf,
    data: PathBuf,
    configuration: PathBuf,
    resources: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let raw =
            std::env::temp_dir().join(format!("oviraptor-native-service-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&raw).unwrap();
        let root = fs::canonicalize(raw).unwrap();
        let value = Self {
            data: root.join("data"),
            configuration: root.join("configuration"),
            resources: root.join("resources"),
            root,
        };
        for path in [&value.data, &value.configuration, &value.resources] {
            fs::create_dir(path).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        value
    }
    fn environment(&self) -> ScanServiceEnvironment {
        ScanServiceEnvironment::from_roots(&self.data, &self.configuration, &self.resources)
            .unwrap()
    }
    fn service(&self) -> Service {
        Service::open(self.environment()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn scope(project: i64) -> HostProject {
    HostProject {
        project_id: project,
        name: "Same host display name".into(),
        status: "active".into(),
        grant_revision: 7,
        permissions: BTreeSet::from(["draft.read".into(), "draft.write".into()]),
    }
}
fn context(project: i64, operation: &str) -> Context {
    Context {
        host_bound_project_id: project,
        grant_revision: 7,
        operation_id: operation.into(),
    }
}
fn input() -> NewDraft {
    NewDraft {
        title: "Original engine draft".into(),
        targets: vec!["https://fixture.invalid/Case".into()],
        scan_mode: Some("quick".into()),
    }
}
fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
fn snapshot(db: &Connection) -> Value {
    let names:Vec<String>=db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|row|row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let mut all = serde_json::Map::new();
    for name in names {
        let escaped = name.replace('"', "\"\"");
        let mut statement = db.prepare(&format!("SELECT * FROM \"{escaped}\"")).unwrap();
        let width = statement.column_count();
        let mut rows: Vec<String> = statement
            .query_map([], |row| {
                let mut values = Vec::new();
                for n in 0..width {
                    let value: rusqlite::types::Value = row.get(n)?;
                    values.push(format!("{value:?}"));
                }
                Ok(values.join("|"))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        rows.sort();
        all.insert(name, json!(rows));
    }
    Value::Object(all)
}

#[cfg(unix)]
#[test]
fn native_bootstrap_creates_a_real_original_mode_draft_without_an_execution() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let receipt = service.create(&context(91, "create-one"), input()).unwrap();
    let task = receipt["taskId"].as_str().unwrap();
    let db = connection(&service.db_path).unwrap();
    let (_, mode) = crate::agent_runtime::web_mode::draft::read(&db, task).unwrap();
    assert_eq!(mode, crate::agent_runtime::web_mode::WebMode::Multi);
    let actual: (String, i64, String) = db
        .query_row(
            "SELECT status,attempt_count,task_path FROM sentinel_scans WHERE id=?1",
            [task],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(actual, ("draft".into(), 0, "".into()));
    assert_eq!(count(&db, "sentinel_targets"), 1);
    assert_eq!(count(&db, "native_web_mode_drafts"), 1);
    for table in [
        "agent_runs",
        "agent_budget_entries",
        "agent_model_cost_facts",
        "agent_http_request_claims",
        "native_branch_dispatches",
        "scan_service_operations",
    ] {
        assert_eq!(
            count(&db, table),
            i64::from(table == "scan_service_operations"),
            "{table}"
        );
    }
    assert_eq!(receipt["executionStarted"], false);
    assert_eq!(service.status().unwrap()["busy"], false);
    assert!(fs::read_dir(&f.configuration).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn original_draft_and_service_receipt_rollback_together_on_ignored_publication() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let db = connection(&service.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_service_publication BEFORE INSERT ON scan_service_operations BEGIN SELECT RAISE(IGNORE);END;").unwrap();
    let before = snapshot(&db);
    assert_eq!(
        service
            .create(&context(91, "ignore-one"), input())
            .unwrap_err(),
        "scan_operation_publication_unconfirmed"
    );
    assert_eq!(snapshot(&db), before);
    assert_eq!(count(&db, "sentinel_scans"), 0);
    assert_eq!(count(&db, "native_web_mode_drafts"), 0);
    assert_eq!(count(&db, "scan_service_operations"), 0);
}

#[cfg(unix)]
#[test]
fn operation_retry_never_mints_another_draft_and_changed_arguments_conflict() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let original = service.create(&context(91, "retry-one"), input()).unwrap();
    let db = connection(&service.db_path).unwrap();
    let before = snapshot(&db);
    assert_eq!(
        service.create(&context(91, "retry-one"), input()).unwrap(),
        original
    );
    assert_eq!(snapshot(&db), before);
    let mut changed = input();
    changed.title = "Changed".into();
    assert_eq!(
        service
            .create(&context(91, "retry-one"), changed)
            .unwrap_err(),
        "scan_operation_conflict"
    );
    assert_eq!(snapshot(&db), before);
}

#[cfg(unix)]
#[test]
fn host_project_names_may_repeat_but_scope_revision_and_task_reads_do_not_cross() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    service.set_project(scope(92)).unwrap();
    let saved = service.create(&context(91, "owned"), input()).unwrap();
    let task = saved["taskId"].as_str().unwrap();
    assert_eq!(
        service.read(&context(91, "read"), task).unwrap()["hostBoundProjectId"],
        91
    );
    assert_eq!(
        service.read(&context(92, "read"), task).unwrap_err(),
        "scan_task_unavailable"
    );
    let db = connection(&service.db_path).unwrap();
    let before = snapshot(&db);
    let mut stale = context(91, "stale");
    stale.grant_revision = 6;
    assert!(service.create(&stale, input()).is_err());
    assert_eq!(snapshot(&db), before);
    let mut revoked = scope(91);
    revoked.grant_revision = 8;
    revoked.permissions.clear();
    service.set_project(revoked).unwrap();
    let before = snapshot(&db);
    let mut current = context(91, "no-permission");
    current.grant_revision = 8;
    assert!(service.create(&current, input()).is_err());
    assert_eq!(snapshot(&db), before);
    let mut archived = scope(92);
    archived.grant_revision = 8;
    archived.status = "archived".into();
    service.set_project(archived).unwrap();
    let mut current = context(92, "archived");
    current.grant_revision = 8;
    assert!(service.create(&current, input()).is_err());
}

#[cfg(unix)]
#[test]
fn existing_foreign_or_incomplete_store_is_refused_without_initializing_it() {
    let f = Fixture::new();
    let engine = f.data.join(STORE);
    fs::create_dir(&engine).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&engine, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let db = crate::db::initialize(&engine).unwrap();
    let before = fs::read(&db).unwrap();
    assert!(Service::open(f.environment()).is_err());
    assert_eq!(fs::read(&db).unwrap(), before);
    assert!(!engine.join("service-store.json").exists());
    fs::remove_dir_all(&engine).unwrap();
    fs::create_dir(&engine).unwrap();
    assert!(Service::open(f.environment()).is_err());
    assert!(!engine.join("oviraptor.sqlite3").exists());
}

#[cfg(unix)]
#[test]
fn reopen_requires_fresh_host_scope_and_never_adopts_foreign_scan_rows() {
    let f = Fixture::new();
    let task;
    {
        let mut service = f.service();
        service.set_project(scope(91)).unwrap();
        task = service.create(&context(91, "original"), input()).unwrap()["taskId"]
            .as_str()
            .unwrap()
            .to_string();
    }
    let mut service = f.service();
    assert!(service.read(&context(91, "read"), &task).is_err());
    service.set_project(scope(91)).unwrap();
    assert_eq!(
        service.read(&context(91, "read"), &task).unwrap()["taskId"],
        task
    );
    let db = connection(&service.db_path).unwrap();
    db.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,task_name) VALUES('foreign-old',91,'Foreign','draft','Old')",[]).unwrap();
    let before = snapshot(&db);
    assert_eq!(
        service
            .read(&context(91, "foreign"), "foreign-old")
            .unwrap_err(),
        "scan_task_unavailable"
    );
    assert_eq!(snapshot(&db), before);
}

#[cfg(unix)]
#[test]
fn bootstrap_protocol_rejects_execution_and_extra_project_arguments_without_writes() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let db = connection(&service.db_path).unwrap();
    let before = snapshot(&db);
    for name in ["start_scan", "resume_scan", "task_start", "task_retry"] {
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tool/call","params":{"name":name,"arguments":{},"context":{"hostBoundProjectId":91,"grantRevision":7,"operationId":"no-execution"}}});
        assert_eq!(
            response(&mut service, request.to_string().as_bytes())["error"]["data"]["code"],
            "scan_execution_not_migrated"
        );
    }
    let request = json!({"jsonrpc":"2.0","id":2,"method":"tool/call","params":{"name":"engine_draft_create","arguments":{"title":"Do not create","targets":["https://fixture.invalid/"],"projectId":92},"context":{"hostBoundProjectId":91,"grantRevision":7,"operationId":"wrong-project"}}});
    assert!(response(&mut service, request.to_string().as_bytes())
        .get("error")
        .is_some());
    assert_eq!(snapshot(&db), before);
    let stop = json!({"jsonrpc":"2.0","id":3,"method":"service/stop","params":{}});
    assert_eq!(
        response(&mut service, stop.to_string().as_bytes())["result"]["busy"],
        false
    );
    assert!(service.stopped);
    assert_eq!(
        response(&mut service, request.to_string().as_bytes())["error"]["data"]["code"],
        "scan_service_stopped"
    );
    assert_eq!(snapshot(&db), before);
}

#[cfg(unix)]
#[test]
fn private_store_has_one_real_controller_and_lock_release_does_not_adopt_a_task() {
    let f = Fixture::new();
    let first = f.service();
    assert!(
        matches!(Service::open(f.environment()), Err(code) if code == "scan_bootstrap_controller_unavailable")
    );
    drop(first);
    let reopened = f.service();
    assert_eq!(reopened.status().unwrap()["newDrafts"], 0);
    assert!(reopened.scopes.is_empty());
}

fn sqlite_bytes(path: &Path) -> Vec<Option<Vec<u8>>> {
    ["", "-wal", "-shm"]
        .into_iter()
        .map(|suffix| fs::read(PathBuf::from(format!("{}{suffix}", path.to_string_lossy()))).ok())
        .collect()
}

#[cfg(unix)]
#[test]
fn live_foreign_database_replacement_refuses_every_entry_without_writing_either_store() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let created = service
        .create(&context(91, "before-replacement"), input())
        .unwrap();
    let foreign = crate::db::initialize(&f.root.join("foreign")).unwrap();
    let moved = service.db_path.with_file_name("original-moved.sqlite3");
    fs::rename(&service.db_path, &moved).unwrap();
    fs::rename(&foreign, &service.db_path).unwrap();
    let before = sqlite_bytes(&service.db_path);
    let original = fs::read(&moved).unwrap();
    let changes = service.database.total_changes();
    assert_eq!(
        service.set_project(scope(92)).unwrap_err(),
        "scan_store_custody_changed"
    );
    assert_eq!(
        service
            .create(&context(91, "after-replacement"), input())
            .unwrap_err(),
        "scan_store_custody_changed"
    );
    assert_eq!(
        service
            .read(
                &context(91, "read-replaced"),
                created["taskId"].as_str().unwrap()
            )
            .unwrap_err(),
        "scan_store_custody_changed"
    );
    assert_eq!(service.status().unwrap_err(), "scan_store_custody_changed");
    assert_eq!(service.database.total_changes(), changes);
    assert_eq!(sqlite_bytes(&service.db_path), before);
    assert_eq!(fs::read(&moved).unwrap(), original);
    assert!(!service.scopes.contains_key(&92));
    let db_path = service.db_path.clone();
    drop(service);
    assert_eq!(sqlite_bytes(&db_path), before);
    assert_eq!(fs::read(&moved).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn live_missing_database_is_not_recreated_and_sqlite_detects_its_opened_file_moved() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let created = service
        .create(&context(91, "before-missing"), input())
        .unwrap();
    let moved = service.db_path.with_file_name("original-missing.sqlite3");
    fs::rename(&service.db_path, &moved).unwrap();
    let original = fs::read(&moved).unwrap();
    let sidecars = sqlite_bytes(&service.db_path);
    let changes = service.database.total_changes();
    assert_eq!(
        sqlite_not_moved(&service.database).unwrap_err(),
        "scan_store_custody_changed"
    );
    assert!(service.set_project(scope(92)).is_err());
    assert!(service
        .create(&context(91, "after-missing"), input())
        .is_err());
    assert!(service
        .read(
            &context(91, "read-missing"),
            created["taskId"].as_str().unwrap()
        )
        .is_err());
    assert!(service.status().is_err());
    assert!(!service.db_path.exists());
    assert_eq!(sqlite_bytes(&service.db_path), sidecars);
    assert_eq!(fs::read(&moved).unwrap(), original);
    assert_eq!(service.database.total_changes(), changes);
    let db_path = service.db_path.clone();
    drop(service);
    assert!(!db_path.exists());
    assert_eq!(sqlite_bytes(&db_path), sidecars);
    assert_eq!(fs::read(&moved).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn custody_preflight_runs_inside_original_write_transaction_before_any_draft_mutation() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let before = snapshot(&service.database);
    let changes = service.database.total_changes();
    let rejected = super::super::create_sentinel_url_scan_publishing_in(
        &service.database,
        91,
        "Must not publish".into(),
        vec!["https://fixture.invalid/".into()],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        false,
        |tx| {
            assert!(!tx.is_autocommit());
            Err("scan_store_custody_changed".into())
        },
        |_, _| panic!("publisher cannot run after failed custody preflight"),
    );
    assert_eq!(rejected.unwrap_err(), "scan_store_custody_changed");
    assert_eq!(snapshot(&service.database), before);
    assert_eq!(service.database.total_changes(), changes);
}

#[cfg(unix)]
#[test]
fn protocol_invalid_large_non_scalar_or_control_ids_are_null_and_never_echoed() {
    let f = Fixture::new();
    let mut service = f.service();
    let before = snapshot(&service.database);
    let changes = service.database.total_changes();
    for id in [
        json!("界".repeat(40_000)),
        json!({"payload":"untrusted-id-marker".repeat(2000)}),
        json!(["private-marker"]),
        json!(true),
        json!(1.5),
        json!(9_007_199_254_740_992_i64),
        json!("bad\nidentifier"),
    ] {
        let request =
            json!({"jsonrpc":"2.0","id":id,"method":"service/status","params":{}}).to_string();
        assert!(request.len() < MAX_FRAME);
        let reply = response(&mut service, request.as_bytes());
        assert_eq!(reply["id"], Value::Null);
        assert!(reply.get("error").is_some());
        let bytes = encoded_frame(&reply).unwrap();
        assert!(bytes.len() <= MAX_RESULT);
        assert!(bytes.len() < 512);
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains("untrusted-id-marker"));
    }
    assert_eq!(snapshot(&service.database), before);
    assert_eq!(service.database.total_changes(), changes);
}

#[test]
fn complete_success_and_error_envelopes_share_the_encoded_wire_budget() {
    for id in [json!(7), json!("\\\"".repeat(60)), json!("界".repeat(40))] {
        let accepted = envelope(id.clone(), Ok(json!({"value":"界".repeat(20_000)})));
        assert!(accepted.get("result").is_some());
        assert!(encoded_frame(&accepted).unwrap().len() <= MAX_RESULT);
        let rejected = envelope(id.clone(), Ok(json!({"value":"x".repeat(MAX_RESULT)})));
        assert_eq!(rejected["error"]["data"]["code"], "scan_result_too_large");
        assert!(encoded_frame(&rejected).unwrap().len() <= MAX_RESULT);
        let code = format!("scan_{}", "x".repeat(MAX_RESULT));
        let failed = failure(id, &code);
        assert_eq!(
            failed["error"]["data"]["code"],
            "scan_domain_operation_rejected"
        );
        assert!(encoded_frame(&failed).unwrap().len() <= MAX_RESULT);
    }
    let body = json!({"value":"x".repeat(MAX_RESULT-64)});
    assert!(serde_json::to_vec(&body).unwrap().len() < MAX_RESULT);
    let overhead = envelope(json!("\"".repeat(128)), Ok(body));
    assert_eq!(overhead["error"]["data"]["code"], "scan_result_too_large");
    assert!(encoded_frame(&overhead).unwrap().len() <= MAX_RESULT);
    assert!(encoded_frame(&json!({"unbounded":"x".repeat(MAX_RESULT)})).is_err());
}

#[cfg(unix)]
#[test]
fn embedded_desktop_separators_cannot_hide_another_credential_target_or_mutate_drafts() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let before = snapshot(&service.database);
    let changes = service.database.total_changes();
    for (index, separator) in [";", ",", "；", "，", " "].into_iter().enumerate() {
        let mut args = input();
        args.targets = vec![format!(
            "https://fixture.invalid/path{separator}https://user:synthetic@other.invalid/"
        )];
        assert_eq!(
            service
                .create(&context(91, &format!("separator-{index}")), args)
                .unwrap_err(),
            "scan_arguments_invalid"
        );
    }
    let mut trim = input();
    trim.targets = vec!["https://fixture.invalid/path'".into()];
    assert!(service.create(&context(91, "trim-wrapper"), trim).is_err());
    assert_eq!(snapshot(&service.database), before);
    assert_eq!(service.database.total_changes(), changes);
}

#[cfg(unix)]
#[test]
fn one_raw_element_cannot_expand_past_32_original_owner_targets() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let before = snapshot(&service.database);
    let changes = service.database.total_changes();
    let expanded = (0..33)
        .map(|i| format!("https://fixture.invalid/{i}"))
        .collect::<Vec<_>>()
        .join(";");
    assert_eq!(expanded.split(';').count(), 33);
    assert!(expanded.len() < 2048);
    let mut args = input();
    args.targets = vec![expanded];
    assert_eq!(
        service
            .create(&context(91, "expanded-33"), args)
            .unwrap_err(),
        "scan_arguments_invalid"
    );
    assert_eq!(snapshot(&service.database), before);
    assert_eq!(service.database.total_changes(), changes);
}

#[cfg(unix)]
#[test]
fn validated_literal_targets_match_real_owner_normalization_without_changing_desktop_rules() {
    let f = Fixture::new();
    let mut service = f.service();
    service.set_project(scope(91)).unwrap();
    let mut args = input();
    args.targets = (0..31)
        .map(|i| format!("https://www.fixture.invalid/Case/{i}/"))
        .collect();
    args.targets
        .push("https://fixture.invalid/Case%3Bsegment?x=a%2Cb".into());
    let expected = args.validate().unwrap();
    assert_eq!(expected.len(), 32);
    let created = service.create(&context(91, "literal-32"), args).unwrap();
    let id = created["taskId"].as_str().unwrap();
    let actual: Vec<String> = service
        .database
        .prepare("SELECT url FROM main.sentinel_targets WHERE scan_id=?1 ORDER BY url")
        .unwrap()
        .query_map([id], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(actual, expected);
    assert!(actual
        .iter()
        .any(|url| url.starts_with("https://www.fixture.invalid/Case/")));
    assert!(actual.contains(&"https://fixture.invalid/Case%3Bsegment?x=a%2Cb".into()));
    assert_eq!(count(&service.database, "agent_runs"), 0);
}
