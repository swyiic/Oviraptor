//! Prepared outside the frozen source tree. Operator-supplied snapshots are
//! immutable references, never arbitrary filesystem roots or execution grants.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::{BTreeMap, BTreeSet};

const MAX_FILES: usize = 128;
const MAX_FILE_BYTES: usize = 128 * 1024;
const MAX_SOURCE_BYTES: usize = 512 * 1024;
const DIRECTORY: &str = "source-references-v1";
const SCHEMA: &str = r#"
CREATE TABLE scan_service_source_schema(version INTEGER PRIMARY KEY CHECK(version=1));
INSERT INTO scan_service_source_schema VALUES(1);
CREATE TABLE scan_service_sources(source_ref TEXT PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),instance_id TEXT NOT NULL REFERENCES scan_service_store(instance_id),operation_id TEXT NOT NULL,arguments_sha256 TEXT NOT NULL,label TEXT NOT NULL,tree_sha256 TEXT NOT NULL,files_json TEXT NOT NULL,file_count INTEGER NOT NULL,byte_count INTEGER NOT NULL,root_device TEXT NOT NULL,root_inode TEXT NOT NULL,receipt_json TEXT NOT NULL,UNIQUE(project_id,operation_id));
CREATE TRIGGER scan_service_source_schema_no_update BEFORE UPDATE ON scan_service_source_schema BEGIN SELECT RAISE(ABORT,'source schema immutable');END;
CREATE TRIGGER scan_service_source_schema_no_delete BEFORE DELETE ON scan_service_source_schema BEGIN SELECT RAISE(ABORT,'source schema immutable');END;
CREATE TRIGGER scan_service_sources_no_update BEFORE UPDATE ON scan_service_sources BEGIN SELECT RAISE(ABORT,'source reference immutable');END;
CREATE TRIGGER scan_service_sources_no_delete BEFORE DELETE ON scan_service_sources BEGIN SELECT RAISE(ABORT,'source reference immutable');END;
"#;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Import {
    label: String,
    files: Vec<InputFile>,
    expected_tree_sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputFile {
    path: String,
    content_base64: String,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileFact {
    path: String,
    sha256: String,
    bytes: u64,
}

pub(super) struct ValidatedImport {
    label: String,
    files: Vec<(FileFact, Vec<u8>)>,
    tree_sha256: String,
    arguments_sha256: String,
    bytes: usize,
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 512
        || path.chars().any(char::is_control)
        || path.contains(['\\', ':'])
        || path.split('/').count() > 16
        || path
            .split('/')
            .any(|p| p.is_empty() || matches!(p, "." | "..") || p.eq_ignore_ascii_case(".git"))
    {
        return Err("scan_source_path_invalid".into());
    }
    Ok(())
}

// SHA256(domain || each sorted file's u32be(path UTF8 length), path UTF8,
// u64be(content length), 32 raw hash bytes). No platform paths or timestamps.
fn tree_hash(files: &[FileFact]) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"oviraptor.source-snapshot.v1\0");
    for file in files {
        valid_path(&file.path)?;
        if !valid_hash(&file.sha256) {
            return Err("scan_source_hash_invalid".into());
        }
        digest.update((file.path.len() as u32).to_be_bytes());
        digest.update(file.path.as_bytes());
        digest.update(file.bytes.to_be_bytes());
        for pair in file.sha256.as_bytes().chunks_exact(2) {
            let hex = std::str::from_utf8(pair).map_err(|_| "scan_source_hash_invalid")?;
            digest.update([u8::from_str_radix(hex, 16).map_err(|_| "scan_source_hash_invalid")?]);
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

impl Import {
    pub(super) fn validate(self) -> Result<ValidatedImport, String> {
        valid_text(&self.label, 128, false)?;
        if self.files.is_empty()
            || self.files.len() > MAX_FILES
            || !valid_hash(&self.expected_tree_sha256)
        {
            return Err("scan_source_bounds_or_digest_invalid".into());
        }
        let mut paths = BTreeSet::new();
        let mut folded = BTreeSet::new();
        let mut bytes = 0usize;
        let mut files = Vec::with_capacity(self.files.len());
        for file in self.files {
            valid_path(&file.path)?;
            if !valid_hash(&file.sha256)
                || !paths.insert(file.path.clone())
                || !folded.insert(file.path.to_lowercase())
                || file.content_base64.len() > (MAX_FILE_BYTES.div_ceil(3) * 4)
            {
                return Err("scan_source_file_invalid".into());
            }
            let content = STANDARD
                .decode(&file.content_base64)
                .map_err(|_| "scan_source_base64_invalid")?;
            if content.len() > MAX_FILE_BYTES
                || STANDARD.encode(&content) != file.content_base64
                || format!("{:x}", Sha256::digest(&content)) != file.sha256
            {
                return Err("scan_source_file_digest_invalid".into());
            }
            bytes = bytes
                .checked_add(content.len())
                .ok_or("scan_source_bounds_or_digest_invalid")?;
            if bytes > MAX_SOURCE_BYTES {
                return Err("scan_source_bounds_or_digest_invalid".into());
            }
            files.push((
                FileFact {
                    path: file.path,
                    sha256: file.sha256,
                    bytes: content.len() as u64,
                },
                content,
            ));
        }
        // Case folding is deliberately conservative for this cross-platform
        // import format. Directory aliases and file-as-parent cannot overwrite.
        for path in &folded {
            let mut parent = path.as_str();
            while let Some((head, _)) = parent.rsplit_once('/') {
                if folded.contains(head) {
                    return Err("scan_source_file_parent_conflict".into());
                }
                parent = head;
            }
        }
        files.sort_by(|a, b| a.0.path.as_bytes().cmp(b.0.path.as_bytes()));
        let facts: Vec<_> = files.iter().map(|(fact, _)| fact.clone()).collect();
        let tree_sha256 = tree_hash(&facts)?;
        if tree_sha256 != self.expected_tree_sha256 {
            return Err("scan_source_tree_digest_invalid".into());
        }
        let label = self.label.trim().to_owned();
        let arguments_sha256 = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&json!([label, tree_sha256]))
                    .map_err(|_| "scan_source_arguments_invalid")?
            )
        );
        Ok(ValidatedImport {
            label,
            files,
            tree_sha256,
            arguments_sha256,
            bytes,
        })
    }
}

pub(super) fn enable(service: &Service) -> Result<(), String> {
    service.require_custody(&service.database)?;
    let tx =
        rusqlite::Transaction::new_unchecked(&service.database, TransactionBehavior::Immediate)
            .map_err(|_| "scan_source_schema_unavailable")?;
    service.require_custody(&tx)?;
    let objects:i64=tx.query_row("SELECT COUNT(*) FROM main.sqlite_schema WHERE name IN ('scan_service_source_schema','scan_service_sources','scan_service_source_schema_no_update','scan_service_source_schema_no_delete','scan_service_sources_no_update','scan_service_sources_no_delete')",[],|r|r.get(0)).map_err(|_| "scan_source_schema_unknown")?;
    if objects == 0 {
        tx.execute_batch(SCHEMA)
            .map_err(|_| "scan_source_schema_unavailable")?;
    } else if objects != 6 {
        return Err("scan_source_schema_partial".into());
    }
    let versions: Vec<i64> = tx
        .prepare("SELECT version FROM main.scan_service_source_schema")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(|_| "scan_source_schema_unknown")?;
    if versions != [1] {
        return Err("scan_source_schema_unknown".into());
    }
    // Verify exact immutable definitions, not merely six attacker-selected names.
    for statement in SCHEMA
        .split(';')
        .filter(|s| s.trim_start().starts_with("CREATE TABLE"))
    {
        let expected = statement.trim();
        let name = expected
            .split_whitespace()
            .nth(2)
            .ok_or("scan_source_schema_unknown")?
            .split('(')
            .next()
            .unwrap();
        let actual: String = tx
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .map_err(|_| "scan_source_schema_unknown")?;
        if actual != expected {
            return Err("scan_source_schema_unknown".into());
        }
    }
    // Triggers contain an inner semicolon; use complete original definitions.
    for line in SCHEMA.lines().filter(|s| s.starts_with("CREATE TRIGGER")) {
        let expected = line.trim_end_matches(';');
        let name = expected
            .split_whitespace()
            .nth(2)
            .ok_or("scan_source_schema_unknown")?;
        let actual: String = tx
            .query_row("SELECT sql FROM sqlite_schema WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .map_err(|_| "scan_source_schema_unknown")?;
        if actual != expected {
            return Err("scan_source_schema_unknown".into());
        }
    }
    service.require_custody(&tx)?;
    tx.commit()
        .map_err(|_| "scan_source_schema_commit_unconfirmed".to_string())
}

#[cfg(unix)]
mod filesystem {
    use super::*;
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    };

    fn name(value: &str) -> Result<std::ffi::CString, String> {
        if value.is_empty() || value.contains('/') || matches!(value, "." | "..") {
            return Err("scan_source_path_invalid".into());
        }
        std::ffi::CString::new(value).map_err(|_| "scan_source_path_invalid".into())
    }
    fn identity(file: &File) -> Result<(u64, u64), String> {
        let m = file
            .metadata()
            .map_err(|_| "scan_source_tree_unavailable")?;
        if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
            return Err("scan_source_directory_identity_invalid".into());
        }
        Ok((m.dev(), m.ino()))
    }
    fn open_directory(parent: &File, component: &str) -> Result<File, String> {
        let n = name(component)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                n.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err("scan_source_directory_identity_invalid".into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        identity(&file)?;
        Ok(file)
    }
    fn create_directory(parent: &File, component: &str) -> Result<File, String> {
        let n = name(component)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), n.as_ptr(), 0o700) } != 0 {
            return Err("scan_source_directory_create_failed".into());
        }
        let file = open_directory(parent, component)?;
        parent
            .sync_all()
            .map_err(|_| "scan_source_tree_sync_failed")?;
        Ok(file)
    }
    fn restrict_directory(file: &File) -> Result<(), String> {
        if unsafe { libc::fchmod(file.as_raw_fd(), 0o500) } != 0 {
            return Err("scan_source_tree_permissions_failed".into());
        }
        file.sync_all()
            .map_err(|_| "scan_source_tree_sync_failed".to_string())
    }

    pub(super) struct Tree {
        path: PathBuf,
        root: File,
        parent: File,
        reference: String,
        identity: (u64, u64),
    }
    impl Tree {
        fn parent(service: &Service, create: bool) -> Result<(PathBuf, File), String> {
            let path = service._environment.data_root().join(DIRECTORY);
            // Reuse the original no-follow directory-component implementation.
            let file = crate::scanning::agent_artifact_directory_chain(&path, create)
                .ok_or("scan_source_store_unavailable")?;
            identity(&file)?;
            Ok((path, file))
        }
        pub(super) fn create(
            service: &Service,
            reference: &str,
            files: &[(FileFact, Vec<u8>)],
        ) -> Result<Self, String> {
            let (parent_path, parent) = Self::parent(service, true)?;
            let root = create_directory(&parent, reference)?;
            let original = identity(&root)?;
            let mut directories = BTreeMap::new();
            for (fact, content) in files {
                let mut components = fact.path.split('/').collect::<Vec<_>>();
                let filename = components.pop().ok_or("scan_source_path_invalid")?;
                let mut current = root
                    .try_clone()
                    .map_err(|_| "scan_source_tree_unavailable")?;
                let mut prefix = String::new();
                for component in components {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(component);
                    if let Some(held) = directories.get(&prefix) {
                        current =
                            File::try_clone(held).map_err(|_| "scan_source_tree_unavailable")?;
                    } else {
                        current = create_directory(&current, component)?;
                        directories.insert(
                            prefix.clone(),
                            current
                                .try_clone()
                                .map_err(|_| "scan_source_tree_unavailable")?,
                        );
                    }
                }
                let n = name(filename)?;
                let fd = unsafe {
                    libc::openat(
                        current.as_raw_fd(),
                        n.as_ptr(),
                        libc::O_WRONLY
                            | libc::O_CREAT
                            | libc::O_EXCL
                            | libc::O_NOFOLLOW
                            | libc::O_CLOEXEC,
                        0o400,
                    )
                };
                if fd < 0 {
                    return Err("scan_source_file_create_failed".into());
                }
                let mut file = unsafe { File::from_raw_fd(fd) };
                file.write_all(content)
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "scan_source_file_write_failed")?;
                current
                    .sync_all()
                    .map_err(|_| "scan_source_tree_sync_failed")?;
            }
            for (_, file) in directories.iter().rev() {
                restrict_directory(file)?;
            }
            restrict_directory(&root)?;
            parent
                .sync_all()
                .map_err(|_| "scan_source_tree_sync_failed")?;
            let tree = Self {
                path: parent_path.join(reference),
                root,
                parent,
                reference: reference.into(),
                identity: original,
            };
            tree.require_identity()?;
            Ok(tree)
        }
        pub(super) fn open(
            service: &Service,
            reference: &str,
            expected: (u64, u64),
        ) -> Result<Self, String> {
            let (parent_path, parent) = Self::parent(service, false)?;
            let root = open_directory(&parent, reference)?;
            let tree = Self {
                path: parent_path.join(reference),
                root,
                parent,
                reference: reference.into(),
                identity: expected,
            };
            tree.require_identity()?;
            Ok(tree)
        }
        fn require_identity(&self) -> Result<(), String> {
            if identity(&self.root)? != self.identity
                || identity(&open_directory(&self.parent, &self.reference)?)? != self.identity
            {
                return Err("scan_source_tree_replaced".into());
            }
            let named = crate::scanning::agent_artifact_directory_chain(&self.path, false)
                .ok_or("scan_source_tree_replaced")?;
            if identity(&named)? != self.identity {
                return Err("scan_source_tree_replaced".into());
            }
            Ok(())
        }
        pub(super) fn facts(&self) -> Result<Vec<FileFact>, String> {
            self.require_identity()?;
            let mut facts = Vec::new();
            let mut directories = BTreeSet::new();
            let mut entries = 0;
            walk(&self.root, "", &mut facts, &mut directories, &mut entries)?;
            facts.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
            let mut expected_directories = BTreeSet::new();
            for fact in &facts {
                let mut parent = fact.path.as_str();
                while let Some((head, _)) = parent.rsplit_once('/') {
                    expected_directories.insert(head.to_owned());
                    parent = head;
                }
            }
            if directories != expected_directories {
                return Err("scan_source_tree_unlisted_directory".into());
            }
            self.require_identity()?;
            Ok(facts)
        }
        pub(super) fn path(&self) -> &Path {
            &self.path
        }
        pub(super) fn identity(&self) -> (u64, u64) {
            self.identity
        }
    }

    struct Entries(*mut libc::DIR);
    impl Drop for Entries {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    fn clear_errno() {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        unsafe {
            *libc::__errno_location() = 0;
        }
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        unsafe {
            *libc::__error() = 0;
        }
    }
    fn children(directory: &File) -> Result<Vec<String>, String> {
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err("scan_source_tree_enumeration_failed".into());
        }
        let ptr = unsafe { libc::fdopendir(fd) };
        if ptr.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err("scan_source_tree_enumeration_failed".into());
        }
        let entries = Entries(ptr);
        let mut names = Vec::new();
        loop {
            clear_errno();
            let entry = unsafe { libc::readdir(entries.0) };
            if entry.is_null() {
                if io::Error::last_os_error().raw_os_error() != Some(0) {
                    return Err("scan_source_tree_enumeration_failed".into());
                }
                break;
            }
            let bytes = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let value = std::str::from_utf8(bytes).map_err(|_| "scan_source_tree_name_invalid")?;
            names.push(value.to_owned());
            if names.len() > MAX_FILES * 17 {
                return Err("scan_source_tree_bounds_invalid".into());
            }
        }
        names.sort();
        Ok(names)
    }
    fn walk(
        directory: &File,
        prefix: &str,
        facts: &mut Vec<FileFact>,
        directories: &mut BTreeSet<String>,
        entries: &mut usize,
    ) -> Result<(), String> {
        let metadata = directory
            .metadata()
            .map_err(|_| "scan_source_tree_unavailable")?;
        identity(directory)?;
        if metadata.mode() & 0o777 != 0o500 {
            return Err("scan_source_tree_permissions_changed".into());
        }
        for child in children(directory)? {
            *entries += 1;
            if *entries > MAX_FILES * 17 {
                return Err("scan_source_tree_bounds_invalid".into());
            }
            let path = if prefix.is_empty() {
                child.clone()
            } else {
                format!("{prefix}/{child}")
            };
            valid_path(&path)?;
            let n = name(&child)?;
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    n.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if fd < 0 {
                return Err("scan_source_tree_entry_invalid".into());
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            let metadata = file
                .metadata()
                .map_err(|_| "scan_source_tree_entry_invalid")?;
            if metadata.is_dir() {
                directories.insert(path.clone());
                walk(&file, &path, facts, directories, entries)?;
            } else {
                if !metadata.is_file()
                    || metadata.nlink() != 1
                    || metadata.uid() != unsafe { libc::geteuid() }
                    || metadata.mode() & 0o777 != 0o400
                    || metadata.len() > MAX_FILE_BYTES as u64
                    || facts.len() >= MAX_FILES
                {
                    return Err("scan_source_tree_entry_invalid".into());
                }
                let mut content = Vec::new();
                (&mut file)
                    .take(MAX_FILE_BYTES as u64 + 1)
                    .read_to_end(&mut content)
                    .map_err(|_| "scan_source_tree_entry_invalid")?;
                if content.len() as u64 != metadata.len() {
                    return Err("scan_source_tree_entry_changed".into());
                }
                facts.push(FileFact {
                    path,
                    sha256: format!("{:x}", Sha256::digest(&content)),
                    bytes: content.len() as u64,
                });
                if facts.iter().map(|f| f.bytes).sum::<u64>() > MAX_SOURCE_BYTES as u64 {
                    return Err("scan_source_tree_bounds_invalid".into());
                }
            }
        }
        Ok(())
    }
}

// Only the operator ProjectAction is wired to this entry. It is deliberately
// absent from model tools; files and expected digest are validated before SQL.
#[cfg(unix)]
pub(super) fn import(service: &Service, context: &Context, input: Import) -> Result<Value, String> {
    service.authorize(context, "source.import")?;
    let validated = input.validate()?;
    service.require_custody(&service.database)?;
    let tx =
        rusqlite::Transaction::new_unchecked(&service.database, TransactionBehavior::Immediate)
            .map_err(|_| "scan_source_publication_unavailable")?;
    service.require_custody(&tx)?;
    let old:Option<(String,String)>=tx.query_row("SELECT arguments_sha256,source_ref FROM scan_service_sources WHERE project_id=?1 AND operation_id=?2",params![context.host_bound_project_id,context.operation_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_| "scan_source_lookup_failed")?;
    if let Some((old_hash, reference)) = old {
        if old_hash != validated.arguments_sha256 {
            return Err("scan_source_operation_conflict".into());
        }
        let verified = resolve_on(service, &tx, context, &reference)?;
        if verified.receipt["treeSha256"] != validated.tree_sha256
            || verified.receipt["files"].as_u64() != Some(validated.files.len() as u64)
            || verified.receipt["bytes"].as_u64() != Some(validated.bytes as u64)
        {
            return Err("scan_source_operation_receipt_changed".into());
        }
        service.require_custody(&tx)?;
        tx.commit()
            .map_err(|_| "scan_source_read_commit_unconfirmed")?;
        return Ok(verified.receipt);
    }
    let reference = uuid::Uuid::new_v4().to_string();
    let tree = filesystem::Tree::create(service, &reference, &validated.files)?;
    let expected: Vec<_> = validated.files.iter().map(|(f, _)| f.clone()).collect();
    if tree.facts()? != expected {
        return Err("scan_source_tree_changed".into());
    }
    let receipt = json!({"sourceRef":reference,"treeSha256":validated.tree_sha256,"files":expected.len(),"bytes":validated.bytes,"hostBoundProjectId":context.host_bound_project_id,"immutable":true,"executionGranted":false});
    let file_count =
        i64::try_from(expected.len()).map_err(|_| "scan_source_bounds_or_digest_invalid")?;
    let byte_count =
        i64::try_from(validated.bytes).map_err(|_| "scan_source_bounds_or_digest_invalid")?;
    let changed = tx
        .execute(
            "INSERT INTO scan_service_sources VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                reference,
                context.host_bound_project_id,
                service.instance_id,
                context.operation_id,
                validated.arguments_sha256,
                validated.label,
                validated.tree_sha256,
                serde_json::to_string(&expected).map_err(|_| "scan_source_arguments_invalid")?,
                file_count,
                byte_count,
                tree.identity().0.to_string(),
                tree.identity().1.to_string(),
                receipt.to_string()
            ],
        )
        .map_err(|_| "scan_source_publication_conflict")?;
    let confirmed = resolve_on(service, &tx, context, &reference)?;
    if changed != 1 || confirmed.receipt != receipt {
        return Err("scan_source_publication_unconfirmed".into());
    }
    service.require_custody(&tx)?;
    // No cleanup on ambiguous commit: files could already be referenced by an
    // accepted commit. Partial/uncommitted trees are never automatically adopted.
    tx.commit()
        .map_err(|_| "scan_source_publication_commit_unconfirmed")?;
    Ok(receipt)
}

#[cfg(unix)]
pub(super) struct VerifiedSource {
    tree: filesystem::Tree,
    pub(super) receipt: Value,
}
#[cfg(unix)]
impl VerifiedSource {
    pub(super) fn path(&self) -> &Path {
        self.tree.path()
    }
}

#[cfg(unix)]
pub(super) fn resolve_on(
    service: &Service,
    db: &Connection,
    context: &Context,
    reference: &str,
) -> Result<VerifiedSource, String> {
    service.require_custody(db)?;
    if !uuid::Uuid::parse_str(reference).is_ok_and(|v| v.to_string() == reference) {
        return Err("scan_source_reference_invalid".into());
    }
    let row:(String,String,String,i64,i64,String,String,String)=db.query_row("SELECT tree_sha256,files_json,receipt_json,file_count,byte_count,root_device,root_inode,label FROM scan_service_sources WHERE source_ref=?1 AND project_id=?2 AND instance_id=?3",params![reference,context.host_bound_project_id,service.instance_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(|_| "scan_source_reference_scope_unavailable")?;
    valid_text(&row.7, 128, false)?;
    let expected: Vec<FileFact> =
        serde_json::from_str(&row.1).map_err(|_| "scan_source_reference_invalid")?;
    if expected.is_empty()
        || expected.len() > MAX_FILES
        || row.3 != expected.len() as i64
        || row.4 < 0
        || row.4 > MAX_SOURCE_BYTES as i64
        || expected.iter().any(|f| f.bytes > MAX_FILE_BYTES as u64)
        || row.4 as u64 != expected.iter().map(|f| f.bytes).sum::<u64>()
        || tree_hash(&expected)? != row.0
    {
        return Err("scan_source_reference_invalid".into());
    }
    let tree = filesystem::Tree::open(
        service,
        reference,
        (
            row.5.parse().map_err(|_| "scan_source_reference_invalid")?,
            row.6.parse().map_err(|_| "scan_source_reference_invalid")?,
        ),
    )?;
    if tree.facts()? != expected {
        return Err("scan_source_tree_changed".into());
    }
    let receipt: Value = serde_json::from_str(&row.2).map_err(|_| "scan_source_receipt_invalid")?;
    if receipt
        != json!({"sourceRef":reference,"treeSha256":row.0,"files":row.3,"bytes":row.4,"hostBoundProjectId":context.host_bound_project_id,"immutable":true,"executionGranted":false})
    {
        return Err("scan_source_receipt_invalid".into());
    }
    service.require_custody(db)?;
    Ok(VerifiedSource { tree, receipt })
}

#[cfg(test)]
#[path = "native_service_sources_tests.rs"]
mod tests;
