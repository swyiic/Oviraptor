use crate::{schema, MAX_SAFE_INTEGER};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MANIFEST_BYTES: u64 = 256 * 1024;
const FILE_BYTES: u64 = 128 * 1024 * 1024;
const PACKAGE_BYTES: u64 = 256 * 1024 * 1024;

fn metadata_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.encode_utf16().count() <= maximum
        && !value.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema_version: u64,
    pub id: String,
    pub version: String,
    pub host_api: String,
    pub entry: String,
    pub data_schema_version: u64,
    pub runtime: Runtime,
    pub tools: Vec<Spec>,
    pub actions: Vec<Spec>,
    pub pages: Vec<Page>,
    pub permissions: Vec<Permission>,
    pub files: Vec<PackageFile>,
    pub lifecycle: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Runtime {
    kind: String,
    platform: String,
    arch: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Spec {
    pub name: String,
    description: String,
    pub input_schema: Value,
    pub permission: String,
    pub scope: Option<String>,
    pub delivery: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Page {
    pub id: String,
    title: String,
    pub path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Permission {
    pub id: String,
    description: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackageFile {
    pub path: String,
    sha256: String,
}

pub(crate) fn local_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
pub(crate) fn permission_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}
fn app_id(s: &str) -> bool {
    s.len() >= 3
        && s.len() <= 128
        && s.contains('.')
        && s.split('.').all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_lowercase()
                && part.split('-').all(|p| {
                    !p.is_empty()
                        && p.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                })
        })
}
fn version(s: &str) -> bool {
    if s.len() > 80 {
        return false;
    }
    let (left, build) = s
        .split_once('+')
        .map(|(a, b)| (a, Some(b)))
        .unwrap_or((s, None));
    let valid_suffix = |v: &str, numeric: bool| {
        !v.is_empty()
            && v.split('.').all(|p| {
                !p.is_empty()
                    && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && (!numeric
                        || !p.bytes().all(|b| b.is_ascii_digit())
                        || p == "0"
                        || !p.starts_with('0'))
            })
    };
    if build.is_some_and(|v| !valid_suffix(v, false)) {
        return false;
    }
    let (core, pre) = left
        .split_once('-')
        .map(|(a, b)| (a, Some(b)))
        .unwrap_or((left, None));
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (*p == "0" || !p.starts_with('0'))
        })
        && pre.is_none_or(|v| valid_suffix(v, true))
}
fn package_path(s: &str) -> bool {
    if s.is_empty()
        || s.len() > 240
        || s.starts_with('/')
        || s.ends_with('/')
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./-".contains(&b))
    {
        return false;
    }
    s.split('/').all(|part| {
        let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.ends_with('.')
            && !matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
            && !(stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    })
}
fn unique(items: impl Iterator<Item = String>) -> bool {
    let mut seen = BTreeSet::new();
    items.into_iter().all(|v| seen.insert(v))
}

impl Manifest {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MANIFEST_BYTES as usize {
            return Err("app_manifest_invalid".into());
        }
        let raw: Value = serde_json::from_slice(bytes).map_err(|_| "app_manifest_invalid")?;
        if raw.get("lifecycle").is_some_and(|v| {
            !schema::object_keys(v, &["start", "stop"], &[])
                || v.as_object()
                    .is_some_and(|m| m.values().any(|v| !v.is_boolean()))
        }) {
            return Err("app_manifest_invalid".into());
        }
        // Tools cannot smuggle an action scope; serde's shared Spec alone would
        // accept it, so validate the two closed wire shapes separately.
        for (field, keys) in [
            (
                "tools",
                vec!["name", "description", "inputSchema", "permission"],
            ),
            (
                "actions",
                vec!["name", "description", "inputSchema", "permission", "scope"],
            ),
        ] {
            let specs = raw
                .get(field)
                .and_then(Value::as_array)
                .ok_or("app_manifest_invalid")?;
            let mut allowed = keys.clone();
            if field == "actions" {
                allowed.push("delivery");
            }
            if specs.iter().any(|v| {
                !schema::object_keys(v, &allowed, &keys)
                    || v.get("delivery").is_some_and(|d| d != "artifact-read")
            }) {
                return Err("app_manifest_invalid".into());
            }
        }
        let m: Self = serde_json::from_value(raw).map_err(|_| "app_manifest_invalid")?;
        if m.schema_version != 2
            || m.host_api != "^2.0.0"
            || !app_id(&m.id)
            || !version(&m.version)
            || m.data_schema_version == 0
            || m.data_schema_version > MAX_SAFE_INTEGER as u64
            || m.runtime.kind != "native"
            || !matches!(m.runtime.platform.as_str(), "linux" | "darwin" | "win32")
            || !matches!(m.runtime.arch.as_str(), "x64" | "arm64")
            || !package_path(&m.entry)
            || !m.entry.starts_with("bin/")
            || m.entry[4..].contains('/')
            || m.permissions.len() > 128
            || m.tools.len() > 64
            || m.actions.len() > 128
            || m.pages.len() > 16
            || m.files.len() > 512
            || !unique(m.permissions.iter().map(|p| p.id.clone()))
            || !unique(m.tools.iter().map(|p| p.name.clone()))
            || !unique(m.actions.iter().map(|p| p.name.clone()))
            || !unique(m.pages.iter().map(|p| p.id.clone()))
        {
            return Err("app_manifest_invalid".into());
        }
        for p in &m.permissions {
            if !permission_name(&p.id) || !metadata_text(&p.description, 1024) {
                return Err("app_manifest_invalid".into());
            }
        }
        for (spec, action) in m
            .tools
            .iter()
            .map(|s| (s, false))
            .chain(m.actions.iter().map(|s| (s, true)))
        {
            if !local_name(&spec.name)
                || !metadata_text(&spec.description, 2048)
                || !m.permissions.iter().any(|p| p.id == spec.permission)
                || spec.input_schema.get("type") != Some(&json!("object"))
                || (action && !matches!(spec.scope.as_deref(), Some("project" | "application")))
                || (spec.delivery.is_some()
                    && (!action
                        || spec.scope.as_deref() != Some("project")
                        || spec.input_schema != artifact_read_schema()))
            {
                return Err("app_manifest_invalid".into());
            }
            schema::validate(&spec.input_schema, action, 0).map_err(|_| "app_manifest_invalid")?;
        }
        for p in &m.pages {
            if !local_name(&p.id)
                || !metadata_text(&p.title, 128)
                || !package_path(&p.path)
                || !p.path.ends_with(".html")
            {
                return Err("app_manifest_invalid".into());
            }
        }
        let paths: Vec<_> = m
            .files
            .iter()
            .map(|f| f.path.to_ascii_lowercase())
            .collect();
        if !unique(paths.iter().cloned()) {
            return Err("app_manifest_invalid".into());
        }
        for f in &m.files {
            if !package_path(&f.path)
                || f.path.eq_ignore_ascii_case("manifest.json")
                || f.sha256.len() != 64
                || !f
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || paths
                    .iter()
                    .any(|other| other.starts_with(&(f.path.to_ascii_lowercase() + "/")))
            {
                return Err("app_manifest_invalid".into());
            }
        }
        if !m.files.iter().any(|f| f.path == m.entry)
            || m.pages
                .iter()
                .any(|p| !m.files.iter().any(|f| f.path == p.path))
        {
            return Err("app_manifest_invalid".into());
        }
        Ok(m)
    }
    pub(crate) fn description(&self, hash: &str) -> Value {
        json!({"protocolVersion":2,"appId":self.id,"version":self.version,"manifestSha256":hash,
            "toolNames":self.tools.iter().map(|s|&s.name).collect::<Vec<_>>(),"actionNames":self.actions.iter().map(|s|&s.name).collect::<Vec<_>>(),"pageIds":self.pages.iter().map(|s|&s.id).collect::<Vec<_>>()})
    }
    pub(crate) fn lifecycle_allowed(&self, name: &str) -> bool {
        self.lifecycle
            .as_ref()
            .and_then(|v| v.get(name))
            .and_then(Value::as_bool)
            != Some(false)
    }
}

pub(crate) fn artifact_read_schema() -> Value {
    json!({"type":"object","properties":{
        "artifactId":{"type":"string","maxLength":128},
        "offset":{"type":"integer","minimum":0},
        "maxBytes":{"type":"integer","minimum":1,"maximum":196608}
    },"required":["artifactId","offset","maxBytes"],"additionalProperties":false})
}

pub(crate) fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "app_directory_invalid")?;
    if !path.is_absolute() || !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("app_directory_invalid".into());
    }
    let canonical = fs::canonicalize(path).map_err(|_| "app_directory_invalid")?;
    if canonical != path {
        return Err("app_directory_invalid".into());
    }
    Ok(canonical)
}

pub(crate) fn load(package: &Path) -> Result<(Manifest, String), String> {
    let mut data = Vec::new();
    let manifest_path = package.join("manifest.json");
    let meta = fs::symlink_metadata(&manifest_path).map_err(|_| "app_manifest_unavailable")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MANIFEST_BYTES {
        return Err("app_manifest_invalid".into());
    }
    fs::File::open(&manifest_path)
        .map_err(|_| "app_manifest_unavailable")?
        .take(MANIFEST_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|_| "app_manifest_unavailable")?;
    let m = Manifest::parse(&data)?;
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        v => v,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        v => v,
    };
    if m.runtime.platform != platform || m.runtime.arch != arch {
        return Err("app_platform_incompatible".into());
    }
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut BTreeSet<String>,
        entries: &mut usize,
    ) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|_| "app_package_invalid")? {
            *entries += 1;
            if *entries > 4096 {
                return Err("app_package_invalid".into());
            }
            let path = entry.map_err(|_| "app_package_invalid")?.path();
            let meta = fs::symlink_metadata(&path).map_err(|_| "app_package_invalid")?;
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "app_package_invalid")?
                .to_str()
                .ok_or("app_package_invalid")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if !package_path(&relative) || meta.file_type().is_symlink() {
                return Err("app_package_invalid".into());
            }
            if meta.is_dir() {
                walk(root, &path, out, entries)?;
            } else if meta.is_file() {
                if !out.insert(relative) || out.len() > 513 {
                    return Err("app_package_invalid".into());
                }
            } else {
                return Err("app_package_invalid".into());
            }
        }
        Ok(())
    }
    let mut actual = BTreeSet::new();
    walk(package, package, &mut actual, &mut 0)?;
    let expected: BTreeSet<_> = m
        .files
        .iter()
        .map(|f| f.path.clone())
        .chain(["manifest.json".into()])
        .collect();
    if actual != expected {
        return Err("app_package_invalid".into());
    }
    let mut total = data.len() as u64;
    for item in &m.files {
        let path = package.join(&item.path);
        let before = fs::symlink_metadata(&path).map_err(|_| "app_package_invalid")?;
        if before.len() > FILE_BYTES || !before.is_file() || before.file_type().is_symlink() {
            return Err("app_package_invalid".into());
        }
        let mut file = fs::File::open(&path).map_err(|_| "app_package_invalid")?;
        let held = file.metadata().map_err(|_| "app_package_invalid")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if held.dev() != before.dev() || held.ino() != before.ino() || held.nlink() != 1 {
                return Err("app_package_invalid".into());
            }
        }
        let mut digest = Sha256::new();
        let mut count = 0u64;
        let mut buf = [0u8; 8192];
        loop {
            let n = file.read(&mut buf).map_err(|_| "app_package_invalid")?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > FILE_BYTES || total.saturating_add(count) > PACKAGE_BYTES {
                return Err("app_package_invalid".into());
            }
            digest.update(&buf[..n]);
        }
        let after = fs::symlink_metadata(&path).map_err(|_| "app_package_invalid")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if held.dev() != after.dev() || held.ino() != after.ino() {
                return Err("app_package_invalid".into());
            }
        }
        if after.file_type().is_symlink()
            || count != before.len()
            || count != after.len()
            || format!("{:x}", digest.finalize()) != item.sha256
        {
            return Err("app_package_invalid".into());
        }
        total += count;
    }
    Ok((m, format!("{:x}", Sha256::digest(&data))))
}
