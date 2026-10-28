//! Current approved cache bytes, held by their original filesystem entities.
//! Approval currently stores no package: absent sources fail, without staging,
//! repair, extraction, execution, or accepting a caller-supplied path.
use super::{
    approval, candidate_cache_key, ToolCapabilityManifest, MAX_MANIFEST_BYTES,
    MAX_OPAQUE_PACKAGE_BYTES, MAX_STREAMED_PACKAGE_BYTES,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub(crate) struct ApprovedPackageSource {
    approval_id: String,
    approval_hash: String,
    manifest: ToolCapabilityManifest,
    canonical_manifest: String,
    configuration_digest: String,
    directories: Vec<HeldDirectory>,
    manifest_file: File,
    manifest_identity: FileIdentity,
    package_file: File,
    package_identity: FileIdentity,
}
struct HeldDirectory {
    path: PathBuf,
    file: File,
    identity: FileIdentity,
    private: bool,
}
#[derive(Clone, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    mode: u32,
    links: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
#[cfg(unix)]
fn identity(metadata: &fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
        mode: metadata.mode(),
        links: metadata.nlink(),
        length: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    }
}
#[cfg(not(unix))]
fn identity(_: &fs::Metadata) -> FileIdentity {
    unreachable!("non-Unix admission always fails")
}
fn check(metadata: &fs::Metadata, directory: bool, private: bool) -> Result<(), String> {
    #[cfg(not(unix))]
    {
        let _ = (metadata, directory, private);
        return Err("unsupported_approved_package_source_platform".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & if private { 0o077 } else { 0o022 } != 0
            || metadata.mode() & if directory { 0o500 } else { 0o400 }
                != if directory { 0o500 } else { 0o400 }
            || if directory {
                !metadata.is_dir()
            } else {
                !metadata.is_file() || metadata.nlink() != 1
            }
        {
            return Err("approved_package_source_permissions".into());
        }
        Ok(())
    }
}
fn require_no_symlink_ancestors(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("approved_package_source_directory_invalid".into());
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::RootDir | std::path::Component::Normal(_) => {
                prefix.push(component.as_os_str())
            }
            _ => return Err("approved_package_source_directory_invalid".into()),
        }
        let metadata =
            fs::symlink_metadata(&prefix).map_err(|error| match error.raw_os_error() {
                Some(libc::ENOENT) => {
                    "approved_package_source_configuration_directory_missing".into()
                }
                Some(code) => format!("approved_package_source_open_failed:{code}"),
                None => "approved_package_source_open_failed:unknown".into(),
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("approved_package_source_directory_changed".into());
        }
    }
    Ok(())
}
fn open_base(path: &Path) -> Result<File, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| match error.raw_os_error() {
                Some(libc::ENOENT) => {
                    "approved_package_source_configuration_directory_missing".into()
                }
                Some(code) => format!("approved_package_source_open_failed:{code}"),
                None => "approved_package_source_open_failed:unknown".into(),
            })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("unsupported_approved_package_source_platform".into())
    }
}
fn open_child(parent: &File, name: &str, directory: bool) -> Result<File, String> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let name =
            std::ffi::CString::new(name).map_err(|_| "approved_package_source_name_invalid")?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if directory { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOENT) => "approved_package_source_missing".into(),
                Some(code) => format!("approved_package_source_open_failed:{code}"),
                None => "approved_package_source_open_failed:unknown".into(),
            });
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    #[cfg(not(unix))]
    {
        let _ = (parent, name, directory);
        Err("unsupported_approved_package_source_platform".into())
    }
}
fn same_directory(a: &FileIdentity, b: &FileIdentity) -> bool {
    a.device == b.device && a.inode == b.inode && a.owner == b.owner && a.mode == b.mode
}
fn read_manifest(
    file: &mut File,
    configuration: &approval::ToolAdministratorConfiguration,
    expected: &ToolCapabilityManifest,
) -> Result<(String, String), String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "approved_package_source_read_failed")?;
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "approved_package_source_read_failed")?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err("approved_package_source_manifest_too_large".into());
    }
    let value: super::SignedToolManifest =
        serde_json::from_slice(&bytes).map_err(|_| "approved_package_source_manifest_invalid")?;
    let (verified, digest) =
        approval::verify_package_manifest_on(configuration, &bytes, &expected.platform)?;
    if &verified != expected {
        return Err("approved_package_source_manifest_changed".into());
    }
    Ok((
        serde_json::to_string(&value).map_err(|_| "approved_package_source_manifest_invalid")?,
        digest,
    ))
}
fn package_hash(file: &mut File) -> Result<(String, u64), String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "approved_package_source_read_failed")?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = file
            .read(&mut buffer)
            .map_err(|_| "approved_package_source_read_failed")?;
        if size == 0 {
            break;
        }
        total = total
            .checked_add(size as u64)
            .ok_or("approved_package_source_too_large")?;
        if total > MAX_STREAMED_PACKAGE_BYTES {
            return Err("approved_package_source_too_large".into());
        }
        hash.update(&buffer[..size]);
    }
    Ok((format!("sha256:{:x}", hash.finalize()), total))
}
impl ApprovedPackageSource {
    pub(crate) fn open_on(
        db: &Connection,
        configuration: &approval::ToolAdministratorConfiguration,
        approval_id: &str,
    ) -> Result<Self, String> {
        if cfg!(not(unix)) {
            return Err("unsupported_approved_package_source_platform".into());
        }
        if db.is_autocommit() {
            let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return Self::open_on(&snapshot, configuration, approval_id);
        }
        let approved = approval::read_original_on(db, configuration, approval_id)?
            .ok_or("approved_package_source_approval_missing")?;
        let canonical_manifest:String=db.query_row("SELECT canonical_manifest FROM main.tool_administrator_decisions WHERE decision_id=?1 AND kind='approve'",[approval_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        let root = configuration.original_package_cache_root();
        let base = root
            .parent()
            .ok_or("approved_package_source_directory_invalid")?;
        require_no_symlink_ancestors(base)?;
        let base_file = open_base(base)?;
        let metadata = base_file
            .metadata()
            .map_err(|_| "approved_package_source_metadata_failed")?;
        check(&metadata, true, false)?;
        let mut directories = vec![HeldDirectory {
            path: base.into(),
            file: base_file,
            identity: identity(&metadata),
            private: false,
        }];
        let key = candidate_cache_key(approved.manifest())?;
        for name in [
            "tool-supply-cache",
            approved.manifest().id.as_str(),
            key.as_str(),
        ] {
            let parent = directories
                .last()
                .ok_or("approved_package_source_directory_invalid")?;
            let file = open_child(&parent.file, name, true)?;
            let metadata = file
                .metadata()
                .map_err(|_| "approved_package_source_metadata_failed")?;
            check(&metadata, true, true)?;
            directories.push(HeldDirectory {
                path: parent.path.join(name),
                file,
                identity: identity(&metadata),
                private: true,
            });
        }
        let parent = &directories
            .last()
            .ok_or("approved_package_source_directory_invalid")?
            .file;
        let mut manifest_file = open_child(parent, "manifest.json", false)?;
        let package_file = open_child(parent, "package.blob", false)?;
        let metadata = manifest_file
            .metadata()
            .map_err(|_| "approved_package_source_metadata_failed")?;
        check(&metadata, false, false)?;
        let manifest_identity = identity(&metadata);
        if manifest_identity.length > MAX_MANIFEST_BYTES as u64 {
            return Err("approved_package_source_manifest_too_large".into());
        }
        let (cached_manifest, configuration_digest) =
            read_manifest(&mut manifest_file, configuration, approved.manifest())?;
        if cached_manifest != canonical_manifest {
            return Err("approved_package_source_manifest_changed".into());
        }
        let metadata = package_file
            .metadata()
            .map_err(|_| "approved_package_source_metadata_failed")?;
        check(&metadata, false, false)?;
        let package_identity = identity(&metadata);
        if package_identity.length > MAX_STREAMED_PACKAGE_BYTES {
            return Err("approved_package_source_too_large".into());
        }
        let mut result = Self {
            approval_id: approval_id.into(),
            approval_hash: approved.envelope_hash().into(),
            manifest: approved.manifest().clone(),
            canonical_manifest,
            configuration_digest,
            directories,
            manifest_file,
            manifest_identity,
            package_file,
            package_identity,
        };
        result.require_on(db, configuration)?;
        Ok(result)
    }
    pub(crate) fn manifest(&self) -> &ToolCapabilityManifest {
        &self.manifest
    }
    pub(crate) fn approval_id(&self) -> &str {
        &self.approval_id
    }
    pub(crate) fn package_size(&self) -> u64 {
        self.package_identity.length
    }
    fn require_entities(&self) -> Result<(), String> {
        require_no_symlink_ancestors(
            &self
                .directories
                .first()
                .ok_or("approved_package_source_directory_invalid")?
                .path,
        )?;
        for directory in &self.directories {
            let held = directory
                .file
                .metadata()
                .map_err(|_| "approved_package_source_metadata_failed")?;
            let located = fs::symlink_metadata(&directory.path)
                .map_err(|_| "approved_package_source_directory_changed")?;
            check(&held, true, directory.private)?;
            check(&located, true, directory.private)?;
            if !same_directory(&identity(&held), &directory.identity)
                || !same_directory(&identity(&located), &directory.identity)
            {
                return Err("approved_package_source_directory_changed".into());
            }
        }
        let parent = self
            .directories
            .last()
            .ok_or("approved_package_source_directory_invalid")?;
        let mut names = fs::read_dir(&parent.path)
            .map_err(|_| "approved_package_source_directory_changed")?
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "approved_package_source_directory_changed")?;
        names.sort();
        if names != ["manifest.json", "package.blob"] {
            return Err("approved_package_source_entries_changed".into());
        }
        for (name, file, original) in [
            (
                "manifest.json",
                &self.manifest_file,
                &self.manifest_identity,
            ),
            ("package.blob", &self.package_file, &self.package_identity),
        ] {
            let held = file
                .metadata()
                .map_err(|_| "approved_package_source_metadata_failed")?;
            let located = open_child(&parent.file, name, false)?
                .metadata()
                .map_err(|_| "approved_package_source_metadata_failed")?;
            check(&held, false, false)?;
            check(&located, false, false)?;
            if identity(&held) != *original || identity(&located) != *original {
                return Err("approved_package_source_file_changed".into());
            }
        }
        Ok(())
    }
    pub(crate) fn require_on(
        &mut self,
        db: &Connection,
        configuration: &approval::ToolAdministratorConfiguration,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.require_on(&snapshot, configuration);
        }
        let approved = approval::read_original_on(db, configuration, &self.approval_id)?
            .ok_or("approved_package_source_approval_missing")?;
        if approved.envelope_hash() != self.approval_hash
            || approved.manifest() != &self.manifest
            || configuration.original_package_cache_root()
                != self
                    .directories
                    .get(1)
                    .ok_or("approved_package_source_directory_invalid")?
                    .path
        {
            return Err("approved_package_source_approval_changed".into());
        }
        self.require_entities()?;
        let (cached_manifest, configuration_digest) =
            read_manifest(&mut self.manifest_file, configuration, &self.manifest)?;
        if cached_manifest != self.canonical_manifest
            || configuration_digest != self.configuration_digest
        {
            return Err("approved_package_source_manifest_changed".into());
        }
        let (digest, length) = package_hash(&mut self.package_file)?;
        if digest != self.manifest.digest || length != self.package_identity.length {
            return Err("approved_package_source_digest_changed".into());
        }
        self.require_entities()?;
        let confirmed = approval::read_original_on(db, configuration, &self.approval_id)?
            .ok_or("approved_package_source_approval_missing")?;
        if confirmed.envelope_hash() != self.approval_hash || confirmed.manifest() != &self.manifest
        {
            return Err("approved_package_source_approval_changed".into());
        }
        Ok(())
    }
    /// Parsing consumes bytes from this held verified entity, never a path or
    /// independently supplied byte vector. A bounded copy is checked again
    /// after reading; even a concurrent in-place writer cannot return new bytes.
    pub(crate) fn read_verified_bytes_on(
        &mut self,
        db: &Connection,
        configuration: &approval::ToolAdministratorConfiguration,
    ) -> Result<Vec<u8>, String> {
        if db.is_autocommit() {
            let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.read_verified_bytes_on(&snapshot, configuration);
        }
        self.require_on(db, configuration)?;
        if self.package_size() > MAX_OPAQUE_PACKAGE_BYTES as u64 {
            return Err("approved_package_source_parser_input_too_large".into());
        }
        self.package_file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "approved_package_source_read_failed")?;
        let mut bytes = Vec::new();
        (&mut self.package_file)
            .take(MAX_OPAQUE_PACKAGE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "approved_package_source_read_failed")?;
        if bytes.len() > MAX_OPAQUE_PACKAGE_BYTES
            || bytes.len() as u64 != self.package_size()
            || format!("sha256:{:x}", Sha256::digest(&bytes)) != self.manifest.digest
        {
            return Err("approved_package_source_digest_changed".into());
        }
        self.require_on(db, configuration)?;
        Ok(bytes)
    }
}

