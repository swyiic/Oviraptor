use super::*;
use crate::native_app_sdk::HostProject;
use std::sync::atomic::AtomicBool;
struct Fixture {
    root: PathBuf,
    db_path: PathBuf,
    db: Connection,
    store: UploadStore,
    context: NativeCallContext,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-exposure-upload-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::create_dir(root.join("configuration")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                root.join("configuration"),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
        }
        let db_path = crate::db::initialize(&root).unwrap();
        let db = crate::db::open(&db_path).unwrap();
        db.execute_batch("CREATE TABLE asset_service_store(singleton INTEGER PRIMARY KEY,instance_id TEXT NOT NULL,schema_version INTEGER NOT NULL);INSERT INTO projects(id,name)VALUES(9001,'Synthetic upload'),(9002,'Other synthetic project');").unwrap();
        db.execute(
            "INSERT INTO asset_service_store VALUES(1,?1,1)",
            [uuid::Uuid::new_v4().to_string()],
        )
        .unwrap();
        let store = UploadStore::open(&db, &root).unwrap();
        let context = NativeCallContext {
            host_bound_project_id: 55,
            grant_revision: 1,
            operation_id: "synthetic-host-op".into(),
            app_revision: 1,
            permissions: vec!["inventory.exposure.manage".into()],
            permission: "inventory.exposure.manage".into(),
            project: HostProject {
                id: 55,
                name: "Synthetic host project".into(),
                status: "active".into(),
            },
        };
        Self {
            root,
            db_path,
            db,
            store,
            context,
        }
    }
    fn metadata(op: &str) -> Value {
        json!({"operationId":op,"channel":"document_import","operator":"Synthetic operator","sourceDeclaration":"Synthetic supplied test document; no network observation"})
    }
    fn begin(&self, op: &str, bodies: &[Vec<u8>]) -> Value {
        self.call("begin_exposure_import_upload",json!({"request":Self::metadata(op),"files":bodies.iter().enumerate().map(|(i,b)|json!({"name":format!("synthetic-{i}.txt"),"format":"text","bytes":b.len(),"sha256":sha(b)})).collect::<Vec<_>>()})).unwrap()
    }
    fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        self.call_at(name, args, 1000)
    }
    fn call_at(&self, name: &str, args: Value, now: i64) -> Result<Value, String> {
        self.store.dispatch_at(
            &self.db,
            9001,
            &self.context,
            name,
            args,
            now,
            crate::commands::exposure::imports::validate_metadata,
            |_| panic!("unexpected import"),
        )
    }
    fn put(&self, upload: &Value, bodies: &[Vec<u8>]) {
        for (i, b) in bodies.iter().enumerate() {
            for (n, chunk) in b.chunks(CHUNK_BYTES).enumerate() {
                self.call("put_exposure_import_chunk",json!({"uploadId":upload["uploadId"],"fileIndex":i,"chunkIndex":n,"contentBase64":base64::engine::general_purpose::STANDARD.encode(chunk)})).unwrap();
            }
        }
    }
    fn finalize(upload: &Value) -> Value {
        json!({"uploadId":upload["uploadId"],"expectedManifestDigest":upload["manifestDigest"]})
    }
    fn commit(
        &self,
        upload: &Value,
        owner: impl FnOnce(Value) -> Result<Value, String>,
    ) -> Result<Value, String> {
        self.store.dispatch_at(
            &self.db,
            9001,
            &self.context,
            "commit_exposure_import_upload",
            Self::finalize(upload),
            1001,
            crate::commands::exposure::imports::validate_metadata,
            owner,
        )
    }
    fn original_import(&self, v: Value) -> Result<Value, String> {
        crate::asset_service::exposure_service::dispatch_project(
            &self.db_path,
            &self.root,
            9001,
            "import_exposure_documents",
            json!({"projectId":9001,"request":v}),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn exposure_upload_large_request_keeps_original_sqlite_import_provenance_and_purges_raw() {
    let f = Fixture::new();
    let bodies = vec![vec![b'x'; 1_200_000]];
    let u = f.begin("large-original-0001", &bodies);
    f.put(&u, &bodies);
    assert!(
        serde_json::to_vec(
            &Manifest {
                request: input(Fixture::metadata("large-original-0001")).unwrap(),
                files: vec![FileSpec {
                    name: "synthetic-0.txt".into(),
                    format: "text".into(),
                    bytes: bodies[0].len(),
                    sha256: sha(&bodies[0])
                }]
            }
            .request(Some(bodies.clone()))
        )
        .unwrap()
        .len()
            > 1024 * 1024
    );
    let report = f.commit(&u, |v| f.original_import(v)).unwrap();
    assert_eq!(report["projectId"], 9001);
    assert_eq!(report["declaredUrlVerified"], false);
    assert!(report["runId"].as_i64().unwrap() > 0);
    assert_eq!(report["files"][0]["bytes"], 1_200_000);
    let(count,network):(i64,i64)=f.db.query_row("SELECT COUNT(*),COALESCE(SUM(json_extract(evidence_json,'$.networkObserved')),0) FROM exposure_observations",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!(count > 0);
    assert_eq!(network, 0);
    let folder = f.store.root.path.join(u["uploadId"].as_str().unwrap());
    assert_eq!(fs::read_dir(folder).unwrap().count(), 0);
    let status = f
        .call(
            "exposure_import_upload_status",
            json!({"uploadId":u["uploadId"]}),
        )
        .unwrap();
    assert_eq!(status["phase"], "completed");
    assert_eq!(status["contentVerified"], true);
    assert!(f
        .commit(&u, |_| panic!("receipt must not reinvoke"))
        .is_ok());
}

#[test]
fn exposure_upload_original_twenty_two_eight_limits_and_zero_file_behavior() {
    let f = Fixture::new();
    let twenty = vec![vec![]; 20];
    let u = f.begin("twenty-empty-0001", &twenty);
    assert_eq!(u["files"].as_array().unwrap().len(), 20);
    f.commit(&u, |v| {
        assert_eq!(v["files"].as_array().unwrap().len(), 20);
        Ok(json!({"synthetic":true}))
    })
    .unwrap();
    let bytes = vec![vec![b'a'; FILE_BYTES]; 4];
    let u = f.begin("eight-megabytes-0001", &bytes);
    f.put(&u, &bytes);
    f.commit(&u, |v| {
        let sizes = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| {
                base64::engine::general_purpose::STANDARD
                    .decode(x["contentBase64"].as_str().unwrap())
                    .unwrap()
                    .len()
            })
            .collect::<Vec<_>>();
        assert_eq!(sizes, vec![FILE_BYTES; 4]);
        Ok(json!({"exactBytes":BATCH_BYTES}))
    })
    .unwrap();
    for files in [
        vec![
            FileSpec {
                name: "empty.txt".into(),
                format: "text".into(),
                bytes: 0,
                sha256: sha(&[])
            };
            21
        ],
        vec![FileSpec {
            name: "large.txt".into(),
            format: "text".into(),
            bytes: FILE_BYTES + 1,
            sha256: sha(&[]),
        }],
        vec![
            FileSpec {
                name: "large.txt".into(),
                format: "text".into(),
                bytes: FILE_BYTES,
                sha256: sha(&[])
            };
            5
        ],
    ] {
        let m = Manifest {
            request: input(Fixture::metadata("limit-refusal-0001")).unwrap(),
            files,
        };
        assert!(m.validate().is_err());
    }
}

#[test]
fn exposure_upload_missing_corrupt_chunks_and_whole_digest_never_call_owner() {
    let f = Fixture::new();
    let bodies = vec![vec![b'z'; CHUNK_BYTES + 1]];
    let u = f.begin("missing-chunk-0001", &bodies);
    let called = AtomicBool::new(false);
    assert!(f
        .commit(&u, |_| {
            called.store(true, Ordering::SeqCst);
            Ok(json!({}))
        })
        .unwrap_err()
        .contains("chunks_missing"));
    assert!(!called.load(Ordering::SeqCst));
    f.put(&u, &bodies);
    let path = f
        .store
        .root
        .path
        .join(u["uploadId"].as_str().unwrap())
        .join("file-0-chunk-1");
    fs::write(path, b"wrong").unwrap();
    assert!(f
        .commit(&u, |_| panic!("corrupt bytes reach owner"))
        .is_err());
    let u=f.call("begin_exposure_import_upload",json!({"request":Fixture::metadata("wrong-sha-0001"),"files":[{"name":"a.txt","format":"text","bytes":1,"sha256":sha(b"b")}]})).unwrap();
    f.put(&u, &[b"a".to_vec()]);
    assert!(f
        .commit(&u, |_| panic!("bad whole digest reach owner"))
        .unwrap_err()
        .contains("file_digest_mismatch"));
}

#[test]
fn exposure_upload_chunks_are_canonical_fixed_and_idempotent_without_overwrite() {
    let f = Fixture::new();
    let bodies = vec![b"abcdef".to_vec()];
    let u = f.begin("chunk-contract-0001", &bodies);
    let args =
        json!({"uploadId":u["uploadId"],"fileIndex":0,"chunkIndex":0,"contentBase64":"YWJjZGVm"});
    assert_eq!(
        f.call("put_exposure_import_chunk", args.clone()).unwrap()["alreadyStored"],
        false
    );
    assert_eq!(
        f.call("put_exposure_import_chunk", args).unwrap()["alreadyStored"],
        true
    );
    for (index, body) in [(0, "YWJjZGVm\n"), (0, "eHh4eHh4"), (1, "YQ=="), (0, "YQ==")] {
        assert!(f.call("put_exposure_import_chunk",json!({"uploadId":u["uploadId"],"fileIndex":0,"chunkIndex":index,"contentBase64":body})).is_err());
    }
    let s = f
        .call(
            "exposure_import_upload_status",
            json!({"uploadId":u["uploadId"]}),
        )
        .unwrap();
    assert_eq!(s["files"][0]["receivedBytes"], 6);
    assert_eq!(s["allChunksRecorded"], true);
    assert_eq!(s["contentVerified"], false);
}

#[test]
fn exposure_upload_scope_permission_and_recipient_generation_never_silently_resume() {
    let f = Fixture::new();
    let u = f.begin("scope-guard-0001", &[b"x".to_vec()]);
    let args = json!({"uploadId":u["uploadId"]});
    assert!(f
        .store
        .dispatch_at(
            &f.db,
            9002,
            &f.context,
            "exposure_import_upload_status",
            args.clone(),
            1001,
            |_| Ok(()),
            |_| unreachable!()
        )
        .is_err());
    for change in 0..4 {
        let mut c = f.context.clone();
        match change {
            0 => c.grant_revision += 1,
            1 => c.app_revision += 1,
            2 => {
                c.host_bound_project_id = 77;
                c.project.id = 77;
            }
            _ => c.permissions.clear(),
        };
        assert!(f
            .store
            .dispatch_at(
                &f.db,
                9001,
                &c,
                "exposure_import_upload_status",
                args.clone(),
                1001,
                |_| Ok(()),
                |_| unreachable!()
            )
            .is_err());
    }
    assert!(f
        .call(
            "exposure_import_upload_status",
            json!({"uploadId":u["uploadId"],"projectId":9002})
        )
        .is_err());
    assert!(f.call("begin_exposure_import_upload",json!({"request":Fixture::metadata("unknown-field-0001"),"files":[],"path":"/tmp/not-allowed"})).is_err());
}

#[test]
fn exposure_upload_staging_discard_and_expiry_have_no_domain_effects_and_release_quota() {
    let f = Fixture::new();
    let a = f.begin("quota-first-0001", &[b"a".to_vec()]);
    let b = f.begin("quota-second-0001", &[b"b".to_vec()]);
    assert!(f.call("begin_exposure_import_upload",json!({"request":Fixture::metadata("quota-third-0001"),"files":[{"name":"a","format":"text","bytes":0,"sha256":sha(&[])}]})).unwrap_err().contains("quota"));
    assert_eq!(
        f.call("discard_exposure_import_upload", Fixture::finalize(&a))
            .unwrap()["businessEffects"],
        false
    );
    assert!(f
        .call("discard_exposure_import_upload", Fixture::finalize(&a))
        .is_err());
    assert!(f
        .call_at(
            "put_exposure_import_chunk",
            json!({"uploadId":b["uploadId"],"fileIndex":0,"chunkIndex":0,"contentBase64":"Yg=="}),
            1000 + TTL_MS
        )
        .unwrap_err()
        .contains("expired"));
    f.store.expire(&f.db, 1000 + TTL_MS).unwrap();
    let count: i64 =
        f.db.query_row("SELECT COUNT(*) FROM exposure_runs", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 0);
    let reserved: i64 =
        f.db.query_row(
            "SELECT COUNT(*) FROM asset_exposure_uploads WHERE cleanup_pending=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 0);
}

#[cfg(unix)]
#[test]
fn exposure_upload_rejects_symlink_hardlink_and_replaced_chunks() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let bodies = vec![b"abc".to_vec()];
    let u = f.begin("file-custody-0001", &bodies);
    f.put(&u, &bodies);
    let path = f
        .store
        .root
        .path
        .join(u["uploadId"].as_str().unwrap())
        .join("file-0-chunk-0");
    fs::rename(&path, path.with_extension("retained")).unwrap();
    fs::write(&path, b"abc").unwrap();
    assert!(f.commit(&u, |_| panic!("replaced inode")).is_err());
    fs::remove_file(&path).unwrap();
    symlink(path.with_extension("retained"), &path).unwrap();
    assert!(f.commit(&u, |_| panic!("symlink")).is_err());
    fs::remove_file(&path).unwrap();
    fs::hard_link(path.with_extension("retained"), &path).unwrap();
    assert!(f.commit(&u, |_| panic!("hardlink")).is_err());
}

#[test]
fn exposure_upload_original_validator_error_and_owner_error_are_preserved() {
    let f = Fixture::new();
    let mut request = Fixture::metadata("original-error-0001");
    request["channel"] = json!("arbitrary_import");
    let failure=f.call("begin_exposure_import_upload",json!({"request":request,"files":[{"name":"a.txt","format":"text","bytes":0,"sha256":sha(&[])}]})).unwrap_err();
    assert!(failure.contains("exposure_import_invalid"));
    let u = f.begin("owner-error-0001", &[vec![]]);
    assert_eq!(
        f.commit(&u, |_| Err(
            "[synthetic_original_owner_error] unchanged".into()
        ))
        .unwrap_err(),
        "[synthetic_original_owner_error] unchanged"
    );
    assert_eq!(
        f.commit(&u, |_| panic!("known error must not reinvoke"))
            .unwrap_err(),
        "[synthetic_original_owner_error] unchanged"
    );
}

#[test]
fn exposure_upload_unknown_commit_cannot_be_replayed_via_same_or_new_upload() {
    let f = Fixture::new();
    let bodies = vec![b"x".to_vec()];
    let a = f.begin("unknown-origin-0001", &bodies);
    let b = f.begin("unknown-origin-0001", &bodies);
    f.put(&a, &bodies);
    f.put(&b, &bodies);
    let id = a["uploadId"].as_str().unwrap();
    let digest = a["manifestDigest"].as_str().unwrap();
    f.db.execute(
        "INSERT INTO asset_exposure_upload_claims VALUES(?1,9001,'unknown-origin-0001',?2,?3)",
        params![f.store.instance, digest, id],
    )
    .unwrap();
    f.db.execute(
        "UPDATE asset_exposure_uploads SET phase='committing' WHERE upload_id=?1",
        [id],
    )
    .unwrap();
    assert!(f.commit(&a, |_| panic!("same UUID blind replay")).is_err());
    assert!(f
        .commit(&b, |_| panic!("new UUID blind replay"))
        .unwrap_err()
        .contains("operation_already_claimed_do_not_replay"));
    assert!(f
        .call("discard_exposure_import_upload", Fixture::finalize(&a))
        .is_err());
    assert!(f
        .db
        .execute(
            "UPDATE asset_exposure_uploads SET phase='receiving' WHERE upload_id=?1",
            [id]
        )
        .is_err());
}

#[test]
fn exposure_upload_status_is_bounded_metadata_not_paths_or_raw_content() {
    let f = Fixture::new();
    let u = f.begin(
        "metadata-only-0001",
        &[b"synthetic private document marker".to_vec()],
    );
    f.put(&u, &[b"synthetic private document marker".to_vec()]);
    let v = f
        .call(
            "exposure_import_upload_status",
            json!({"uploadId":u["uploadId"]}),
        )
        .unwrap();
    let bytes = serde_json::to_vec(&v).unwrap();
    assert!(bytes.len() < 32768);
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("synthetic private document marker"));
    assert!(!text.contains("Synthetic supplied test"));
    assert!(!text.contains(&f.root.to_string_lossy().to_string()));
}

fn seed_sync_birth(f: &Fixture, u: &Value, operation: &str) -> i64 {
    let id = u["uploadId"].as_str().unwrap();
    f.db.execute(
        "INSERT INTO asset_exposure_upload_claims VALUES(?1,9001,?2,?3,?4)",
        params![
            f.store.instance,
            operation,
            u["manifestDigest"].as_str().unwrap(),
            id
        ],
    )
    .unwrap();
    f.db.execute(
        "UPDATE asset_exposure_uploads SET phase='committing' WHERE upload_id=?1",
        [id],
    )
    .unwrap();
    let tx = rusqlite::Transaction::new_unchecked(&f.db, TransactionBehavior::Immediate).unwrap();
    tx.execute("INSERT INTO exposure_runs(project_id,status,stage,created_at)VALUES(9001,'queued','planned','synthetic-sync-birth')",[]).unwrap();
    let run = tx.last_insert_rowid();
    tx.execute("INSERT INTO exposure_run_plans(run_id,project_id,attempt_number,scope_version,sources_json,planned_items,created_at)VALUES(?1,9001,1,1,'[\"document_import\"]',1,'synthetic-sync-birth')",[run]).unwrap();
    tx.execute("INSERT INTO exposure_work_items(run_id,project_id,work_key,kind,source_key,input_hash)VALUES(?1,9001,'synthetic','document_import','document_import','digest')",[run]).unwrap();
    tx.execute("INSERT INTO exposure_import_batches(project_id,run_id,operation_id,payload_hash,channel,operator,source_declaration,import_version,created_at)VALUES(9001,?1,?2,'synthetic-payload','document_import','synthetic','synthetic','oviraptor.exposure.import.v1','synthetic-sync-birth')",params![run,operation]).unwrap();
    tx.commit().unwrap();
    run
}
#[test]
fn exposure_upload_sync_unknown_is_review_only_after_os_owner_exit_but_never_replayable() {
    let f = Fixture::new();
    let u = f.begin("sync-unknown-0001", &[vec![]]);
    let run = seed_sync_birth(&f, &u, "sync-unknown-0001");
    assert!(!recovered_sync_import_on(&f.db, &f.store.instance, run).unwrap());
    let controller = crate::agent_runtime::execution_owner::claim_native_invocation(
        &f.db_path,
        "",
        0,
        "assets_service",
        "controller",
    )
    .unwrap();
    f.store
        .recover_after_exclusive_owner(&f.db, &controller, 0)
        .unwrap();
    let proofs = f
        .store
        .quiesced_import_proofs(&f.db, &controller, 0)
        .unwrap();
    assert_eq!(proofs.len(), 1);
    let status = crate::asset_service::owned_work::exposure_status_with_quiesced_imports(
        &f.db_path, 0, &proofs,
    )
    .unwrap();
    assert_eq!(status["activeRows"], 1);
    assert_eq!(status["physicalActiveRows"], 0);
    assert_eq!(status["unresolvedImportRows"], 1);
    assert_eq!(status["busy"], false);
    assert_eq!(status["uncertain"], false);
    assert_eq!(status["operationResultsRequireReview"], true);
    assert!(f
        .commit(&u, |_| panic!("quiescent is not permission to replay"))
        .is_err());
    let live = crate::asset_service::owned_work::exposure_status_with_quiesced_imports(
        &f.db_path, 1, &proofs,
    )
    .unwrap();
    assert_eq!(live["busy"], true);
    assert_eq!(live["physicalActiveRows"], 1);
    f.db.execute(
        "UPDATE exposure_runs SET stage='resuming' WHERE id=?1",
        [run],
    )
    .unwrap();
    f.db.execute(
        "UPDATE exposure_runs SET stage='executing' WHERE id=?1",
        [run],
    )
    .unwrap();
    let reused = crate::asset_service::owned_work::exposure_status_with_quiesced_imports(
        &f.db_path, 0, &proofs,
    )
    .unwrap();
    assert_eq!(reused["busy"], true);
    assert_eq!(reused["physicalActiveRows"], 1);
}

#[test]
fn exposure_upload_sync_birth_proof_never_hides_future_async_receipt_or_changed_attempt() {
    let f = Fixture::new();
    let u = f.begin("future-async-0001", &[vec![]]);
    let run = seed_sync_birth(&f, &u, "future-async-0001");
    let controller = crate::agent_runtime::execution_owner::claim_native_invocation(
        &f.db_path,
        "",
        0,
        "assets_service",
        "controller",
    )
    .unwrap();
    f.store
        .recover_after_exclusive_owner(&f.db, &controller, 0)
        .unwrap();
    let proofs = f
        .store
        .quiesced_import_proofs(&f.db, &controller, 0)
        .unwrap();
    assert_eq!(proofs.len(), 1);
    crate::asset_service::owned_work::record(
        &f.db_path,
        crate::asset_service::owned_work::Kind::ExposureRun,
        9001,
        &run.to_string(),
    )
    .unwrap();
    let async_status = crate::asset_service::owned_work::exposure_status_with_quiesced_imports(
        &f.db_path, 0, &proofs,
    )
    .unwrap();
    assert_eq!(async_status["busy"], true);
    assert_eq!(async_status["unresolvedImportRows"], 0);
    f.db.execute(
        "UPDATE exposure_run_plans SET attempt_number=2 WHERE run_id=?1",
        [run],
    )
    .unwrap();
    f.db.execute(
        "UPDATE exposure_run_plans SET attempt_number=1 WHERE run_id=?1",
        [run],
    )
    .unwrap();
    assert!(!recovered_sync_import_on(&f.db, &f.store.instance, run).unwrap());
}

#[test]
fn exposure_upload_global_quota_and_declared_bytes_reservation_are_atomic() {
    let f = Fixture::new();
    for project in 9002..9006 {
        f.db.execute(
            "INSERT OR IGNORE INTO projects(id,name)VALUES(?1,?2)",
            params![project, format!("Synthetic quota {project}")],
        )
        .unwrap();
    }
    for i in 0..8 {
        let project = 9001 + i / 2;
        let value = json!({"request":Fixture::metadata(&format!("global-quota-{i:04}")),"files":(0..4).map(|i|json!({"name":format!("declared-{i}.txt"),"format":"text","bytes":FILE_BYTES,"sha256":sha(&[])})).collect::<Vec<_>>()});
        f.store
            .dispatch_at(
                &f.db,
                project,
                &f.context,
                "begin_exposure_import_upload",
                value,
                1000,
                crate::commands::exposure::imports::validate_metadata,
                |_| unreachable!(),
            )
            .unwrap();
    }
    let value = json!({"request":Fixture::metadata("global-quota-overflow"),"files":[{"name":"empty.txt","format":"text","bytes":0,"sha256":sha(&[])}]});
    assert!(f
        .store
        .dispatch_at(
            &f.db,
            9005,
            &f.context,
            "begin_exposure_import_upload",
            value,
            1000,
            crate::commands::exposure::imports::validate_metadata,
            |_| unreachable!()
        )
        .unwrap_err()
        .contains("quota"));
    let count: i64 =
        f.db.query_row("SELECT COUNT(*) FROM asset_exposure_uploads", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 8);
    let reserved: i64 =
        f.db.query_row(
            "SELECT SUM(total_bytes) FROM asset_exposure_uploads WHERE cleanup_pending=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, RESERVED_BYTES);
}

#[test]
fn exposure_upload_active_rpc_or_unknown_async_owner_never_becomes_idle_from_missing_report() {
    let f = Fixture::new();
    let u = f.begin("rpc-owner-0001", &[vec![]]);
    f.commit(&u, |_| {
        let status = f.store.activity_status(&f.db).unwrap();
        assert_eq!(status["busy"], true);
        assert_eq!(status["liveWorkers"], 1);
        assert_eq!(status["operationResultsRequireReview"], true);
        assert!(f
            .call("discard_exposure_import_upload", Fixture::finalize(&u))
            .is_err());
        Ok(json!({"synthetic":true}))
    })
    .unwrap();
    assert_eq!(f.store.activity_status(&f.db).unwrap()["busy"], false);
    f.db.execute("INSERT INTO exposure_runs(project_id,status,stage,created_at)VALUES(9001,'running','resuming','synthetic-foreign-async')",[]).unwrap();
    let plain =
        crate::asset_service::owned_work::exposure_status_with_quiesced_imports(&f.db_path, 0, &[])
            .unwrap();
    assert_eq!(plain["busy"], true);
    assert_eq!(plain["uncertain"], true);
}

#[test]
fn exposure_upload_receipt_failure_after_owner_completion_never_reinvokes_or_invents_physical_worker(
) {
    let f = Fixture::new();
    let u = f.begin("finalize-failure-0001", &[vec![]]);
    f.db.execute_batch("CREATE TRIGGER synthetic_stage_finalize_failure BEFORE UPDATE ON asset_exposure_uploads WHEN NEW.phase IN ('completed','failed') BEGIN SELECT RAISE(ABORT,'synthetic receipt failure');END;").unwrap();
    let calls = AtomicUsize::new(0);
    let error = f
        .commit(&u, |v| {
            calls.fetch_add(1, Ordering::SeqCst);
            f.original_import(v)
        })
        .unwrap_err();
    assert!(error.contains("finalize_uncertain"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(f
        .commit(&u, |_| panic!(
            "unknown final receipt is not permission to retry"
        ))
        .is_err());
    let state = f.store.activity_status(&f.db).unwrap();
    assert_eq!(state["busy"], false);
    assert_eq!(state["operationResultsRequireReview"], true);
    let owner = crate::asset_service::exposure_service::status_on(&f.db_path).unwrap();
    assert_eq!(owner["busy"], false);
}

#[test]
fn exposure_upload_changed_private_config_or_store_instance_rejects_old_receipt() {
    let f = Fixture::new();
    let u = f.begin("instance-scope-0001", &[vec![]]);
    f.db.execute(
        "UPDATE asset_service_store SET instance_id=?1",
        [uuid::Uuid::new_v4().to_string()],
    )
    .unwrap();
    assert!(f
        .call(
            "exposure_import_upload_status",
            json!({"uploadId":u["uploadId"]})
        )
        .unwrap_err()
        .contains("instance_changed"));
}

#[test]
fn exposure_upload_regrant_may_only_discard_receiving_bytes_not_continue_or_delete_commit_evidence()
{
    let f = Fixture::new();
    let bodies = vec![b"private receiving bytes".to_vec()];
    let u = f.begin("fresh-cleanup-0001", &bodies);
    f.put(&u, &bodies);
    let mut c = f.context.clone();
    c.grant_revision += 1;
    c.app_revision += 1;
    for (name, args) in [
        (
            "exposure_import_upload_status",
            json!({"uploadId":u["uploadId"]}),
        ),
        (
            "put_exposure_import_chunk",
            json!({"uploadId":u["uploadId"],"fileIndex":0,"chunkIndex":0,"contentBase64":base64::engine::general_purpose::STANDARD.encode(&bodies[0])}),
        ),
        ("commit_exposure_import_upload", Fixture::finalize(&u)),
    ] {
        assert!(f
            .store
            .dispatch_at(
                &f.db,
                9001,
                &c,
                name,
                args,
                1001,
                |_| Ok(()),
                |_| panic!("stale selection may not import")
            )
            .is_err());
    }
    let mut revoked = c.clone();
    revoked.permissions.clear();
    assert!(f
        .store
        .dispatch_at(
            &f.db,
            9001,
            &revoked,
            "discard_exposure_import_upload",
            Fixture::finalize(&u),
            1001,
            |_| Ok(()),
            |_| unreachable!()
        )
        .is_err());
    assert!(f
        .store
        .dispatch_at(
            &f.db,
            9002,
            &c,
            "discard_exposure_import_upload",
            Fixture::finalize(&u),
            1001,
            |_| Ok(()),
            |_| unreachable!()
        )
        .is_err());
    assert_eq!(
        f.store
            .dispatch_at(
                &f.db,
                9001,
                &c,
                "discard_exposure_import_upload",
                Fixture::finalize(&u),
                1001,
                |_| Ok(()),
                |_| unreachable!()
            )
            .unwrap()["businessEffects"],
        false
    );
    assert!(f
        .store
        .dispatch_at(
            &f.db,
            9001,
            &c,
            "discard_exposure_import_upload",
            Fixture::finalize(&u),
            1001,
            |_| Ok(()),
            |_| unreachable!()
        )
        .is_err());
    let claimed = f.begin("fresh-cleanup-claimed-0001", &[vec![]]);
    seed_sync_birth(&f, &claimed, "fresh-cleanup-claimed-0001");
    assert!(f
        .store
        .dispatch_at(
            &f.db,
            9001,
            &c,
            "discard_exposure_import_upload",
            Fixture::finalize(&claimed),
            1001,
            |_| Ok(()),
            |_| unreachable!()
        )
        .is_err());
}
