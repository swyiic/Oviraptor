//! §8 — Role config drafts, execution instances, and collaboration messages stay
//! separate. Capabilities may only come from the on-disk capability_bundles
//! directory. A draft is never executable.

use super::contract::AgentRole;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// User-facing status for every role config draft (§8).
pub const DRAFT_STATUS_LABEL: &str = "配置草稿，尚不能执行";

/// On-disk capability bundle loaded from `resources/capability_bundles/*.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityBundle {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub sandbox_default: String,
    pub side_effect_class: String,
    #[serde(default)]
    pub compatible_roles: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
}

/// Role configuration draft. Not an execution instance and not a mailbox message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleConfigDraft {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub role: String,
    pub objective: String,
    pub capability_bundle_ids: Vec<String>,
    /// Always `draft` for this type; kept explicit so callers cannot confuse it
    /// with a live run status.
    pub status: String,
    pub status_label: String,
    pub created_at: String,
    pub updated_at: String,
}

impl RoleConfigDraft {
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        role: AgentRole,
        objective: impl Into<String>,
        capability_bundle_ids: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            description: String::new(),
            role: role.as_str().to_string(),
            objective: objective.into(),
            capability_bundle_ids,
            status: "draft".to_string(),
            status_label: DRAFT_STATUS_LABEL.to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// Drafts never start tools, model loops, or assignments.
    pub fn is_executable(&self) -> bool {
        false
    }
}

/// Live (or historical) execution instance — always tied to a run, never a draft.
///
/// The two views below and their constructors are exercised only by the §8
/// contract test in this file until the collaboration surface (Stage 11) reads
/// them; keeping them `cfg(test)` states that instead of shipping unreachable code.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionInstanceRef {
    pub run_id: String,
    pub scan_id: String,
    pub attempt_number: i64,
    pub role: String,
    pub status: String,
    pub assignment_id: Option<String>,
}

/// Collaboration / mailbox message — append-only facts between agents, not a
/// role template and not an execution lease.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationMessageRef {
    pub id: String,
    pub run_id: String,
    pub kind: String,
    pub from_agent: String,
    pub to_agent: String,
    pub summary: String,
}

/// Resolve the shipped capability directory. Prefers an explicit override, then
/// the crate `resources/capability_bundles` next to the Tauri crate.
pub fn default_capability_directory() -> PathBuf {
    if let Ok(override_dir) = std::env::var("OVIRAPTOR_CAPABILITY_BUNDLES_DIR") {
        return PathBuf::from(override_dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/capability_bundles")
}

/// Load every `*.json` capability bundle from a real directory. Unknown files
/// without an `id` are rejected; empty directories are an error so callers cannot
/// silently invent capabilities.
pub fn load_capability_directory(dir: &Path) -> Result<Vec<CapabilityBundle>, String> {
    if !dir.is_dir() {
        return Err(format!("能力目录不存在或不是目录：{}", dir.display()));
    }
    let mut bundles = Vec::new();
    let mut seen = BTreeSet::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|error| format!("无法读取能力目录 {}：{error}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        })
        .collect();
    entries.sort();
    if entries.is_empty() {
        return Err(format!("能力目录为空，拒绝使用虚构能力：{}", dir.display()));
    }
    for path in entries {
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("无法读取能力包 {}：{error}", path.display()))?;
        let bundle: CapabilityBundle = serde_json::from_str(&raw)
            .map_err(|error| format!("能力包 JSON 无效 {}：{error}", path.display()))?;
        if bundle.id.trim().is_empty() {
            return Err(format!("能力包缺少 id：{}", path.display()));
        }
        if !seen.insert(bundle.id.clone()) {
            return Err(format!("能力包 id 重复：{}", bundle.id));
        }
        bundles.push(bundle);
    }
    Ok(bundles)
}

pub fn capability_index(dir: &Path) -> Result<BTreeMap<String, CapabilityBundle>, String> {
    let bundles = load_capability_directory(dir)?;
    Ok(bundles
        .into_iter()
        .map(|bundle| (bundle.id.clone(), bundle))
        .collect())
}

/// New drafts require the canonical role spelling. Legacy aliases remain
/// readable in historical rows, but must never create new Coordinator drafts
/// through the display parser's unknown-role fallback.
pub fn parse_draft_role(value: &str) -> Result<AgentRole, String> {
    AgentRole::try_parse(value)
        .filter(|role| role.as_str() == value)
        .ok_or_else(|| "角色配置草稿使用了未知或非规范的角色".into())
}

/// Every capability id on the draft must exist in the real directory and be
/// compatible with the draft's role. Invented ids are rejected.
pub fn validate_draft_capabilities(
    draft: &RoleConfigDraft,
    directory: &Path,
) -> Result<(), String> {
    if draft.capability_bundle_ids.is_empty() {
        return Err("角色配置草稿至少选择一个真实能力包".into());
    }
    let index = capability_index(directory)?;
    let role = parse_draft_role(&draft.role)?;
    for id in &draft.capability_bundle_ids {
        let Some(bundle) = index.get(id) else {
            return Err(format!(
                "能力包 `{id}` 不在真实目录 {} 中，拒绝写入草稿",
                directory.display()
            ));
        };
        if !bundle.compatible_roles.is_empty()
            && !bundle
                .compatible_roles
                .iter()
                .any(|item| item == role.as_str())
        {
            return Err(format!("能力包 `{id}` 与角色 `{}` 不兼容", role.as_str()));
        }
    }
    Ok(())
}

/// Persist a draft. Never touches `agent_runs` or `agent_messages`.
pub fn save_role_config_draft(
    connection: &Connection,
    draft: &RoleConfigDraft,
    directory: &Path,
) -> Result<RoleConfigDraft, String> {
    if draft.is_executable() {
        return Err("内部错误：草稿被标记为可执行".into());
    }
    if draft.status != "draft" {
        return Err(format!(
            "角色配置只接受 status=draft，收到 `{}`",
            draft.status
        ));
    }
    validate_draft_capabilities(draft, directory)?;
    let capability_json = serde_json::to_string(&draft.capability_bundle_ids)
        .map_err(|error| format!("序列化能力包列表失败：{error}"))?;
    connection
        .execute(
            "INSERT INTO agent_role_config_drafts(
                id, display_name, description, role, objective,
                capability_bundle_ids_json, status, status_label, updated_at
            ) VALUES(?1,?2,?3,?4,?5,?6,'draft',?7,datetime('now','localtime'))
            ON CONFLICT(id) DO UPDATE SET
                display_name=excluded.display_name,
                description=excluded.description,
                role=excluded.role,
                objective=excluded.objective,
                capability_bundle_ids_json=excluded.capability_bundle_ids_json,
                status='draft',
                status_label=excluded.status_label,
                updated_at=datetime('now','localtime')",
            params![
                draft.id,
                draft.display_name,
                draft.description,
                parse_draft_role(&draft.role)?.as_str(),
                draft.objective,
                capability_json,
                DRAFT_STATUS_LABEL,
            ],
        )
        .map_err(|error| format!("写入角色配置草稿失败：{error}"))?;
    load_role_config_draft(connection, &draft.id)?
        .ok_or_else(|| "写入后无法读回角色配置草稿".into())
}

pub fn load_role_config_draft(
    connection: &Connection,
    id: &str,
) -> Result<Option<RoleConfigDraft>, String> {
    connection
        .query_row(
            "SELECT id, display_name, description, role, objective,
                    capability_bundle_ids_json, status, status_label,
                    created_at, updated_at
             FROM agent_role_config_drafts WHERE id=?1",
            [id],
            row_to_draft,
        )
        .optional()
        .map_err(|error| format!("读取角色配置草稿失败：{error}"))
}

pub fn list_role_config_drafts(connection: &Connection) -> Result<Vec<RoleConfigDraft>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, display_name, description, role, objective,
                    capability_bundle_ids_json, status, status_label,
                    created_at, updated_at
             FROM agent_role_config_drafts
             ORDER BY updated_at DESC, id ASC",
        )
        .map_err(|error| format!("列出角色配置草稿失败：{error}"))?;
    let rows = statement
        .query_map([], row_to_draft)
        .map_err(|error| format!("列出角色配置草稿失败：{error}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|error| format!("读取角色配置草稿行失败：{error}"))?);
    }
    Ok(out)
}

fn row_to_draft(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoleConfigDraft> {
    let capability_json: String = row.get(5)?;
    let capability_bundle_ids: Vec<String> =
        serde_json::from_str(&capability_json).unwrap_or_default();
    Ok(RoleConfigDraft {
        id: row.get(0)?,
        display_name: row.get(1)?,
        description: row.get(2)?,
        role: row.get(3)?,
        objective: row.get(4)?,
        capability_bundle_ids,
        status: row.get(6)?,
        status_label: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

#[cfg(test)]
include!("role_config/tests.rs");
