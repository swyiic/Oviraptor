//! Active exposure probe + technology fingerprinting — the deterministic
//! identification + targeted-check layer (docs/AGENT_NEST_INTELLIGENT_FLOW_DESIGN.md
//! §10b, §12).
//!
//! Two deterministic, read-only capabilities, both driven by data (JSON) so new
//! products/checks are added without recompiling:
//!   1. Fingerprinting: match a target's root response (headers, cookies, body,
//!      characteristic paths) against a signal catalog to identify products
//!      (DedeCMS, WordPress, ...). A larger corpus is meant to be synced as a
//!      rule-pack; the shipped set is the vetted default.
//!   2. Exposure checks: GET a small, high-signal set of sensitive paths (git
//!      leak, .env, backups, and product-specific disclosure paths). A check runs
//!      when its `appliesTo` is empty, names "http", or names an identified
//!      product — so product-specific checks fire only on matching targets.
//!
//! A soft-404 baseline plus content matching keeps sites that answer 200 for
//! everything from producing false positives. Pure: the caller records the
//! identified products and hits as findings.

use super::fingerprint_logic::{self, Condition, BOOLEAN_RUNTIME};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

// ---------------------------------------------------------------- exposure checks

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExposureCatalog {
    #[serde(default)]
    #[allow(dead_code)]
    pub schema_version: u32,
    pub checks: Vec<ExposureCheck>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExposureCheck {
    pub id: String,
    pub title: String,
    pub severity: String,
    #[serde(default)]
    pub applies_to: Vec<String>,
    pub path: String,
    #[serde(default, rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default)]
    pub negate_body_contains: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MatchRule {
    #[serde(default)]
    pub status: Vec<u16>,
    #[serde(default)]
    pub body_contains: Vec<String>,
    #[serde(default)]
    pub body_regex: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExposureHit {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub url: String,
    pub evidence: String,
}

pub(crate) fn builtin_checks() -> Vec<ExposureCheck> {
    serde_json::from_str::<ExposureCatalog>(include_str!(
        "../../resources/rules/active_exposures.json"
    ))
    .map(|catalog| catalog.checks)
    .unwrap_or_default()
}

// ---------------------------------------------------------------- fingerprints

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FingerprintCatalog {
    #[serde(default)]
    fingerprints: Vec<Fingerprint>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Fingerprint {
    id: String,
    name: String,
    #[serde(default)]
    header_regex: std::collections::HashMap<String, String>,
    #[serde(default)]
    set_cookie_regex: Vec<String>,
    #[serde(default)]
    body_regex: Vec<String>,
    #[serde(default)]
    path_signals: Vec<PathSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    condition: Option<Condition>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PathSignal {
    path: String,
    #[serde(default)]
    status: Vec<u16>,
    #[serde(default)]
    body_regex: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Identified {
    pub id: String,
    pub name: String,
}

fn starter_fingerprint_catalog() -> Vec<Fingerprint> {
    serde_json::from_str::<FingerprintCatalog>(include_str!(
        "../../resources/rules/fingerprints.json"
    ))
    .map(|catalog| catalog.fingerprints)
    .unwrap_or_default()
}

// The original starter set keeps its exact signal/path ordering. Additional
// defaults are validated once from the resource pack and deduplicated by ID.
fn fingerprint_catalog_checked() -> Result<Vec<Fingerprint>, String> {
    static CATALOG: std::sync::OnceLock<Result<Vec<Fingerprint>, String>> =
        std::sync::OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let mut all = starter_fingerprint_catalog();
            let mut seen: std::collections::HashSet<String> =
                all.iter().map(|fp| fp.id.clone()).collect();
            let bundled: Vec<Fingerprint> =
                serde_json::from_value(super::fingerprint_pack::bundled_fingerprints_json()?)
                    .map_err(|error| format!("内置指纹包格式错误：{error}"))?;
            for fp in bundled {
                if seen.insert(fp.id.clone()) {
                    validate_fingerprint(&fp)?;
                    all.push(fp);
                }
            }
            Ok(all)
        })
        .clone()
}

fn fingerprint_catalog() -> Vec<Fingerprint> {
    fingerprint_catalog_checked().unwrap_or_else(|error| {
        eprintln!("内置指纹包未加载，仅使用原始规则：{error}");
        starter_fingerprint_catalog()
    })
}

// ------------------------------------------------- user (self-service) rules
// Legacy flat rules remain in the original file. Condition trees live in a
// separate, versioned file so an old running App never treats an AND as OR.
static USER_RULE_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn user_rules_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("fingerprints.user.json")
}

pub(crate) fn boolean_rules_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("fingerprints.boolean.json")
}

fn validate_fingerprint(fp: &Fingerprint) -> Result<(), String> {
    if fp.id.is_empty()
        || !fp
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!("id 非法（仅限小写字母、数字、连字符）：{}", fp.id));
    }
    if fp.name.trim().is_empty() {
        return Err(format!("指纹 {} 缺少名称", fp.id));
    }
    let signal_count = fp.header_regex.len()
        + fp.set_cookie_regex.len()
        + fp.body_regex.len()
        + fp.path_signals.len();
    if let Some(condition) = &fp.condition {
        if signal_count != 0 {
            return Err(format!(
                "指纹 {} 不得同时包含 condition 与旧格式信号",
                fp.id
            ));
        }
        if fp.id.len() > 128 || fp.name.len() > 256 {
            return Err("条件树指纹 ID/名称超过长度上限".into());
        }
        return fingerprint_logic::validate_condition(condition)
            .map_err(|e| format!("指纹 {}：{e}", fp.id));
    }
    // Do not change legacy validation or matching semantics.
    if signal_count == 0 {
        return Err(format!("指纹 {} 至少需要一个信号", fp.id));
    }
    for pattern in fp
        .header_regex
        .values()
        .chain(fp.set_cookie_regex.iter())
        .chain(fp.body_regex.iter())
    {
        regex::Regex::new(pattern)
            .map_err(|e| format!("指纹 {} 的正则无效：{pattern}（{e}）", fp.id))?;
    }
    for signal in &fp.path_signals {
        if !signal.path.starts_with('/')
            || signal.path.contains("://")
            || signal.path.contains("..")
        {
            return Err(format!(
                "指纹 {} 的路径必须是站内绝对路径（以 / 开头，不含 URL 或 ..）：{}",
                fp.id, signal.path
            ));
        }
        if let Some(pattern) = &signal.body_regex {
            regex::Regex::new(pattern)
                .map_err(|e| format!("指纹 {} 的路径正则无效：{pattern}（{e}）", fp.id))?;
        }
    }
    Ok(())
}

fn validate_input_match_mode(value: &serde_json::Value) -> Result<(), String> {
    if let Some(mode) = value.get("matchMode") {
        if mode.as_str() != Some("any") {
            return Err("当前规则请用 condition 条件树表达 AND/混合逻辑，不接受 matchMode:all 或未知匹配方式".into());
        }
    }
    Ok(())
}

fn parse_user_catalog(text: &str) -> Result<Vec<Fingerprint>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("JSON 解析失败：{e}"))?;
    let marked_boolean = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        == Some(2)
        && value
            .get("booleanRuntime")
            .and_then(serde_json::Value::as_str)
            == Some(BOOLEAN_RUNTIME);
    let array = if value.get("kind").and_then(serde_json::Value::as_str)
        == Some("oviraptor-fingerprints")
    {
        super::fingerprint_pack::validated_pack_rules(text)?
    } else {
        value.get("fingerprints").cloned().unwrap_or(value)
    };
    if let Some(rows) = array.as_array() {
        for row in rows {
            validate_input_match_mode(row)?;
        }
    }
    let list: Vec<Fingerprint> =
        serde_json::from_value(array).map_err(|e| format!("指纹格式不符：{e}"))?;
    let mut seen = std::collections::HashSet::new();
    for fp in &list {
        if fp.condition.is_some() && !marked_boolean {
            return Err(
                "条件树导入必须使用 schemaVersion:2 和 booleanRuntime:condition-tree-v1".into(),
            );
        }
        validate_fingerprint(fp)?;
        if !seen.insert(fp.id.clone()) {
            return Err(format!("指纹 id 重复：{}", fp.id));
        }
    }
    Ok(list)
}

fn read_user_fingerprints(data_dir: &Path) -> Result<Vec<Fingerprint>, String> {
    match std::fs::read_to_string(user_rules_path(data_dir)) {
        Ok(text) if text.trim().is_empty() => Ok(Vec::new()),
        Ok(text) => {
            let list = parse_user_catalog(&text)?;
            if list.iter().any(|fp| fp.condition.is_some()) {
                return Err("旧指纹文件中发现条件树；请将条件树存入 fingerprints.boolean.json，以免旧版 App 误读".into());
            }
            Ok(list)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("读取用户指纹失败：{e}")),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BooleanCatalog {
    schema_version: u32,
    boolean_runtime: String,
    fingerprints: Vec<Fingerprint>,
}

pub(crate) fn parse_boolean_catalog(text: &str) -> Result<Vec<Fingerprint>, String> {
    if text.len() > 5 * 1024 * 1024 {
        return Err("条件树指纹文件超过 5 MiB 上限".into());
    }
    let catalog: BooleanCatalog =
        serde_json::from_str(text).map_err(|e| format!("条件树指纹格式错误：{e}"))?;
    if catalog.schema_version != 2 || catalog.boolean_runtime != BOOLEAN_RUNTIME {
        return Err(
            "条件树文件必须使用 schemaVersion:2 和 booleanRuntime:condition-tree-v1".into(),
        );
    }
    if catalog.fingerprints.len() > 5000 {
        return Err("条件树指纹超过 5000 条上限".into());
    }
    let raw: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    for rule in raw["fingerprints"]
        .as_array()
        .ok_or("条件树 fingerprints 必须是数组")?
    {
        let object = rule.as_object().ok_or("条件树规则必须是对象")?;
        if object
            .keys()
            .any(|key| !["id", "name", "condition"].contains(&key.as_str()))
        {
            return Err("条件树文件的规则仅接受 id/name/condition，不接受漏洞或旧信号字段".into());
        }
    }
    let mut seen = std::collections::HashSet::new();
    for fp in &catalog.fingerprints {
        if fp.condition.is_none() {
            return Err(format!("条件树文件内的 {} 必须包含 condition", fp.id));
        }
        validate_fingerprint(fp)?;
        if !seen.insert(fp.id.clone()) {
            return Err(format!("条件树 id 重复：{}", fp.id));
        }
    }
    Ok(catalog.fingerprints)
}

pub(crate) fn read_boolean_fingerprints(data_dir: &Path) -> Result<Vec<Fingerprint>, String> {
    let path = boolean_rules_path(data_dir);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err("条件树规则路径必须是普通文件".into())
        }
        Ok(metadata) if metadata.len() > 5 * 1024 * 1024 => {
            return Err("条件树指纹文件超过 5 MiB 上限".into())
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取条件树指纹失败：{e}")),
    }
    std::fs::read_to_string(path)
        .map_err(|e| format!("读取条件树指纹失败：{e}"))
        .and_then(|text| parse_boolean_catalog(&text))
}

fn combined_user_rules(data_dir: &Path) -> Result<Vec<Fingerprint>, String> {
    let mut legacy = read_user_fingerprints(data_dir)?;
    let boolean = read_boolean_fingerprints(data_dir)?;
    let mut occupied: std::collections::HashSet<String> = starter_fingerprint_catalog()
        .into_iter()
        .map(|fp| fp.id)
        .collect();
    occupied.extend(legacy.iter().map(|fp| fp.id.clone()));
    for fp in boolean {
        if !occupied.insert(fp.id.clone()) {
            return Err(format!(
                "条件树 ID {} 与现有规则冲突，原有规则受保护",
                fp.id
            ));
        }
        legacy.push(fp);
    }
    Ok(legacy)
}

fn atomic_rules_write(path: &Path, content: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("规则文件目录无效")?;
    let old = match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err("规则路径必须是普通文件".into())
        }
        Ok(_) => Some(std::fs::read(path).map_err(|e| e.to_string())?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    if old.as_deref() == Some(content) {
        return Ok(());
    }
    let temporary = parent.join(format!(".fingerprints-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(content)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        if let Some(bytes) = &old {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("fingerprints");
            let backup = parent.join(format!("{name}.{}.bak.json", uuid::Uuid::new_v4()));
            let mut backup_file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(backup)
                .map_err(|e| e.to_string())?;
            backup_file
                .write_all(bytes)
                .and_then(|_| backup_file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        let current = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if current != old {
            return Err("规则文件已被其他操作修改，请重新加载后重试".into());
        }
        std::fs::rename(&temporary, path).map_err(|e| format!("保存规则失败：{e}"))?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|e| format!("规则已保存，但目录同步失败，请重新加载确认：{e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn write_user_fingerprints(data_dir: &Path, rules: &[Fingerprint]) -> Result<(), String> {
    if rules.iter().any(|fp| fp.condition.is_some()) {
        return Err("条件树不能写入旧格式文件".into());
    }
    let document = serde_json::json!({ "schemaVersion": 1, "fingerprints": rules });
    let text = serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?;
    atomic_rules_write(&user_rules_path(data_dir), text.as_bytes())
}

fn write_boolean_fingerprints(data_dir: &Path, rules: &[Fingerprint]) -> Result<(), String> {
    let fingerprints: Vec<_> = rules
        .iter()
        .map(|fp| serde_json::json!({"id":fp.id,"name":fp.name,"condition":fp.condition}))
        .collect();
    let document = serde_json::json!({"schemaVersion":2,"booleanRuntime":BOOLEAN_RUNTIME,"fingerprints":fingerprints});
    let text = serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?;
    parse_boolean_catalog(&text)?;
    atomic_rules_write(&boolean_rules_path(data_dir), text.as_bytes())
}

// User overrides retain their original precedence. Conditional user rules may
// override resource-pack defaults, but never original starter or legacy user IDs.
fn all_fingerprints(user_dir: Option<&Path>) -> Vec<Fingerprint> {
    let mut all = fingerprint_catalog();
    let mut positions: std::collections::HashMap<String, usize> = all
        .iter()
        .enumerate()
        .map(|(index, fp)| (fp.id.clone(), index))
        .collect();
    let mut protected: std::collections::HashSet<String> = starter_fingerprint_catalog()
        .into_iter()
        .map(|fp| fp.id)
        .collect();
    if let Some(dir) = user_dir {
        if let Ok(user) = read_user_fingerprints(dir) {
            for fp in user {
                protected.insert(fp.id.clone());
                if let Some(index) = positions.get(&fp.id).copied() {
                    all[index] = fp;
                } else {
                    positions.insert(fp.id.clone(), all.len());
                    all.push(fp);
                }
            }
        }
        match read_boolean_fingerprints(dir) {
            Ok(boolean) => {
                for fp in boolean {
                    if protected.contains(&fp.id) {
                        eprintln!("条件树规则 ID {} 与原有规则冲突，已保留原有规则", fp.id);
                    } else if let Some(index) = positions.get(&fp.id).copied() {
                        all[index] = fp;
                    } else {
                        positions.insert(fp.id.clone(), all.len());
                        all.push(fp);
                    }
                }
            }
            Err(error) => eprintln!("条件树指纹未加载：{error}"),
        }
    }
    all
}

fn user_list_value(rules: &[Fingerprint]) -> serde_json::Value {
    serde_json::to_value(rules).unwrap_or_else(|_| serde_json::Value::Array(Vec::new()))
}

#[tauri::command]
pub fn scan_rule_library(state: tauri::State<crate::AppState>) -> serde_json::Value {
    let dir = &state.app_data_dir;
    let legacy = read_user_fingerprints(dir);
    let boolean = read_boolean_fingerprints(dir);
    let combined = combined_user_rules(dir);
    let defaults = fingerprint_catalog_checked();
    let bundle_error = defaults.as_ref().err().cloned().unwrap_or_default();
    let user_error = combined.as_ref().err().cloned().unwrap_or_default();
    let error = [bundle_error.as_str(), user_error.as_str()]
        .into_iter()
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>()
        .join("；");
    serde_json::json!({
        "fingerprints":{"fingerprints":defaults.as_ref().map(|list|user_list_value(list)).unwrap_or_else(|_|serde_json::json!([])),"error":bundle_error},
        "exposures":serde_json::from_str::<serde_json::Value>(include_str!("../../resources/rules/active_exposures.json")).unwrap_or(serde_json::Value::Null),
        "booleanRuntime":BOOLEAN_RUNTIME,
        "user":{"path":user_rules_path(dir).to_string_lossy(),"booleanPath":boolean_rules_path(dir).to_string_lossy(),
            "fingerprints":combined.as_ref().map(|list|user_list_value(list)).unwrap_or_else(|_|serde_json::json!([])),
            "legacyFingerprints":legacy.as_ref().map(|list|user_list_value(list)).unwrap_or_else(|_|serde_json::json!([])),
            "booleanFingerprints":boolean.as_ref().map(|list|user_list_value(list)).unwrap_or_else(|_|serde_json::json!([])),"error":user_error},
        "effective":{"fingerprints":if error.is_empty(){user_list_value(&all_fingerprints(Some(dir)))}else{serde_json::json!([])},"error":error}
    })
}

#[tauri::command]
pub fn scan_fingerprint_template() -> serde_json::Value {
    serde_json::json!({"id":"my-product","name":"我的产品","headerRegex":{"Server":"(?i)my-product"},"setCookieRegex":["(?i)MYSESSION"],"bodyRegex":["(?i)my-product"],"pathSignals":[{"path":"/login","status":[200],"bodyRegex":"(?i)my-product"}]})
}

#[tauri::command]
pub fn scan_add_user_fingerprint(
    state: tauri::State<crate::AppState>,
    fingerprint: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let _guard = USER_RULE_WRITE_LOCK
        .lock()
        .map_err(|_| "规则写入锁不可用")?;
    validate_input_match_mode(&fingerprint)?;
    let fp: Fingerprint =
        serde_json::from_value(fingerprint).map_err(|e| format!("指纹格式不符：{e}"))?;
    validate_fingerprint(&fp)?;
    let dir = &state.app_data_dir;
    let mut legacy = read_user_fingerprints(dir)?;
    let mut boolean = read_boolean_fingerprints(dir)?;
    // Validate existing storage before performing any edit.
    combined_user_rules(dir)?;
    if fp.condition.is_some() {
        if legacy.iter().any(|x| x.id == fp.id)
            || starter_fingerprint_catalog().iter().any(|x| x.id == fp.id)
        {
            return Err("条件树不能覆盖内置或旧格式 ID，请使用新 ID".into());
        }
        if let Some(existing) = boolean.iter_mut().find(|x| x.id == fp.id) {
            *existing = fp;
        } else {
            boolean.push(fp);
        }
        write_boolean_fingerprints(dir, &boolean)?;
    } else {
        if boolean.iter().any(|x| x.id == fp.id) {
            return Err("旧格式规则不能覆盖条件树 ID，请使用新 ID".into());
        }
        if let Some(existing) = legacy.iter_mut().find(|x| x.id == fp.id) {
            *existing = fp;
        } else {
            legacy.push(fp);
        }
        write_user_fingerprints(dir, &legacy)?;
    }
    Ok(user_list_value(&combined_user_rules(dir)?))
}

#[tauri::command]
pub fn scan_delete_user_fingerprint(
    state: tauri::State<crate::AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    let _guard = USER_RULE_WRITE_LOCK
        .lock()
        .map_err(|_| "规则写入锁不可用")?;
    let dir = &state.app_data_dir;
    let mut legacy = read_user_fingerprints(dir)?;
    let mut boolean = read_boolean_fingerprints(dir)?;
    combined_user_rules(dir)?;
    let a = legacy.len();
    legacy.retain(|x| x.id != id);
    if a != legacy.len() {
        write_user_fingerprints(dir, &legacy)?;
    }
    let b = boolean.len();
    boolean.retain(|x| x.id != id);
    if b != boolean.len() {
        write_boolean_fingerprints(dir, &boolean)?;
    }
    Ok(user_list_value(&combined_user_rules(dir)?))
}

fn merge_import(data_dir: &Path, text: &str) -> Result<Vec<Fingerprint>, String> {
    let incoming = parse_user_catalog(text)?;
    let mut legacy = read_user_fingerprints(data_dir)?;
    let mut boolean = read_boolean_fingerprints(data_dir)?;
    combined_user_rules(data_dir)?;
    let mut occupied: std::collections::HashSet<_> =
        fingerprint_catalog().into_iter().map(|x| x.id).collect();
    occupied.extend(legacy.iter().chain(boolean.iter()).map(|x| x.id.clone()));
    let old_legacy = legacy.len();
    let old_boolean = boolean.len();
    for fp in incoming {
        if !occupied.insert(fp.id.clone()) {
            continue;
        }
        if fp.condition.is_some() {
            boolean.push(fp);
        } else {
            legacy.push(fp);
        }
    }
    // Existing rules are never removed or replaced by a bulk import.
    if old_legacy != legacy.len() {
        write_user_fingerprints(data_dir, &legacy)?;
    }
    if old_boolean != boolean.len() {
        write_boolean_fingerprints(data_dir, &boolean)
            .map_err(|e| format!("条件树保存失败，请重新加载确认（原有规则未删除）：{e}"))?;
    }
    combined_user_rules(data_dir)
}

#[tauri::command]
pub fn scan_import_user_fingerprints(
    state: tauri::State<crate::AppState>,
    text: String,
) -> Result<serde_json::Value, String> {
    let _guard = USER_RULE_WRITE_LOCK
        .lock()
        .map_err(|_| "规则写入锁不可用")?;
    Ok(user_list_value(&merge_import(&state.app_data_dir, &text)?))
}

#[tauri::command]
pub fn scan_export_user_fingerprints(
    state: tauri::State<crate::AppState>,
) -> Result<String, String> {
    let list = combined_user_rules(&state.app_data_dir)?;
    let document = if list.iter().any(|fp| fp.condition.is_some()) {
        serde_json::json!({"schemaVersion":2,"booleanRuntime":BOOLEAN_RUNTIME,"fingerprints":list})
    } else {
        serde_json::json!({"schemaVersion":1,"fingerprints":list})
    };
    serde_json::to_string_pretty(&document).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- HTTP

struct Probe {
    status: u16,
    headers: Vec<(String, String)>,
    set_cookie: Vec<String>,
    body: String,
}

const MAX_BODY: usize = 65536;
const MAX_EVIDENCE: usize = 160;

fn build_client(follow_redirects: bool) -> Option<reqwest::blocking::Client> {
    // Mirror the proven native-recon client: accept invalid certs (a scanner must
    // probe targets with broken/self-signed certs; the default-verifying rustls
    // build can also fail in-app) and present a browser-like UA. Fingerprinting
    // follows redirects so the real homepage is matched (many sites 302 the root);
    // exposure path-checks do NOT follow, so a redirect-to-home is never mistaken
    // for a path existing.
    let redirect = if follow_redirects {
        reqwest::redirect::Policy::limited(5)
    } else {
        reqwest::redirect::Policy::none()
    };
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(redirect)
        .danger_accept_invalid_certs(true)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36")
        .build()
        .ok()
}

fn fetch(client: &reqwest::blocking::Client, base: &str, path: &str) -> Option<Probe> {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let response = client.get(&url).send().ok()?;
    let status = response.status().as_u16();
    let set_cookie = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let bytes = response.bytes().ok()?;
    let slice = &bytes[..bytes.len().min(MAX_BODY)];
    Some(Probe {
        status,
        headers,
        set_cookie,
        body: String::from_utf8_lossy(slice).to_string(),
    })
}

fn regex_matches(pattern: &str, haystack: &str) -> bool {
    regex::Regex::new(pattern).is_ok_and(|regex| regex.is_match(haystack))
}

// ---------------------------------------------------------------- identification

/// Identify products running on `base_url` from its root response (and a few
/// characteristic-path probes). Signal-based: any matching signal identifies.
pub(crate) fn identify_products(base_url: &str, user_dir: Option<&Path>) -> Vec<Identified> {
    let Some(client) = build_client(true) else {
        return Vec::new();
    };
    let Some(root) = fetch(&client, base_url, "/") else {
        return Vec::new();
    };
    let mut identified = Vec::new();
    for fingerprint in all_fingerprints(user_dir) {
        if fingerprint_matches(&client, base_url, &fingerprint, &root) {
            identified.push(Identified {
                id: fingerprint.id,
                name: fingerprint.name,
            });
        }
    }
    identified
}

fn fingerprint_matches(
    client: &reqwest::blocking::Client,
    base_url: &str,
    fingerprint: &Fingerprint,
    root: &Probe,
) -> bool {
    if let Some(condition) = &fingerprint.condition {
        return fingerprint_logic::condition_matches(
            condition,
            &root.body,
            &root.headers,
            &root.set_cookie,
        );
    }
    for (name, pattern) in &fingerprint.header_regex {
        let lower = name.to_ascii_lowercase();
        if root
            .headers
            .iter()
            .any(|(header, value)| *header == lower && regex_matches(pattern, value))
        {
            return true;
        }
    }
    if fingerprint.set_cookie_regex.iter().any(|pattern| {
        root.set_cookie
            .iter()
            .any(|cookie| regex_matches(pattern, cookie))
    }) {
        return true;
    }
    if fingerprint
        .body_regex
        .iter()
        .any(|pattern| regex_matches(pattern, &root.body))
    {
        return true;
    }
    for signal in &fingerprint.path_signals {
        if let Some(probe) = fetch(client, base_url, &signal.path) {
            let status_ok = signal.status.is_empty() || signal.status.contains(&probe.status);
            let body_ok = signal
                .body_regex
                .as_ref()
                .is_none_or(|pattern| regex_matches(pattern, &probe.body));
            if status_ok && body_ok && !probe.body.to_ascii_lowercase().contains("<html") {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------- exposure probe

/// Probe `base_url` for the applicable checks. A check runs when its `appliesTo`
/// is empty, names "http", or names one of `products` (so product-specific checks
/// fire only on matching targets). Never panics; a dead host yields no hits.
pub(crate) fn probe_exposures(
    base_url: &str,
    checks: &[ExposureCheck],
    products: &[String],
) -> Vec<ExposureHit> {
    let Some(client) = build_client(false) else {
        return Vec::new();
    };
    // Soft-404 baseline: a path that cannot exist. A site that answers 200 for
    // everything reveals its generic page here, so a genuine hit must differ.
    let baseline = fetch(&client, base_url, "/oviraptor-nonexistent-9f3a2b7c1e5d");
    let mut hits = Vec::new();
    for check in checks {
        let applicable = check.applies_to.is_empty()
            || check
                .applies_to
                .iter()
                .any(|tag| tag == "http" || products.iter().any(|product| product == tag));
        if !applicable {
            continue;
        }
        let Some(probe) = fetch(&client, base_url, &check.path) else {
            continue;
        };
        if let Some(base) = &baseline {
            if base.status == probe.status && base.body == probe.body {
                continue; // identical to the not-found baseline → soft 404
            }
        }
        if matches_rule(&check.match_rule, &check.negate_body_contains, &probe) {
            hits.push(ExposureHit {
                id: check.id.clone(),
                title: check.title.clone(),
                severity: check.severity.clone(),
                url: format!("{}{}", base_url.trim_end_matches('/'), check.path),
                evidence: probe.body.chars().take(MAX_EVIDENCE).collect(),
            });
        }
    }
    hits
}

fn matches_rule(rule: &MatchRule, negate: &[String], probe: &Probe) -> bool {
    if !rule.status.is_empty() && !rule.status.contains(&probe.status) {
        return false;
    }
    if negate
        .iter()
        .any(|needle| probe.body.contains(needle.as_str()))
    {
        return false;
    }
    if !rule
        .body_contains
        .iter()
        .all(|needle| probe.body.contains(needle.as_str()))
    {
        return false;
    }
    if let Some(pattern) = &rule.body_regex {
        if !regex_matches(pattern, &probe.body) {
            return false;
        }
    }
    // A bare status match is never a hit: require a positive content signal.
    rule.body_regex.is_some() || !rule.body_contains.is_empty()
}

// ---------------------------------------------------------------- findings

/// Record exposure hits as findings (independent of the model/value gate).
pub(crate) fn record_exposures(
    db_path: &Path,
    scan_id: &str,
    target_url: &str,
    hits: &[ExposureHit],
) -> usize {
    let Ok(connection) = crate::db::open(db_path) else {
        return 0;
    };
    let mut recorded = 0;
    for hit in hits {
        let record = serde_json::json!({
            "source": "exposure_probe",
            "checkId": hit.id,
            "url": hit.url,
            "severity": hit.severity,
            "evidence": hit.evidence,
        })
        .to_string();
        let written = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'exposure','vulnerability',?3,?4,?5,?6) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            rusqlite::params![scan_id, target_url, hit.id, hit.title, hit.severity, record],
        );
        if written.is_ok() {
            recorded += 1;
        }
    }
    recorded
}

/// Record identified products as info findings so the identification is visible
/// and so later phases can target it.
pub(crate) fn record_identifications(
    db_path: &Path,
    scan_id: &str,
    target_url: &str,
    products: &[Identified],
) -> usize {
    let Ok(connection) = crate::db::open(db_path) else {
        return 0;
    };
    let mut recorded = 0;
    for product in products {
        let record = serde_json::json!({
            "source": "fingerprint",
            "productId": product.id,
            "product": product.name,
        })
        .to_string();
        let written = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'fingerprint','tech_identified',?3,?4,'info',?5) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            rusqlite::params![
                scan_id,
                target_url,
                product.id,
                format!("技术识别：{}", product.name),
                record
            ],
        );
        if written.is_ok() {
            recorded += 1;
        }
    }
    recorded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    // A site that leaks .git/HEAD and .env, serves a DedeCMS-flavoured root, and
    // answers 200 + an HTML page for every other path (the soft-404 case).
    fn start_site() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (extra, body): (&str, &[u8]) = match path {
                    "/" => ("Set-Cookie: DedeLoginTime=1; path=/\r\n", b"<html><body>Power by DedeCMS. <a href=/plus/list.php>list</a></body></html>"),
                    "/.git/HEAD" => ("", b"ref: refs/heads/main\n"),
                    "/.env" => ("", b"SECRET_KEY=topsecret\n"),
                    "/data/admin/ver.txt" => ("", b"20230512\n"),
                    _ => ("", b"<html><body>Not Found</body></html>"),
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
                let _ = stream.flush();
            }
        });
        port
    }

    #[test]
    fn identifies_dedecms_from_cookie_and_body() {
        let port = start_site();
        let base = format!("http://127.0.0.1:{port}");
        let ids: Vec<String> = identify_products(&base, None)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert!(
            ids.contains(&"dedecms".to_string()),
            "dedecms not identified: {ids:?}"
        );
        assert!(
            !ids.contains(&"wordpress".to_string()),
            "false WordPress id"
        );
    }

    #[test]
    fn detects_generic_and_product_gated_exposures() {
        let port = start_site();
        let base = format!("http://127.0.0.1:{port}");
        // Without identifying dedecms, the product-gated check must NOT run.
        let generic = probe_exposures(&base, &builtin_checks(), &[]);
        let generic_ids: Vec<&str> = generic.iter().map(|h| h.id.as_str()).collect();
        assert!(
            generic_ids.contains(&"git-head"),
            "git leak missed: {generic_ids:?}"
        );
        assert!(generic_ids.contains(&"dotenv"), ".env leak missed");
        assert!(
            !generic_ids.contains(&"dedecms-version"),
            "product check ran without identification"
        );
        // With dedecms identified, the product-gated check runs.
        let targeted = probe_exposures(&base, &builtin_checks(), &["dedecms".to_string()]);
        let targeted_ids: Vec<&str> = targeted.iter().map(|h| h.id.as_str()).collect();
        assert!(
            targeted_ids.contains(&"dedecms-version"),
            "product check missed: {targeted_ids:?}"
        );
    }

    fn synthetic_root(body: &str) -> Probe {
        Probe {
            status: 200,
            headers: Vec::new(),
            set_cookie: Vec::new(),
            body: body.into(),
        }
    }

    fn boolean_fixture(id: &str) -> serde_json::Value {
        serde_json::json!({"id":id,"name":"Synthetic product","condition":{"type":"all","children":[
            {"type":"body","pattern":"BrandAlpha"},{"type":"any","children":[
                {"type":"body","pattern":"ModuleBeta"},{"type":"header","name":"x-brand","pattern":"Beta"}
            ]}
        ]}})
    }

    #[test]
    fn old_flat_rules_keep_their_original_any_behavior() {
        let client = build_client(false).unwrap();
        for (fields, body, headers, cookies, expected) in [
            (
                serde_json::json!({"bodyRegex":["BrandAlpha","BrandBeta"]}),
                "BrandAlpha",
                vec![],
                vec![],
                true,
            ),
            (
                serde_json::json!({"bodyRegex":["BrandAlpha","BrandBeta"]}),
                "Login",
                vec![],
                vec![],
                false,
            ),
            (
                serde_json::json!({"headerRegex":{"X-Brand":"Alpha"},"bodyRegex":["Missing"]}),
                "",
                vec![("x-brand".into(), "Alpha".into())],
                vec![],
                true,
            ),
            (
                serde_json::json!({"setCookieRegex":["SESSION="],"bodyRegex":["Missing"]}),
                "",
                vec![],
                vec!["SESSION=123".into()],
                true,
            ),
            (
                serde_json::json!({"bodyRegex":[""]}),
                "",
                vec![],
                vec![],
                true,
            ),
        ] {
            let mut value = fields;
            value["id"] = serde_json::json!("legacy-test");
            value["name"] = serde_json::json!("Legacy");
            let fp: Fingerprint = serde_json::from_value(value).unwrap();
            validate_fingerprint(&fp).unwrap();
            let mut root = synthetic_root(body);
            root.headers = headers;
            root.set_cookie = cookies;
            assert!(fp.condition.is_none());
            assert_eq!(
                fingerprint_matches(&client, "http://127.0.0.1:9", &fp, &root),
                expected
            );
        }
        let fp: Fingerprint = serde_json::from_value(boolean_fixture("tree-test")).unwrap();
        assert!(!fingerprint_matches(
            &client,
            "http://127.0.0.1:9",
            &fp,
            &synthetic_root("BrandAlpha")
        ));
        assert!(fingerprint_matches(
            &client,
            "http://127.0.0.1:9",
            &fp,
            &synthetic_root("BrandAlpha ModuleBeta")
        ));
    }

    #[test]
    fn legacy_html_path_behavior_is_not_changed_by_condition_support() {
        let port = start_site();
        let client = build_client(false).unwrap();
        let fp:Fingerprint=serde_json::from_value(serde_json::json!({"id":"legacy-path","name":"Legacy","pathSignals":[{"path":"/not-found","status":[200],"bodyRegex":"Not Found"}]})).unwrap();
        assert!(!fingerprint_matches(
            &client,
            &format!("http://127.0.0.1:{port}"),
            &fp,
            &synthetic_root("")
        ));
    }

    #[test]
    fn boolean_marker_and_no_mixed_flat_signals_are_required() {
        let value = boolean_fixture("tree-test");
        assert!(parse_user_catalog(
            &serde_json::json!({"schemaVersion":1,"fingerprints":[value.clone()]}).to_string()
        )
        .is_err());
        assert!(parse_boolean_catalog(&serde_json::json!({"schemaVersion":1,"booleanRuntime":BOOLEAN_RUNTIME,"fingerprints":[value.clone()]}).to_string()).is_err());
        let document = serde_json::json!({"schemaVersion":2,"booleanRuntime":BOOLEAN_RUNTIME,"fingerprints":[value.clone()]});
        assert_eq!(
            parse_boolean_catalog(&document.to_string()).unwrap().len(),
            1
        );
        let mut mixed = value;
        mixed["bodyRegex"] = serde_json::json!(["Extra"]);
        let fp: Fingerprint = serde_json::from_value(mixed).unwrap();
        assert!(validate_fingerprint(&fp).is_err());
        assert!(parse_user_catalog(&serde_json::json!({"schemaVersion":1,"fingerprints":[{"id":"flat-all","name":"Bad","matchMode":"all","bodyRegex":["A","B"]}]}).to_string()).is_err());
        assert!(parse_boolean_catalog(&serde_json::json!({"schemaVersion":2,"booleanRuntime":BOOLEAN_RUNTIME,"fingerprints":[{"id":"tree","name":"Tree","condition":{"type":"body","pattern":"Brand"},"cve":"bad"}]}).to_string()).is_err());
    }

    #[test]
    fn additive_boolean_import_preserves_legacy_bytes_and_is_idempotent() {
        let directory =
            std::env::temp_dir().join(format!("ovi-boolean-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let legacy = r#"{
  "schemaVersion": 1, "fingerprints": [{"id":"test-legacy","name":"Legacy","bodyRegex":["LegacyBrand"]}]
}"#;
        std::fs::write(user_rules_path(&directory), legacy).unwrap();
        let text=serde_json::json!({"schemaVersion":2,"booleanRuntime":BOOLEAN_RUNTIME,"fingerprints":[boolean_fixture("test-boolean")]}).to_string();
        assert_eq!(merge_import(&directory, &text).unwrap().len(), 2);
        assert_eq!(
            std::fs::read_to_string(user_rules_path(&directory)).unwrap(),
            legacy
        );
        assert_eq!(
            all_fingerprints(Some(&directory)).len(),
            fingerprint_catalog().len() + 2
        );
        let before = std::fs::read(boolean_rules_path(&directory)).unwrap();
        assert_eq!(merge_import(&directory, &text).unwrap().len(), 2);
        assert_eq!(
            std::fs::read(boolean_rules_path(&directory)).unwrap(),
            before
        );
        let mut conflict = boolean_fixture(&fingerprint_catalog()[0].id);
        conflict["name"] = serde_json::json!("Should not replace builtin");
        let text=serde_json::json!({"schemaVersion":2,"booleanRuntime":BOOLEAN_RUNTIME,"fingerprints":[conflict]}).to_string();
        merge_import(&directory, &text).unwrap();
        assert_eq!(
            std::fs::read(boolean_rules_path(&directory)).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read_to_string(user_rules_path(&directory)).unwrap(),
            legacy
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    struct BundledTestDirectory(std::path::PathBuf);
    impl BundledTestDirectory {
        fn new() -> Self {
            let directory = std::env::temp_dir()
                .join(format!("ovi-bundled-fingerprints-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }
    }
    impl Drop for BundledTestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn bundled_expected_ids() -> std::collections::HashSet<String> {
        let bundled: Vec<Fingerprint> = serde_json::from_value(
            super::super::fingerprint_pack::bundled_fingerprints_json().unwrap(),
        )
        .unwrap();
        let ids: std::collections::HashSet<_> =
            bundled.iter().map(|rule| rule.id.clone()).collect();
        assert_eq!(
            ids.len(),
            bundled.len(),
            "Resource provider contains duplicate IDs"
        );
        assert!(starter_fingerprint_catalog()
            .iter()
            .all(|rule| ids.contains(&rule.id)));
        ids
    }

    #[test]
    fn bundled_defaults_work_in_fresh_profile_without_creating_user_files() {
        let directory = BundledTestDirectory::new();
        let starter = starter_fingerprint_catalog();
        let defaults = fingerprint_catalog_checked().unwrap();
        assert_eq!(starter.len(), 23);
        let expected_ids = bundled_expected_ids();
        assert_eq!(defaults.len(), expected_ids.len());
        assert_eq!(
            defaults
                .iter()
                .map(|rule| rule.id.clone())
                .collect::<std::collections::HashSet<_>>(),
            expected_ids
        );
        assert_eq!(
            serde_json::to_value(&defaults[..starter.len()]).unwrap(),
            serde_json::to_value(&starter).unwrap()
        );
        for loaded in [all_fingerprints(Some(&directory.0)), all_fingerprints(None)] {
            assert_eq!(loaded.len(), expected_ids.len());
            assert_eq!(
                loaded
                    .iter()
                    .map(|rule| rule.id.clone())
                    .collect::<std::collections::HashSet<_>>(),
                expected_ids
            );
        }
        assert!(combined_user_rules(&directory.0).unwrap().is_empty());
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    #[test]
    fn already_imported_pack_copies_are_deduplicated_without_rewriting_files() {
        let directory = BundledTestDirectory::new();
        let defaults = fingerprint_catalog_checked().unwrap();
        let expected_ids = bundled_expected_ids();
        let original_defaults = parse_user_catalog(include_str!(
            "../../resources/fingerprint-packs/default.fingerprints.json"
        ))
        .unwrap();
        let starter_ids: std::collections::HashSet<_> = starter_fingerprint_catalog()
            .into_iter()
            .map(|fp| fp.id)
            .collect();
        let legacy: Vec<_> = original_defaults
            .iter()
            .filter(|fp| fp.condition.is_none() && !starter_ids.contains(&fp.id))
            .cloned()
            .collect();
        let boolean: Vec<_> = original_defaults
            .iter()
            .filter(|fp| fp.condition.is_some())
            .cloned()
            .collect();
        assert_eq!(legacy.len(), 699);
        assert_eq!(boolean.len(), 76);
        write_user_fingerprints(&directory.0, &legacy).unwrap();
        write_boolean_fingerprints(&directory.0, &boolean).unwrap();
        let old_legacy = std::fs::read(user_rules_path(&directory.0)).unwrap();
        let old_boolean = std::fs::read(boolean_rules_path(&directory.0)).unwrap();
        assert_eq!(combined_user_rules(&directory.0).unwrap().len(), 775);
        let loaded = all_fingerprints(Some(&directory.0));
        assert_eq!(loaded.len(), expected_ids.len());
        assert_eq!(
            loaded
                .iter()
                .map(|fp| fp.id.clone())
                .collect::<std::collections::HashSet<_>>(),
            expected_ids
        );
        let default_by_id: std::collections::HashMap<_, _> = defaults
            .iter()
            .map(|fp| (&fp.id, serde_json::to_value(fp).unwrap()))
            .collect();
        for fp in &loaded {
            assert_eq!(
                default_by_id.get(&fp.id),
                Some(&serde_json::to_value(fp).unwrap())
            );
        }
        assert_eq!(
            std::fs::read(user_rules_path(&directory.0)).unwrap(),
            old_legacy
        );
        assert_eq!(
            std::fs::read(boolean_rules_path(&directory.0)).unwrap(),
            old_boolean
        );
    }

    #[test]
    fn local_boolean_override_wins_over_pack_and_removal_restores_default() {
        let directory = BundledTestDirectory::new();
        let expected_ids = bundled_expected_ids();
        let default = fingerprint_catalog_checked()
            .unwrap()
            .into_iter()
            .find(|fp| fp.condition.is_some())
            .unwrap();
        let mut custom = default.clone();
        custom.name = "Explicit custom override".into();
        custom.condition = Some(Condition::Body {
            pattern: "SyntheticLocalBrand".into(),
        });
        write_boolean_fingerprints(&directory.0, &[custom.clone()]).unwrap();
        assert_eq!(combined_user_rules(&directory.0).unwrap().len(), 1);
        let loaded = all_fingerprints(Some(&directory.0));
        assert_eq!(loaded.len(), expected_ids.len());
        assert_eq!(
            loaded
                .iter()
                .map(|fp| fp.id.clone())
                .collect::<std::collections::HashSet<_>>(),
            expected_ids
        );
        assert_eq!(
            serde_json::to_value(loaded.iter().find(|fp| fp.id == custom.id).unwrap()).unwrap(),
            serde_json::to_value(&custom).unwrap()
        );
        write_boolean_fingerprints(&directory.0, &[]).unwrap();
        let restored = all_fingerprints(Some(&directory.0));
        assert_eq!(restored.len(), expected_ids.len());
        assert_eq!(
            restored
                .iter()
                .map(|fp| fp.id.clone())
                .collect::<std::collections::HashSet<_>>(),
            expected_ids
        );
        assert_eq!(
            serde_json::to_value(restored.iter().find(|fp| fp.id == default.id).unwrap()).unwrap(),
            serde_json::to_value(default).unwrap()
        );
        assert!(!user_rules_path(&directory.0).exists());
    }

    #[test]
    fn importing_full_bundled_catalog_is_a_read_only_noop() {
        let directory = BundledTestDirectory::new();
        let defaults = fingerprint_catalog_checked().unwrap();
        let expected_ids = bundled_expected_ids();
        assert_eq!(defaults.len(), expected_ids.len());
        let text=serde_json::json!({"schemaVersion":2,"booleanRuntime":BOOLEAN_RUNTIME,"fingerprints":defaults}).to_string();
        assert!(merge_import(&directory.0, &text).unwrap().is_empty());
        assert!(merge_import(&directory.0, &text).unwrap().is_empty());
        let loaded = all_fingerprints(Some(&directory.0));
        assert_eq!(loaded.len(), expected_ids.len());
        assert_eq!(
            loaded
                .iter()
                .map(|fp| fp.id.clone())
                .collect::<std::collections::HashSet<_>>(),
            expected_ids
        );
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    #[test]
    fn builtin_catalogs_parse() {
        assert!(builtin_checks().len() >= 6);
        assert!(fingerprint_catalog().iter().any(|f| f.id == "dedecms"));
    }
}
