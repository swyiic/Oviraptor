// A read-only inventory, not a sandbox or an immutable installation. Never run
// `node -v` here: preflight must not execute a candidate before admission.
const WEB_TOOLCHAIN_FILE_LIMIT: u64 = 512 * 1024 * 1024;
const WEB_TOOLCHAIN_TOTAL_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

fn web_browser_candidates(
    platform: &str, environment: impl Fn(&str) -> Option<OsString>,
) -> Result<Vec<PathBuf>, String> {
    let locations: JsonValue = serde_json::from_str(include_str!("../../resources/config/browser-locations.json"))
        .map_err(|_| "web_binding_browser_locations_invalid")?;
    let mut paths = Vec::new();
    if let Some(path) = environment("OVIRAPTOR_BROWSER_EXECUTABLE").filter(|v| !v.is_empty()) {
        paths.push(PathBuf::from(path));
    }
    let key = match platform { "darwin" => "darwin", "win32" => "win32", _ => "linux" };
    let entries = locations[key].as_array().ok_or("web_binding_browser_locations_invalid")?;
    let roots = if platform == "win32" {
        ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"].into_iter()
            .filter_map(environment).filter(|v| !v.is_empty()).map(PathBuf::from).collect::<Vec<_>>()
    } else { vec![PathBuf::new()] };
    for root in roots {
        for entry in entries {
            paths.push(root.join(entry.as_str().ok_or("web_binding_browser_locations_invalid")?));
        }
    }
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    Ok(paths)
}

fn web_tool_file_stamp(metadata: &fs::Metadata) -> Result<JsonValue, String> {
    let modified = metadata.modified().map_err(|_| "web_binding_tool_metadata_unavailable")?
        .duration_since(std::time::UNIX_EPOCH).map_err(|_| "web_binding_tool_metadata_invalid")?;
    let stamp = serde_json::json!({"length":metadata.len(),"modifiedSeconds":modified.as_secs(),
        "modifiedNanos":modified.subsec_nanos()});
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        Ok(serde_json::json!({"portable":stamp,"unix":{"device":metadata.dev(),"inode":metadata.ino(),
            "mode":metadata.mode(),"ctime":metadata.ctime(),"ctimeNanos":metadata.ctime_nsec()}}))
    }
    #[cfg(not(unix))]
    Ok(stamp)
}

fn web_tool_candidate_binding(path: &Path, remaining: &mut u64) -> Result<JsonValue, String> {
    if !path.is_absolute() { return Err("web_binding_tool_path_not_absolute".into()); }
    let named = path.to_str().ok_or("web_binding_tool_path_not_utf8")?;
    let canonical = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(serde_json::json!({"path":named,"state":"missing"}));
        },
        Err(_) => return Err("web_binding_tool_path_unavailable".into()),
    };
    // Symlink entry points are common in installed toolchains. Bind both their
    // resolution and bytes, but never follow the canonical leaf while opening.
    let metadata = fs::symlink_metadata(&canonical).map_err(|_| "web_binding_tool_metadata_unavailable")?;
    if !metadata.is_file() || metadata.len() > WEB_TOOLCHAIN_FILE_LIMIT || metadata.len() > *remaining {
        return Err("web_binding_tool_unsafe_or_oversized".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let mut file = options.open(&canonical).map_err(|_| "web_binding_tool_unavailable")?;
    let opened = file.metadata().map_err(|_| "web_binding_tool_metadata_unavailable")?;
    let stamp = web_tool_file_stamp(&metadata)?;
    if !opened.is_file() || stamp != web_tool_file_stamp(&opened)? {
        return Err("web_binding_tool_changed_during_read".into());
    }
    // Inventory may read large Node/browser binaries repeatedly. Use the
    // existing native crypto backend without caching or skipping any bytes.
    // Keep the SHA-256 wire format and the before/after identity checks intact.
    let mut hash = aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256);
    let mut read_bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "web_binding_tool_read_failed")?;
        if count == 0 { break; }
        read_bytes += count as u64;
        if read_bytes > WEB_TOOLCHAIN_FILE_LIMIT || read_bytes > *remaining {
            return Err("web_binding_tool_unsafe_or_oversized".into());
        }
        hash.update(&buffer[..count]);
    }
    if read_bytes != metadata.len()
        || stamp != web_tool_file_stamp(&file.metadata().map_err(|_| "web_binding_tool_metadata_unavailable")?)?
        || stamp != web_tool_file_stamp(&fs::symlink_metadata(&canonical).map_err(|_| "web_binding_tool_metadata_unavailable")?)?
        || fs::canonicalize(path).map_err(|_| "web_binding_tool_path_unavailable")? != canonical {
        return Err("web_binding_tool_changed_during_read".into());
    }
    *remaining -= read_bytes;
    let digest = hash.finish();
    let sha256: String = digest.as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(serde_json::json!({"path":named,"state":"present",
        "resolvedPath":canonical.to_str().ok_or("web_binding_tool_path_not_utf8")?,
        "stamp":stamp,"sha256":sha256}))
}

fn web_toolchain_candidates_binding(node: &[PathBuf], browser: &[PathBuf]) -> Result<JsonValue, String> {
    if node.len() + browser.len() > 256 { return Err("web_binding_too_many_tool_candidates".into()); }
    let mut remaining = WEB_TOOLCHAIN_TOTAL_LIMIT;
    let mut bind = |paths: &[PathBuf]| -> Result<Vec<JsonValue>,String> {
        paths.iter().map(|path| web_tool_candidate_binding(path,&mut remaining)).collect()
    };
    Ok(serde_json::json!({"schemaVersion":1,"nodeCandidates":bind(node)?,"browserCandidates":bind(browser)?}))
}

fn web_toolchain_binding(runtime_path: &std::ffi::OsStr) -> Result<JsonValue, String> {
    let node = helper_node_candidates(runtime_path,std::env::var_os("OVIRAPTOR_NODE_EXECUTABLE"));
    let platform = if cfg!(target_os="macos") { "darwin" } else if cfg!(windows) { "win32" } else { "linux" };
    let browser = web_browser_candidates(platform,|key| std::env::var_os(key))?;
    web_toolchain_candidates_binding(&node,&browser)
}
