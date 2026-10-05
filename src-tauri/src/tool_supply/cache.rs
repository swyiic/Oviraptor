use super::*;

fn open_regular_tool_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| "tool_package_invalid")?;
    let metadata = file.metadata().map_err(|_| "tool_package_invalid")?;
    if !metadata.is_file() {
        return Err("tool_package_invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("tool_package_invalid".into());
        }
    }
    if metadata.len() > MAX_STREAMED_PACKAGE_BYTES {
        return Err("tool_package_too_large".into());
    }
    Ok(file)
}

fn stream_tool_file(path: &Path, mut destination: Option<&mut File>) -> Result<String, String> {
    let mut source = open_regular_tool_file(path)?;
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let size = source
            .read(&mut buffer)
            .map_err(|_| "tool_package_read_failed")?;
        if size == 0 {
            break;
        }
        total = total
            .checked_add(size as u64)
            .ok_or("tool_package_too_large")?;
        if total > MAX_STREAMED_PACKAGE_BYTES {
            return Err("tool_package_too_large".into());
        }
        hash.update(&buffer[..size]);
        if let Some(file) = destination.as_deref_mut() {
            file.write_all(&buffer[..size])
                .map_err(|_| "tool_staging_write_failed")?;
        }
    }
    if let Some(file) = destination {
        file.sync_all().map_err(|_| "tool_staging_write_failed")?;
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}

/// Task-external streaming staging for a locally supplied large package. The
/// first pass rejects invalid input before claiming the persistent preparation
/// lease; the second pass detects replacement during copy. A failure after the
/// lease requires manual recovery. This only publishes an inert candidate.
pub(crate) fn stage_offline_tool_candidate_from_file(
    db_path: &Path,
    cache_root: &Path,
    manifest_bytes: &[u8],
    package_path: &Path,
    anchors: &[ToolTrustAnchor],
    platform: &str,
) -> Result<PathBuf, String> {
    if cfg!(not(unix)) {
        return Err("unsupported_tool_cache_platform".into());
    }
    let payload = verify_signed_tool_manifest(manifest_bytes, anchors, platform)?;
    if stream_tool_file(package_path, None)? != payload.digest {
        return Err("tool_package_digest_mismatch".into());
    }
    let preparation = crate::db::begin_environment_preparation(db_path)?;
    let id_dir = private_cache_directory(cache_root, true)?.join(&payload.id);
    private_cache_directory(&id_dir, true)?;
    let identity_key = candidate_cache_key(&payload)?;
    let destination = id_dir.join(&identity_key);
    if destination.exists() || fs::symlink_metadata(&destination).is_ok() {
        verify_cached_candidate(&destination, anchors, platform, &payload)?;
        preparation.record_tool_candidate(&payload.id, &payload.digest, &identity_key, true)?;
        preparation.complete()?;
        return Ok(destination);
    }
    let staging = id_dir.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    create_private_directory(&staging, "tool_staging_create_failed")?;
    private_cache_directory(&staging, false)?;
    write_staged_file(&staging.join("manifest.json"), manifest_bytes)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o400);
    }
    let mut blob = options
        .open(staging.join("package.blob"))
        .map_err(|_| "tool_staging_write_failed")?;
    if stream_tool_file(package_path, Some(&mut blob))? != payload.digest {
        return Err("tool_package_digest_mismatch".into());
    }
    drop(blob);
    verify_cached_candidate(&staging, anchors, platform, &payload)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o500))
            .map_err(|_| "tool_staging_permissions_failed")?;
    }
    fs::rename(&staging, &destination).map_err(|_| "tool_staging_publish_failed")?;
    File::open(&id_dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| "tool_cache_sync_failed")?;
    verify_cached_candidate(&destination, anchors, platform, &payload)?;
    preparation.record_tool_candidate(&payload.id, &payload.digest, &identity_key, false)?;
    preparation.complete()?;
    Ok(destination)
}

/// Only a caller with an independently authenticated administrator trust store
/// may supply `anchors`. This is deliberately not registered as a Tauri or
/// Agent command. The cache is an inert, opaque candidate, never an executable
/// capability: approval, safe extraction, and sandbox execution are separate.
pub(crate) fn stage_offline_tool_candidate(
    db_path: &Path,
    cache_root: &Path,
    manifest_bytes: &[u8],
    package_bytes: &[u8],
    anchors: &[ToolTrustAnchor],
    platform: &str,
) -> Result<PathBuf, String> {
    // Windows reparse-point and directory durability handling require a
    // separate audited implementation; never silently reuse Unix assumptions.
    if cfg!(not(unix)) {
        return Err("unsupported_tool_cache_platform".into());
    }

    // Reject untrusted bytes before taking a persistent preparation lease. A
    // write failure after claiming it remains fenced for manual investigation.
    let payload = verify_staged_tool_blob(manifest_bytes, package_bytes, anchors, platform)?;
    let preparation = crate::db::begin_environment_preparation(db_path)?;
    let id_dir = private_cache_directory(cache_root, true)?;
    let id_dir = id_dir.join(&payload.id);
    private_cache_directory(&id_dir, true)?;
    let identity_key = candidate_cache_key(&payload)?;
    let destination = id_dir.join(&identity_key);
    if destination.exists() || fs::symlink_metadata(&destination).is_ok() {
        verify_cached_candidate(&destination, anchors, platform, &payload)?;
        preparation.record_tool_candidate(&payload.id, &payload.digest, &identity_key, true)?;
        preparation.complete()?;
        return Ok(destination);
    }

    let staging = id_dir.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    create_private_directory(&staging, "tool_staging_create_failed")?;
    private_cache_directory(&staging, false)?;
    write_staged_file(&staging.join("manifest.json"), manifest_bytes)?;
    write_staged_file(&staging.join("package.blob"), package_bytes)?;
    verify_cached_candidate(&staging, anchors, platform, &payload)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o500))
            .map_err(|_| "tool_staging_permissions_failed")?;
    }
    fs::rename(&staging, &destination).map_err(|_| "tool_staging_publish_failed")?;
    File::open(&id_dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| "tool_cache_sync_failed")?;
    verify_cached_candidate(&destination, anchors, platform, &payload)?;
    preparation.record_tool_candidate(&payload.id, &payload.digest, &identity_key, false)?;
    preparation.complete()?;
    Ok(destination)
}

// Set the mode on the mkdir itself: chmod after mkdir exposes a newly created
// candidate directory for a window when the process umask is permissive.
fn create_private_directory(path: &Path, error: &'static str) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| error.into())
}

fn private_cache_directory(path: &Path, create: bool) -> Result<PathBuf, String> {
    if create && !path.exists() {
        create_private_directory(path, "tool_cache_directory_create_failed")?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "tool_cache_directory_invalid")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("tool_cache_directory_invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("tool_cache_directory_not_private".into());
        }
    }
    Ok(path.to_path_buf())
}

fn write_staged_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o400);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "tool_staging_write_failed")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "tool_staging_write_failed")?;
    Ok(())
}

fn verify_cached_candidate(
    directory: &Path,
    anchors: &[ToolTrustAnchor],
    platform: &str,
    expected: &ToolCapabilityManifest,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(directory).map_err(|_| "tool_cache_invalid")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("tool_cache_invalid".into());
    }
    // An inert candidate is exactly the signed manifest and one opaque blob.
    // Extra entries must not become implicit entrypoints during later import.
    let mut names = Vec::new();
    for entry in fs::read_dir(directory).map_err(|_| "tool_cache_invalid")? {
        names.push(entry.map_err(|_| "tool_cache_invalid")?.file_name());
        if names.len() > 2 {
            return Err("tool_cache_invalid".into());
        }
    }
    names.sort();
    if names != ["manifest.json", "package.blob"] {
        return Err("tool_cache_invalid".into());
    }
    let manifest_path = directory.join("manifest.json");
    let package_path = directory.join("package.blob");
    for path in [&manifest_path, &package_path] {
        let metadata = fs::symlink_metadata(path).map_err(|_| "tool_cache_invalid")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("tool_cache_invalid".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return Err("tool_cache_invalid".into());
            }
        }
    }
    let manifest = read_bounded_file(&manifest_path, MAX_MANIFEST_BYTES)?;
    let actual = verify_signed_tool_manifest(&manifest, anchors, platform)?;
    if stream_tool_file(&package_path, None)? != actual.digest {
        return Err("tool_package_digest_mismatch".into());
    }
    if &actual != expected {
        return Err("tool_cache_manifest_mismatch".into());
    }
    Ok(())
}

fn read_bounded_file(path: &Path, maximum: usize) -> Result<Vec<u8>, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| "tool_cache_invalid")?;
    let metadata = file.metadata().map_err(|_| "tool_cache_invalid")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err("tool_cache_invalid".into());
        }
    }
    if metadata.len() > maximum as u64 {
        return Err("tool_package_too_large".into());
    }
    // A post-read size check catches growth between metadata and the read.
    let mut bounded = file.take((maximum + 1) as u64);
    let mut bytes = Vec::new();
    bounded
        .read_to_end(&mut bytes)
        .map_err(|_| "tool_cache_invalid")?;
    if bytes.len() > maximum {
        return Err("tool_package_too_large".into());
    }
    Ok(bytes)
}
