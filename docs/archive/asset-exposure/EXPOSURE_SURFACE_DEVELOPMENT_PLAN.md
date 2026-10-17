# Oviraptor 互联网暴露面低成本全流程开发规格

> 状态：待实施  
> 适用版本：v1.1.59+  
> 目标规模：单工作空间 4,000～100,000 条原始资产  
> 核心原则：全量确定性采集，少量 AI 裁决；覆盖率可证明，未测试不伪装成安全。

## 1. 背景与目标

本规格用于把现有“资产中心 + 归属门禁 + 被动公开暴露面”扩展为可支撑企业互联网暴露面排查的完整工作流，覆盖：

1. 搜索引擎基础域名与公开信息摸排；
2. 空间测绘、证书透明度与全量资产建档；
3. 端口、Web、目录、附件和敏感文件检查；
4. 代码仓库、文档、网盘与社交渠道排查；
5. 外部情报和暗网结果导入；
6. 归属、授权、复核、整改、复测和归档闭环。

系统不得把 4,000 个资产逐个交给模型自由调查。模型只接收规则无法确定的压缩证据包，不能参与全量抓取、分页、重试和状态机控制。

## 2. 非目标

- 不承诺“绝对无遗漏”；必须给出真实覆盖率和缺口。
- 不把模型输出直接认定为漏洞或泄漏事实。
- 不进行无界爬虫、无界目录枚举或无界多 Agent 循环。
- 不因 AI 预算耗尽而中断确定性采集。
- 不把 ASN、WHOIS、集团共享域名或名称相似直接视为资产归属。
- 不自行建设不可审计的暗网爬虫；暗网能力通过合规情报源或标准导入接入。

## 3. 不可违反的架构约束

### 3.1 成本约束

- L0～L3 阶段不得调用模型。
- 每个 AI 候选最多一次主裁决和一次技术性重试。
- 单证据包默认输入上限 6,000 Token、输出上限 800 Token。
- 云端预算耗尽只暂停 AI 队列，其他阶段继续运行并正常收口。
- 相同内容哈希、规则版本和裁决策略的结果必须复用。
- 模型不得读取整站 HTML、完整 JS 集合、完整 HTTP 历史或巨大 JSON。

### 3.2 正确性约束

- “完成”表示计划内所有批次均到达终态，而不是进程退出。
- `partial` 必须列出未完成对象、阶段、原因和可重试性。
- 未配置数据源显示“未配置”，不得显示“已完成”。
- 达到预算显示“等待预算”，不得显示“失败”。
- 归属、授权、暴露风险和漏洞结论必须是四个独立维度。
- 历史轮次只进入历史页，不得污染当前任务状态和 AI 输入。

### 3.3 范围约束

- 只有人工确认归属且授权允许的资产可进入主动探测。
- 主体级公开情报可以保存，但必须标记为“未绑定资产”。
- 第三方、集团关联和网络登记证据不得自动进入主动扫描。
- WAF、验证码、机器人挑战或明确限流触发目标级停止；普通 401/403 只记录权限边界。

## 4. 总体流水线

```text
主体档案与范围
  ↓
L0 原始数据接入与规范化（零 Token）
  ↓
L1 探活、指纹、来源归并（零 Token）
  ↓
L2 资产聚类与结果复用（零 Token）
  ↓
归属与授权门禁
  ↓
L3 被动/主动采集与规则检测（零 Token）
  ↓
候选去重、证据评分、误报过滤（零 Token）
  ↓
L4 AI 单次语义裁决（仅模糊高价值候选）
  ↓
人工复核 → 整改 → 复测 → 关闭
```

### 4.1 L0：规范化

输入可来自 FOFA、Quake、Hunter、crt.sh、搜索引擎、Nmap、人工导入和历史任务。统一生成：

- `canonical_asset_key`：协议、主机、端口规范化后的唯一键；
- `root_domain`、`hostname`、`ip`、`port`、`transport`；
- `source_observation`：来源、查询语句、来源记录 ID、首次/最近发现；
- 原始数据只读归档，解析结果单独存储。

### 4.2 L1：基础探测

对全量资产执行 DNS、TCP、TLS、HTTP 最小探测，产生确定性事实：

- DNS 解析和 CNAME；
- TCP 可达性、服务和 banner；
- TLS 证书主体、SAN、指纹和有效期；
- HTTP 状态、标题、重定向、响应头、favicon 哈希；
- HTML、关键 JS 和页面结构哈希；
- WAF/验证码/登录/静态站/业务站分类。

### 4.3 L2：资产聚类

聚类不是删除资产，而是避免重复分析。聚类信号：

- 相同根域、证书指纹或 SAN 集合；
- 相同 IP/CNAME/CDN 入口；
- 相同 favicon、HTML 结构、JS bundle 哈希；
- 相同登录页、产品特征或重定向目标；
- 人工指定同一业务系统。

每个簇包含一个代表入口和若干成员。完整内容分析优先在代表入口执行，其他成员只验证已发现的高价值事实。所有继承结果必须保存 `inherited_from_asset_id`，不能伪装成该成员的直接观察。

### 4.4 L3：确定性暴露面采集

按数据源拆分为独立阶段：

1. `search_engine`：搜索语句与结果；
2. `space_mapping`：FOFA/Quake/Hunter/CT；
3. `passive_web`：首页、robots、sitemap、security.txt、自然引用资源；
4. `port_scan`：授权资产端口检查；
5. `directory_scan`：分级目录与敏感文件验证；
6. `document_analysis`：文档下载、提取、OCR、元数据和规则；
7. `repository_search`：仓库、代码、提交历史和 Secret；
8. `public_storage`：公开对象存储线索；
9. `social_document_platform`：文档、网盘和社交结果；
10. `external_intelligence`：外部情报和暗网导入。

单数据源失败不终止其他数据源。每个阶段必须支持 checkpoint、停止、继续和单独重试。

### 4.5 L4：AI 裁决

只有满足以下全部条件才进入模型队列：

- 证据真实存在且可定位；
- 规则无法直接确认或排除；
- 具有业务敏感性、归属歧义或公开合理性判断价值；
- 不与已有内容哈希和裁决缓存重复；
- 当前工作空间仍有 AI 预算。

AI 只做分类、解释和人工复核建议，不直接执行网络请求，不改变资产归属，不自动关闭风险。

## 5. 数据库设计

现有表继续保留。新增表应放入独立迁移，禁止继续依赖 `extra_json` 承载核心查询字段。

### 5.1 主体与组织层级

```sql
CREATE TABLE organization_entities (
  id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL,
  parent_id INTEGER,
  entity_type TEXT NOT NULL,          -- headquarters/subsidiary/branch/brand
  legal_name TEXT NOT NULL,
  jurisdiction TEXT NOT NULL DEFAULT '',
  aliases_json TEXT NOT NULL DEFAULT '[]',
  registration_id TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'active',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

### 5.2 资产业务台账

```sql
CREATE TABLE asset_inventory_profiles (
  project_id INTEGER NOT NULL,
  asset_id INTEGER NOT NULL,
  entity_id INTEGER,
  system_name TEXT NOT NULL DEFAULT '',
  asset_type TEXT NOT NULL DEFAULT 'unknown',
  business_unit TEXT NOT NULL DEFAULT '',
  lifecycle_status TEXT NOT NULL DEFAULT 'unknown',
  filing_number TEXT NOT NULL DEFAULT '',
  hosting_provider TEXT NOT NULL DEFAULT '',
  responsible_team TEXT NOT NULL DEFAULT '',
  responsible_person TEXT NOT NULL DEFAULT '',
  authorization_expires_at TEXT,
  criticality TEXT NOT NULL DEFAULT 'normal',
  PRIMARY KEY(project_id, asset_id)
);
```

### 5.3 多来源观察

```sql
CREATE TABLE asset_observations (
  id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL,
  asset_id INTEGER,
  run_id INTEGER NOT NULL,
  source_key TEXT NOT NULL,
  source_record_id TEXT NOT NULL DEFAULT '',
  query_text TEXT NOT NULL DEFAULT '',
  observed_value_json TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  UNIQUE(project_id, source_key, source_record_id, content_hash)
);
```

### 5.4 资产簇

```sql
CREATE TABLE asset_clusters (
  id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL,
  representative_asset_id INTEGER,
  cluster_type TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  confidence INTEGER NOT NULL,
  reason_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(project_id, fingerprint)
);

CREATE TABLE asset_cluster_members (
  cluster_id INTEGER NOT NULL,
  asset_id INTEGER NOT NULL,
  relation TEXT NOT NULL,
  confidence INTEGER NOT NULL,
  PRIMARY KEY(cluster_id, asset_id)
);
```

### 5.5 统一任务、阶段与批次

```sql
CREATE TABLE exposure_jobs (
  id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL,
  mode TEXT NOT NULL,                -- baseline/incremental/recheck
  status TEXT NOT NULL,
  planned_items INTEGER NOT NULL DEFAULT 0,
  completed_items INTEGER NOT NULL DEFAULT 0,
  failed_items INTEGER NOT NULL DEFAULT 0,
  skipped_items INTEGER NOT NULL DEFAULT 0,
  waiting_items INTEGER NOT NULL DEFAULT 0,
  ai_budget_tokens INTEGER NOT NULL DEFAULT 0,
  ai_used_tokens INTEGER NOT NULL DEFAULT 0,
  terminal_reason TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  started_at TEXT,
  completed_at TEXT
);

CREATE TABLE exposure_job_stages (
  id INTEGER PRIMARY KEY,
  job_id INTEGER NOT NULL,
  stage_key TEXT NOT NULL,
  source_key TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL,
  planned_items INTEGER NOT NULL DEFAULT 0,
  completed_items INTEGER NOT NULL DEFAULT 0,
  failed_items INTEGER NOT NULL DEFAULT 0,
  skipped_items INTEGER NOT NULL DEFAULT 0,
  cursor_json TEXT NOT NULL DEFAULT '{}',
  error_summary TEXT NOT NULL DEFAULT '',
  UNIQUE(job_id, stage_key, source_key)
);

CREATE TABLE exposure_work_items (
  id INTEGER PRIMARY KEY,
  job_id INTEGER NOT NULL,
  stage_id INTEGER NOT NULL,
  asset_id INTEGER,
  cluster_id INTEGER,
  item_key TEXT NOT NULL,
  status TEXT NOT NULL,
  attempt INTEGER NOT NULL DEFAULT 0,
  retryable INTEGER NOT NULL DEFAULT 1,
  next_retry_at TEXT,
  error_code TEXT NOT NULL DEFAULT '',
  error_message TEXT NOT NULL DEFAULT '',
  result_hash TEXT NOT NULL DEFAULT '',
  UNIQUE(job_id, stage_id, item_key)
);
```

### 5.6 证据与候选

```sql
CREATE TABLE exposure_evidence (
  id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL,
  job_id INTEGER NOT NULL,
  asset_id INTEGER,
  source_key TEXT NOT NULL,
  evidence_type TEXT NOT NULL,
  source_url TEXT NOT NULL DEFAULT '',
  locator TEXT NOT NULL DEFAULT '',
  content_hash TEXT NOT NULL,
  excerpt_masked TEXT NOT NULL DEFAULT '',
  facts_json TEXT NOT NULL DEFAULT '{}',
  observed_at TEXT NOT NULL,
  UNIQUE(project_id, source_key, content_hash, locator)
);

CREATE TABLE exposure_candidates (
  id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL,
  fingerprint TEXT NOT NULL,
  category TEXT NOT NULL,
  status TEXT NOT NULL,              -- rule_confirmed/needs_ai/needs_human/dismissed
  severity TEXT NOT NULL,
  rule_confidence INTEGER NOT NULL,
  evidence_ids_json TEXT NOT NULL,
  decision_reason TEXT NOT NULL DEFAULT '',
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL,
  UNIQUE(project_id, fingerprint)
);
```

### 5.7 AI 裁决与缓存

```sql
CREATE TABLE exposure_ai_decisions (
  id INTEGER PRIMARY KEY,
  candidate_id INTEGER NOT NULL,
  evidence_pack_hash TEXT NOT NULL,
  policy_version TEXT NOT NULL,
  model_profile TEXT NOT NULL,
  decision TEXT NOT NULL,
  confidence INTEGER NOT NULL,
  reasoning_summary TEXT NOT NULL,
  missing_evidence_json TEXT NOT NULL,
  recommended_action TEXT NOT NULL,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  UNIQUE(evidence_pack_hash, policy_version, model_profile)
);
```

### 5.8 整改闭环

```sql
CREATE TABLE remediation_cases (
  id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL,
  candidate_id INTEGER NOT NULL,
  status TEXT NOT NULL,              -- open/assigned/fixing/retest/closed/accepted
  owner_team TEXT NOT NULL DEFAULT '',
  owner_person TEXT NOT NULL DEFAULT '',
  due_at TEXT,
  ticket_ref TEXT NOT NULL DEFAULT '',
  resolution TEXT NOT NULL DEFAULT '',
  closed_at TEXT
);

CREATE TABLE remediation_events (
  id INTEGER PRIMARY KEY,
  case_id INTEGER NOT NULL,
  event_type TEXT NOT NULL,
  note TEXT NOT NULL,
  evidence_ids_json TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL
);
```

## 6. 任务状态机

### 6.1 任务终态

- `completed`：所有计划阶段完成，允许存在明确记录的非关键跳过项；
- `partial`：有不可继续的阶段或工作项，但已保存有效结果；
- `failed`：预检失败或没有任何阶段产生有效结果；
- `cancelled`：用户取消，已有结果保留；
- `waiting_budget`：仅 AI 阶段等待预算，确定性阶段已完成；
- `waiting_input`：缺少 API 凭据、人工范围或外部文件；
- `blocked_protection`：目标级 WAF/验证码/限流阻断。

### 6.2 工作项状态

`queued → running → completed | skipped | retry_wait | failed | cancelled`

重新执行创建新 job，但不删除证据历史；继续执行复用同一 job，只选择非终态或可重试工作项。启动新 job 时不得复制旧错误、旧进度和旧终态标识。

### 6.3 完成判定

```text
任务完成 = 所有启用阶段到达终态
阶段完成 = planned = completed + skipped
阶段部分完成 = failed > 0 且 completed > 0
任务失败 = completed = 0 且存在不可恢复错误
```

禁止用“线程结束”“没有新日志”或“模型结束回答”直接推导任务完成。

## 7. 数据源适配器接口

后端新增 `commands/exposure/` 目录：

```text
exposure/mod.rs
exposure/orchestrator.rs
exposure/models.rs
exposure/repository.rs
exposure/evidence.rs
exposure/rules.rs
exposure/ai_queue.rs
exposure/remediation.rs
exposure/adapters/
  search_engine.rs
  fofa.rs
  quake.rs
  hunter.rs
  certificate_transparency.rs
  wayback.rs
  passive_web.rs
  port_scan.rs
  directory_scan.rs
  document.rs
  repository_platform.rs
  external_intelligence.rs
```

统一 trait：

```rust
trait ExposureAdapter {
    fn key(&self) -> &'static str;
    fn preflight(&self, context: &RunContext) -> AdapterHealth;
    fn plan(&self, context: &RunContext) -> Result<Vec<WorkItemPlan>, AdapterError>;
    fn execute(&self, item: &WorkItem, context: &RunContext) -> WorkItemOutcome;
    fn normalize(&self, raw: RawObservation) -> Vec<NormalizedEvidence>;
}
```

适配器不得直接修改任务总状态，只返回工作项结果，由 orchestrator 聚合。

## 8. 各阶段实现要求

### 8.1 搜索引擎

查询规划器根据法定名称、简称、品牌、根域和内部项目词生成固定模板。查询本身零 Token；AI 只能对候选关键词做一次离线补充。

必须支持：

- 官方与关联站点；
- PDF/DOCX/XLSX/PPTX/ZIP/RAR/SQL/BAK/OLD；
- 后台、登录、测试、临时、运维页面；
- 内网 IP、组织架构、人员名单、项目、财务和运维资料；
- 查询语句、结果排名、摘要、目标 URL、获取时间和页面快照哈希留证。

搜索平台没有稳定 API 时使用可插拔浏览器适配器或人工导入，不得在后台绕过验证码。

### 8.2 空间测绘

- FOFA 保留为默认数据源；
- Quake、Hunter 为可选适配器；
- crt.sh 发现的子域必须回流资产中心，而不是只计数；
- 每个来源保留独立观察，发生字段冲突时展示来源差异；
- 新资产依次进入探活、归属和授权，不直接进入主动扫描。

### 8.3 端口扫描

- 基线模式对确认授权的 IP 执行配置化端口集；
- 增量模式只检查新 IP、变化 IP 和高价值服务；
- 扫描并发、速率、超时和网络段配额必须可配置；
- 发现 Web 服务时创建新 Web 入口；
- 数据库、远程管理、文件传输等服务进入人工复核，不自动尝试认证。

### 8.4 目录与敏感文件

采用两级词表：

- 一级：高价值、小规模路径，所有授权 Web 簇代表入口执行；
- 二级：只有一级出现正向信号后执行；
- 相同技术簇共享候选路径，但每个成员单独验证；
- 只读 GET/HEAD 优先；状态变更请求必须进入单次授权契约；
- 明确 WAF/验证码/限流立即停止该目标；
- 401/403 作为存在性和权限边界，不判定为风险。

### 8.5 文档分析

支持 PDF、DOCX、XLSX、PPTX、TXT、CSV 和 ZIP 文件清单。处理顺序：

1. 下载大小与类型校验；
2. SHA-256 去重；
3. 文本提取，扫描件进入 OCR；
4. 作者、公司、创建工具、时间等元数据；
5. 内网 IP、主机名、邮箱、手机号、密钥、路径和项目词规则；
6. 生成脱敏摘要和证据定位；
7. 只有公开合理性不明确时进入 AI。

原文件保存位置、保留周期和访问权限必须配置；UI 默认只展示脱敏片段。

### 8.6 代码仓库

- 根据公司名称、域名、邮箱后缀、项目名、系统名和基础设施关键词主动搜索；
- 覆盖仓库、代码、Issue、Wiki、Release、Gist/片段和提交历史；
- Secret 扫描由确定性规则执行；
- 记录仓库、文件、行号、commit、作者和首次/最近发现；
- 不在扫描日志中保存完整密钥；
- 相同 Secret 指纹跨仓库聚类，验证有效性必须单独授权。

### 8.7 文档、网盘和社交渠道

统一通过 `public_content_observations` 表达：平台、账号、标题、URL、发布时间、作者、摘要、快照哈希和主体关联度。无法自动采集的平台允许 CSV/JSON 导入，但仍走同一去重、AI 裁决和整改流程。

### 8.8 暗网与外部情报

仅实现适配器和导入协议：

```json
{
  "provider": "vendor-name",
  "observedAt": "ISO-8601",
  "category": "credential|database|source_code|document|mention",
  "entity": "目标主体",
  "sourceRef": "供应商事件编号",
  "summaryMasked": "脱敏摘要",
  "confidence": 80
}
```

原始内容由供应商系统管理；Oviraptor 保存事件编号、脱敏证据和处置状态。

## 9. 证据评分与去噪

评分由确定性信号组成，模型不得改写原始分值：

- 可直接访问：+20；
- 目标确认域名或邮箱后缀：+20；
- 文档元数据关联：+10；
- 内网地址、真实主机名或环境名：+15；
- 有效 Source Map/源码结构：+20；
- 密钥格式和熵命中：+20；
- 仅名称相似：最多 +10；
- ASN/WHOIS/CIDR 单独命中：最多 55 总分；
- 集团共享域名：最多 52 总分；
- 历史归档但当前不可访问：降低 15～30；
- 广告、遥测、公共库、字体、图片和备案文本直接降噪。

候选分流：

- `>= 90` 且证据确定：规则确认，人工复核，不调用 AI；
- `65～89`：根据语义歧义进入 AI 或人工；
- `< 65`：保留线索或自动排除，不调用云端模型；
- 高危但证据不完整：进入“补充证据”，不得伪确认。

## 10. AI 证据包契约

```json
{
  "candidateId": 123,
  "category": "document",
  "entity": {
    "legalName": "示例公司",
    "aliases": ["示例简称"],
    "approvedDomains": ["example.com"]
  },
  "source": {
    "type": "search_engine",
    "url": "https://example.com/file.pdf",
    "observedAt": "2026-09-03T10:00:00+08:00"
  },
  "deterministicFacts": [
    "无需登录可下载",
    "包含 12 个内网 IP",
    "作者邮箱后缀与目标一致"
  ],
  "maskedExcerpt": "……",
  "question": "判断是否可能超出正常公开范围，并说明人工复核重点"
}
```

模型必须返回固定结构：

```json
{
  "decision": "likely_sensitive|likely_public|insufficient_evidence",
  "confidence": 0,
  "reason": "不超过 300 字",
  "missingEvidence": [],
  "recommendedAction": "不超过 200 字"
}
```

解析失败只允许一次修复请求；第二次失败进入人工队列。不同模型的结果独立版本化，不能覆盖人工结论。

## 11. 预算与调度

### 11.1 默认预算

| 项目 | 默认值 |
| --- | ---: |
| 单候选输入 | 6,000 Token |
| 单候选输出 | 800 Token |
| 单候选调用 | 1 次 + 1 次技术重试 |
| 单任务 AI 候选 | 100 条 |
| 单任务输入预算 | 600,000 Token |
| 单任务输出预算 | 80,000 Token |
| 云端并发 | 2 |
| 本地模型并发 | 1 |

预算应根据模型配置估算货币成本并在启动前展示。模型请求完成后记录实际 Token；供应商未返回 usage 时使用本地估算并标注“估算”。

### 11.2 优先队列

排序依据：严重度、证据完整度、资产关键性、新颖度、首次出现、外部可访问性和整改价值。重复、低价值或缺少定位的候选不能抢占 AI 预算。

### 11.3 调度策略

- 基线任务：首次完整建档，可持续数小时或数天；
- 增量任务：只处理新增和哈希变化；
- 周期复查：按资产关键性分层；
- 相同资产簇和内容哈希复用结果；
- Worker 重启后从数据库工作项恢复，不能依赖内存队列。

## 12. 前端信息架构

公开暴露面保留为 Asset 下的一级菜单，内部页签调整为：

1. **总览**：覆盖率、风险、待处置、预算；
2. **采集任务**：任务、阶段、批次、错误和继续按钮；
3. **资产簇**：代表入口、成员、共享证据和差异；
4. **线索复核**：规则确认、AI 待判、人工待判、已排除；
5. **整改闭环**：责任人、期限、复测、关闭；
6. **数据源**：配置、健康、覆盖量、最近执行；
7. **覆盖缺口**：未配置、失败、受保护、未授权和预算等待。

### 12.1 总览必须展示

- 原始资产数、有效资产数、资产簇数、已准入数；
- 各阶段计划/完成/失败/跳过/等待；
- 确定性发现、AI 候选、人工待处理；
- 当前轮与上轮的新增、消失和变化；
- AI 已用 Token、预计费用、缓存命中率；
- “本轮覆盖范围”说明，禁止只给一个模糊完成状态。

### 12.2 任务详情

任务详情采用树状结构：任务 → 阶段 → 批次 → 工作项。默认展示面向用户的解释，技术 JSON 仅放在折叠的“调试证据”中。

### 12.3 线索详情

必须展示：结论、风险、主体、资产、来源、证据、首次/最近发现、当前可访问性、确定性命中、AI 判断、人工结论、整改记录。不得要求用户肉眼阅读混合 JSON。

## 13. 后端 API

新增或替换命令：

```text
create_exposure_job
preflight_exposure_job
start_exposure_job
pause_exposure_job
continue_exposure_job
retry_exposure_stage
cancel_exposure_job
get_exposure_job
list_exposure_jobs
list_exposure_job_stages
list_exposure_work_items
get_exposure_coverage
list_asset_clusters
rebuild_asset_clusters
list_exposure_candidates
get_exposure_candidate
submit_exposure_human_decision
list_exposure_ai_queue
run_exposure_ai_batch
get_exposure_budget
list_remediation_cases
update_remediation_case
submit_retest_result
export_exposure_report
```

旧 `start_exposure_scan` 保留一版兼容门面，但内部必须调用新 orchestrator，并在后续大版本删除。

## 14. 当前代码必须优先清理的问题

1. 删除暴露面资产查询中的静默 `LIMIT 200`，改为工作项分页；
2. 删除线索查询中的静默 `LIMIT 500`，改为服务端分页；
3. 把每资产固定 20 个资源改为价值预算与覆盖记录；
4. crt.sh 结果写入资产观察并回流资产中心；
5. `completed` 必须汇总 CT、Wayback 和被动 Web 的真实终态；
6. “开始完整采集”在新流水线完成前改为“开始被动采集”；
7. `exposure.rs` 拆分为 orchestrator、adapter、repository、evidence、rules；
8. UI 线索列表改为服务端分页；
9. 任务退出、应用闪退和用户取消必须保存 checkpoint；
10. 现有历史结果迁移为 `legacy_passive_web`，不得冒充新基线任务。

## 15. 开发阶段

### P0：真实性、规模与成本基础

- 新任务/阶段/工作项表和状态机；
- 移除 200/500 静默限制；
- 分批执行、checkpoint、继续和单阶段重试；
- 资产聚类和内容哈希缓存；
- AI 队列与确定性任务彻底分离；
- 覆盖率、预算和缺口 UI；
- 把当前功能准确命名为被动采集。

验收：4,000 个资产任务可以分批跑完；应用重启后继续；零 AI 预算仍能完成全部确定性阶段。

### P1：高回报发现能力

- 搜索引擎查询任务；
- 文档下载、解析、OCR 与敏感规则；
- GitHub/Gitee/GitLab 主动搜索和历史检查；
- crt.sh 增量资产回流；
- 证据包、单次 AI 裁决和缓存。

验收：相同文件只分析一次；明确泄漏无需 AI；模糊文档才进入模型队列。

### P2：主动资产深挖

- 授权端口扫描；
- 两级目录和敏感文件验证；
- Web 新端口回流；
- WAF/验证码/限流目标级停止；
- 按资产簇复用路径候选。

验收：同技术簇不会重复跑完整词表；停止一个目标不影响其他目标。

### P3：整改和多渠道

- 责任人、期限、工单、复测和关闭；
- 文档平台、网盘、社交适配器或标准导入；
- 外部情报/暗网导入协议；
- 完整报告、覆盖矩阵和审计记录。

## 16. 测试策略

### 16.1 单元测试

- URL、域名、IP、端口规范化；
- 聚类边界和错误继承防护；
- 规则评分、敏感信息脱敏；
- 状态机所有合法/非法转换；
- AI 证据包大小和字段完整性；
- 预算耗尽和缓存命中；
- 旧任务迁移与历史隔离。

### 16.2 集成测试

- 4,000、20,000、100,000 条模拟资产分页；
- Worker 中断和应用重启恢复；
- 单数据源超时不影响其他阶段；
- 本地模型超时、上下文超限和云端鉴权失败；
- CT 新域名回流且必须重新过门禁；
- 目录扫描遇 WAF 停止；
- 文档、仓库和搜索结果去重；
- 重跑不继承旧错误和旧终态。

### 16.3 UI 测试

- 100/200 条列表滚动、分页和固定表头；
- 长 URL、长标题、长错误不破坏布局；
- 任务完成、部分完成、等待预算、等待输入清楚区分；
- 技术 JSON 默认折叠；
- 4K/高 DPI/窗口缩放下布局可用。

### 16.4 性能门槛

- 4,000 条资产列表查询 P95 < 500 ms；
- 100,000 条资产服务端分页 P95 < 1 s；
- 任务恢复时间 < 5 s；
- 聚类和规则处理不能随历史轮次数量无限增长；
- AI 候选默认不超过有效资产的 5%，目标控制在 1%～2%。

## 17. 验收定义

一个项目可以宣称“基线排查完成”必须同时满足：

- 所有启用数据源均为完成或有明确批准的跳过原因；
- 所有计划资产和资产簇均有工作项终态；
- 所有失败项均有错误码、对象和可重试性；
- 新增资产已经完成归属处理或明确停留在待归属；
- 高价值候选已进入规则确认、AI 队列或人工队列之一；
- AI 预算耗尽被记录为等待预算，不影响确定性覆盖率；
- 报告能够区分已测试、未发现、未测试、被阻断和未配置；
- 整改关闭必须有复测证据，不能只修改状态字段。

## 18. 外部依赖与配置

### 内置或可随应用打包

- DNS/TLS/HTTP 探测；
- 文档文本提取基础库；
- 敏感规则、哈希、聚类和证据包；
- Worker、状态机、预算和报告；
- 端口/目录执行器的受控调用层。

### 需要用户或组织提供

- FOFA、Quake、Hunter 等平台凭据；
- 搜索引擎 API 或允许的浏览器采集方式；
- GitHub/Gitee/GitLab Token（用于提高限额和历史访问）；
- OCR 运行时或云 OCR 配置；
- 暗网/外部情报供应商接口或导出文件；
- 主动扫描的授权范围、速率和时间窗口。

缺少外部依赖时，对应阶段显示“未配置”，整个产品仍可完成其余阶段。

## 19. 代码边界

- `commands.rs` 只装配模块，不继续堆积暴露面实现；
- 网络适配器不得直接写 UI 状态；
- 数据库访问集中在 repository；
- 状态聚合集中在 orchestrator；
- 规则判断必须可单元测试且不依赖模型；
- AI 模块只接受证据包，不访问原始数据库全集；
- Vue 页面通过 `src/features/assets/api.ts` 访问稳定门面；
- 页面级逻辑拆为 `ExposureOverview`、`ExposureJobs`、`ExposureCandidates`、`ExposureRemediation`、`ExposureSources` 和对应 composable。

## 20. 最终交付物

1. 数据库迁移及回滚说明；
2. 暴露面 orchestrator 与适配器 SDK；
3. 资产聚类和增量缓存；
4. 搜索、文档、仓库、端口和目录核心适配器；
5. AI 证据包、预算和裁决缓存；
6. 整改闭环；
7. 覆盖率与缺口 UI；
8. CSV/JSON/可打印报告；
9. 规模、恢复、错误隔离和 UI 自动化测试；
10. 运维文档和数据源配置说明。

实施时必须先完成 P0，再增加数据源。否则新来源只会放大现有的分页、状态、误报和成本问题。
