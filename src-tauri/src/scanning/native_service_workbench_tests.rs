use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    root: PathBuf,
    service: Service,
    source: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-source-workbench-{}",
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
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let environment = ScanServiceEnvironment::from_roots(
            &root.join("data"),
            &root.join("configuration"),
            &root.join("resources"),
        )
        .unwrap();
        let mut service = Service::open(environment).unwrap();
        execution::enable(&service).unwrap();
        sources::enable(&service).unwrap();
        enable(&service).unwrap();
        service
            .set_execution_project(HostProject {
                project_id: 71,
                name: "Source fixture".into(),
                status: "active".into(),
                grant_revision: 1,
                permissions: BTreeSet::from([
                    "source.import".into(),
                    "scan.execute".into(),
                    "draft.read".into(),
                    "scan.control".into(),
                ]),
            })
            .unwrap();
        let bytes = b"const local_fixture = 1;\n";
        let path = "example.js";
        // Independently encode the documented cross-language digest protocol.
        let mut tree = Sha256::new();
        tree.update(b"oviraptor.source-snapshot.v1\0");
        tree.update((path.len() as u32).to_be_bytes());
        tree.update(path.as_bytes());
        tree.update((bytes.len() as u64).to_be_bytes());
        tree.update(Sha256::digest(bytes));
        let input: sources::Import=serde_json::from_value(json!({"label":"Explicit source files","expectedTreeSha256":format!("{:x}",tree.finalize()),"files":[{"path":path,"contentBase64":STANDARD.encode(bytes),"sha256":format!("{:x}",Sha256::digest(bytes))}]})).unwrap();
        let source = sources::import(&service, &Self::context("import-source"), input).unwrap()
            ["sourceRef"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            root,
            service,
            source,
        }
    }
    fn context(op: &str) -> Context {
        Context {
            host_bound_project_id: 71,
            grant_revision: 1,
            operation_id: op.into(),
        }
    }
    fn input(&self) -> Start {
        serde_json::from_value(json!({"sourceRef":self.source,"title":"Original source task","scanType":"code","urls":[],"scanMode":"deep"})).unwrap()
    }
    fn claim(&self, input: &Start, op: &str) -> String {
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(input).unwrap()));
        self.service
            .database
            .execute(
                "INSERT INTO scan_service_workbench_starts VALUES(71,?1,?2,?3,?4,1,'prepared','')",
                params![op, self.service.instance_id, input.source_ref, hash],
            )
            .unwrap();
        hash
    }
    fn original_counts(&self) -> Vec<i64> {
        [
            "sentinel_scans",
            "sentinel_scan_attempts",
            "sentinel_targets",
            "source_scope_contracts",
            "source_runtime_contracts",
            "native_scan_branches",
            "native_branch_dispatches",
            "agent_runs",
            "agent_root_budget_attempts",
            "agent_model_cost_facts",
            "scan_service_operations",
            "scan_service_source_tasks",
            "scan_service_execution_births",
        ]
        .into_iter()
        .map(|t| {
            self.service
                .database
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap()
        })
        .collect()
    }
}
fn writable(path: &Path) {
    let m = fs::symlink_metadata(path).unwrap();
    if m.is_dir() && !m.file_type().is_symlink() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        for e in fs::read_dir(path).unwrap() {
            writable(&e.unwrap().path());
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        writable(&self.root);
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn original_source_cost_admission_remains_before_any_attempt_and_replays_only_its_rejection_receipt(
) {
    let f = Fixture::new();
    let mut input = f.input();
    input.max_budget_usd = Some(1.0);
    let before = f.original_counts();
    let receipt = start(&f.service, &Fixture::context("cost-gate"), input).unwrap();
    assert_eq!(receipt["outcome"]["accepted"], false);
    assert_eq!(receipt["outcome"]["disposition"], "rejected");
    assert_eq!(
        receipt["outcome"]["code"],
        "source_model_cost_accounting_unavailable"
    );
    assert!(receipt["taskId"].is_null());
    assert_eq!(f.original_counts(), before);
    assert!(!unresolved_on(&f.service.database).unwrap());
    let changes = f.service.database.total_changes();
    let mut input = f.input();
    input.max_budget_usd = Some(1.0);
    assert_eq!(
        start(&f.service, &Fixture::context("cost-gate"), input).unwrap(),
        receipt
    );
    assert_eq!(f.service.database.total_changes(), changes);
    assert_eq!(f.original_counts(), before);
    let mut changed = f.input();
    changed.title = "Changed request".into();
    assert_eq!(
        start(&f.service, &Fixture::context("cost-gate"), changed).unwrap_err(),
        "scan_operation_conflict"
    );
    assert_eq!(f.service.database.total_changes(), changes);
    assert_eq!(f.original_counts(), before);
}

#[test]
fn pending_workbench_operation_is_busy_and_never_reexecutes_after_reentry_or_page_detach() {
    let f = Fixture::new();
    let input = f.input();
    f.claim(&input, "pending-start");
    let before = f.original_counts();
    let changes = f.service.database.total_changes();
    assert!(unresolved_on(&f.service.database).unwrap());
    assert_eq!(
        start(&f.service, &Fixture::context("pending-start"), input).unwrap_err(),
        "scan_action_outcome_uncertain_no_automatic_replay"
    );
    assert_eq!(f.original_counts(), before);
    assert_eq!(f.service.database.total_changes(), changes);
}

#[test]
fn workbench_rejects_changed_source_and_foreign_reference_before_claim_or_execution() {
    let f = Fixture::new();
    let mut other = f.input();
    other.source_ref = uuid::Uuid::new_v4().to_string();
    let before = f.original_counts();
    let changes = f.service.database.total_changes();
    assert!(start(&f.service, &Fixture::context("foreign-ref"), other).is_err());
    let source = sources::resolve_on(
        &f.service,
        &f.service.database,
        &Fixture::context("inspect"),
        &f.source,
    )
    .unwrap();
    let file = source.path().join("example.js");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&file, b"modified snapshot").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(start(&f.service, &Fixture::context("changed-source"), f.input()).is_err());
    assert_eq!(
        f.service
            .database
            .query_row(
                "SELECT COUNT(*) FROM scan_service_workbench_starts",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(f.original_counts(), before);
    assert_eq!(f.service.database.total_changes(), changes);
}

#[test]
fn closed_workbench_input_excludes_paths_projects_models_auth_values_and_fabricated_git_claims() {
    let f = Fixture::new();
    let base = serde_json::to_value(f.input()).unwrap();
    for (name, value) in [
        ("sourcePath", json!("/operator/path")),
        ("projectId", json!(72)),
        ("apiKey", json!("synthetic")),
        ("model", json!("changed")),
        ("authValue", json!("synthetic")),
        ("diffBase", json!("not-a-git-commit")),
        ("scopeMode", json!("diff")),
        ("commitSha", json!("unverified")),
    ] {
        let mut bad = base.clone();
        bad[name] = value;
        assert!(serde_json::from_value::<Start>(bad).is_err(), "{name}");
    }
    let verified = sources::resolve_on(
        &f.service,
        &f.service.database,
        &Fixture::context("original-input"),
        &f.source,
    )
    .unwrap();
    let original = f.input().original(71, verified.path()).unwrap();
    assert_eq!(original.scope_mode, "full");
    assert!(original.diff_base.is_empty());
    assert!(original.commit_sha.is_empty());
    assert!(original.auth_value.is_empty());
    let mut multiline = f.input();
    multiline.instruction =
        "Review only supplied code.\nPreserve its case.\tNo fake findings.".into();
    multiline.validate().unwrap();
    let mut full = f.input();
    full.scan_type = "greybox".into();
    full.urls = (0..200)
        .map(|n| format!("https://fixture.invalid/{n}"))
        .collect();
    full.validate().unwrap();
    full.urls.push("https://fixture.invalid/201".into());
    assert!(full.validate().is_err());
    assert!(f.original_counts().iter().all(|n| *n == 0));
}

// This invokes the original owner publisher, including the original source
// Scope/HMAC/branch SQL, with the *same* production service callback. No worker
// is launched and no model/target transport is contacted by this helper.
fn publish_original(
    f: &Fixture,
    input: &Start,
    hash: &str,
) -> Result<crate::scanning_models::SentinelScan, String> {
    let db = &f.service.database;
    db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles',json(?1),'$.activeModelProfileId','source-fixture') WHERE id=(SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1)",[json!([{"id":"source-fixture","llm":"synthetic-source","apiKey":"synthetic-not-contacted","apiBase":"http://127.0.0.1:9/v1","deployment":"cloud"}]).to_string()]).unwrap();
    let context = Fixture::context("publish-original");
    let verified = sources::resolve_on(&f.service, db, &context, &f.source)?;
    let work = f.root.join("prepared-attempt");
    fs::create_dir(&work).unwrap();
    fs::set_permissions(&work, fs::Permissions::from_mode(0o700)).unwrap();
    let model = owner::model_runtime_env(&owner::sentinel_settings(db))?;
    let mut policy = owner::build_workbench_investigation_policy("deep", None, vec![], &[], "")?;
    policy["maxCritical"] = json!(0);
    policy["maxHigh"] = json!(5);
    policy["blockRelease"] = json!(false);
    let project_name: String = db
        .query_row("SELECT name FROM projects WHERE id=71", [], |r| r.get(0))
        .unwrap();
    let record:owner::WorkbenchStartRecord=serde_json::from_value(json!({"scanId":"owned-source-real","projectId":71,"projectName":project_name,"taskName":input.title,"scanType":"code","attempt":1,"urls":[],"sourcePath":verified.path(),"scopeMode":"full","diffBase":"","skills":"","environment":"","authProfileName":"","authType":"none","authSessionIds":[],"authenticated":false,"ciProvider":"","repositoryUrl":"","branch":"","commitSha":"","buildId":"","policy":policy,"llmPolicy":owner::source_model_policy(&model)})).unwrap();
    fs::write(work.join("task.json"),serde_json::to_vec(&json!({"scanId":record.scan_id,"projectId":71,"scanType":"code","attempt":1,"sourcePath":verified.path(),"scopeMode":"full","diffBase":"","scanMode":"deep","maxBudgetUsd":null,"policy":policy,"llmPolicy":record.llm_policy})).unwrap()).unwrap();
    let plan = owner::ScanBackendPlan {
        scan_id: record.scan_id.clone(),
        attempt_number: 1,
        targets: vec![],
        requires_node: false,
        requires_browser: false,
    };
    let binary = execution::executable_sha256()?;
    owner::publish_workbench_start_publishing_in(
        db,
        &record,
        false,
        "",
        &work,
        &plan,
        PathBuf::new(),
        &model,
        |tx| require_claim(tx, &f.service, &context, input, hash),
        |tx, scan| publish_ownership_on(&f.service, &context, input, hash, &binary, tx, scan),
    )
    .map(|(scan, _)| scan)
}

#[test]
fn original_source_publication_binds_scope_attempt_branch_membership_and_birth_in_one_transaction()
{
    let f = Fixture::new();
    let input = f.input();
    let hash = f.claim(&input, "publish-original");
    let scan = publish_original(&f, &input, &hash).unwrap();
    assert_eq!(scan.id, "owned-source-real");
    assert_eq!(scan.status, "scanning");
    let actual:(i64,i64,i64,i64,i64,i64)=f.service.database.query_row("SELECT (SELECT COUNT(*) FROM sentinel_scan_attempts),(SELECT COUNT(*) FROM source_scope_contracts),(SELECT COUNT(*) FROM source_runtime_contracts),(SELECT COUNT(*) FROM native_scan_branches),(SELECT COUNT(*) FROM scan_service_source_tasks),(SELECT COUNT(*) FROM scan_service_execution_births)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(actual, (1, 1, 1, 1, 1, 1));
    verify_task_on(
        &f.service,
        &f.service.database,
        &Fixture::context("read"),
        &scan.id,
    )
    .unwrap();
    let listed = execution::list(
        &f.service,
        &Fixture::context("list"),
        serde_json::from_value(json!({})).unwrap(),
    )
    .unwrap();
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["items"][0]["taskId"], scan.id);
    assert_eq!(
        f.service
            .database
            .query_row("SELECT COUNT(*) FROM agent_runs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        unresolved_on(&f.service.database).unwrap(),
        "publication alone is not a confirmed outer operation outcome"
    );
}

#[test]
fn ignored_service_membership_birth_or_source_binding_rolls_back_the_complete_original_publication()
{
    for table in [
        "scan_service_operations",
        "scan_service_source_tasks",
        "scan_service_execution_births",
    ] {
        let f = Fixture::new();
        let input = f.input();
        let hash = f.claim(&input, "publish-original");
        f.service.database.execute_batch(&format!("CREATE TRIGGER refuse_service_publication BEFORE INSERT ON {table} BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
        assert!(publish_original(&f, &input, &hash).is_err(), "{table}");
        assert!(f.original_counts().iter().all(|n| *n == 0), "{table}");
        assert!(
            unresolved_on(&f.service.database).unwrap(),
            "ambiguous/prepared work cannot be reexecuted or reported idle"
        );
    }
}
