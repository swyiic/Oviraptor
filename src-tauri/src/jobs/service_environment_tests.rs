use super::*;
use crate::asset_models::StartJobInput;
use std::{
    net::TcpListener,
    sync::{mpsc, Condvar},
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Sink {
    events: Mutex<Vec<(String, Value)>>,
    gate: Mutex<bool>,
    condition: Condvar,
    entered: Mutex<Option<mpsc::Sender<()>>>,
}
impl AssetJobEventSink for Sink {
    fn publish(&self, name: &str, payload: &Value) -> Result<(), String> {
        self.events
            .lock()
            .unwrap()
            .push((name.into(), payload.clone()));
        if name == "job-progress" {
            if let Some(tx) = self.entered.lock().unwrap().take() {
                let _ = tx.send(());
                let guard = self.gate.lock().unwrap();
                let _ = self
                    .condition
                    .wait_timeout_while(guard, Duration::from_secs(10), |released| !*released)
                    .unwrap();
            }
        }
        Ok(())
    }
}
impl Sink {
    fn release(&self) {
        *self.gate.lock().unwrap() = true;
        self.condition.notify_all();
    }
}
struct Fixture {
    root: PathBuf,
    path: PathBuf,
    context: AssetJobContext,
    instance: String,
    profile: i64,
    sink: Arc<Sink>,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-headless-job-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("configuration")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for p in [&root, &root.join("configuration")] {
                fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let path = db::initialize(&root).unwrap();
        let connection = db::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE asset_service_store(singleton INTEGER PRIMARY KEY CHECK(singleton=1),instance_id TEXT NOT NULL UNIQUE,schema_version INTEGER NOT NULL CHECK(schema_version=1));").unwrap();
        let instance = uuid::Uuid::new_v4().to_string();
        connection
            .execute(
                "INSERT INTO asset_service_store VALUES(1,?1,1)",
                [&instance],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO projects(id,name) VALUES(9001,'Synthetic headless asset job')",
                [],
            )
            .unwrap();
        let profile = connection
            .query_row(
                "SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        connection.execute("UPDATE config_profiles SET settings_json=?1 WHERE id=?2",params![json!({"fofaKey":"","configPath":"","modelProfiles":[],"allowPrivate":true,"workers":1,"probeTimeout":2,"probeTotalTimeout":3,"probeRetries":0,"schemeFallback":false,"perHostInterval":0,"priorityRate":1000,"otherRate":1000,"runProbe":true,"runRefine":false}).to_string(),profile]).unwrap();
        let sink = Arc::new(Sink::default());
        let context =
            AssetJobContext::service(&path, &root, instance.clone(), sink.clone()).unwrap();
        Self {
            root,
            path,
            context,
            instance,
            profile,
            sink,
        }
    }
    fn asset(&self, url: &str) {
        let c = db::open(&self.path).unwrap();
        c.execute("INSERT INTO assets(asset_key,host,link,ip,port,protocol) VALUES('synthetic-local-job',?1,?1,'127.0.0.1',?2,'http')",params![url,url.rsplit(':').next().unwrap()]).unwrap();
        let id = c.last_insert_rowid();
        c.execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(9001,?1)",
            [id],
        )
        .unwrap();
    }
    fn start(&self) -> i64 {
        super::super::start_job_in(
            &self.context,
            StartJobInput {
                project_id: 9001,
                profile_id: self.profile,
                name: "Synthetic owned reprobe".into(),
                pipeline: "reprobe".into(),
            },
        )
        .unwrap()
    }
    fn idle(&self) {
        let until = Instant::now() + Duration::from_secs(12);
        while self.context.busy().unwrap() {
            assert!(
                Instant::now() < until,
                "real job must finish within test deadline"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn status(&self, id: i64) -> String {
        db::open(&self.path)
            .unwrap()
            .query_row("SELECT status FROM runs WHERE id=?1", [id], |r| r.get(0))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.sink.release();
        let until = Instant::now() + Duration::from_secs(12);
        while self.context.has_local_activity().unwrap_or(true) && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        if !self.context.has_local_activity().unwrap_or(true) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
fn local_http() -> (String, Arc<AtomicBool>, thread::JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let worker = thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(15);
        let mut requests = 0;
        while !flag.load(Ordering::SeqCst) && Instant::now() < until {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut input = [0; 8192];
                    let _ = stream.read(&mut input);
                    let body="<html><title>Synthetic local service</title>Safe synthetic test content.</html>";
                    let reply=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{}",body.len(),body);
                    let _ = stream.write_all(reply.as_bytes());
                    requests += 1
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("local fixture: {error}"),
            }
        }
        requests
    });
    (url, stop, worker)
}
#[test]
fn service_job_origin_receipt_is_in_original_start_transaction_and_rolls_back_with_it() {
    let f = Fixture::new();
    let mut c = db::open(&f.path).unwrap();
    let id;
    {
        let tx = c.transaction().unwrap();
        tx.execute(
            "INSERT INTO runs(project_id,name,pipeline) VALUES(9001,'rollback','reprobe')",
            [],
        )
        .unwrap();
        id = tx.last_insert_rowid();
        f.context.record_started_on(&tx, id).unwrap();
        assert_eq!(
            tx.query_row(
                "SELECT COUNT(*) FROM asset_service_job_owners WHERE run_id=?1",
                [id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM runs WHERE id=?1", [id], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM asset_service_job_owners WHERE run_id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    c.execute(
        "INSERT INTO runs(project_id,name,pipeline) VALUES(9001,'historical','reprobe')",
        [],
    )
    .unwrap();
    let historical = c.last_insert_rowid();
    assert!(super::super::resume_job_in(&f.context, historical)
        .unwrap_err()
        .contains("historical_run_not_owned"));
    assert!(super::super::cancel_job_in(&f.context, historical)
        .unwrap_err()
        .contains("historical_run_not_owned"));
    assert!(
        AssetJobContext::service(&f.path, &f.root, "wrong-instance".into(), f.sink.clone())
            .is_err()
    );
}
#[test]
fn service_reprobe_runs_original_pipeline_with_local_http_and_persists_owner_terminal_and_events() {
    let f = Fixture::new();
    let (url, stop, server) = local_http();
    f.asset(&url);
    let run = f.start();
    f.idle();
    stop.store(true, Ordering::SeqCst);
    assert!(
        server.join().unwrap() > 0,
        "must have used real loopback HTTP, not a fake probe result"
    );
    assert_eq!(f.status(run), "completed");
    let c = db::open(&f.path).unwrap();
    let owner: String = c
        .query_row(
            "SELECT instance_id FROM asset_service_job_owners WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(owner, f.instance);
    let(status,alive):(String,Option<String>)=c.query_row("SELECT a.status_code,a.last_alive FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=9001 ORDER BY a.id DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(status, "200");
    assert!(alive.is_some());
    assert!(
        c.query_row("SELECT COUNT(*) FROM logs WHERE run_id=?1", [run], |r| r
            .get::<_, i64>(0))
            .unwrap()
            > 0
    );
    let events = f.sink.events.lock().unwrap();
    assert!(events
        .iter()
        .any(|(n, v)| n == "job-progress" && v["status"] == "completed"));
    assert!(events.iter().any(|(n, _)| n == "asset-log-committed"));
}
#[test]
fn service_job_busy_cancel_and_resume_keep_original_run_identity_and_owned_journal() {
    let f = Fixture::new();
    let (url, stop, server) = local_http();
    f.asset(&url);
    let (tx, rx) = mpsc::channel();
    *f.sink.entered.lock().unwrap() = Some(tx);
    let run = f.start();
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(f.context.busy().unwrap());
    assert_eq!(f.context.status().unwrap()["runIds"], json!([run]));
    super::super::cancel_job_in(&f.context, run).unwrap();
    assert_eq!(f.status(run), "cancel_requested");
    assert!(
        f.context.busy().unwrap(),
        "requesting cancellation is not proof the worker exited"
    );
    f.sink.release();
    f.idle();
    assert_eq!(f.status(run), "cancelled");
    let resumed = super::super::resume_job_in(&f.context, run).unwrap();
    assert_eq!(resumed, run);
    f.idle();
    stop.store(true, Ordering::SeqCst);
    assert!(server.join().unwrap() > 0);
    assert_eq!(f.status(run), "completed");
    let c = db::open(&f.path).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM runs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM asset_service_job_owners WHERE run_id=?1",
            [run],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn service_configuration_import_is_bounded_write_only_private_and_reuses_pure_legacy_parser() {
    let f = Fixture::new();
    let content = "[fofa]\nkey = synthetic-configuration-fixture\n";
    let receipt = f.context.import_collection_configuration(content).unwrap();
    assert_eq!(receipt["fofaKeyConfigured"], true);
    assert!(!receipt
        .to_string()
        .contains("synthetic-configuration-fixture"));
    assert!(receipt.get("path").is_none());
    let id = receipt["configurationId"].as_str().unwrap();
    let path = f.context.configuration_path(id).unwrap();
    assert!(path.starts_with(f.root.join("configuration")));
    let settings = json!({"configPath":path,"fofaKey":""});
    assert_eq!(
        f.context.fofa_key(&settings).unwrap(),
        "synthetic-configuration-fixture"
    );
    assert!(f
        .context
        .import_collection_configuration("[other]\nkey=synthetic\n")
        .is_err());
    assert!(f
        .context
        .import_collection_configuration(&"x".repeat(65_537))
        .is_err());
    assert!(f.context.configuration_path("../escape").is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    }
    fs::write(&path, "[fofa]\nkey=changed-fixture\n").unwrap();
    assert!(f
        .context
        .fofa_key(&settings)
        .unwrap_err()
        .contains("content_changed"));
}
#[cfg(unix)]
#[test]
fn service_configuration_rejects_outside_alias_symlink_and_changed_file_before_any_credential_resolution(
) {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let receipt = f
        .context
        .import_collection_configuration("[fofa]\nkey=synthetic-private-key\n")
        .unwrap();
    let path = f
        .context
        .configuration_path(receipt["configurationId"].as_str().unwrap())
        .unwrap();
    let settings = json!({"configPath":path});
    let alias = f.root.join("alias.ini");
    fs::hard_link(&path, &alias).unwrap();
    assert!(f.context.fofa_key(&settings).is_err());
    fs::remove_file(&alias).unwrap();
    let outside = f.root.join("outside.ini");
    fs::rename(&path, &outside).unwrap();
    symlink(&outside, &path).unwrap();
    assert!(f.context.fofa_key(&settings).is_err());
    assert!(f.context.fofa_key(&json!({"configPath":outside})).is_err());
}

#[test]
fn service_job_idle_requires_durable_terminal_evidence_and_never_adopts_unowned_nonterminal_rows() {
    let f = Fixture::new();
    let c = db::open(&f.path).unwrap();
    c.execute("INSERT INTO runs(project_id,name,pipeline,status) VALUES(9001,'unresolved-owned','reprobe','running')",[]).unwrap();
    let owned = c.last_insert_rowid();
    f.context.record_started_on(&c, owned).unwrap();
    let status = f.context.status().unwrap();
    assert_eq!(status["activeJobs"], 0);
    assert_eq!(status["busy"], true);
    assert_eq!(status["uncertain"], true);
    assert_eq!(status["unresolvedRunIds"], json!([owned]));
    c.execute("UPDATE runs SET status='failed' WHERE id=?1", [owned])
        .unwrap();
    assert_eq!(f.context.status().unwrap()["busy"], false);
    c.execute("INSERT INTO runs(project_id,name,pipeline,status) VALUES(9001,'unowned-history','reprobe','running')",[]).unwrap();
    let history = c.last_insert_rowid();
    assert!(f.context.busy().unwrap());
    assert!(super::super::resume_job_in(&f.context, history).is_err());
    assert_eq!(
        c.query_row("SELECT status FROM runs WHERE id=?1", [history], |r| r
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "running"
    );
}

#[test]
fn environment_common_event_port_keeps_legacy_progress_and_log_identity_sequence() {
    let sink = Arc::new(Sink::default());
    let environment = JobEnvironment::headless(Some(sink.clone()));
    let progress = crate::asset_models::JobProgressEvent {
        run_id: 73,
        status: "running".into(),
        stage: "import".into(),
        progress: 95.0,
        message: "Synthetic UTF8 导入进度".into(),
    };
    let committed = crate::asset_logs::LogHint {
        log_id: 101,
        run_id: 73,
        project_id: Some(9001),
    };
    environment.emit("job-progress", progress.clone()).unwrap();
    environment.emit("asset-log-committed", &committed).unwrap();
    environment
        .emit(
            "job-progress",
            crate::asset_models::JobProgressEvent {
                status: "completed".into(),
                stage: "completed".into(),
                progress: 100.0,
                message: "Synthetic completed".into(),
                ..progress.clone()
            },
        )
        .unwrap();
    let events = sink.events.lock().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(
        events[0],
        (
            "job-progress".into(),
            serde_json::to_value(progress).unwrap()
        )
    );
    assert_eq!(
        events[1],
        (
            "asset-log-committed".into(),
            serde_json::to_value(committed).unwrap()
        )
    );
    assert_eq!(events[2].0, "job-progress");
    assert_eq!(events[2].1["status"], "completed");
    let mut keys = events[1]
        .1
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(keys, vec!["logId", "projectId", "runId"]);
}

#[test]
fn environment_erased_activity_keeps_headless_ownership_until_actual_guard_drop_without_gui_handle()
{
    let environment = JobEnvironment::headless(None);
    let active = Arc::new(AtomicUsize::new(0));
    let cancellations = Arc::new(Mutex::new(HashMap::new()));
    cancellations
        .lock()
        .unwrap()
        .insert(73, Arc::new(AtomicBool::new(false)));
    let guard = environment.begin_activity(active.clone(), cancellations.clone(), 73);
    assert_eq!(active.load(Ordering::SeqCst), 1);
    drop(environment.clone());
    assert_eq!(active.load(Ordering::SeqCst), 1);
    assert!(cancellations.lock().unwrap().contains_key(&73));
    drop(guard);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert!(cancellations.lock().unwrap().is_empty());
}

#[test]
fn environment_notification_failure_does_not_release_or_restart_an_active_job() {
    struct FailedOutput(AtomicUsize);
    impl AssetJobEventSink for FailedOutput {
        fn publish(&self, _: &str, _: &Value) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err("synthetic closed output".into())
        }
    }
    let output = Arc::new(FailedOutput(AtomicUsize::new(0)));
    let environment = JobEnvironment::headless(Some(output.clone()));
    let active = Arc::new(AtomicUsize::new(0));
    let cancellations = Arc::new(Mutex::new(HashMap::new()));
    cancellations
        .lock()
        .unwrap()
        .insert(73, Arc::new(AtomicBool::new(false)));
    let guard = environment.begin_activity(active.clone(), cancellations.clone(), 73);
    assert_eq!(
        environment
            .emit("job-progress", json!({"runId":73,"status":"running"}))
            .unwrap_err(),
        "asset_job_notification_unavailable"
    );
    assert_eq!(output.0.load(Ordering::SeqCst), 1);
    assert_eq!(active.load(Ordering::SeqCst), 1);
    assert!(cancellations.lock().unwrap().contains_key(&73));
    drop(guard);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert!(cancellations.lock().unwrap().is_empty());
}
