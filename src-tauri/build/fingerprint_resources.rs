//! Build-time, data-only validation for the explicitly indexed fingerprint packs.
//! No Tauri, application state, user configuration or database access.
use super::fingerprint_logic::{validate_condition, Condition, BOOLEAN_RUNTIME};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_BYTES: u64 = 5 * 1024 * 1024;
const MAX_RULES: usize = 5000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Index {
    schema_version: u32,
    packs: Vec<IndexedPack>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IndexedPack {
    file: String,
    sha256: String,
    count: usize,
    rule_sha256: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pack {
    schema_version: u32,
    kind: String,
    #[serde(default)]
    boolean_runtime: Option<String>,
    manifest: Manifest,
    fingerprints: Vec<Fingerprint>,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    name: String,
    version: String,
    #[serde(default)]
    sources: Vec<Source>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    #[serde(default)]
    name: String,
    #[serde(default)]
    repository: String,
    #[serde(default)]
    revision: String,
    #[serde(default)]
    license: String,
}
#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum MatchMode {
    #[default]
    Any,
    All,
}
fn is_any(mode: &MatchMode) -> bool {
    *mode == MatchMode::Any
}

// Field order and omission rules deliberately match the runtime serializer.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fingerprint {
    id: String,
    name: String,
    #[serde(
        default,
        deserialize_with = "unique_headers",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    header_regex: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    set_cookie_regex: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    body_regex: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    path_signals: Vec<PathSignal>,
    #[serde(default, skip_serializing_if = "is_any")]
    match_mode: MatchMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    condition: Option<Condition>,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PathSignal {
    path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    status: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body_regex: Option<String>,
}

fn unique_headers<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    struct Headers;
    impl<'de> serde::de::Visitor<'de> for Headers {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a header-pattern map without duplicate keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut access: A,
        ) -> Result<Self::Value, A::Error> {
            let mut result = BTreeMap::new();
            while let Some((name, pattern)) = access.next_entry::<String, String>()? {
                if result.insert(name, pattern).is_some() {
                    return Err(serde::de::Error::custom("Duplicate header pattern key"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Headers)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn check_digest(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid fingerprint SHA-256".into());
    }
    Ok(())
}
fn bounded(value: &str, limit: usize, required: bool) -> Result<(), String> {
    if value.len() > limit
        || value.chars().any(char::is_control)
        || (required && value.trim().is_empty())
    {
        return Err("Invalid bounded fingerprint text".into());
    }
    Ok(())
}
fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>, String> {
    let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if before.file_type().is_symlink() || !before.is_file() || before.len() > maximum {
        return Err("Fingerprint resource must be a bounded ordinary file".into());
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file() || opened.len() > maximum {
        return Err("Fingerprint resource changed before reading".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.ino() != before.ino() || opened.dev() != before.dev() {
            return Err("Fingerprint resource changed before reading".into());
        }
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > maximum {
        return Err("Fingerprint resource exceeds byte limit".into());
    }
    Ok(bytes)
}
fn resource_path(directory: &Path, relative: &str) -> Result<PathBuf, String> {
    bounded(relative, 1024, true)?;
    if relative.contains(['\\', ':'])
        || relative
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
        || Path::new(relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Unsafe fingerprint resource path".into());
    }
    let mut path = directory.to_path_buf();
    for component in Path::new(relative).components() {
        path.push(component.as_os_str());
        if fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Fingerprint resource symlinks are not allowed".into());
        }
    }
    if path.extension().and_then(|s| s.to_str()) != Some("json")
        || !path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(directory)
    {
        return Err("Fingerprint resource escapes directory or is not JSON".into());
    }
    Ok(path)
}
fn canonical_condition(condition: &mut Condition) {
    match condition {
        Condition::All { children } | Condition::Any { children } => {
            for child in children.iter_mut() {
                canonical_condition(child);
            }
            children.sort_by_cached_key(|c| serde_json::to_string(c).unwrap_or_default());
            children.dedup();
        }
        Condition::Header { name, .. } => *name = name.to_ascii_lowercase(),
        _ => {}
    }
}
fn validate_pattern(pattern: &str) -> Result<(), String> {
    validate_condition(&Condition::Body {
        pattern: pattern.into(),
    })
}
fn canonical_rule(rule: &mut Fingerprint) -> Result<(), String> {
    bounded(&rule.name, 256, true)?;
    if rule.id.is_empty()
        || rule.id.len() > 128
        || !rule
            .id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err("Invalid bundled fingerprint ID".into());
    }
    if rule.match_mode != MatchMode::Any {
        return Err("Flat AND is unsafe; use a condition tree".into());
    }
    let count = rule.header_regex.len()
        + rule.set_cookie_regex.len()
        + rule.body_regex.len()
        + rule.path_signals.len();
    if let Some(condition) = &mut rule.condition {
        if count != 0 {
            return Err("Condition tree cannot mix legacy signals".into());
        }
        validate_condition(condition)?;
        canonical_condition(condition);
        return Ok(());
    }
    if count == 0 || count > 128 || rule.path_signals.len() > 32 {
        return Err("Invalid bundled signal count".into());
    }
    let mut headers = BTreeMap::new();
    for (name, pattern) in &rule.header_regex {
        validate_condition(&Condition::Header {
            name: name.clone(),
            pattern: pattern.clone(),
        })?;
        if let Some(previous) = headers.insert(name.to_ascii_lowercase(), pattern.clone()) {
            if previous != *pattern {
                return Err("Conflicting normalized header conditions".into());
            }
        }
    }
    rule.header_regex = headers;
    for pattern in rule.body_regex.iter().chain(&rule.set_cookie_regex) {
        validate_pattern(pattern)?;
    }
    rule.body_regex.sort();
    rule.body_regex.dedup();
    rule.set_cookie_regex.sort();
    rule.set_cookie_regex.dedup();
    for signal in &mut rule.path_signals {
        bounded(&signal.path, 1024, true)?;
        let lower = signal.path.to_ascii_lowercase();
        if !signal.path.starts_with('/')
            || signal.path.starts_with("//")
            || signal
                .path
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '\\' | ':' | '?' | '#'))
            || signal.path.split('/').any(|p| p == "." || p == "..")
            || ["%2e", "%2f", "%5c", "%0d", "%0a", "%00", "%3a", "%3f"]
                .iter()
                .any(|s| lower.contains(s))
            || signal.status.iter().any(|s| !(100..=599).contains(s))
        {
            return Err("Invalid bundled characteristic path or status".into());
        }
        signal.status.sort_unstable();
        signal.status.dedup();
        validate_pattern(
            signal
                .body_regex
                .as_deref()
                .ok_or("Path must have a body regex")?,
        )?;
    }
    rule.path_signals.sort_by(|a, b| {
        (&a.path, &a.status, &a.body_regex).cmp(&(&b.path, &b.status, &b.body_regex))
    });
    rule.path_signals.dedup();
    Ok(())
}
fn canonical_pack(mut pack: Pack) -> Result<(Vec<Fingerprint>, String), String> {
    if pack.kind != "oviraptor-fingerprints"
        || ![1, 2].contains(&pack.schema_version)
        || (pack.schema_version == 2 && pack.boolean_runtime.as_deref() != Some(BOOLEAN_RUNTIME))
        || (pack.schema_version == 1 && pack.boolean_runtime.is_some())
    {
        return Err("Unsupported bundled pack schema/runtime".into());
    }
    bounded(&pack.manifest.name, 256, true)?;
    bounded(&pack.manifest.version, 64, true)?;
    if pack.manifest.sources.len() > 128 {
        return Err("Too many fingerprint source records".into());
    }
    for source in &pack.manifest.sources {
        bounded(&source.name, 256, false)?;
        bounded(&source.repository, 1024, false)?;
        bounded(&source.revision, 128, false)?;
        bounded(&source.license, 128, false)?;
        if !source.repository.is_empty() {
            let authority = source
                .repository
                .strip_prefix("https://")
                .ok_or("Source must use HTTPS")?
                .split('/')
                .next()
                .unwrap_or("");
            if authority.is_empty()
                || authority.contains('@')
                || source.repository.contains(['?', '#'])
            {
                return Err("Source repository cannot contain credentials/query/fragment".into());
            }
        }
    }
    if pack.fingerprints.is_empty() || pack.fingerprints.len() > MAX_RULES {
        return Err("Invalid bundled rule count".into());
    }
    let mut ids = HashSet::new();
    for rule in &mut pack.fingerprints {
        canonical_rule(rule)?;
        if !ids.insert(rule.id.clone()) {
            return Err("Duplicate bundled fingerprint ID".into());
        }
        if pack.schema_version == 1 && rule.condition.is_some() {
            return Err("Condition trees require schemaVersion 2".into());
        }
    }
    pack.fingerprints.sort_by(|a, b| a.id.cmp(&b.id));
    let actual = digest(&serde_json::to_vec(&pack.fingerprints).map_err(|e| e.to_string())?);
    check_digest(&pack.sha256)?;
    if !actual.eq_ignore_ascii_case(&pack.sha256) {
        return Err("Bundled rule-content SHA-256 mismatch".into());
    }
    Ok((pack.fingerprints, actual))
}

/// Validate only the explicit resource index. Returns the aggregate unique count.
pub fn verify(directory: &Path) -> Result<usize, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Fingerprint resources must be an ordinary directory".into());
    }
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let manifest: Index = serde_json::from_slice(&read_bounded(
        &directory.join("manifest.json"),
        1024 * 1024,
    )?)
    .map_err(|e| format!("Invalid fingerprint index: {e}"))?;
    if manifest.schema_version != 1 || manifest.packs.is_empty() || manifest.packs.len() > 64 {
        return Err("Unsupported or empty fingerprint resource index".into());
    }
    let mut files = HashSet::new();
    let mut ids = HashSet::new();
    for entry in manifest.packs {
        if !files.insert(entry.file.clone()) || entry.count == 0 || entry.count > MAX_RULES {
            return Err("Duplicate indexed file or invalid count".into());
        }
        check_digest(&entry.sha256)?;
        check_digest(&entry.rule_sha256)?;
        let raw = read_bounded(&resource_path(&directory, &entry.file)?, MAX_BYTES)?;
        if !digest(&raw).eq_ignore_ascii_case(&entry.sha256) {
            return Err(format!("Bundled file SHA-256 mismatch: {}", entry.file));
        }
        let pack: Pack = serde_json::from_slice(&raw)
            .map_err(|e| format!("Invalid identification pack: {e}"))?;
        let (rules, rule_digest) = canonical_pack(pack)?;
        if rules.len() != entry.count || !rule_digest.eq_ignore_ascii_case(&entry.rule_sha256) {
            return Err("Indexed count or rule-content hash mismatch".into());
        }
        for rule in rules {
            if !ids.insert(rule.id) {
                return Err("Duplicate fingerprint ID across indexed packs".into());
            }
        }
        if ids.len() > MAX_RULES {
            return Err("Aggregate bundled fingerprint count exceeds limit".into());
        }
    }
    Ok(ids.len())
}
