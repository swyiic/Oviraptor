// A private, versioned commitment to a Web startup's inputs. This is NOT an
// authorization to replay. A recovery command must also own the lifecycle and
// branch locks, revalidate scope/fuses/leases, and durably claim before effects.
const WEB_DISPATCH_KEY_FILE: &str = ".web-dispatch-key";
const WEB_DISPATCH_BINDING_LIMIT: u64 = 16 * 1024 * 1024;

// Ordinary Web launches carry their frozen settings/config into the worker.
// Combined/source startup has not yet adopted this binding contract.
struct WebDispatchAdmission {
    home: PathBuf,
    settings: JsonValue,
    recon_config: FrontendReconConfig,
    // Recovery transfers an already committed claim; the worker must not claim
    // it a second time or drop ownership between admission and dispatch.
    claimed_guard: Option<NativeBranchGuard>,
}

fn verify_live_web_dispatch_binding_in(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
    work_dir: &Path, worker: &Path, home: &Path,
) -> Result<(), String> {
    let settings = sentinel_settings(connection);
    let model = model_runtime_env(&settings)?;
    let runtime = resolve_agent_web_pipeline_runtime(connection,scan_id,&model.deployment,worker.into())?;
    let path = sentinel_runtime_path(home);
    let descriptor = web_dispatch_runtime_binding(connection,&runtime,&model,&path)?;
    verify_web_dispatch_binding_in(connection,scan_id,attempt,&WebBindingDirectory::open(work_dir)?,&descriptor)
}

struct WebBindingDirectory {
    path: PathBuf,
    #[cfg(unix)]
    handle: File,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl WebBindingDirectory {
    fn open(path: &Path) -> Result<Self, String> {
        if fs::canonicalize(path).map_err(|_| "web_binding_directory_missing")? != path {
            return Err("web_binding_directory_not_canonical".into());
        }
        let metadata = fs::symlink_metadata(path).map_err(|_| "web_binding_directory_missing")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("web_binding_directory_unsafe".into());
        }
        #[cfg(unix)] {
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
            let handle = OpenOptions::new().read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path).map_err(|_| "web_binding_directory_unsafe")?;
            let opened = handle.metadata().map_err(|_| "web_binding_directory_unsafe")?;
            if (metadata.dev(), metadata.ino()) != (opened.dev(), opened.ino())
                || opened.permissions().mode() & 0o077 != 0 {
                return Err("web_binding_directory_unsafe".into());
            }
            Ok(Self { path: path.into(), handle, identity: (opened.dev(), opened.ino()) })
        }
        #[cfg(not(unix))]
        Ok(Self { path: path.into() })
    }

    fn check_identity(&self) -> Result<(), String> {
        if fs::canonicalize(&self.path).map_err(|_| "web_binding_directory_changed")? != self.path {
            return Err("web_binding_directory_changed".into());
        }
        let metadata = fs::symlink_metadata(&self.path).map_err(|_| "web_binding_directory_changed")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("web_binding_directory_changed".into());
        }
        #[cfg(unix)] {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if (metadata.dev(), metadata.ino()) != self.identity || metadata.permissions().mode() & 0o077 != 0 {
                return Err("web_binding_directory_changed".into());
            }
        }
        Ok(())
    }

    // All names are compile-time constants below. openat binds reads to the
    // directory descriptor, not to a path an attacker can swap between reads.
    fn open_file(&self, name: &str, create: bool) -> Result<File, std::io::Error> {
        if name.is_empty() || name.contains(['/', '\\']) || matches!(name, "." | "..") {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        #[cfg(unix)] {
            use std::os::fd::{AsRawFd, FromRawFd};
            let name = std::ffi::CString::new(name).map_err(|_| std::io::ErrorKind::InvalidInput)?;
            let flags = libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC
                | if create { libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL } else { libc::O_RDONLY };
            // SAFETY: name is NUL-terminated; the live directory owns its fd.
            let fd = unsafe { libc::openat(self.handle.as_raw_fd(), name.as_ptr(), flags, 0o600) };
            if fd < 0 { return Err(std::io::Error::last_os_error()); }
            // SAFETY: openat returned a new, exclusively owned file descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
        #[cfg(not(unix))] {
            let mut options = OpenOptions::new();
            options.read(!create).write(create).create_new(create);
            #[cfg(windows)] {
                use std::os::windows::fs::OpenOptionsExt;
                options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
            }
            options.open(self.path.join(name))
        }
    }

    fn read(&self, name: &str, limit: u64, optional: bool, sync: bool) -> Result<Option<Vec<u8>>, String> {
        let mut file = match self.open_file(name, false) {
            Ok(file) => file,
            Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("web_binding_file_unavailable".into()),
        };
        let metadata = file.metadata().map_err(|_| "web_binding_file_unavailable")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
            return Err("web_binding_file_unsafe_or_oversized".into());
        }
        #[cfg(unix)] {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.nlink() != 1
                || (matches!(name, WEB_DISPATCH_KEY_FILE | SOURCE_RUNTIME_KEY_FILE) && metadata.permissions().mode() & 0o077 != 0) {
                return Err("web_binding_file_unsafe".into());
            }
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file).take(limit + 1).read_to_end(&mut bytes)
            .map_err(|_| "web_binding_file_unavailable")?;
        if bytes.len() as u64 > limit { return Err("web_binding_file_oversized".into()); }
        if sync { file.sync_all().map_err(|_| "web_binding_file_sync_failed")?; }
        Ok(Some(bytes))
    }

    fn sync(&self) -> Result<(), String> {
        self.check_identity()?;
        #[cfg(unix)] {
            self.handle.sync_all().map_err(|_| "web_binding_directory_sync_failed")?;
            // The attempt and its two freshly allocated parent directories must
            // survive alongside the SQLite commit, not just the files' bytes.
            for parent in self.path.ancestors().skip(1).take(3) {
                File::open(parent).and_then(|file| file.sync_all())
                    .map_err(|_| "web_binding_parent_sync_failed")?;
            }
        }
        Ok(())
    }
}

fn web_dispatch_auth_document(
    connection: &rusqlite::Connection, project_id: i64, policy: &JsonValue,
) -> Result<Option<JsonValue>, String> {
    let mut ids = investigation_strings(policy.get("authSessionIds"));
    if let Some(id) = policy.get("authSessionId").and_then(JsonValue::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        ids.push(id.to_string());
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() { return Ok(None); }
    // Rechecks ownership, expiry, validity and distinct identity material. A
    // frozen auth JSON file alone is never proof the session remains usable.
    let mut documents = crate::auth_session::distinct_session_documents_for_scan(connection, &ids, project_id)?;
    Ok(Some(if documents.len() == 1 { documents.remove(0) } else {
        serde_json::json!({"schemaVersion":2,"kind":"identity-matrix","sessions":documents,
            "comparisonPolicy":"same-target-same-action-plan",
            "identityIsolation":"dedicated-webview-and-distinct-auth-material"})
    }))
}

// Values here can contain credentials. Keep them in memory only: never log,
// return through IPC, derive Debug, or persist this descriptor as plain JSON.
fn web_dispatch_runtime_binding(
    connection: &rusqlite::Connection, runtime: &AgentWebPipelineRuntime,
    model: &ModelRuntimeEnv, runtime_path: &std::ffi::OsStr,
) -> Result<JsonValue, String> {
    verify_bundled_worker(&runtime.worker)?;
    let mut executable = File::open(std::env::current_exe().map_err(|_| "web_binding_build_unavailable")?)
        .map_err(|_| "web_binding_build_unavailable")?;
    let mut build_hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = executable.read(&mut buffer).map_err(|_| "web_binding_build_unavailable")?;
        if size == 0 { break; }
        build_hash.update(&buffer[..size]);
    }
    let mut environment = serde_json::Map::new();
    for name in ["OVIRAPTOR_NODE_EXECUTABLE", "OVIRAPTOR_BROWSER_EXECUTABLE", "OVIRAPTOR_CDP_TRANSPORT", "NODE_OPTIONS", "NODE_PATH",
        "HOME", "PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA",
        "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY", "http_proxy", "https_proxy", "all_proxy", "no_proxy",
        "NODE_EXTRA_CA_CERTS", "NODE_TLS_REJECT_UNAUTHORIZED", "SSL_CERT_FILE", "SSL_CERT_DIR",
        "LD_PRELOAD", "LD_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH"] {
        let value = std::env::var_os(name).map(|value| value.into_string().map_err(|_| "web_binding_environment_not_utf8")).transpose()?;
        environment.insert(name.into(), serde_json::json!(value));
    }
    Ok(serde_json::json!({"settings":sentinel_settings(connection),"runtime":runtime,"model":model,
        "runtimePath":runtime_path.to_str().ok_or("web_binding_path_not_utf8")?,
        "frontendConfig":frontend_recon_config(&runtime.worker),"environment":environment,
        "toolchain":web_toolchain_binding(runtime_path)?,
        "buildSha256":format!("{:x}",build_hash.finalize()),"version":env!("CARGO_PKG_VERSION")}))
}

fn web_binding_database_snapshot(connection: &rusqlite::Connection, scan_id: &str) -> Result<JsonValue, String> {
    let mut groups = Vec::new();
    // Bind execution inputs, not mutable UI summaries or updated_at timestamps.
    // A harmless status read/description edit must not invalidate admission.
    // New execution-affecting fields require an explicit binding-schema review.
    for sql in [
        "SELECT id,project_id,project_name,scan_type,source_path,task_name,task_path,skill_names,attempt_count,status FROM sentinel_scans WHERE id=?1",
        "SELECT scan_id,environment,auth_profile_name,auth_type,authenticated,policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
        "SELECT p.id,p.name,p.status FROM projects p JOIN sentinel_scans s ON s.project_id=p.id WHERE s.id=?1",
        "SELECT a.scan_id,a.attempt_number,a.execution_mode,a.status,a.work_dir,a.backend_plan_json FROM sentinel_scan_attempts a JOIN sentinel_scans s ON s.id=a.scan_id AND s.attempt_count=a.attempt_number WHERE s.id=?1",
        "SELECT id,project_id,scan_id,company,url,status,last_attempt_number FROM sentinel_targets WHERE scan_id=?1 ORDER BY id",
    ] {
        let mut statement = connection.prepare(sql).map_err(|_| "web_binding_snapshot_unavailable")?;
        let names = statement.column_names().iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let rows = statement.query_map([scan_id], |row| {
            (0..names.len()).map(|index| row.get::<_,SqlValue>(index).map(|value| match value {
                SqlValue::Null => JsonValue::Null,
                SqlValue::Integer(value) => serde_json::json!(value),
                SqlValue::Real(value) => serde_json::json!(value),
                SqlValue::Text(value) => serde_json::json!(value),
                SqlValue::Blob(value) => serde_json::json!({"blob":BASE64_STANDARD.encode(value)}),
            })).collect::<Result<Vec<_>,_>>()
        }).map_err(|_| "web_binding_snapshot_unavailable")?
            .collect::<Result<Vec<_>,_>>().map_err(|_| "web_binding_snapshot_unavailable")?;
        if rows.is_empty() { return Err("web_binding_snapshot_missing".into()); }
        groups.push(serde_json::json!({"columns":names,"rows":rows}));
    }
    Ok(serde_json::json!(groups))
}

fn web_binding_message(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
    directory: &WebBindingDirectory, runtime_binding: &JsonValue, sync: bool,
) -> Result<Vec<u8>, String> {
    if connection.is_autocommit() { return Err("web_binding_requires_transaction".into()); }
    if !native_dispatch_attempt_eligible(connection,scan_id,attempt,"web")? {
        return Err("web_binding_attempt_ineligible".into());
    }
    // A target can be fused after startup publication but before the worker's
    // claim. Recheck the live exclusion policy, including after claim triggers;
    // do not treat an unchanged settings/file commitment as current authority.
    let fused: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_targets t JOIN sentinel_fuse_zone f
         ON f.project_id=t.project_id AND f.normalized_url=lower(rtrim(trim(t.url),'/'))
         WHERE t.scan_id=?1 AND t.last_attempt_number=?2 AND f.archived=0)",
        params![scan_id,attempt], |row| row.get(0),
    ).map_err(|_| "web_binding_fuse_check_unavailable")?;
    if fused { return Err("web_binding_target_fused".into()); }
    // Project ownership alone is not enough: a session can be reassigned to
    // another task without changing its JSON or credentials. Revalidate each
    // current target's task-bound identity and captured host before dispatch.
    let mut targets = connection.prepare(
        "SELECT url FROM sentinel_targets WHERE scan_id=?1 AND last_attempt_number=?2 ORDER BY id",
    ).map_err(|_| "web_binding_identity_targets_unavailable")?;
    let urls = targets.query_map(params![scan_id,attempt],|row|row.get::<_,String>(0))
        .map_err(|_| "web_binding_identity_targets_unavailable")?
        .collect::<Result<Vec<_>,_>>().map_err(|_| "web_binding_identity_targets_unavailable")?;
    if urls.is_empty() { return Err("web_binding_identity_targets_missing".into()); }
    for url in urls {
        crate::auth_session::validated_scan_identities(connection,scan_id,&url)
            .map_err(|_| "web_binding_task_identity_scope_invalid")?;
    }
    let (project_id, policy): (i64,String) = connection.query_row(
        "SELECT s.project_id,c.policy_json FROM sentinel_scans s JOIN sentinel_scan_contexts c ON c.scan_id=s.id
         JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=s.attempt_count
         JOIN native_branch_dispatches d ON d.scan_id=s.id AND d.attempt_number=s.attempt_count AND d.branch='web'
         WHERE s.id=?1 AND s.attempt_count=?2 AND s.scan_type='web' AND a.status='scanning'
         AND a.work_dir=?3 AND s.task_path=?4",
        params![scan_id,attempt,directory.path.to_str().ok_or("web_binding_path_not_utf8")?,
            directory.path.join("task.json").to_str().ok_or("web_binding_path_not_utf8")?],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).map_err(|_| "web_binding_attempt_receipt_missing")?;
    let policy: JsonValue = serde_json::from_str(&policy).map_err(|_| "web_binding_policy_invalid")?;
    let auth = web_dispatch_auth_document(connection,project_id,&policy)?;
    let mut artifacts = Vec::new();
    for name in [".oviraptor-scan-id","task.json","targets.json","targets.txt","agent-instruction.md",
        "model-prompt-audit.json","auth-sessions.json"] {
        let bytes = directory.read(name,WEB_DISPATCH_BINDING_LIMIT,
            name == "model-prompt-audit.json" || (name == "auth-sessions.json" && auth.is_none()),sync)?;
        if name == ".oviraptor-scan-id" && bytes.as_deref() != Some(scan_id.as_bytes()) {
            return Err("web_binding_marker_mismatch".into());
        }
        if name == "auth-sessions.json" {
            let actual = bytes.as_ref().map(|raw| serde_json::from_slice::<JsonValue>(raw)
                .map_err(|_| "web_binding_auth_invalid")).transpose()?;
            if actual != auth { return Err("web_binding_auth_changed".into()); }
        }
        artifacts.push(serde_json::json!({"name":name,"sha256":bytes.map(|b|format!("{:x}",Sha256::digest(b)))}));
    }
    directory.check_identity()?;
    #[cfg(unix)]
    let directory_identity = serde_json::json!(directory.identity);
    #[cfg(not(unix))]
    let directory_identity = JsonValue::Null;
    let message = serde_json::to_vec(&serde_json::json!({
        "domain":"oviraptor.web-dispatch-binding.v1","scanId":scan_id,"attempt":attempt,
        "workDir":directory.path,"directoryIdentity":directory_identity,
        "database":web_binding_database_snapshot(connection,scan_id)?,
        "runtime":runtime_binding,"settings":sentinel_settings(connection),"artifacts":artifacts,
    })).map_err(|_| "web_binding_serialization_failed")?;
    if message.len() as u64 > WEB_DISPATCH_BINDING_LIMIT { return Err("web_binding_snapshot_oversized".into()); }
    Ok(message)
}

fn verify_web_dispatch_binding_in(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64,
    directory: &WebBindingDirectory, runtime_binding: &JsonValue,
) -> Result<(), String> {
    let (schema,tag): (i64,Vec<u8>) = connection.query_row(
        "SELECT schema_version,binding_tag FROM native_web_dispatch_bindings WHERE scan_id=?1 AND attempt_number=?2 AND branch='web'",
        params![scan_id,attempt], |row| Ok((row.get(0)?,row.get(1)?)),
    ).map_err(|_| "web_binding_receipt_missing")?;
    if schema != 1 || tag.len() != 32 { return Err("web_binding_receipt_invalid".into()); }
    // Never create/rotate a key during verification; a lost key is not proof
    // that previously frozen configuration can be safely substituted.
    let key = directory.read(WEB_DISPATCH_KEY_FILE,32,false,false)?.ok_or("web_binding_key_missing")?;
    if key.len() != 32 { return Err("web_binding_key_invalid".into()); }
    let message = web_binding_message(connection,scan_id,attempt,directory,runtime_binding,false)?;
    aws_lc_rs::hmac::verify(&aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256,&key),&message,&tag)
        .map_err(|_| "web_binding_inputs_changed".into())
}

fn register_web_dispatch_binding_in(
    connection: &rusqlite::Connection, startup: &WebStartup, runtime_binding: &JsonValue,
) -> Result<(), String> {
    let directory = WebBindingDirectory::open(&startup.files.path)?;
    let attempt = i64::from(startup.attempt);
    let message = web_binding_message(connection,&startup.scan_id,attempt,&directory,runtime_binding,true)?;
    let mut key = [0u8;32];
    aws_lc_rs::rand::fill(&mut key).map_err(|_| "web_binding_entropy_unavailable")?;
    let mut file = directory.open_file(WEB_DISPATCH_KEY_FILE,true).map_err(|_| "web_binding_key_create_failed")?;
    file.write_all(&key).and_then(|()| file.sync_all()).map_err(|_| "web_binding_key_sync_failed")?;
    directory.sync()?;
    let tag = aws_lc_rs::hmac::sign(&aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256,&key),&message);
    let changed = connection.execute(
        "INSERT INTO native_web_dispatch_bindings(scan_id,attempt_number,schema_version,binding_tag) VALUES(?1,?2,1,?3)",
        params![startup.scan_id,attempt,tag.as_ref()],
    ).map_err(|_| "web_binding_registration_failed")?;
    if changed != 1 { return Err("web_binding_registration_not_persisted".into()); }
    // Recompute after every write/trigger. IGNORE, deleted receipts, forged
    // tags, or any trigger changing the task/target/policy roll back startup.
    verify_web_dispatch_binding_in(connection,&startup.scan_id,attempt,&directory,runtime_binding)?;
    let unclaimed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=?2 AND branch='web' AND claim_id='' AND claimed_at='')",
        params![startup.scan_id,attempt], |row| row.get(0),
    ).map_err(|_| "web_binding_dispatch_receipt_unavailable")?;
    if !unclaimed { return Err("web_binding_dispatch_already_claimed".into()); }
    Ok(())
}
