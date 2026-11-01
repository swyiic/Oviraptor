//! Standalone, identification-only fingerprint packages. No database or CVE corpus.
//! Imports validate the entire input before merging or writing any user rules.

use super::fingerprint_logic::{validate_condition, Condition, BOOLEAN_RUNTIME};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub(crate) const MAX_PACK_BYTES: usize = 5 * 1024 * 1024;
const MAX_RULES: usize = 5_000;
const PACK_KIND: &str = "oviraptor-fingerprints";
static IMPORT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MatchMode {
    #[default]
    Any,
    All,
}
fn is_any(mode: &MatchMode) -> bool {
    *mode == MatchMode::Any
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Fingerprint {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub header_regex: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set_cookie_regex: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_regex: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_signals: Vec<PathSignal>,
    #[serde(default, skip_serializing_if = "is_any")]
    pub match_mode: MatchMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PathSignal {
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_regex: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PackSource {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub repository: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub revision: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    name: String,
    version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sources: Vec<PackSource>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FingerprintPack {
    schema_version: u32,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    boolean_runtime: Option<String>,
    manifest: Manifest,
    fingerprints: Vec<Fingerprint>,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyCatalog {
    #[serde(default)]
    schema_version: Option<u32>,
    #[serde(default)]
    boolean_runtime: Option<String>,
    #[serde(default)]
    note: Option<String>,
    fingerprints: Vec<Fingerprint>,
}

#[derive(Debug)]
struct ParsedPack {
    manifest: Manifest,
    fingerprints: Vec<Fingerprint>,
    duplicates: usize,
    legacy: bool,
    sha256: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn rules_digest(rules: &[Fingerprint]) -> Result<String, String> {
    serde_json::to_vec(rules)
        .map(|bytes| digest(&bytes))
        .map_err(|e| e.to_string())
}
fn bounded_text(text: &str, maximum: usize, label: &str, required: bool) -> Result<(), String> {
    if text.len() > maximum
        || text.chars().any(char::is_control)
        || (required && text.trim().is_empty())
    {
        return Err(format!("{label}为空、过长或含控制字符"));
    }
    Ok(())
}
fn validate_regex(pattern: &str, label: &str) -> Result<(), String> {
    if pattern.is_empty() || pattern.len() > 4096 {
        return Err(format!("{label}正则为空或超过 4096 字节"));
    }
    let regex = regex::RegexBuilder::new(pattern)
        .size_limit(2 * 1024 * 1024)
        .dfa_size_limit(2 * 1024 * 1024)
        .nest_limit(64)
        .build()
        .map_err(|e| format!("{label}正则无效：{e}"))?;
    if regex.is_match("") {
        return Err(format!("{label}正则不能匹配空字符串"));
    }
    Ok(())
}
fn validate_path(path: &str) -> Result<(), String> {
    let lower = path.to_ascii_lowercase();
    if path.len() > 1024
        || !path.starts_with('/')
        || path.starts_with("//")
        || path
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\\' | ':' | '?' | '#'))
        || path.split('/').any(|p| p == "." || p == "..")
        || ["%2e", "%2f", "%5c", "%0d", "%0a", "%00", "%3a", "%3f"]
            .iter()
            .any(|p| lower.contains(p))
    {
        return Err(format!(
            "不安全的识别路径：{path}；仅支持站内绝对路径，不支持 URL、查询参数或路径穿越"
        ));
    }
    Ok(())
}

fn canonicalize_rule(rule: &mut Fingerprint) -> Result<(), String> {
    if rule.id.is_empty()
        || rule.id.len() > 128
        || !rule
            .id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return Err("指纹 ID 仅允许 1–128 位字母、数字、点、下划线或连字符".into());
    }
    bounded_text(&rule.name, 256, "产品名称", true)?;
    if let Some(condition) = &mut rule.condition {
        if !rule.header_regex.is_empty()
            || !rule.set_cookie_regex.is_empty()
            || !rule.body_regex.is_empty()
            || !rule.path_signals.is_empty()
            || rule.match_mode != MatchMode::Any
        {
            return Err("条件树不能混入旧式信号或 matchMode".into());
        }
        if !rule
            .id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("条件树 ID 仅支持小写字母、数字和连字符".into());
        }
        validate_condition(condition)?;
        canonicalize_condition(condition);
        return Ok(());
    }
    let count = rule.header_regex.len()
        + rule.set_cookie_regex.len()
        + rule.body_regex.len()
        + rule.path_signals.len();
    if count == 0 || count > 128 || rule.path_signals.len() > 32 {
        return Err(format!(
            "指纹 {} 必须有识别信号，且信号不超过 128 个、路径不超过 32 个",
            rule.id
        ));
    }
    let mut headers = BTreeMap::new();
    for (name, pattern) in &rule.header_regex {
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&c))
        {
            return Err(format!("指纹 {} 的 Header 名无效", rule.id));
        }
        validate_regex(pattern, &rule.id)?;
        let key = name.to_ascii_lowercase();
        if let Some(old) = headers.insert(key, pattern.clone()) {
            if old != *pattern {
                return Err(format!("指纹 {} 的同名 Header 条件冲突", rule.id));
            }
        }
    }
    rule.header_regex = headers;
    for pattern in rule.set_cookie_regex.iter().chain(&rule.body_regex) {
        validate_regex(pattern, &rule.id)?;
    }
    rule.set_cookie_regex.sort();
    rule.set_cookie_regex.dedup();
    rule.body_regex.sort();
    rule.body_regex.dedup();
    for signal in &mut rule.path_signals {
        validate_path(&signal.path)?;
        if signal.status.iter().any(|s| !(100..=599).contains(s)) {
            return Err(format!("指纹 {} 的 HTTP 状态码无效", rule.id));
        }
        signal.status.sort_unstable();
        signal.status.dedup();
        let pattern = signal.body_regex.as_ref().ok_or_else(|| {
            format!(
                "指纹 {} 的路径信号必须有 bodyRegex，不能仅凭状态码识别",
                rule.id
            )
        })?;
        validate_regex(pattern, &rule.id)?;
    }
    rule.path_signals.sort_by(|a, b| {
        (&a.path, &a.status, &a.body_regex).cmp(&(&b.path, &b.status, &b.body_regex))
    });
    rule.path_signals.dedup();
    Ok(())
}

fn canonicalize_condition(condition: &mut Condition) {
    match condition {
        Condition::All { children } | Condition::Any { children } => {
            for child in children.iter_mut() {
                canonicalize_condition(child);
            }
            children.sort_by_cached_key(|child| serde_json::to_string(child).unwrap_or_default());
            children.dedup();
        }
        Condition::Header { name, .. } => *name = name.to_ascii_lowercase(),
        _ => {}
    }
}

fn canonicalize(rules: Vec<Fingerprint>) -> Result<(Vec<Fingerprint>, usize), String> {
    if rules.len() > MAX_RULES {
        return Err(format!("指纹包超过 {MAX_RULES} 条规则上限"));
    }
    let mut by_id = BTreeMap::new();
    let mut duplicates = 0;
    for mut rule in rules {
        canonicalize_rule(&mut rule)?;
        if let Some(existing) = by_id.get(&rule.id) {
            if existing != &rule {
                return Err(format!(
                    "同一包内 ID {} 对应不同规则，请先消除冲突",
                    rule.id
                ));
            }
            duplicates += 1;
        } else {
            by_id.insert(rule.id.clone(), rule);
        }
    }
    Ok((by_id.into_values().collect(), duplicates))
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    bounded_text(&manifest.name, 256, "指纹包名称", true)?;
    bounded_text(&manifest.version, 64, "指纹包版本", true)?;
    if manifest.sources.len() > 128 {
        return Err("来源记录超过 128 条".into());
    }
    for source in &manifest.sources {
        bounded_text(&source.name, 256, "来源名称", false)?;
        bounded_text(&source.repository, 1024, "来源仓库", false)?;
        bounded_text(&source.revision, 128, "来源修订", false)?;
        bounded_text(&source.license, 128, "来源许可证", false)?;
        if !source.repository.is_empty() && !source.repository.starts_with("https://") {
            return Err("来源仓库只支持 HTTPS 地址".into());
        }
        if source.repository.starts_with("https://") {
            let authority = source
                .repository
                .trim_start_matches("https://")
                .split('/')
                .next()
                .unwrap_or("");
            if authority.is_empty()
                || authority.contains('@')
                || source.repository.contains('?')
                || source.repository.contains('#')
            {
                return Err("来源仓库地址不得包含凭证、查询参数或片段".into());
            }
        }
    }
    Ok(())
}

fn parse_pack(content: &str) -> Result<ParsedPack, String> {
    if content.len() > MAX_PACK_BYTES {
        return Err("指纹包超过 5 MiB 上限".into());
    }
    let shape: Value = serde_json::from_str(content).map_err(|e| format!("JSON 无效：{e}"))?;
    let (manifest, raw_rules, expected, legacy, schema, runtime) = if shape.get("kind").is_some() {
        let pack: FingerprintPack = serde_json::from_str(content)
            .map_err(|e| format!("独立指纹包格式错误（不接受 CVE、PoC 或检测模板字段）：{e}"))?;
        if pack.kind != PACK_KIND || ![1, 2].contains(&pack.schema_version) {
            return Err("仅支持 schemaVersion=1/2 的 oviraptor-fingerprints 指纹包".into());
        }
        validate_manifest(&pack.manifest)?;
        (
            pack.manifest,
            pack.fingerprints,
            Some(pack.sha256),
            false,
            pack.schema_version,
            pack.boolean_runtime,
        )
    } else {
        let catalog: LegacyCatalog = serde_json::from_str(content).map_err(|e| {
            format!("旧指纹格式错误（仅接受识别字段，不接受 CVE、PoC 或检测模板）：{e}")
        })?;
        if catalog.schema_version.is_some_and(|v| ![1, 2].contains(&v)) {
            return Err("不支持的旧指纹 schemaVersion".into());
        }
        if catalog.note.as_ref().is_some_and(|note| note.len() > 4096) {
            return Err("旧指纹说明超过 4096 字节".into());
        }
        (
            Manifest {
                name: "Imported fingerprints".into(),
                version: "1".into(),
                sources: vec![],
            },
            catalog.fingerprints,
            None,
            true,
            catalog.schema_version.unwrap_or(1),
            catalog.boolean_runtime,
        )
    };
    let has_tree = raw_rules.iter().any(|rule| rule.condition.is_some());
    if (has_tree && schema != 2) || (schema == 2 && runtime.as_deref() != Some(BOOLEAN_RUNTIME)) {
        return Err("条件树要求 schemaVersion=2 且 booleanRuntime=condition-tree-v1".into());
    }
    if runtime
        .as_deref()
        .is_some_and(|value| value != BOOLEAN_RUNTIME)
        || (schema == 1 && runtime.is_some())
    {
        return Err("不支持的条件树运行时标记".into());
    }
    let (fingerprints, duplicates) = canonicalize(raw_rules)?;
    let sha256 = rules_digest(&fingerprints)?;
    if let Some(expected) = expected {
        if expected.len() != 64 || !expected.eq_ignore_ascii_case(&sha256) {
            return Err("指纹包 SHA-256 校验失败，内容可能损坏或已修改".into());
        }
    }
    Ok(ParsedPack {
        manifest,
        fingerprints,
        duplicates,
        legacy,
        sha256,
    })
}

include!(concat!(env!("OUT_DIR"), "/fingerprint_pack_resources.rs"));

#[derive(Clone)]
struct BundledCatalog {
    rules: Vec<Fingerprint>,
    sources: Vec<PackSource>,
}
fn bundled_catalog() -> Result<&'static BundledCatalog, String> {
    static CACHE: std::sync::OnceLock<Result<BundledCatalog, String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let mut rules = Vec::new();
            let mut sources = Vec::new();
            let mut ids = std::collections::HashSet::new();
            for &(file, content, file_sha, count, rule_sha) in BUNDLED_FINGERPRINT_PACKS {
                if digest(content.as_bytes()) != file_sha {
                    return Err(format!("内置指纹包文件校验失败：{file}"));
                }
                let parsed =
                    parse_pack(content).map_err(|error| format!("内置指纹包 {file}：{error}"))?;
                if parsed.fingerprints.len() != count
                    || parsed.duplicates != 0
                    || parsed.sha256 != rule_sha
                {
                    return Err(format!("内置指纹包数量或规则校验值不匹配：{file}"));
                }
                if parsed
                    .fingerprints
                    .iter()
                    .any(|rule| rule.match_mode != MatchMode::Any)
                {
                    return Err(format!("内置包不能包含未转换的 matchMode=all：{file}"));
                }
                for rule in parsed.fingerprints {
                    if !ids.insert(rule.id.clone()) {
                        return Err(format!("内置指纹包 ID 重复：{}", rule.id));
                    }
                    rules.push(rule);
                }
                for source in parsed.manifest.sources {
                    if !sources.contains(&source) {
                        sources.push(source);
                    }
                }
            }
            Ok(BundledCatalog { rules, sources })
        })
        .as_ref()
        .map_err(Clone::clone)
}
pub(crate) fn bundled_fingerprints_json() -> Result<Value, String> {
    serde_json::to_value(&bundled_catalog()?.rules).map_err(|e| e.to_string())
}
pub(crate) fn builtin_fingerprints() -> Result<Vec<Fingerprint>, String> {
    // Starter definitions remain authoritative; packages add new default IDs.
    let mut rules =
        parse_pack(include_str!("../../resources/rules/fingerprints.json"))?.fingerprints;
    let mut seen: std::collections::HashSet<String> =
        rules.iter().map(|rule| rule.id.clone()).collect();
    for rule in &bundled_catalog()?.rules {
        if seen.insert(rule.id.clone()) {
            rules.push(rule.clone());
        }
    }
    if rules.len() > MAX_RULES {
        return Err("默认指纹超过 5000 条上限".into());
    }
    Ok(rules)
}

/// Canonical formatting never imports into a profile or drops builtin duplicates.
pub(crate) fn normalize_pack(text: &str, boolean_only: bool) -> Result<Value, String> {
    let parsed = parse_pack(text)?;
    if boolean_only
        && parsed
            .fingerprints
            .iter()
            .any(|rule| rule.condition.is_none())
    {
        return Err("布尔规范化只接受条件树".into());
    }
    if parsed
        .fingerprints
        .iter()
        .any(|rule| rule.match_mode != MatchMode::Any)
    {
        return Err("请将平面 all 模式转换为显式条件树".into());
    }
    let count = parsed.fingerprints.len();
    let (content, sha256) = package_content(
        &parsed.manifest.name,
        parsed.manifest.sources,
        parsed.fingerprints,
    )?;
    Ok(json!({"content":content,"count":count,"sha256":sha256}))
}
pub(crate) fn user_rules_path(data_dir: &Path) -> PathBuf {
    data_dir.join("fingerprints.user.json")
}

fn read_current(data_dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = user_rules_path(data_dir);
    match fs::symlink_metadata(&path) {
        Ok(meta) => {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err("用户指纹路径必须是普通文件，不能是目录或符号链接".into());
            }
            if meta.len() > MAX_PACK_BYTES as u64 {
                return Err("用户指纹文件超过 5 MiB 上限".into());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("读取用户指纹失败：{e}")),
    }
    let file = fs::File::open(path).map_err(|e| format!("读取用户指纹失败：{e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_PACK_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_PACK_BYTES {
        return Err("用户指纹文件超过 5 MiB 上限".into());
    }
    Ok(Some(bytes))
}
fn parsed_current(data_dir: &Path) -> Result<(Option<Vec<u8>>, Option<ParsedPack>), String> {
    let bytes = read_current(data_dir)?;
    let parsed = bytes
        .as_ref()
        .map(|b| {
            let text = std::str::from_utf8(b).map_err(|_| "用户指纹不是 UTF-8 JSON".to_string())?;
            parse_pack(text).map_err(|e| format!("现有用户指纹无效，已保留原文件：{e}"))
        })
        .transpose()?;
    Ok((bytes, parsed))
}
fn revision(bytes: Option<&[u8]>) -> String {
    bytes.map(digest).unwrap_or_else(|| "missing".into())
}

pub(crate) fn load_user_fingerprints(data_dir: Option<&Path>) -> Result<Vec<Fingerprint>, String> {
    let Some(dir) = data_dir else {
        return Ok(vec![]);
    };
    let mut legacy = parsed_current(dir)?
        .1
        .map(|p| p.fingerprints)
        .unwrap_or_default();
    let legacy_ids: std::collections::HashSet<_> =
        legacy.iter().map(|rule| rule.id.clone()).collect();
    for rule in boolean_current(dir)?.1 {
        if legacy_ids.contains(&rule.id) {
            return Err(format!("条件树与旧规则 ID 冲突：{}", rule.id));
        }
        legacy.push(rule);
    }
    Ok(legacy)
}
pub(crate) fn effective_fingerprints(data_dir: Option<&Path>) -> Result<Vec<Fingerprint>, String> {
    merge_with_builtin(load_user_fingerprints(data_dir)?)
}

fn merge_with_builtin(user_rules: Vec<Fingerprint>) -> Result<Vec<Fingerprint>, String> {
    let mut rules: BTreeMap<String, Fingerprint> = builtin_fingerprints()?
        .into_iter()
        .map(|r| (r.id.clone(), r))
        .collect();
    for rule in user_rules {
        rules.insert(rule.id.clone(), rule);
    }
    if rules.len() > MAX_RULES {
        return Err("合并后的指纹超过 5000 条上限".into());
    }
    Ok(rules.into_values().collect())
}

struct MergePlan {
    preview: Value,
    user_rules: Vec<Fingerprint>,
    sources: Vec<PackSource>,
    old_bytes: Option<Vec<u8>>,
    changed: bool,
}
fn merge_plan(
    data_dir: &Path,
    content: &str,
    replace_conflicts: bool,
) -> Result<MergePlan, String> {
    let incoming = parse_pack(content)?;
    if incoming
        .fingerprints
        .iter()
        .any(|rule| rule.condition.is_some())
    {
        return Err("条件树必须通过独立布尔指纹导入，不能写入旧规则文件".into());
    }
    if incoming.fingerprints.is_empty() {
        return Err("待导入的指纹包为空".into());
    }
    let (old_bytes, old_pack) = parsed_current(data_dir)?;
    let mut user: BTreeMap<String, Fingerprint> = old_pack
        .as_ref()
        .map(|p| p.fingerprints.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|r| (r.id.clone(), r))
        .collect();
    let mut effective: BTreeMap<String, Fingerprint> = builtin_fingerprints()?
        .into_iter()
        .map(|r| (r.id.clone(), r))
        .collect();
    effective.extend(user.clone());
    let mut added = 0;
    let mut updated = 0;
    let mut unchanged = 0;
    let mut skipped = 0;
    for rule in &incoming.fingerprints {
        match effective.get(&rule.id) {
            Some(existing) if existing == rule => {
                unchanged += 1;
            }
            Some(_) if !replace_conflicts => {
                skipped += 1;
            }
            old => {
                if old.is_some() {
                    updated += 1;
                } else {
                    added += 1;
                }
                user.insert(rule.id.clone(), rule.clone());
                effective.insert(rule.id.clone(), rule.clone());
            }
        }
    }
    if effective.len() > MAX_RULES {
        return Err("合并后的指纹超过 5000 条上限".into());
    }
    let old_sources = old_pack
        .as_ref()
        .map(|p| p.manifest.sources.clone())
        .unwrap_or_default();
    let mut sources = old_sources.clone();
    // Record provenance for accepted or identical rules, including builtins.
    // A wholly skipped conflict must not attribute the retained rule to this pack.
    if added + updated + unchanged > 0 {
        for source in &incoming.manifest.sources {
            if !sources.contains(source) {
                sources.push(source.clone());
            }
        }
    }
    let metadata_updated = sources != old_sources;
    if sources.len() > 128 {
        return Err("合并后的来源记录超过 128 条上限".into());
    }
    let mut warnings = vec![];
    if incoming.legacy {
        warnings.push("旧格式将转换为独立指纹包；说明文本不会导出".to_string());
    }
    if incoming.manifest.sources.is_empty() {
        warnings.push("该包未填写来源及许可证信息".to_string());
    }
    if skipped > 0 {
        warnings.push(format!("{skipped} 条同 ID 不同内容规则将保留现有版本"));
    }
    if metadata_updated {
        warnings.push("将保存或补充已接受规则的来源元数据".to_string());
    }
    let user_rules: Vec<Fingerprint> = user.into_values().collect();
    // Validate final serialization before the user confirms, not only at write time.
    package_content(
        "Oviraptor user fingerprints",
        sources.clone(),
        user_rules.clone(),
    )?;
    Ok(MergePlan {
        preview: json!({"revision": revision(old_bytes.as_deref()), "name": incoming.manifest.name, "version": incoming.manifest.version, "sha256": incoming.sha256, "incoming": incoming.fingerprints.len(), "added": added, "updated": updated, "unchanged": unchanged, "skipped": skipped, "duplicates": incoming.duplicates, "total": effective.len(), "metadataUpdated": metadata_updated, "warnings": warnings}),
        user_rules,
        sources,
        old_bytes,
        changed: added + updated > 0 || metadata_updated,
    })
}

fn package_content(
    name: &str,
    sources: Vec<PackSource>,
    rules: Vec<Fingerprint>,
) -> Result<(String, String), String> {
    let (fingerprints, _) = canonicalize(rules)?;
    let sha256 = rules_digest(&fingerprints)?;
    let has_tree = fingerprints.iter().any(|rule| rule.condition.is_some());
    let pack = FingerprintPack {
        schema_version: if has_tree { 2 } else { 1 },
        kind: PACK_KIND.into(),
        boolean_runtime: has_tree.then(|| BOOLEAN_RUNTIME.to_string()),
        manifest: Manifest {
            name: name.into(),
            version: "1".into(),
            sources,
        },
        fingerprints,
        sha256: sha256.clone(),
    };
    let content = serde_json::to_string_pretty(&pack).map_err(|e| e.to_string())?;
    if content.len() > MAX_PACK_BYTES {
        return Err("合并或导出的指纹包超过 5 MiB 上限".into());
    }
    Ok((content, sha256))
}

fn atomic_write(data_dir: &Path, content: &[u8], old_bytes: Option<&[u8]>) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let temporary = data_dir.join(format!(".fingerprints.{}.tmp", uuid::Uuid::new_v4()));
    let result: Result<(), String> = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(content)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        if let Some(old) = old_bytes {
            let backup = data_dir.join(format!(
                "fingerprints.user.{}.bak.json",
                uuid::Uuid::new_v4()
            ));
            let mut backup_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(backup)
                .map_err(|e| e.to_string())?;
            backup_file
                .write_all(old)
                .and_then(|_| backup_file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        // Do not overwrite an externally edited file after validation/preview.
        let current = read_current(data_dir)?;
        if current.as_deref() != old_bytes {
            return Err("用户指纹已变化，请重新预览后导入".into());
        }
        fs::rename(&temporary, user_rules_path(data_dir))
            .map_err(|e| format!("启用指纹包失败：{e}"))?;
        #[cfg(unix)]
        fs::File::open(data_dir)
            .and_then(|directory| directory.sync_all())
            .map_err(|e| format!("指纹已启用，但目录同步失败，请重新加载确认：{e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub(crate) fn preview_pack(
    data_dir: &Path,
    content: &str,
    replace_conflicts: bool,
) -> Result<Value, String> {
    merge_plan(data_dir, content, replace_conflicts).map(|p| p.preview)
}
pub(crate) fn import_pack(
    data_dir: &Path,
    content: &str,
    replace_conflicts: bool,
    expected_revision: &str,
) -> Result<Value, String> {
    let _guard = IMPORT_LOCK
        .lock()
        .map_err(|_| "指纹导入锁不可用".to_string())?;
    let mut plan = merge_plan(data_dir, content, replace_conflicts)?;
    if plan.preview["revision"].as_str() != Some(expected_revision) {
        return Err("用户指纹已变化，请重新预览后导入".into());
    }
    if plan.changed {
        let (content, _) =
            package_content("Oviraptor user fingerprints", plan.sources, plan.user_rules)?;
        atomic_write(data_dir, content.as_bytes(), plan.old_bytes.as_deref())?;
        plan.preview["revision"] = Value::String(digest(content.as_bytes()));
    }
    Ok(plan.preview)
}
pub(crate) fn export_pack(data_dir: &Path, scope: &str) -> Result<Value, String> {
    // Metadata and rules must come from one file snapshot. An import or manual
    // edit between independent reads must not pair old sources with new rules.
    let (_, current) = parsed_current(data_dir)?;
    let (mut sources, user_rules) = current
        .map(|pack| (pack.manifest.sources, pack.fingerprints))
        .unwrap_or_default();
    if scope == "effective" {
        for source in &bundled_catalog()?.sources {
            if !sources.contains(source) {
                sources.push(source.clone());
            }
        }
    }
    let mut user_rules = user_rules;
    let old_ids: std::collections::HashSet<_> =
        user_rules.iter().map(|rule| rule.id.clone()).collect();
    for rule in boolean_current(data_dir)?.1 {
        if old_ids.contains(&rule.id) {
            return Err(format!("条件树与旧规则 ID 冲突：{}", rule.id));
        }
        user_rules.push(rule);
    }
    let rules = match scope {
        "user" => user_rules,
        "boolean" => user_rules
            .into_iter()
            .filter(|rule| rule.condition.is_some())
            .collect(),
        "effective" => merge_with_builtin(user_rules)?,
        _ => return Err("导出范围仅支持 user 或 effective".into()),
    };
    let count = rules.len();
    let (content, sha256) = package_content("Oviraptor fingerprints", sources, rules)?;
    Ok(
        json!({"fileName": format!("oviraptor-fingerprints-{scope}.json"), "content": content, "count": count, "sha256": sha256}),
    )
}

/// Check a standalone pack and return canonical rule values to the live parser.
pub(crate) fn validated_pack_rules(text: &str) -> Result<Value, String> {
    let parsed = parse_pack(text)?;
    if parsed
        .fingerprints
        .iter()
        .any(|rule| rule.match_mode != MatchMode::Any)
    {
        return Err(
            "平面 matchMode=all 不能交给旧式任一匹配器；请使用 schemaVersion=2 条件树".into(),
        );
    }
    serde_json::to_value(parsed.fingerprints).map_err(|e| e.to_string())
}

pub(crate) fn validate_rule_values(values: Vec<Value>) -> Vec<Value> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let id = value.get("id").cloned().unwrap_or(Value::Null);
            let result = serde_json::from_value::<Fingerprint>(value)
                .map_err(|e| e.to_string())
                .and_then(|mut rule| canonicalize_rule(&mut rule));
            match result {
                Ok(()) => json!({"index":index,"id":id,"valid":true}),
                Err(error) => json!({"index":index,"id":id,"valid":false,"error":error}),
            }
        })
        .collect()
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BooleanCatalog {
    schema_version: u32,
    boolean_runtime: String,
    fingerprints: Vec<Fingerprint>,
}
pub(crate) fn boolean_rules_path(dir: &Path) -> PathBuf {
    dir.join("fingerprints.boolean.json")
}
fn boolean_bytes(dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = boolean_rules_path(dir);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(metadata)
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_PACK_BYTES as u64 =>
        {
            return Err("布尔指纹必须是大小不超过 5 MiB 的普通文件".into())
        }
        _ => {}
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_PACK_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_PACK_BYTES {
        return Err("布尔指纹超过 5 MiB".into());
    }
    Ok(Some(bytes))
}
fn boolean_current(dir: &Path) -> Result<(Option<Vec<u8>>, Vec<Fingerprint>), String> {
    let bytes = boolean_bytes(dir)?;
    let Some(content) = bytes.as_ref() else {
        return Ok((None, vec![]));
    };
    let doc: BooleanCatalog = serde_json::from_slice(content)
        .map_err(|e| format!("现有布尔指纹无效，保留原文件：{e}"))?;
    if doc.schema_version != 2 || doc.boolean_runtime != BOOLEAN_RUNTIME {
        return Err("布尔指纹版本不支持".into());
    }
    let (rules, _) = canonicalize(doc.fingerprints)?;
    if rules.iter().any(|rule| rule.condition.is_none()) {
        return Err("布尔文件只接受条件树".into());
    }
    Ok((bytes, rules))
}
fn boolean_plan(
    dir: &Path,
    text: &str,
) -> Result<(Value, Vec<Fingerprint>, Option<Vec<u8>>), String> {
    let incoming = parse_pack(text)?;
    if incoming.fingerprints.is_empty()
        || incoming
            .fingerprints
            .iter()
            .any(|rule| rule.condition.is_none())
    {
        return Err("增量布尔导入只接受非空条件树规则包".into());
    }
    let (old_bytes, old) = boolean_current(dir)?;
    let legacy = parsed_current(dir)?
        .1
        .map(|p| p.fingerprints)
        .unwrap_or_default();
    let mut effective: BTreeMap<String, Fingerprint> = builtin_fingerprints()?
        .into_iter()
        .chain(legacy)
        .chain(old.clone())
        .map(|r| (r.id.clone(), r))
        .collect();
    let mut boolean: BTreeMap<String, Fingerprint> =
        old.into_iter().map(|r| (r.id.clone(), r)).collect();
    let (mut added, mut unchanged, mut skipped) = (0, 0, 0);
    for rule in &incoming.fingerprints {
        match effective.get(&rule.id) {
            Some(existing) if existing == rule => unchanged += 1,
            Some(_) => skipped += 1,
            None => {
                added += 1;
                boolean.insert(rule.id.clone(), rule.clone());
                effective.insert(rule.id.clone(), rule.clone());
            }
        }
    }
    if effective.len() > MAX_RULES {
        return Err("合并后超过 5000 条指纹上限".into());
    }
    let rules: Vec<_> = boolean.into_values().collect();
    let doc = BooleanCatalog {
        schema_version: 2,
        boolean_runtime: BOOLEAN_RUNTIME.into(),
        fingerprints: rules.clone(),
    };
    if serde_json::to_vec_pretty(&doc)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_PACK_BYTES
    {
        return Err("合并后布尔指纹超过 5 MiB".into());
    }
    let preview = json!({"revision":revision(old_bytes.as_deref()),"name":incoming.manifest.name,"version":incoming.manifest.version,"sha256":incoming.sha256,"incoming":incoming.fingerprints.len(),"added":added,"updated":0,"unchanged":unchanged,"skipped":skipped,"duplicates":incoming.duplicates,"total":effective.len(),"warnings":if skipped>0 {vec![format!("{skipped} 个现有 ID 已保留")]} else {vec![]}});
    Ok((preview, rules, old_bytes))
}
pub(crate) fn preview_boolean_pack(dir: &Path, text: &str) -> Result<Value, String> {
    boolean_plan(dir, text).map(|p| p.0)
}
pub(crate) fn import_boolean_pack(dir: &Path, text: &str, expected: &str) -> Result<Value, String> {
    let _lock = IMPORT_LOCK.lock().map_err(|_| "指纹导入锁不可用")?;
    let (mut preview, rules, old) = boolean_plan(dir, text)?;
    if preview["revision"].as_str() != Some(expected) {
        return Err("布尔指纹已变化，请重新预览".into());
    }
    if preview["added"].as_u64().unwrap_or(0) == 0 {
        return Ok(preview);
    }
    let doc = BooleanCatalog {
        schema_version: 2,
        boolean_runtime: BOOLEAN_RUNTIME.into(),
        fingerprints: rules,
    };
    let content = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let temporary = dir.join(format!(
        ".fingerprints-boolean-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let result: Result<(), String> = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&content)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        if let Some(previous) = old.as_ref() {
            let backup = dir.join(format!(
                "fingerprints.boolean.{}.bak.json",
                uuid::Uuid::new_v4()
            ));
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(backup)
                .map_err(|e| e.to_string())?;
            file.write_all(previous)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        if boolean_bytes(dir)? != old {
            return Err("布尔指纹在保存前变化，请重新预览".into());
        }
        fs::rename(&temporary, boolean_rules_path(dir)).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        fs::File::open(dir)
            .and_then(|f| f.sync_all())
            .map_err(|e| format!("规则已保存，但目录同步失败：{e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    preview["revision"] = Value::String(digest(&content));
    Ok(preview)
}

#[tauri::command]
pub async fn scan_preview_fingerprint_pack(
    state: tauri::State<'_, crate::AppState>,
    content: String,
    replace_conflicts: bool,
) -> Result<Value, String> {
    let directory = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        preview_pack(&directory, &content, replace_conflicts)
    })
    .await
    .map_err(|e| format!("指纹预览任务失败：{e}"))?
}
#[tauri::command]
pub async fn scan_import_fingerprint_pack(
    state: tauri::State<'_, crate::AppState>,
    content: String,
    replace_conflicts: bool,
    expected_revision: String,
) -> Result<Value, String> {
    let directory = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        import_pack(&directory, &content, replace_conflicts, &expected_revision)
    })
    .await
    .map_err(|e| format!("指纹导入任务失败：{e}"))?
}
#[tauri::command]
pub async fn scan_export_fingerprint_pack(
    state: tauri::State<'_, crate::AppState>,
    scope: String,
) -> Result<Value, String> {
    let directory = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || export_pack(&directory, &scope))
        .await
        .map_err(|e| format!("指纹导出任务失败：{e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "oviraptor-fingerprint-test-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn sample(id: &str, pattern: &str) -> Value {
        json!({"id":id,"name":id,"bodyRegex":[pattern]})
    }
    fn legacy(rules: Vec<Value>) -> String {
        json!({"schemaVersion":1,"fingerprints":rules}).to_string()
    }
    fn import(dir: &Path, content: &str, replace: bool) -> Value {
        let p = preview_pack(dir, content, replace).unwrap();
        import_pack(dir, content, replace, p["revision"].as_str().unwrap()).unwrap()
    }

    fn tree_pack(id: &str) -> String {
        json!({"schemaVersion":2,"booleanRuntime":BOOLEAN_RUNTIME,"fingerprints":[{
            "id":id,"name":"Example","condition":{"type":"all","children":[
                {"type":"body","pattern":"BrandMarker"},
                {"type":"any","children":[{"type":"body","pattern":"FeatureA"},{"type":"header","name":"X-Feature","pattern":"FeatureB"}]}
            ]}
        }]}).to_string()
    }

    #[test]
    fn boolean_import_is_additive_and_never_changes_legacy_bytes() {
        let dir = TempDir::new();
        import(
            &dir.0,
            &legacy(vec![sample("old-rule", "OldMarker")]),
            false,
        );
        let old = fs::read(user_rules_path(&dir.0)).unwrap();
        let text = tree_pack("new-tree");
        let preview = preview_boolean_pack(&dir.0, &text).unwrap();
        let result =
            import_boolean_pack(&dir.0, &text, preview["revision"].as_str().unwrap()).unwrap();
        assert_eq!(result["added"], 1);
        assert_eq!(fs::read(user_rules_path(&dir.0)).unwrap(), old);
        let saved = fs::read(boolean_rules_path(&dir.0)).unwrap();
        let repeated = preview_boolean_pack(&dir.0, &text).unwrap();
        assert_eq!(repeated["added"], 0);
        assert_eq!(repeated["unchanged"], 1);
        assert!(import_boolean_pack(&dir.0, &text, "missing").is_err());
        assert_eq!(fs::read(boolean_rules_path(&dir.0)).unwrap(), saved);
        let output = export_pack(&dir.0, "effective").unwrap();
        let parsed = parse_pack(output["content"].as_str().unwrap()).unwrap();
        assert_eq!(
            parsed.fingerprints.len(),
            builtin_fingerprints().unwrap().len() + 2
        );
        assert!(validated_pack_rules(output["content"].as_str().unwrap()).is_ok());
    }

    #[test]
    fn boolean_schema_empty_nodes_unknown_fields_and_legacy_downgrade_are_rejected() {
        let dir = TempDir::new();
        let mut value: Value = serde_json::from_str(&tree_pack("tree")).unwrap();
        value["schemaVersion"] = json!(1);
        assert!(parse_pack(&value.to_string()).is_err());
        value["schemaVersion"] = json!(2);
        value["fingerprints"][0]["condition"]["children"] = json!([]);
        assert!(parse_pack(&value.to_string()).is_err());
        value["fingerprints"][0]["condition"] =
            json!({"type":"body","pattern":"Brand","cve":"bad"});
        assert!(parse_pack(&value.to_string()).is_err());
        assert!(preview_pack(&dir.0, &tree_pack("tree"), false).is_err());
        let all=json!({"schemaVersion":1,"fingerprints":[{"id":"flat-all","name":"Flat","matchMode":"all","bodyRegex":["alpha","beta"]}]}).to_string();
        assert!(validated_pack_rules(&all).is_err());
        assert!(!boolean_rules_path(&dir.0).exists());
    }

    #[test]
    fn normalization_preserves_rules_already_in_defaults() {
        let text = include_str!("../../resources/fingerprint-packs/default.fingerprints.json");
        let normalized = normalize_pack(text, false).unwrap();
        let input_count = parse_pack(text).unwrap().fingerprints.len();
        assert_eq!(normalized["count"], input_count);
        let parsed = parse_pack(normalized["content"].as_str().unwrap()).unwrap();
        assert_eq!(parsed.fingerprints.len(), input_count);
    }

    #[test]
    fn builtins_validate_without_cve_data() {
        assert_eq!(
            builtin_fingerprints().unwrap().len(),
            bundled_catalog().unwrap().rules.len()
        );
        assert!(!include_str!("../../resources/rules/fingerprints.json").contains("CVE-"));
    }
    #[test]
    fn rejects_cve_fields_and_invalid_regex_or_path() {
        for mut rule in [sample("x", "product"), sample("x", "("), sample("x", ".*")] {
            if rule["bodyRegex"][0] == "product" {
                rule["cve"] = json!("CVE-2026-0001");
            }
            assert!(parse_pack(&legacy(vec![rule])).is_err());
        }
        for path in [
            "https://example.com/",
            "//other/",
            "/../secret",
            "/%2e%2e/",
            "/x?token=secret",
            "/x\r\nY:z",
            "/x\\y",
        ] {
            let rule = json!({"id":"x","name":"x","pathSignals":[{"path":path,"status":[200],"bodyRegex":"product"}]});
            assert!(
                parse_pack(&legacy(vec![rule])).is_err(),
                "accepted {path:?}"
            );
        }
        assert!(parse_pack(&json!({"schemaVersion":2,"fingerprints":[]}).to_string()).is_err());
        assert!(parse_pack(&json!({"fingerprints":[],"checks":[]}).to_string()).is_err());
        assert!(parse_pack(&legacy(vec![
            json!({"id":"x","name":"x","pathSignals":[{"path":"/login","status":[200]}]})
        ]))
        .is_err());
    }
    #[test]
    fn canonical_duplicates_and_conflicting_ids() {
        let a = json!({"id":"x","name":"x","bodyRegex":["two","one","one"],"headerRegex":{"Server":"product"}});
        let b = json!({"id":"x","name":"x","bodyRegex":["one","two"],"headerRegex":{"server":"product"}});
        let p = parse_pack(&legacy(vec![a, b])).unwrap();
        assert_eq!(p.duplicates, 1);
        assert_eq!(p.fingerprints.len(), 1);
        assert!(parse_pack(&legacy(vec![sample("x", "first"), sample("x", "other")])).is_err());
    }
    #[test]
    fn roundtrip_checksum_and_unknown_nested_metadata() {
        let p = parse_pack(&legacy(vec![sample("x", "product")])).unwrap();
        let (text, _) = package_content("Shared", vec![], p.fingerprints).unwrap();
        assert!(!parse_pack(&text).unwrap().legacy);
        let mut value: Value = serde_json::from_str(&text).unwrap();
        value["fingerprints"][0]["name"] = json!("changed");
        assert!(parse_pack(&value.to_string())
            .unwrap_err()
            .contains("SHA-256"));
        value = serde_json::from_str(&text).unwrap();
        value["manifest"]["cve"] = json!([]);
        assert!(parse_pack(&value.to_string()).is_err());
        value = serde_json::from_str(&text).unwrap();
        value["kind"] = json!("vulnerability-library");
        assert!(parse_pack(&value.to_string()).is_err());
    }
    #[test]
    fn merge_overrides_once_and_preserves_conflicts_by_default() {
        let dir = TempDir::new();
        let content = legacy(vec![
            sample("nginx", "my-nginx"),
            sample("custom", "custom-product"),
        ]);
        let p = import(&dir.0, &content, false);
        assert_eq!(p["added"], 1);
        assert_eq!(p["skipped"], 1);
        let p = import(&dir.0, &content, true);
        assert_eq!(p["updated"], 1);
        assert_eq!(p["unchanged"], 1);
        let effective = effective_fingerprints(Some(&dir.0)).unwrap();
        assert_eq!(effective.iter().filter(|r| r.id == "nginx").count(), 1);
        assert_eq!(
            effective
                .iter()
                .find(|r| r.id == "nginx")
                .unwrap()
                .body_regex,
            vec!["my-nginx"]
        );
        let export = export_pack(&dir.0, "effective").unwrap();
        let exported = parse_pack(export["content"].as_str().unwrap()).unwrap();
        assert_eq!(exported.fingerprints, effective);
        let backup_count = fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".bak."))
            .count();
        assert_eq!(backup_count, 1);
    }
    #[test]
    fn stale_preview_and_invalid_existing_file_never_overwrite() {
        let dir = TempDir::new();
        let content = legacy(vec![sample("x", "product")]);
        let p = preview_pack(&dir.0, &content, false).unwrap();
        fs::write(user_rules_path(&dir.0), legacy(vec![sample("y", "other")])).unwrap();
        let original = fs::read(user_rules_path(&dir.0)).unwrap();
        assert!(import_pack(&dir.0, &content, false, p["revision"].as_str().unwrap()).is_err());
        assert_eq!(fs::read(user_rules_path(&dir.0)).unwrap(), original);
        fs::write(user_rules_path(&dir.0), b"broken").unwrap();
        assert!(preview_pack(&dir.0, &content, false).is_err());
        assert!(import_pack(&dir.0, &content, true, "missing").is_err());
        assert_eq!(fs::read(user_rules_path(&dir.0)).unwrap(), b"broken");
    }
    #[test]
    fn preserves_source_metadata_across_imports_and_exports() {
        let dir = TempDir::new();
        for (id, repository) in [
            ("one", "https://example.com/one"),
            ("two", "https://example.com/two"),
        ] {
            let parsed = parse_pack(&legacy(vec![sample(id, id)])).unwrap();
            let source = PackSource {
                name: id.into(),
                repository: repository.into(),
                revision: "fixed-revision".into(),
                license: "MIT".into(),
            };
            let (content, _) = package_content(id, vec![source], parsed.fingerprints).unwrap();
            import(&dir.0, &content, false);
        }
        let export = export_pack(&dir.0, "user").unwrap();
        let parsed = parse_pack(export["content"].as_str().unwrap()).unwrap();
        assert_eq!(parsed.manifest.sources.len(), 2);
        assert_eq!(
            parsed.manifest.sources[0].repository,
            "https://example.com/one"
        );
        assert_eq!(parsed.manifest.sources[1].license, "MIT");
    }
    #[test]
    fn rejects_oversized_packages_and_unsafe_source_credentials() {
        assert!(parse_pack(&" ".repeat(MAX_PACK_BYTES + 1)).is_err());
        let parsed = parse_pack(&legacy(vec![sample("x", "product")])).unwrap();
        let source = PackSource {
            repository: "https://user:password@example.com/repo".into(),
            ..Default::default()
        };
        let (content, _) =
            package_content("Invalid source", vec![source], parsed.fingerprints).unwrap();
        assert!(parse_pack(&content).is_err());
    }
    #[test]
    fn identical_rules_preserve_new_provenance_but_skipped_conflicts_do_not() {
        let dir = TempDir::new();
        let nginx = builtin_fingerprints()
            .unwrap()
            .into_iter()
            .find(|rule| rule.id == "nginx")
            .unwrap();
        let source = PackSource {
            name: "Reviewed source".into(),
            repository: "https://example.com/fingerprints".into(),
            revision: "commit".into(),
            license: "MIT".into(),
        };
        let (content, _) =
            package_content("Identical builtin", vec![source.clone()], vec![nginx]).unwrap();
        let result = import(&dir.0, &content, false);
        assert_eq!(result["unchanged"], 1);
        assert_eq!(result["metadataUpdated"], true);
        assert!(load_user_fingerprints(Some(&dir.0)).unwrap().is_empty());
        let exported = export_pack(&dir.0, "effective").unwrap();
        let exported_sources = parse_pack(exported["content"].as_str().unwrap())
            .unwrap()
            .manifest
            .sources;
        let mut expected_sources = vec![source];
        for bundled_source in &bundled_catalog().unwrap().sources {
            if !expected_sources.contains(bundled_source) {
                expected_sources.push(bundled_source.clone());
            }
        }
        assert_eq!(exported_sources, expected_sources);
        let before = fs::read(user_rules_path(&dir.0)).unwrap();
        let conflict = parse_pack(&legacy(vec![sample("nginx", "different-nginx")])).unwrap();
        let source = PackSource {
            name: "Skipped source".into(),
            license: "Other".into(),
            ..Default::default()
        };
        let (content, _) = package_content("Skipped", vec![source], conflict.fingerprints).unwrap();
        let result = import(&dir.0, &content, false);
        assert_eq!(result["skipped"], 1);
        assert_eq!(result["metadataUpdated"], false);
        assert_eq!(fs::read(user_rules_path(&dir.0)).unwrap(), before);
    }
    #[test]
    fn preview_rejects_output_size_overflow_before_writing() {
        let dir = TempDir::new();
        let patterns: Vec<String> = (0..128)
            .map(|i| format!("{i:03}{}", "a".repeat(4087)))
            .collect();
        let rules:Vec<Value>=(0..10).map(|i|json!({"id":format!("rule-{i}"),"name":format!("rule-{i}"),"bodyRegex":patterns})).collect();
        let content = legacy(rules);
        assert!(content.len() < MAX_PACK_BYTES);
        assert!(preview_pack(&dir.0, &content, false)
            .unwrap_err()
            .contains("5 MiB"));
        assert!(!user_rules_path(&dir.0).exists());
    }
}
