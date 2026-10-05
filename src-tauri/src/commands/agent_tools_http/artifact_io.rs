/// Traverse every directory component from the filesystem root through fixed
/// handles. A no-follow open of just the last component would still accept a
/// symlinked parent and could send a raw credential dump outside the task tree.
#[cfg(unix)]
fn agent_artifact_directory_chain(path: &Path, create: bool) -> Option<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Component;

    if !path.is_absolute() { return None; }
    let mut directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/").ok()?;
    for component in path.components() {
        let name = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => std::ffi::CString::new(name.as_bytes()).ok()?,
            Component::ParentDir | Component::Prefix(_) => return None,
        };
        if create {
            // SAFETY: the parent descriptor stays live and the name is a single
            // NUL-terminated component; an existing entry is checked by openat.
            let result = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
                return None;
            }
        }
        // SAFETY: a successful openat returns a new owned descriptor. Each
        // component is opened relative to the previously verified directory.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 { return None; }
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    Some(directory)
}

#[cfg(unix)]
fn open_agent_artifact_directory(path: &Path) -> Option<std::fs::File> {
    agent_artifact_directory_chain(path, false)
}

#[cfg(not(unix))]
fn open_agent_artifact_directory(_path: &Path) -> Option<std::fs::File> {
    None
}

/// The writer opens the target and its artifact child through fixed, no-follow
/// directory handles. Existing components cannot redirect the raw payload.
#[cfg(unix)]
fn prepare_agent_artifact_write_directory(
    target_dir: &Path,
    child: &str,
) -> Result<(std::path::PathBuf, Option<std::fs::File>), String> {
    use std::os::fd::{AsRawFd, FromRawFd};

    if !matches!(child, AGENT_HTTP_DIRECTORY | AGENT_DIFF_DIRECTORY) {
        return Err("artifact_directory_invalid".into());
    }
    let parent = agent_artifact_directory_chain(target_dir, true)
        .ok_or("artifact_target_directory_invalid")?;
    let child_name = std::ffi::CString::new(child).map_err(|_| "artifact_directory_invalid")?;
    // SAFETY: parent is a live directory descriptor; child_name is a fixed,
    // NUL-terminated single component. EEXIST is handled by the no-follow open.
    let created = unsafe { libc::mkdirat(parent.as_raw_fd(), child_name.as_ptr(), 0o700) };
    if created != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        return Err("artifact_directory_create_failed".into());
    }
    // SAFETY: successful openat transfers one new owned fd into File.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            child_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err("artifact_directory_invalid".into());
    }
    let directory = unsafe { std::fs::File::from_raw_fd(fd) };
    // Older attempts created this directory with the process umask. Restrict
    // it before storing a fresh raw payload, without touching its contents.
    if unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) } != 0 {
        return Err("artifact_directory_permissions_failed".into());
    }
    Ok((target_dir.join(child), Some(directory)))
}

#[cfg(not(unix))]
fn prepare_agent_artifact_write_directory(
    _target_dir: &Path,
    _child: &str,
) -> Result<(std::path::PathBuf, Option<std::fs::File>), String> {
    // Reparse-point-safe directory creation and private file handles require
    // an independently audited Windows adapter. Never put raw credentials on
    // a path whose target may be replaced by another process.
    Err("unsupported_agent_artifact_write_platform".into())
}

#[cfg(unix)]
fn write_agent_artifact_file(
    _directory_path: &Path,
    directory: Option<&std::fs::File>,
    name: &str,
    content: &[u8],
) -> Result<(), String> {
    use std::{io::Write, os::fd::{AsRawFd, FromRawFd}};
    let directory = directory.ok_or("artifact_directory_invalid")?;
    if name.is_empty() || matches!(name, "." | "..")
        || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err("artifact_file_invalid".into());
    }
    let name = std::ffi::CString::new(name).map_err(|_| "artifact_file_invalid")?;
    // SAFETY: directory stays open; name is one NUL-terminated component.
    // O_EXCL prevents overwriting any existing artifact, even a symlink.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err("artifact_file_create_failed".into());
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(content)
        .and_then(|_| file.sync_all())
        .map_err(|_| "artifact_file_write_failed".into())
}

#[cfg(unix)]
fn agent_artifact_slot_exists(directory: Option<&std::fs::File>, name: &str) -> Result<bool, String> {
    use std::os::fd::AsRawFd;
    let directory = directory.ok_or("artifact_directory_invalid")?;
    let name = std::ffi::CString::new(name).map_err(|_| "artifact_file_invalid")?;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: the name is generated as one component and the directory handle
    // remains live; fstatat writes metadata only on success.
    let result = unsafe { libc::fstatat(directory.as_raw_fd(), name.as_ptr(),
        metadata.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
    if result == 0 { return Ok(true); }
    if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) { return Ok(false); }
    Err("artifact_slot_check_failed".into())
}

#[cfg(not(unix))]
fn agent_artifact_slot_exists(_directory: Option<&std::fs::File>, _name: &str) -> Result<bool, String> {
    Err("unsupported_agent_artifact_write_platform".into())
}

#[cfg(not(unix))]
fn write_agent_artifact_file(
    _directory_path: &Path,
    _directory: Option<&std::fs::File>,
    _name: &str,
    _content: &[u8],
) -> Result<(), String> {
    Err("unsupported_agent_artifact_write_platform".into())
}

#[cfg(unix)]
fn read_agent_artifact_file(
    directory: &std::fs::File,
    name: &str,
    maximum: u64,
) -> Option<(Vec<u8>, u64)> {
    use std::{io::Read, os::fd::{AsRawFd, FromRawFd}, os::unix::fs::MetadataExt};
    if name == "." || name == ".." || name.is_empty()
        || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return None;
    }
    let name = std::ffi::CString::new(name).ok()?;
    // SAFETY: `directory` remains open for the lifetime of the returned fd;
    // `name` is NUL-terminated and contains no path separators. A successful
    // openat returns a new owned descriptor, transferred immediately to File.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(), name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 { return None; }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > maximum {
        return None;
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file).take(maximum + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 != metadata.len() { return None; }
    Some((bytes, metadata.len()))
}

#[cfg(not(unix))]
fn read_agent_artifact_file(
    _directory: &std::fs::File,
    _name: &str,
    _maximum: u64,
) -> Option<(Vec<u8>, u64)> {
    None
}

