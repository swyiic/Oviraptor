// O1 项目预填闭环（设计文档 NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08 §4/§5/§11）。
// - 唯一共享组织目录 org_entities / org_entity_identifiers：只记录组织事实，不携带任何授权。
// - project_org_bindings：项目绑定集团总账或单位；绑定不授予任何权限，授权仍按各项目 asset_ownership_decisions。
// - 候选建议只读：由当前项目 company 标签按确定规则生成；地名只作线索，CIDR/ASN 只作网络线索，不生成法人。
// - 写目录与绑定使用 CAS（目录版本 + 绑定版本），冲突不写入，前端草稿保留。

const ORG_DIRECTORY_CONFLICT: &str = "ORG_DIRECTORY_VERSION_CONFLICT";
const PROJECT_ORG_BINDING_CONFLICT: &str = "PROJECT_ORG_BINDING_VERSION_CONFLICT";
const ORG_DIRECTORY_INVALID: &str = "ORG_DIRECTORY_INPUT_INVALID";
const ORG_ENTITY_KINDS: [&str; 9] = [
    "group",
    "headquarters",
    "province_company",
    "city_branch",
    "county_branch",
    "professional_subsidiary",
    "overseas_entity",
    "supplier",
    "unknown",
];
const ORG_NAME_IDENTIFIER_KINDS: [&str; 4] =
    ["short_name", "english_name", "historical_name", "brand"];
const ORG_DOMAIN_IDENTIFIER_KINDS: [&str; 2] = ["official_domain", "shared_domain"];
const ORG_PROVINCE_NAMES: [&str; 34] = [
    "北京",
    "天津",
    "河北",
    "山西",
    "内蒙古",
    "辽宁",
    "吉林",
    "黑龙江",
    "上海",
    "江苏",
    "浙江",
    "安徽",
    "福建",
    "江西",
    "山东",
    "河南",
    "湖北",
    "湖南",
    "广东",
    "广西",
    "海南",
    "重庆",
    "四川",
    "贵州",
    "云南",
    "西藏",
    "陕西",
    "甘肃",
    "青海",
    "宁夏",
    "新疆",
    "香港",
    "澳门",
    "台湾",
];
/// 候选建议最多展开的公司标签数；超出部分只计数并在 truncatedLabels 中报告，不静默丢弃。
const ORG_SUGGESTION_LABEL_LIMIT: usize = 500;
const ORG_SUGGESTION_DOMAIN_LIMIT: usize = 50;
const ORG_DIRECTORY_ENTITY_LIMIT: usize = 5000;

/// 名称规范化只处理全/半角与空白、大小写；不删除法人后缀或行政词（§5）。
fn normalize_org_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch),
            '【' => '[',
            '】' => ']',
            _ => ch,
        })
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// company 为 CIDR / IP / ASN 时只是网络线索，不能生成法人（§5 示例 161.81.120.0/22）。
/// company 本身是域名（例如 cmi.chinamobile.com）时只是域名线索，不能生成法人。
fn org_label_is_domain_like(label: &str) -> bool {
    let value = label.trim().trim_end_matches('.');
    let Some(last) = value.rsplit('.').next() else {
        return false;
    };
    value.contains('.')
        && !value.contains(char::is_whitespace)
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '.')
        && last.len() >= 2
        && last.chars().all(|ch| ch.is_ascii_alphabetic())
}

fn org_label_is_network_clue(label: &str) -> bool {
    let value = label.trim();
    let first = value.split_whitespace().next().unwrap_or("").to_uppercase();
    if let Some(rest) = first.strip_prefix("AS") {
        if !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()) {
            return true;
        }
    }
    let head = value.split('/').next().unwrap_or("").trim();
    head.parse::<std::net::IpAddr>().is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OrgLabelGuess {
    kind: &'static str,
    /// 管理单位地域线索（省公司/市分支由法定名称结构给出）。
    region_clue: String,
    /// 所在地线索（例如专业子公司括号中的城市），不用于推断上级省公司。
    location_clue: String,
    group_prefix: String,
    /// 市分支名称中“…有限公司”前缀（规范化），用于按法定名称结构匹配上级，不按地名。
    parent_name_prefix: String,
    alias_like: bool,
}

fn org_paren_clue() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\(([^()]{2,10})\)").expect("static regex"))
}

/// 确定规则的单位类型猜测（只产生候选，不确认）。
fn guess_org_label(label: &str) -> OrgLabelGuess {
    let norm = normalize_org_name(label);
    let mut guess = OrgLabelGuess {
        kind: "unknown",
        region_clue: String::new(),
        location_clue: String::new(),
        group_prefix: String::new(),
        parent_name_prefix: String::new(),
        alias_like: false,
    };
    if let Some(index) = norm.find("集团") {
        let prefix = &norm[..index + "集团".len()];
        if prefix.chars().count() >= 4 {
            guess.group_prefix = prefix.to_string();
        }
    }
    if norm.ends_with("集团有限公司") || norm.ends_with("集团公司") || norm.ends_with("集团")
    {
        guess.kind = "group";
        return guess;
    }
    for province in ORG_PROVINCE_NAMES {
        if norm.ends_with(&format!("集团{province}有限公司"))
            || norm.ends_with(&format!("集团{province}分公司"))
        {
            guess.kind = "province_company";
            guess.region_clue = province.to_string();
            return guess;
        }
    }
    if let Some(before) = norm.strip_suffix("分公司") {
        if let Some(index) = before.rfind("有限公司") {
            let split = index + "有限公司".len();
            let tail = before[split..].trim_end_matches('市');
            if tail.chars().count() >= 2 {
                guess.kind = "city_branch";
                guess.region_clue = tail.to_string();
                guess.parent_name_prefix = before[..split].to_string();
                return guess;
            }
        }
    }
    if let Some(captures) = org_paren_clue().captures(&norm) {
        if norm.contains("公司") {
            guess.kind = "professional_subsidiary";
            guess.location_clue = captures[1].to_string();
            return guess;
        }
    }
    if let Some(province) = ORG_PROVINCE_NAMES
        .iter()
        .find(|province| norm.contains(**province))
    {
        if !norm.contains("公司") && norm.chars().count() <= 8 {
            guess.alias_like = true;
        }
        guess.location_clue = province.to_string();
    }
    guess
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgIdentifierView {
    pub id: i64,
    pub kind: String,
    pub value: String,
    pub source: String,
    pub source_ref: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgEntityView {
    pub id: i64,
    pub group_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub entity_kind: String,
    pub legal_name: String,
    pub region_code: String,
    pub unified_code: String,
    pub source: String,
    pub source_ref: String,
    pub directory_version: i64,
    pub identifiers: Vec<OrgIdentifierView>,
    #[serde(skip)]
    normalized_name: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOrgBindingView {
    pub project_kind: String,
    pub group_entity_id: i64,
    pub unit_entity_id: Option<i64>,
    pub binding_version: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOwnershipAuthorizationSummary {
    /// 仅当前项目自己的授权/准入计数；目录与其他项目不提供授权。
    pub allowed: i64,
    pub exposure_eligible: i64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOwnershipBootstrap {
    pub project_id: i64,
    pub project_name: String,
    pub profile: AssetOwnershipProfile,
    pub profile_saved: bool,
    /// 字段 → 来源：saved / directory / directory_exact_host（目录域名只按精确主机直接参与评估，不写入档案）/ none。
    /// 项目标题从不作为法定名称预填。
    pub profile_prefill_sources: std::collections::BTreeMap<String, String>,
    pub binding: Option<ProjectOrgBindingView>,
    pub directory_version: i64,
    pub entities: Vec<OrgEntityView>,
    pub entities_truncated: bool,
    pub scope_entity_ids: Vec<i64>,
    pub authorization: ProjectOwnershipAuthorizationSummary,
    pub data_revision: String,
    pub readiness: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipSuggestion {
    pub key: String,
    /// group / member_entity / alias / official_domain / legal_name / network_clue
    pub field: String,
    pub value: String,
    pub entity_id: Option<i64>,
    pub entity_kind: String,
    pub parent_entity_id: Option<i64>,
    pub parent_hint: String,
    pub region_clue: String,
    pub location_clue: String,
    pub source: String,
    pub support_rows: i64,
    pub conflict_rows: i64,
    pub examples_ref: Vec<i64>,
    pub reason: String,
    pub directory_version: i64,
    pub policy_version: i64,
    pub data_revision: String,
    /// saved / directory_confirmed / pending / conflict / unknown
    pub state: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipSuggestionPreview {
    pub project_id: i64,
    pub directory_version: i64,
    pub policy_version: i64,
    pub data_revision: String,
    pub scope_rows: i64,
    pub unlabeled_rows: i64,
    pub label_count: i64,
    pub truncated_labels: i64,
    pub suggestions: Vec<OwnershipSuggestion>,
    pub readiness: Vec<String>,
}

pub(crate) fn org_directory_version(connection: &rusqlite::Connection) -> Result<i64, String> {
    Ok(connection
        .query_row(
            "SELECT version FROM org_directory_state WHERE id=1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0))
}

pub(crate) fn load_org_entities(connection: &rusqlite::Connection) -> Result<Vec<OrgEntityView>, String> {
    let mut entities = {
        let mut statement = connection
            .prepare("SELECT id,group_id,parent_id,entity_kind,legal_name,region_code,unified_code,source,source_ref,directory_version,normalized_name FROM org_entities WHERE status='confirmed' ORDER BY id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(OrgEntityView {
                    id: row.get(0)?,
                    group_id: row.get(1)?,
                    parent_id: row.get(2)?,
                    entity_kind: row.get(3)?,
                    legal_name: row.get(4)?,
                    region_code: row.get(5)?,
                    unified_code: row.get(6)?,
                    source: row.get(7)?,
                    source_ref: row.get(8)?,
                    directory_version: row.get(9)?,
                    identifiers: Vec::new(),
                    normalized_name: row.get(10)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let index: HashMap<i64, usize> = entities
        .iter()
        .enumerate()
        .map(|(position, entity)| (entity.id, position))
        .collect();
    let mut statement = connection
        .prepare("SELECT entity_id,id,kind,value,source,source_ref FROM org_entity_identifiers ORDER BY id")
        .map_err(|error| error.to_string())?;
    let identifiers = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                OrgIdentifierView {
                    id: row.get(1)?,
                    kind: row.get(2)?,
                    value: row.get(3)?,
                    source: row.get(4)?,
                    source_ref: row.get(5)?,
                },
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (entity_id, identifier) in identifiers {
        if let Some(position) = index.get(&entity_id) {
            entities[*position].identifiers.push(identifier);
        }
    }
    Ok(entities)
}

pub(crate) fn load_project_org_binding(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<Option<ProjectOrgBindingView>, String> {
    connection
        .query_row(
            "SELECT project_kind,group_entity_id,unit_entity_id,binding_version,updated_at FROM project_org_bindings WHERE project_id=?1",
            [project_id],
            |row| Ok(ProjectOrgBindingView { project_kind: row.get(0)?, group_entity_id: row.get(1)?, unit_entity_id: row.get(2)?, binding_version: row.get(3)?, updated_at: row.get(4)? }),
        )
        .optional()
        .map_err(|error| error.to_string())
}

/// 绑定范围：集团总账 = 集团及其全部成员；单位项目 = 该单位及其下级（按 parent_id 闭包）。
pub(crate) fn org_binding_scope(
    entities: &[OrgEntityView],
    binding: Option<&ProjectOrgBindingView>,
) -> HashSet<i64> {
    let Some(binding) = binding else {
        return HashSet::new();
    };
    if binding.project_kind == "group_ledger" {
        return entities
            .iter()
            .filter(|entity| {
                entity.id == binding.group_entity_id
                    || entity.group_id == Some(binding.group_entity_id)
            })
            .map(|entity| entity.id)
            .collect();
    }
    let Some(unit) = binding.unit_entity_id else {
        return HashSet::new();
    };
    let mut scope = HashSet::from([unit]);
    loop {
        let before = scope.len();
        for entity in entities {
            if entity
                .parent_id
                .is_some_and(|parent| scope.contains(&parent))
            {
                scope.insert(entity.id);
            }
        }
        if scope.len() == before {
            break;
        }
    }
    scope.retain(|id| entities.iter().any(|entity| entity.id == *id));
    scope
}

/// 项目已绑定目录时供 assess 使用的确定性词表：名称/别名（带实体出处）、确认域名、共享域名。
#[derive(Debug, Default)]
struct ProjectDirectoryTerms {
    names: Vec<(String, String)>,
    official_domains: Vec<(String, String)>,
    shared_domains: Vec<(String, String)>,
}

fn project_directory_terms(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<ProjectDirectoryTerms, String> {
    let binding = load_project_org_binding(connection, project_id)?;
    if binding.is_none() {
        return Ok(ProjectDirectoryTerms::default());
    }
    let entities = load_org_entities(connection)?;
    let scope = org_binding_scope(&entities, binding.as_ref());
    let mut terms = ProjectDirectoryTerms::default();
    for entity in entities.iter().filter(|entity| scope.contains(&entity.id)) {
        terms
            .names
            .push((entity.legal_name.clone(), entity.legal_name.clone()));
        for identifier in &entity.identifiers {
            let pair = (identifier.value.clone(), entity.legal_name.clone());
            match identifier.kind.as_str() {
                "official_domain" => terms.official_domains.push(pair),
                "shared_domain" => terms.shared_domains.push(pair),
                kind if ORG_NAME_IDENTIFIER_KINDS.contains(&kind) => terms.names.push(pair),
                _ => {}
            }
        }
    }
    Ok(terms)
}

fn project_data_revision(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<(i64, String), String> {
    connection
        .query_row(
            &format!("SELECT COUNT(*),COALESCE(MAX(a.last_seen),''),COALESCE(MAX(o.updated_at),''),COALESCE(MAX(pa.last_seen),'') FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL}"),
            [project_id],
            |row| {
                let rows: i64 = row.get(0)?;
                Ok((rows, format!("rows={rows};seen={};decided={};linked={}", row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)))
            },
        )
        .map_err(|error| error.to_string())
}

fn ownership_readiness(
    binding: Option<&ProjectOrgBindingView>,
    profile_saved: bool,
) -> Vec<String> {
    let mut readiness = vec!["rules_only".to_string(), "model_not_ready".to_string()];
    if binding.is_none() {
        readiness.push("directory_unbound".into());
    }
    if !profile_saved {
        readiness.push("profile_unsaved".into());
    }
    readiness
}

fn ownership_bootstrap_for_connection(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<ProjectOwnershipBootstrap, String> {
    let project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id=?1",
            [project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "工作空间不存在".to_string())?;
    let mut profile = read_ownership_profile(connection, project_id)?;
    let profile_saved = !profile.updated_at.is_empty();
    let binding = load_project_org_binding(connection, project_id)?;
    let directory_version = org_directory_version(connection)?;
    let mut entities = load_org_entities(connection)?;
    let scope = org_binding_scope(&entities, binding.as_ref());
    let mut sources = std::collections::BTreeMap::new();
    let fields = [
        "legalName",
        "aliases",
        "approvedDomains",
        "sharedDomains",
        "jurisdictions",
    ];
    if profile_saved {
        for field in fields {
            sources.insert(field.to_string(), "saved".to_string());
        }
    } else {
        for field in fields {
            sources.insert(field.to_string(), "none".to_string());
        }
        if let Some(binding) = binding.as_ref() {
            let main_id = if binding.project_kind == "unit" {
                binding.unit_entity_id
            } else {
                Some(binding.group_entity_id)
            };
            if let Some(main) =
                main_id.and_then(|id| entities.iter().find(|entity| entity.id == id))
            {
                profile.legal_name = main.legal_name.clone();
                sources.insert("legalName".into(), "directory".into());
            }
            let mut aliases = Vec::new();
            let mut directory_domains = false;
            for entity in entities.iter().filter(|entity| scope.contains(&entity.id)) {
                if Some(entity.id) != main_id {
                    aliases.push(entity.legal_name.clone());
                }
                for identifier in &entity.identifiers {
                    if ORG_DOMAIN_IDENTIFIER_KINDS.contains(&identifier.kind.as_str()) {
                        directory_domains = true;
                    } else {
                        aliases.push(identifier.value.clone());
                    }
                }
            }
            if !aliases.is_empty() {
                profile.aliases = clean_ownership_list(aliases);
                sources.insert("aliases".into(), "directory".into());
            }
            // 目录域名是“精确主机”证据（§8 域规则默认精确 host），assess 直接按精确主机读取目录；
            // 档案的 approved/shared 域名字段按根域名匹配子域，复制过去会在保存后悄悄泛化到子域，所以不复制。
            if directory_domains {
                sources.insert("approvedDomains".into(), "directory_exact_host".into());
                sources.insert("sharedDomains".into(), "directory_exact_host".into());
            }
            // 管理单位地域不是服务/授权地域：jurisdictions 不从目录复制（§3）。
        }
    }
    let (allowed, exposure_eligible) = connection
        .query_row(
            &format!("SELECT COALESCE(SUM(CASE WHEN o.authorization_status='allowed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.exposure_eligible=1 THEN 1 ELSE 0 END),0) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL}"),
            [project_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let (_, data_revision) = project_data_revision(connection, project_id)?;
    let entities_truncated = entities.len() > ORG_DIRECTORY_ENTITY_LIMIT;
    if entities_truncated {
        // 目录过大时只回传集团与本项目范围实体，并明确标注截断。
        entities.retain(|entity| entity.entity_kind == "group" || scope.contains(&entity.id));
    }
    let mut scope_entity_ids: Vec<i64> = scope.into_iter().collect();
    scope_entity_ids.sort_unstable();
    Ok(ProjectOwnershipBootstrap {
        project_id,
        project_name,
        readiness: ownership_readiness(binding.as_ref(), profile_saved),
        profile,
        profile_saved,
        profile_prefill_sources: sources,
        binding,
        directory_version,
        entities,
        entities_truncated,
        scope_entity_ids,
        authorization: ProjectOwnershipAuthorizationSummary {
            allowed,
            exposure_eligible,
        },
        data_revision,
    })
}

#[tauri::command]
pub fn get_project_ownership_bootstrap(
    state: State<AppState>,
    project_id: i64,
) -> Result<ProjectOwnershipBootstrap, String> {
    get_project_ownership_bootstrap_at(&state.db_path, project_id)
}

/// Shared desktop/application entry. Paths are supplied only by the trusted owner.
pub(crate) fn get_project_ownership_bootstrap_at(
    db_path: &std::path::Path,
    project_id: i64,
) -> Result<ProjectOwnershipBootstrap, String> {
    ownership_bootstrap_for_connection(&db::open(db_path)?, project_id)
}

struct OrgLabelAggregate {
    display: String,
    raw_labels: Vec<String>,
    rows: i64,
    conflict_rows: i64,
}

fn ownership_suggestions_for_connection(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<OwnershipSuggestionPreview, String> {
    let project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id=?1",
            [project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "工作空间不存在".to_string())?;
    let profile = read_ownership_profile(connection, project_id)?;
    let profile_saved = !profile.updated_at.is_empty();
    let policy_version = if profile_saved {
        profile.policy_version
    } else {
        0
    };
    let binding = load_project_org_binding(connection, project_id)?;
    let directory_version = org_directory_version(connection)?;
    let entities = load_org_entities(connection)?;
    let scope = org_binding_scope(&entities, binding.as_ref());
    let (scope_rows, data_revision) = project_data_revision(connection, project_id)?;
    let by_id: HashMap<i64, &OrgEntityView> =
        entities.iter().map(|entity| (entity.id, entity)).collect();
    let mut name_index: HashMap<String, Vec<i64>> = HashMap::new();
    for entity in &entities {
        name_index
            .entry(entity.normalized_name.clone())
            .or_default()
            .push(entity.id);
        for identifier in entity
            .identifiers
            .iter()
            .filter(|identifier| ORG_NAME_IDENTIFIER_KINDS.contains(&identifier.kind.as_str()))
        {
            let ids = name_index
                .entry(normalize_org_name(&identifier.value))
                .or_default();
            if !ids.contains(&entity.id) {
                ids.push(entity.id);
            }
        }
    }
    let group_of = |entity: &OrgEntityView| {
        if entity.entity_kind == "group" {
            entity.id
        } else {
            entity.group_id.unwrap_or(entity.id)
        }
    };
    let bound_group = binding.as_ref().map(|binding| binding.group_entity_id);
    let find_group = |prefix: &str| -> Option<i64> {
        if prefix.is_empty() {
            return None;
        }
        entities
            .iter()
            .filter(|entity| entity.entity_kind == "group")
            .find(|entity| {
                entity.normalized_name.starts_with(prefix)
                    || entity
                        .identifiers
                        .iter()
                        .any(|identifier| normalize_org_name(&identifier.value) == prefix)
            })
            .map(|entity| entity.id)
    };
    let suggestion =
        |key: String, field: &str, value: String, state: &str, source: &str, reason: String| {
            OwnershipSuggestion {
                key,
                field: field.into(),
                value,
                entity_id: None,
                entity_kind: String::new(),
                parent_entity_id: None,
                parent_hint: String::new(),
                region_clue: String::new(),
                location_clue: String::new(),
                source: source.into(),
                support_rows: 0,
                conflict_rows: 0,
                examples_ref: Vec::new(),
                reason,
                directory_version,
                policy_version,
                data_revision: data_revision.clone(),
                state: state.into(),
            }
        };

    // 1. 公司标签聚合（只读）。
    let mut aggregates: Vec<OrgLabelAggregate> = Vec::new();
    let mut unlabeled_rows = 0i64;
    {
        let mut statement = connection
            .prepare(&format!("SELECT a.company,COUNT(*),COALESCE(SUM(CASE WHEN o.source='manual' AND o.status IN ('excluded','third_party') THEN 1 ELSE 0 END),0) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL} GROUP BY a.company ORDER BY COUNT(*) DESC,a.company"))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([project_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let mut positions: HashMap<String, usize> = HashMap::new();
        for (label, rows, conflicts) in rows {
            let normalized = normalize_org_name(&label);
            if normalized.is_empty() {
                unlabeled_rows += rows;
                continue;
            }
            match positions.get(&normalized) {
                Some(position) => {
                    let aggregate = &mut aggregates[*position];
                    aggregate.rows += rows;
                    aggregate.conflict_rows += conflicts;
                    aggregate.raw_labels.push(label);
                }
                None => {
                    positions.insert(normalized, aggregates.len());
                    aggregates.push(OrgLabelAggregate {
                        display: label.trim().to_string(),
                        raw_labels: vec![label],
                        rows,
                        conflict_rows: conflicts,
                    });
                }
            }
        }
    }
    aggregates.sort_by(|left, right| {
        right
            .rows
            .cmp(&left.rows)
            .then_with(|| left.display.cmp(&right.display))
    });
    let label_count = aggregates.len() as i64;
    let truncated_labels = aggregates.len().saturating_sub(ORG_SUGGESTION_LABEL_LIMIT) as i64;
    aggregates.truncate(ORG_SUGGESTION_LABEL_LIMIT);

    let mut examples_statement = connection
        .prepare(&format!("SELECT a.id FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL} AND a.company=?2 ORDER BY a.id LIMIT 3"))
        .map_err(|error| error.to_string())?;
    let mut suggestions = Vec::new();
    let mut group_candidates: HashMap<String, (i64, i64, Vec<i64>)> = HashMap::new();
    let mut matched_labels: HashMap<i64, Vec<String>> = HashMap::new();
    let mut label_groups: HashSet<i64> = HashSet::new();
    let mut label_group_prefixes: HashSet<String> = HashSet::new();
    for aggregate in &aggregates {
        let examples: Vec<i64> = examples_statement
            .query_map(params![project_id, aggregate.raw_labels[0]], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let normalized = normalize_org_name(&aggregate.display);
        let key = format!("label:{normalized}");
        if org_label_is_network_clue(&aggregate.display) {
            let mut item = suggestion(
                key,
                "network_clue",
                aggregate.display.clone(),
                "unknown",
                "company_label",
                "company 为 CIDR / IP / ASN，只是网络登记线索，不能据此生成法人或归属".into(),
            );
            item.support_rows = aggregate.rows;
            item.conflict_rows = aggregate.conflict_rows;
            item.examples_ref = examples;
            suggestions.push(item);
            continue;
        }
        if org_label_is_domain_like(&aggregate.display) {
            let mut item = suggestion(
                key,
                "domain_clue",
                aggregate.display.clone(),
                "unknown",
                "company_label",
                "company 字段是域名，只能作为域名线索，不能据此生成法人或单位".into(),
            );
            item.support_rows = aggregate.rows;
            item.conflict_rows = aggregate.conflict_rows;
            item.examples_ref = examples;
            suggestions.push(item);
            continue;
        }
        let guess = guess_org_label(&aggregate.display);
        if !guess.group_prefix.is_empty() {
            let entry = group_candidates
                .entry(guess.group_prefix.clone())
                .or_insert((0, 0, Vec::new()));
            entry.0 += aggregate.rows;
            entry.1 += aggregate.conflict_rows;
            for id in &examples {
                if entry.2.len() < 3 {
                    entry.2.push(*id);
                }
            }
        }
        let matches = name_index.get(&normalized).cloned().unwrap_or_default();
        let mut item = suggestion(
            key,
            if guess.alias_like {
                "alias"
            } else {
                "member_entity"
            },
            aggregate.display.clone(),
            "pending",
            "company_label",
            String::new(),
        );
        item.support_rows = aggregate.rows;
        item.conflict_rows = aggregate.conflict_rows;
        item.examples_ref = examples;
        item.region_clue = guess.region_clue.clone();
        item.location_clue = guess.location_clue.clone();
        match matches.as_slice() {
            [entity_id] => {
                let entity = by_id[entity_id];
                item.entity_id = Some(entity.id);
                item.entity_kind = entity.entity_kind.clone();
                item.parent_entity_id = entity.parent_id;
                item.field = if entity.entity_kind == "group" {
                    "group"
                } else {
                    "member_entity"
                }
                .into();
                if entity.entity_kind == "group" {
                    label_groups.insert(entity.id);
                }
                if bound_group.is_some_and(|group| group != group_of(entity)) {
                    item.state = "conflict".into();
                    item.reason = format!(
                        "标签匹配目录实体「{}」，但该实体不属于本项目绑定的集团",
                        entity.legal_name
                    );
                } else if binding.is_some() && !scope.contains(&entity.id) {
                    item.state = "conflict".into();
                    item.reason = format!(
                        "标签匹配目录实体「{}」，但不在本项目绑定的单位范围内",
                        entity.legal_name
                    );
                } else {
                    item.state = "directory_confirmed".into();
                    item.reason =
                        format!("规范化名称精确匹配已确认目录实体「{}」", entity.legal_name);
                    matched_labels
                        .entry(entity.id)
                        .or_default()
                        .extend(aggregate.raw_labels.iter().cloned());
                }
            }
            [] => {
                item.entity_kind = guess.kind.to_string();
                let mut reasons = Vec::new();
                match guess.kind {
                    "group" => {
                        item.field = "group".into();
                        label_group_prefixes.insert(guess.group_prefix.clone());
                        reasons.push("法定名称以“集团（有限）公司”结尾，候选集团；首次需人工确认".into());
                    }
                    "province_company" => reasons.push(format!("法定名称结构为“集团{}有限公司”，候选省公司", guess.region_clue)),
                    "city_branch" => {
                        reasons.push(format!("法定名称结构为“…有限公司{}分公司”，候选市分支", guess.region_clue));
                        if let Some(parent) = name_index.get(&guess.parent_name_prefix).and_then(|ids| (ids.len() == 1).then(|| ids[0])) {
                            item.parent_entity_id = Some(parent);
                            item.parent_hint = by_id[&parent].legal_name.clone();
                        }
                    }
                    "professional_subsidiary" => reasons.push(format!("括号内“{}”仅作所在地线索；候选专业子公司，上级按集团处理，不按地名归入省公司", guess.location_clue)),
                    _ if guess.alias_like => reasons.push(format!("像简称/品牌，“{}”只是地域线索；需人工绑定到目录实体后才生效", guess.location_clue)),
                    _ => reasons.push("单位类型未知，需人工确认".into()),
                }
                if guess.kind == "city_branch" && item.parent_entity_id.is_none() {
                    // 市分支的上级只按法定名称结构匹配；不回退到集团或按地名猜省公司。
                    reasons.push("上级单位未在目录中确认，需先确认上级".into());
                } else if guess.kind != "group" && item.parent_entity_id.is_none() {
                    // 专业子公司/省公司/未知单位：候选上级为集团，绝不按地名归入省公司。
                    if let Some(group) = bound_group.or_else(|| find_group(&guess.group_prefix)) {
                        item.parent_entity_id = Some(group);
                        item.parent_hint = by_id
                            .get(&group)
                            .map(|entity| entity.legal_name.clone())
                            .unwrap_or_default();
                    } else if !guess.group_prefix.is_empty() {
                        item.parent_hint = guess.group_prefix.clone();
                    }
                }
                if aggregate.conflict_rows > 0 {
                    reasons.push(format!(
                        "{} 行已有人工排除/第三方结论",
                        aggregate.conflict_rows
                    ));
                }
                item.reason = reasons.join("；");
            }
            many => {
                item.state = "conflict".into();
                item.reason = format!(
                    "规范化名称同时匹配 {} 个目录实体（{}），不自动选择",
                    many.len(),
                    many.iter()
                        .map(|id| by_id[id].legal_name.clone())
                        .collect::<Vec<_>>()
                        .join("、")
                );
            }
        }
        suggestions.push(item);
    }

    // 2. 集团建议：已绑定 → saved；目录中名称出现在项目标题或标签中 → directory_confirmed；标签前缀 → pending。
    if let Some(binding) = binding.as_ref() {
        if let Some(group) = by_id.get(&binding.group_entity_id) {
            let mut item = suggestion(
                format!("group:{}", group.id),
                "group",
                group.legal_name.clone(),
                "saved",
                "project_binding",
                format!(
                    "本项目已绑定为{}",
                    if binding.project_kind == "unit" {
                        "单位项目"
                    } else {
                        "集团总账"
                    }
                ),
            );
            item.entity_id = Some(group.id);
            item.entity_kind = "group".into();
            suggestions.insert(0, item);
        }
    } else {
        let project_norm = normalize_org_name(&project_name);
        let mut seen_groups = label_groups.clone();
        let mut group_candidates: Vec<_> = group_candidates.into_iter().collect();
        // insert(0, …) 逆序插入：按支持行数升序遍历，最终展示为支持行数降序，结果稳定。
        group_candidates.sort_by(|left, right| {
            left.1
                 .0
                .cmp(&right.1 .0)
                .then_with(|| right.0.cmp(&left.0))
        });
        for (prefix, (rows, conflicts, examples)) in &group_candidates {
            if let Some(group) = find_group(prefix) {
                if seen_groups.insert(group) {
                    let mut item = suggestion(
                        format!("group:{group}"),
                        "group",
                        by_id[&group].legal_name.clone(),
                        "directory_confirmed",
                        "company_label",
                        format!("标签前缀“{prefix}”匹配已确认集团"),
                    );
                    item.entity_id = Some(group);
                    item.entity_kind = "group".into();
                    item.support_rows = *rows;
                    item.conflict_rows = *conflicts;
                    item.examples_ref = examples.clone();
                    suggestions.insert(0, item);
                }
            } else if !label_group_prefixes.contains(prefix) {
                let mut item = suggestion(
                    format!("group-prefix:{prefix}"),
                    "group",
                    prefix.clone(),
                    "pending",
                    "company_label",
                    "company 标签共同前缀；首次需人工确认集团正式名称（前缀不一定是法定名称）"
                        .into(),
                );
                item.entity_kind = "group".into();
                item.support_rows = *rows;
                item.conflict_rows = *conflicts;
                item.examples_ref = examples.clone();
                suggestions.insert(0, item);
            }
        }
        for entity in entities
            .iter()
            .filter(|entity| entity.entity_kind == "group" && !seen_groups.contains(&entity.id))
        {
            let terms = std::iter::once(entity.normalized_name.clone()).chain(
                entity
                    .identifiers
                    .iter()
                    .filter(|identifier| {
                        ORG_NAME_IDENTIFIER_KINDS.contains(&identifier.kind.as_str())
                    })
                    .map(|identifier| normalize_org_name(&identifier.value)),
            );
            if let Some(term) = terms
                .filter(|term| term.chars().count() >= 2)
                .find(|term| project_norm.contains(term.as_str()))
            {
                let mut item = suggestion(
                    format!("group:{}", entity.id),
                    "group",
                    entity.legal_name.clone(),
                    "directory_confirmed",
                    "project_title",
                    format!("项目标题包含已确认集团名称/简称“{term}”"),
                );
                item.entity_id = Some(entity.id);
                item.entity_kind = "group".into();
                suggestions.insert(0, item);
            }
        }
    }

    // 3. 法定名称：已保存档案 → saved；否则项目标题只作待确认建议，不直接当法人。
    if profile_saved {
        suggestions.insert(
            0,
            suggestion(
                "legal-name:saved".into(),
                "legal_name",
                profile.legal_name.clone(),
                "saved",
                "saved_profile",
                format!("已保存档案 v{}", profile.policy_version),
            ),
        );
    } else {
        suggestions.insert(
            0,
            suggestion(
                "legal-name:title".into(),
                "legal_name",
                project_name.clone(),
                "pending",
                "project_title",
                "项目标题不一定是法人名称，需人工确认或绑定目录实体".into(),
            ),
        );
    }

    // 4. 域名建议：仅对已匹配本项目范围目录实体的标签，按精确主机聚合；新域名只建议，不自动生效。
    if !matched_labels.is_empty() {
        let existing: HashSet<String> = entities
            .iter()
            .flat_map(|entity| entity.identifiers.iter())
            .filter(|identifier| ORG_DOMAIN_IDENTIFIER_KINDS.contains(&identifier.kind.as_str()))
            .map(|identifier| normalize_ownership_host(&identifier.value))
            .collect();
        let mut hosts: HashMap<(i64, String), (i64, i64, Vec<i64>)> = HashMap::new();
        let mut statement = connection
            .prepare(&format!("SELECT a.id,a.link,a.host,CASE WHEN o.source='manual' AND o.status IN ('excluded','third_party') THEN 1 ELSE 0 END FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL} AND a.company=?2 ORDER BY a.id LIMIT 50000"))
            .map_err(|error| error.to_string())?;
        for (entity_id, labels) in &matched_labels {
            for label in labels {
                let rows = statement
                    .query_map(params![project_id, label], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })
                    .map_err(|error| error.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                for (asset_id, link, host, conflict) in rows {
                    let host = ownership_endpoint_host(&link, &host);
                    if host.is_empty()
                        || !host.contains('.')
                        || host.parse::<std::net::IpAddr>().is_ok()
                        || existing.contains(&host)
                    {
                        continue;
                    }
                    let entry = hosts
                        .entry((*entity_id, host))
                        .or_insert((0, 0, Vec::new()));
                    entry.0 += 1;
                    entry.1 += conflict;
                    if entry.2.len() < 3 {
                        entry.2.push(asset_id);
                    }
                }
            }
        }
        let mut hosts: Vec<_> = hosts.into_iter().collect();
        hosts.sort_by(|left, right| {
            right
                .1
                 .0
                .cmp(&left.1 .0)
                .then_with(|| left.0 .1.cmp(&right.0 .1))
        });
        for ((entity_id, host), (rows, conflicts, examples)) in
            hosts.into_iter().take(ORG_SUGGESTION_DOMAIN_LIMIT)
        {
            let entity = by_id[&entity_id];
            let mut item = suggestion(format!("domain:{entity_id}:{host}"), "official_domain", host.clone(), if conflicts > 0 { "conflict" } else { "pending" }, "asset_host", format!("{rows} 行标签为「{}」的资产使用该精确主机；只建议精确主机，不泛化到根域或共享平台", entity.legal_name));
            item.entity_id = Some(entity_id);
            item.entity_kind = entity.entity_kind.clone();
            item.support_rows = rows;
            item.conflict_rows = conflicts;
            item.examples_ref = examples;
            suggestions.push(item);
        }
    }

    Ok(OwnershipSuggestionPreview {
        project_id,
        directory_version,
        policy_version,
        data_revision,
        scope_rows,
        unlabeled_rows,
        label_count,
        truncated_labels,
        suggestions,
        readiness: ownership_readiness(binding.as_ref(), profile_saved),
    })
}

#[tauri::command]
pub fn preview_project_ownership_suggestions(
    state: State<AppState>,
    project_id: i64,
) -> Result<OwnershipSuggestionPreview, String> {
    preview_project_ownership_suggestions_at(&state.db_path, project_id)
}

/// Shared desktop/application entry. Paths are supplied only by the trusted owner.
pub(crate) fn preview_project_ownership_suggestions_at(
    db_path: &std::path::Path,
    project_id: i64,
) -> Result<OwnershipSuggestionPreview, String> {
    ownership_suggestions_for_connection(&db::open(db_path)?, project_id)
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgEntityRef {
    #[serde(default)]
    pub entity_id: Option<i64>,
    #[serde(default)]
    pub draft_key: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgEntityDraft {
    pub draft_key: String,
    pub legal_name: String,
    pub entity_kind: String,
    #[serde(default)]
    pub parent: Option<OrgEntityRef>,
    #[serde(default)]
    pub region_code: String,
    #[serde(default)]
    pub unified_code: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub source_ref: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgIdentifierDraft {
    pub entity: OrgEntityRef,
    pub kind: String,
    pub value: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub source_ref: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOrgBindingDraft {
    pub project_kind: String,
    pub group: OrgEntityRef,
    #[serde(default)]
    pub unit: Option<OrgEntityRef>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOwnershipDirectorySave {
    pub project_id: i64,
    pub expected_directory_version: i64,
    pub expected_binding_version: i64,
    #[serde(default)]
    pub entities: Vec<OrgEntityDraft>,
    #[serde(default)]
    pub identifiers: Vec<OrgIdentifierDraft>,
    #[serde(default)]
    pub binding: Option<ProjectOrgBindingDraft>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgCreatedEntity {
    pub draft_key: String,
    pub entity_id: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOwnershipDirectorySaveResult {
    pub project_id: i64,
    pub directory_version: i64,
    pub binding_version: i64,
    pub created_entities: Vec<OrgCreatedEntity>,
    pub created_identifiers: i64,
}

fn clean_org_text(value: &str, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

/// O1 目录与绑定保存：IMMEDIATE 事务内比对目录版本与项目绑定版本，任一不一致整批拒绝。
/// 只写组织目录/绑定/目录审计；不写 asset_ownership_decisions、规则或任何授权。
fn save_project_ownership_directory_for_connection(
    connection: &mut rusqlite::Connection,
    input: ProjectOwnershipDirectorySave,
) -> Result<ProjectOwnershipDirectorySaveResult, String> {
    if input.entities.is_empty() && input.identifiers.is_empty() && input.binding.is_none() {
        return Err(format!(
            "{ORG_DIRECTORY_INVALID}: 没有需要保存的目录或绑定变更"
        ));
    }
    if input.entities.len() > 500 || input.identifiers.len() > 2000 {
        return Err(format!(
            "{ORG_DIRECTORY_INVALID}: 单次最多新增 500 个实体、2000 个标识"
        ));
    }
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let project_exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            [input.project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !project_exists {
        return Err("工作空间不存在，未写入".into());
    }
    let current_directory = org_directory_version(&transaction)?;
    if current_directory != input.expected_directory_version {
        return Err(format!("{ORG_DIRECTORY_CONFLICT}: 组织目录已被其他操作更新（当前 v{current_directory}，页面基于 v{}），本次未写入；草稿仍保留，请载入最新目录后再保存", input.expected_directory_version));
    }
    let current_binding: i64 = transaction
        .query_row(
            "SELECT binding_version FROM project_org_bindings WHERE project_id=?1",
            [input.project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0);
    if current_binding != input.expected_binding_version {
        return Err(format!("{PROJECT_ORG_BINDING_CONFLICT}: 项目目录绑定已被其他操作更新（当前 v{current_binding}，页面基于 v{}），本次未写入；草稿仍保留", input.expected_binding_version));
    }
    let directory_changes = !input.entities.is_empty() || !input.identifiers.is_empty();
    let next_directory = if directory_changes {
        current_directory + 1
    } else {
        current_directory
    };
    let mut created: HashMap<String, i64> = HashMap::new();
    let mut created_entities = Vec::new();
    // (id, kind, group_id)
    let lookup = |transaction: &rusqlite::Transaction<'_>,
                  id: i64|
     -> Result<(String, Option<i64>), String> {
        transaction
            .query_row(
                "SELECT entity_kind,group_id FROM org_entities WHERE id=?1 AND status='confirmed'",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{ORG_DIRECTORY_INVALID}: 目录实体 #{id} 不存在或已停用"))
    };
    let resolve =
        |created: &HashMap<String, i64>, reference: &OrgEntityRef| -> Result<i64, String> {
            match (reference.entity_id, reference.draft_key.as_deref()) {
                (Some(id), None) if id > 0 => Ok(id),
                (None, Some(key)) => created.get(key).copied().ok_or_else(|| {
                    format!("{ORG_DIRECTORY_INVALID}: 草稿实体 {key} 未在本次保存中先行创建")
                }),
                _ => Err(format!(
                    "{ORG_DIRECTORY_INVALID}: 实体引用必须且只能指定 entityId 或 draftKey"
                )),
            }
        };
    for draft in &input.entities {
        let legal_name = clean_org_text(&draft.legal_name, 240);
        let normalized = normalize_org_name(&legal_name);
        if normalized.chars().count() < 2 {
            return Err(format!(
                "{ORG_DIRECTORY_INVALID}: 实体正式名称至少 2 个字符"
            ));
        }
        if org_label_is_domain_like(&legal_name) {
            return Err(format!(
                "{ORG_DIRECTORY_INVALID}: 「{legal_name}」是域名，不能作为法人或单位名称"
            ));
        }
        if org_label_is_network_clue(&legal_name) {
            return Err(format!("{ORG_DIRECTORY_INVALID}: 「{legal_name}」是 CIDR / IP / ASN 网络线索，不能作为法人或单位"));
        }
        if !ORG_ENTITY_KINDS.contains(&draft.entity_kind.as_str()) {
            return Err(format!(
                "{ORG_DIRECTORY_INVALID}: 单位类型 {} 无效",
                draft.entity_kind
            ));
        }
        let key = draft.draft_key.trim();
        if key.is_empty() || created.contains_key(key) {
            return Err(format!(
                "{ORG_DIRECTORY_INVALID}: 草稿实体缺少唯一 draftKey"
            ));
        }
        if let Some(existing) = transaction
            .query_row(
                "SELECT id,legal_name FROM org_entities WHERE normalized_name=?1",
                [&normalized],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?
        {
            return Err(format!("{ORG_DIRECTORY_INVALID}: 目录已存在同名实体「{}」(#{})，请直接绑定已有实体，未写入", existing.1, existing.0));
        }
        let (parent_id, group_id) = if draft.entity_kind == "group" {
            if draft.parent.is_some() {
                return Err(format!("{ORG_DIRECTORY_INVALID}: 集团实体不能设置上级"));
            }
            (None, None)
        } else {
            let parent = draft.parent.as_ref().ok_or_else(|| {
                format!("{ORG_DIRECTORY_INVALID}: 「{legal_name}」必须指定上级（集团或其下单位）")
            })?;
            let parent_id = resolve(&created, parent)?;
            let (parent_kind, parent_group) = lookup(&transaction, parent_id)?;
            let group_id = if parent_kind == "group" {
                parent_id
            } else {
                parent_group.ok_or_else(|| {
                    format!("{ORG_DIRECTORY_INVALID}: 上级 #{parent_id} 未归属任何集团")
                })?
            };
            (Some(parent_id), Some(group_id))
        };
        let source = {
            let value = clean_org_text(&draft.source, 40);
            if value.is_empty() {
                "manual".to_string()
            } else {
                value
            }
        };
        transaction
            .execute(
                "INSERT INTO org_entities(group_id,parent_id,entity_kind,legal_name,normalized_name,unified_code,region_code,source,source_ref,directory_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![group_id, parent_id, draft.entity_kind, legal_name, normalized, clean_org_text(&draft.unified_code, 64), clean_org_text(&draft.region_code, 60), source, clean_org_text(&draft.source_ref, 400), next_directory],
            )
            .map_err(|error| error.to_string())?;
        let entity_id = transaction.last_insert_rowid();
        transaction
            .execute(
                "INSERT INTO org_directory_changes(directory_version,via_project_id,change_type,entity_id,payload_json) VALUES(?1,?2,'entity_added',?3,?4)",
                params![next_directory, input.project_id, entity_id, serde_json::json!({"legalName": legal_name, "entityKind": draft.entity_kind, "parentId": parent_id, "source": source, "sourceRef": draft.source_ref}).to_string()],
            )
            .map_err(|error| error.to_string())?;
        created.insert(key.to_string(), entity_id);
        created_entities.push(OrgCreatedEntity {
            draft_key: key.to_string(),
            entity_id,
        });
    }
    // Historical normalized_value may have collapsed www into the apex. The
    // raw value still preserves its exact host; never rewrite that evidence.
    let mut domain_index: HashMap<(String, String), HashSet<i64>> = HashMap::new();
    {
        let mut statement = transaction.prepare("SELECT entity_id,kind,value FROM org_entity_identifiers WHERE kind IN ('official_domain','shared_domain')").map_err(|error| error.to_string())?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))).map_err(|error| error.to_string())?;
        for row in rows {
            let (entity, kind, value) = row.map_err(|error| error.to_string())?;
            let host = normalize_ownership_host(&value);
            if !host.is_empty() { domain_index.entry((kind, host)).or_default().insert(entity); }
        }
    }
    let mut created_identifiers = 0i64;
    for draft in &input.identifiers {
        let entity_id = resolve(&created, &draft.entity)?;
        lookup(&transaction, entity_id)?;
        let kind = draft.kind.as_str();
        let value = clean_org_text(&draft.value, 240);
        let normalized = if ORG_DOMAIN_IDENTIFIER_KINDS.contains(&kind) {
            let host = normalize_ownership_host(&value);
            if host.is_empty() || !host.contains('.') || host.parse::<std::net::IpAddr>().is_ok() {
                return Err(format!(
                    "{ORG_DIRECTORY_INVALID}: 「{value}」不是有效域名（IP 不作为组织域名）"
                ));
            }
            format!("host-v2:{host}")
        } else if ORG_NAME_IDENTIFIER_KINDS.contains(&kind) {
            if org_label_is_network_clue(&value) {
                return Err(format!(
                    "{ORG_DIRECTORY_INVALID}: 「{value}」是网络线索，不能作为单位别名"
                ));
            }
            let name = normalize_org_name(&value);
            if name.chars().count() < 2 {
                return Err(format!("{ORG_DIRECTORY_INVALID}: 别名至少 2 个字符"));
            }
            name
        } else {
            return Err(format!("{ORG_DIRECTORY_INVALID}: 标识类型 {kind} 无效"));
        };
        let domain_key = ORG_DOMAIN_IDENTIFIER_KINDS.contains(&kind).then(|| (kind.to_string(), normalize_ownership_host(&value)));
        if let Some(key) = &domain_key {
            if let Some(owners) = domain_index.get(key) {
                if kind == "official_domain" {
                    if let Some(other) = owners.iter().filter(|other| **other != entity_id).min() {
                        return Err(format!("{ORG_DIRECTORY_INVALID}: 「{value}」已属于目录实体 #{other}；同一别名/确认域名不能指向两个单位，未写入"));
                    }
                }
                if owners.contains(&entity_id) { continue; } // explicit raw-host reuse, not an ignored insert
            }
        } else {
            let clash: Option<i64> = transaction
                .query_row(
                    "SELECT entity_id FROM org_entity_identifiers WHERE kind IN ('short_name','english_name','historical_name','brand') AND normalized_value=?1 AND entity_id<>?2 UNION ALL SELECT id FROM org_entities WHERE normalized_name=?1 AND id<>?2 LIMIT 1",
                    params![normalized, entity_id], |row| row.get(0),
                ).optional().map_err(|error| error.to_string())?;
            if let Some(other) = clash {
                return Err(format!("{ORG_DIRECTORY_INVALID}: 「{value}」已属于目录实体 #{other}；同一别名/确认域名不能指向两个单位，未写入"));
            }
        }
        let source = {
            let value = clean_org_text(&draft.source, 40);
            if value.is_empty() {
                "manual".to_string()
            } else {
                value
            }
        };
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO org_entity_identifiers(entity_id,kind,value,normalized_value,source,source_ref,directory_version) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![entity_id, kind, value, normalized, source, clean_org_text(&draft.source_ref, 400), next_directory],
            )
            .map_err(|error| error.to_string())? as i64;
        if let Some(key) = domain_key {
            if inserted != 1 {
                return Err(format!("{ORG_DIRECTORY_CONFLICT}: 精确主机唯一键与历史记录冲突，未新增任何标识，请重新核对目录"));
            }
            domain_index.entry(key).or_default().insert(entity_id);
        }
        if inserted > 0 {
            transaction
                .execute(
                    "INSERT INTO org_directory_changes(directory_version,via_project_id,change_type,entity_id,payload_json) VALUES(?1,?2,'identifier_added',?3,?4)",
                    params![next_directory, input.project_id, entity_id, serde_json::json!({"kind": kind, "value": value, "source": source, "sourceRef": draft.source_ref}).to_string()],
                )
                .map_err(|error| error.to_string())?;
        }
        created_identifiers += inserted;
    }
    let mut binding_version = current_binding;
    if let Some(binding) = &input.binding {
        let group_id = resolve(&created, &binding.group)?;
        let (group_kind, _) = lookup(&transaction, group_id)?;
        if group_kind != "group" {
            return Err(format!(
                "{ORG_DIRECTORY_INVALID}: 绑定的集团 #{group_id} 不是集团实体"
            ));
        }
        let unit_id = match (binding.project_kind.as_str(), binding.unit.as_ref()) {
            ("group_ledger", None) => None,
            ("group_ledger", Some(_)) => {
                return Err(format!(
                    "{ORG_DIRECTORY_INVALID}: 集团总账项目不绑定单一单位"
                ))
            }
            ("unit", Some(unit)) => {
                let unit_id = resolve(&created, unit)?;
                let (unit_kind, unit_group) = lookup(&transaction, unit_id)?;
                if unit_kind == "group" || unit_group != Some(group_id) {
                    return Err(format!(
                        "{ORG_DIRECTORY_INVALID}: 单位 #{unit_id} 不是所选集团下的成员单位"
                    ));
                }
                Some(unit_id)
            }
            ("unit", None) => {
                return Err(format!("{ORG_DIRECTORY_INVALID}: 单位项目必须选择具体单位"))
            }
            (kind, _) => return Err(format!("{ORG_DIRECTORY_INVALID}: 项目类型 {kind} 无效")),
        };
        binding_version = current_binding + 1;
        transaction
            .execute(
                "INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,unit_entity_id,binding_version) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(project_id) DO UPDATE SET project_kind=excluded.project_kind,group_entity_id=excluded.group_entity_id,unit_entity_id=excluded.unit_entity_id,binding_version=excluded.binding_version,updated_at=datetime('now','localtime')",
                params![input.project_id, binding.project_kind, group_id, unit_id, binding_version],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO org_directory_changes(directory_version,via_project_id,change_type,entity_id,payload_json) VALUES(?1,?2,'project_bound',?3,?4)",
                params![next_directory, input.project_id, unit_id.unwrap_or(group_id), serde_json::json!({"projectKind": binding.project_kind, "groupEntityId": group_id, "unitEntityId": unit_id, "bindingVersion": binding_version}).to_string()],
            )
            .map_err(|error| error.to_string())?;
    }
    if directory_changes {
        transaction
            .execute(
                "INSERT INTO org_directory_state(id,version) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET version=excluded.version,updated_at=datetime('now','localtime')",
                [next_directory],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(ProjectOwnershipDirectorySaveResult {
        project_id: input.project_id,
        directory_version: next_directory,
        binding_version,
        created_entities,
        created_identifiers,
    })
}

#[tauri::command]
pub fn save_project_ownership_directory(
    state: State<AppState>,
    input: ProjectOwnershipDirectorySave,
) -> Result<ProjectOwnershipDirectorySaveResult, String> {
    save_project_ownership_directory_at(&state.db_path, input)
}

/// Shared desktop/application entry. Paths are supplied only by the trusted owner.
pub(crate) fn save_project_ownership_directory_at(
    db_path: &std::path::Path,
    input: ProjectOwnershipDirectorySave,
) -> Result<ProjectOwnershipDirectorySaveResult, String> {
    save_project_ownership_directory_for_connection(&mut db::open(db_path)?, input)
}

