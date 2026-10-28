//! Inert durable package publication. No extraction, approval INSERT or execution.
use super::{approval, candidate_cache_key};
use approval::ToolAdministratorConfiguration;
use rusqlite::Transaction;

pub(crate) struct PublishedOriginalPackage {
    approval_id: String,
    cache_key: String,
}
impl PublishedOriginalPackage {
    pub(crate) fn approval_id(&self) -> &str {
        &self.approval_id
    }
    pub(crate) fn cache_key(&self) -> &str {
        &self.cache_key
    }
}

/// Caller owns the real preparation lease and decides whether its DB decision
/// commits. A filesystem error leaves that fence for explicit investigation.
pub(crate) fn publish_on(
    tx: &Transaction<'_>,
    preparation: &crate::db::EnvironmentPreparationLease,
    configuration: &ToolAdministratorConfiguration,
    decision: &[u8],
    manifest: &[u8],
    package: &[u8],
) -> Result<PublishedOriginalPackage, String> {
    preparation.require_tool_supply_on(tx)?;
    let verified = approval::preflight_approve(configuration, decision, manifest, package)?;
    let key = candidate_cache_key(verified.manifest())?;
    #[cfg(target_os = "macos")]
    {
        platform::publish(
            configuration,
            &verified.manifest().id,
            &key,
            manifest,
            package,
        )?;
        let repeated = approval::preflight_approve(configuration, decision, manifest, package)?;
        if repeated.envelope_hash() != verified.envelope_hash()
            || repeated.manifest() != verified.manifest()
        {
            return Err("tool_package_publish_original_changed".into());
        }
        preparation.require_tool_supply_on(tx)?;
        Ok(PublishedOriginalPackage {
            approval_id: verified.id().into(),
            cache_key: key,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (configuration, key, verified);
        Err("unsupported_tool_package_exclusive_publish_platform".into())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        fs::File,
        io::{Read, Write},
        os::fd::{AsRawFd, FromRawFd},
        os::unix::fs::MetadataExt,
        path::{Component, Path},
    };
    fn name(value: &str) -> Result<CString, String> {
        if value.is_empty() || value == "." || value == ".." || value.contains('/') {
            return Err("tool_package_publish_name_invalid".into());
        }
        CString::new(value).map_err(|_| "tool_package_publish_name_invalid".into())
    }
    fn open_at(parent: &File, value: &str, flags: i32, mode: u16) -> Result<File, String> {
        let value = name(value)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                value.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                mode as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(format!(
                "tool_package_publish_open:{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    fn private_dir(dir: &File) -> Result<(), String> {
        let m = dir.metadata().map_err(|e| e.to_string())?;
        if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
            return Err("tool_package_publish_directory_permissions".into());
        }
        Ok(())
    }
    fn child_dir(parent: &File, value: &str, create: bool) -> Result<File, String> {
        if create {
            let value = name(value)?;
            if unsafe { libc::mkdirat(parent.as_raw_fd(), value.as_ptr(), 0o700) } != 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::EEXIST) {
                    return Err(format!("tool_package_publish_mkdir:{e}"));
                }
            } else {
                parent.sync_all().map_err(|e| e.to_string())?;
            }
        }
        let f = open_at(parent, value, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        private_dir(&f)?;
        Ok(f)
    }
    // Anchor every path component by a held directory. Symlinks are rejected,
    // including ancestors, rather than merely checking the last pathname.
    fn fixed_root(path: &Path, create: bool) -> Result<File, String> {
        if !path.is_absolute() {
            return Err("tool_package_publish_root_invalid".into());
        }
        let mut dir = File::open("/").map_err(|e| e.to_string())?;
        let components = path.components().collect::<Vec<_>>();
        for (i, c) in components.iter().enumerate() {
            match c {
                Component::RootDir => (),
                Component::Normal(value) => {
                    let value = value.to_str().ok_or("tool_package_publish_root_invalid")?;
                    if i + 1 == components.len() {
                        dir = child_dir(&dir, value, create)?;
                    } else {
                        dir = open_at(&dir, value, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
                    }
                }
                _ => return Err("tool_package_publish_root_invalid".into()),
            }
        }
        private_dir(&dir)?;
        Ok(dir)
    }
    fn same_entity(a: &File, b: &File) -> Result<(), String> {
        let a = a.metadata().map_err(|e| e.to_string())?;
        let b = b.metadata().map_err(|e| e.to_string())?;
        if a.dev() != b.dev() || a.ino() != b.ino() {
            return Err("tool_package_publish_fixed_path_changed".into());
        }
        Ok(())
    }
    fn require_fixed(
        configuration: &ToolAdministratorConfiguration,
        id: &str,
        root: &File,
        release: &File,
    ) -> Result<(), String> {
        let current = fixed_root(&configuration.original_package_cache_root(), false)?;
        same_entity(root, &current)?;
        let current_release = child_dir(&current, id, false)?;
        same_entity(release, &current_release)
    }
    fn entries(dir: &File) -> Result<Vec<String>, String> {
        let fd = unsafe { libc::dup(dir.as_raw_fd()) };
        if fd < 0 {
            return Err("tool_package_publish_list_failed".into());
        }
        let d = unsafe { libc::fdopendir(fd) };
        if d.is_null() {
            unsafe { libc::close(fd) };
            return Err("tool_package_publish_list_failed".into());
        }
        let result = (|| {
            unsafe { libc::rewinddir(d) };
            let mut names = Vec::new();
            loop {
                unsafe { *libc::__error() = 0 };
                let e = unsafe { libc::readdir(d) };
                if e.is_null() {
                    if unsafe { *libc::__error() } != 0 {
                        return Err("tool_package_publish_list_failed".into());
                    }
                    break;
                }
                let n = unsafe { CStr::from_ptr((*e).d_name.as_ptr()) }
                    .to_str()
                    .map_err(|_| "tool_package_publish_entry_invalid")?;
                if n != "." && n != ".." {
                    names.push(n.to_owned());
                }
                if names.len() > 2 {
                    return Err("tool_package_publish_extra_entry".into());
                }
            }
            names.sort();
            Ok(names)
        })();
        if unsafe { libc::closedir(d) } != 0 {
            return Err("tool_package_publish_list_failed".into());
        }
        result
    }
    fn file_bytes(dir: &File, value: &str, maximum: usize) -> Result<Vec<u8>, String> {
        let mut f = open_at(dir, value, libc::O_RDONLY, 0)?;
        let m = f.metadata().map_err(|e| e.to_string())?;
        if !m.is_file()
            || m.nlink() != 1
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o222 != 0
            || m.len() > maximum as u64
        {
            return Err("tool_package_publish_file_invalid".into());
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut f)
            .take(maximum as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > maximum {
            return Err("tool_package_publish_file_invalid".into());
        }
        let after = f.metadata().map_err(|e| e.to_string())?;
        if m.dev() != after.dev()
            || m.ino() != after.ino()
            || m.len() != after.len()
            || m.mtime() != after.mtime()
            || m.mtime_nsec() != after.mtime_nsec()
        {
            return Err("tool_package_publish_file_changed".into());
        }
        Ok(bytes)
    }
    fn verify(dir: &File, manifest: &[u8], package: &[u8]) -> Result<(), String> {
        if entries(dir)? != ["manifest.json", "package.blob"]
            || file_bytes(dir, "manifest.json", manifest.len())? != manifest
            || file_bytes(dir, "package.blob", package.len())? != package
        {
            return Err("tool_package_publish_existing_mismatch".into());
        }
        Ok(())
    }
    fn write(dir: &File, value: &str, bytes: &[u8]) -> Result<(), String> {
        let mut f = open_at(
            dir,
            value,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o400,
        )?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())
    }
    pub(super) fn publish(
        configuration: &ToolAdministratorConfiguration,
        id: &str,
        key: &str,
        manifest: &[u8],
        package: &[u8],
    ) -> Result<(), String> {
        let root = fixed_root(&configuration.original_package_cache_root(), true)?;
        let release = child_dir(&root, id, true)?;
        let key_name = name(key)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                release.as_raw_fd(),
                key_name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            let existing = child_dir(&release, key, false)?;
            verify(&existing, manifest, package)?;
            require_fixed(configuration, id, &root, &release)?;
            return Ok(());
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT) {
            return Err("tool_package_publish_destination_invalid".into());
        }
        let staged = format!(".staging-{}", uuid::Uuid::new_v4());
        let staging = child_dir(&release, &staged, true)?;
        write(&staging, "manifest.json", manifest)?;
        write(&staging, "package.blob", package)?;
        verify(&staging, manifest, package)?;
        if unsafe { libc::fchmod(staging.as_raw_fd(), 0o500) } != 0 {
            return Err("tool_package_publish_chmod_failed".into());
        }
        staging.sync_all().map_err(|e| e.to_string())?;
        require_fixed(configuration, id, &root, &release)?;
        let staged_name = name(&staged)?;
        // Atomic no-replace. A racing/existing destination is never overwritten.
        if unsafe {
            libc::renameatx_np(
                release.as_raw_fd(),
                staged_name.as_ptr(),
                release.as_raw_fd(),
                key_name.as_ptr(),
                libc::RENAME_EXCL,
            )
        } != 0
        {
            return Err(format!(
                "tool_package_publish_exclusive_rename:{}",
                std::io::Error::last_os_error()
            ));
        }
        release.sync_all().map_err(|e| e.to_string())?;
        let existing = child_dir(&release, key, false)?;
        let a = staging.metadata().map_err(|e| e.to_string())?;
        let b = existing.metadata().map_err(|e| e.to_string())?;
        if a.dev() != b.dev() || a.ino() != b.ino() {
            return Err("tool_package_publish_entity_changed".into());
        }
        verify(&existing, manifest, package)?;
        require_fixed(configuration, id, &root, &release)
    }
}

