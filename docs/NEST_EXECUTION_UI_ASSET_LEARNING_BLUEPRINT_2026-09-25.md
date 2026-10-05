# Nest 任务工作台、联合对话、资产画像与知识演进：实现蓝图

状态：**待实施的产品/数据/验收合同**，2026-09-25。交给 Qoder 时同时提供 `NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md`、`NEST_PRODUCT_REDESIGN_AND_LEARNING_CONTRACT_2026-09-25.md` 和 `NEST_IMPLEMENTATION_PROGRESS_2026-09-23.md`。授权、预算、证据来源、Reviewer 与迁移规则以 Master Plan 为最高约束；本文件只细化页面和数据交互，不声称功能已经完成。开工前重新读代码和 Git 状态，不覆盖用户现有修改。

## 1. 产品对象和唯一身份

```text
Project ─ Asset ─ AuthorizationScope
   └── Scan（长期任务，创建后 ID 不变）
        ├── Attempt 1..N（每次尝试的冻结计划、预算、身份、终态）
        │    ├── Coordinator / child run / assignment / capability lease
        │    ├── 对话事件 / 人工指令 / 工具事件 / 停机诊断
        │    ├── Evidence revision / 候选 / Reviewer decision / 覆盖缺口
        │    └── L0 任务学习摘要及候选
        └── 历史结果投影（原 JSON 只读导入，绝不重写原件）
```

每一个列表、会话、证据、图表、学习记录与深链必须携带 `project_id + scan_id`，轮次数据再携带 `attempt_number`；run/assignment/revision 是该轮下的进一步限定。后端不因前端给出一个有效 ID 就放弃项目、轮次和目标归属校验。旧 JSON 的原路径、原始字节、导入批次与 provenance 保留；界面只统一投影，不合并/篡改审查状态。新任务 Native-only；任何“历史 Strix”字样只能是来源标签或只读导入入口，不是后端选择。

## 2. 信息架构和具体页面

### 2.1 导航与桌面结构

保留现有项目入口和扫描能力，不另建与原结果页竞争的第二份详情。一级导航为“概览｜任务｜协作｜结果与证据｜资产与知识｜配置”；新建任务是任务页的突出操作，验证台是结果的有权限子页。选中任务后所有页保留同一个任务上下文条：目标（脱敏）、scan/attempt、模式、状态、授权摘要、预算、最近一次可信更新时间和“查看未完成义务”。

```text
┌ 项目/任务上下文 ──────────────────────────────── 授权/预算/状态 ┐
│ 任务列表/频道  │ 主区：概况/执行/对话/结果        │ 上下文抽屉     │
│ 待处理        │ 消息或证据摘要、真实状态          │ 事实与假设     │
│ 已结束        │ 可点击证据/合同/Reviewer 引用      │ 覆盖缺口       │
│ 全部/搜索     │ 人工输入/审批卡（只在协作页）      │ 下一步         │
└─────────────────────────────────────────────────────────────┘
```

窄屏依次显示任务/频道抽屉、主内容、上下文抽屉；不会把三列压成不可读的小字。键盘可达、焦点可见、状态不能只靠颜色。切页保持所选 scan/attempt 和滚动锚点；任务切换时丢弃旧请求返回。URL/页面路由可恢复到同一任务同一轮次同一证据，不以组件内临时状态作为唯一导航身份。

### 2.2 任务中心：完成任务放哪里

“待处理”收纳运行中、暂停、仍可续跑的 `partial`、需要批准、能力缺失和未清理；“已结束”收纳已完成、带明确缺口的完成、失败、取消。所有状态都留在“全部”，历史 attempt 不随视图切换消失。`archived` 是**用户可逆的展示属性**，不是删除、不是任务成功状态，不影响历史结果/导入记录/资产聚合；归档后仍可从已结束或全部搜索，默认折叠而非隐藏。删除继续沿用既有删除合同，绝不可让归档按钮复用删除逻辑。

卡片固定呈现：任务名称/模式/项目、目标数、当前或末次 attempt、真实状态、已审发现数、覆盖缺口数、最近活动、停机代码与下一步。排序默认待行动优先，其次更新时间＋任务 ID 稳定游标；全库搜索必须后端分页过滤，不能把“当前已加载页面搜索”误称全局检索。批量操作只做筛选/导出/可逆归档，暂停与重试逐任务确认。

### 2.3 单任务详情：原功能组合而非消失

“概况”显示冻结 scope、运行/尝试时间轴、预算消耗、终态和覆盖矩阵；“执行”按 attempt 展示 agent 消息、工具状态、runner log、停机/清理义务；“证据”回到既有结果页的机会/指纹/API/端点/漏洞/验证入口，新增 revision 与来源导航；“学习”显示该任务 L0 摘要、候选、引用和审批状态。历史旧任务没有 Native 消息就显示“历史导入/此轮无原生事件”，不能合成 Agent 发言。

选择旧 attempt 时仅展示该轮快照和该轮日志/消息/工具；当前轮的 `stopDiagnostic` 不能贴在旧轮。结果计数明确区分“观察事实/模型假设/候选/Reviewer confirmed/被驳回/证据不足”。点击聊天中的引用能定位证据视图中的精确 `scan,attempt,revision,node/artifact`；证据页反向可跳回当时消息。任何引用已删除、导入失败或哈希失配时显示不可验证，不能静默转到最新版本。

### 2.4 协作对话：真实多 Agent 聊天，不是模拟群聊

频道：`#团队总览`、每个 Candidate/Gap/Assignment 的子线程、`#人工指令与审批`。每条消息显示发送者角色、时间、attempt、事实/假设/提案/审查/系统状态类型、关联合同与证据引用、投递/确认状态。Agent 间只展示脱敏摘要；原始凭据、HTTP body、链式思维、私有路径全文只在具权限的证据存储，不进聊天。`正在分析`来自活跃 child/run；断线按持久 sequence 补拉，重复事件按 event ID 幂等，断线期间不能凭动画推断仍在执行。

人工输入先持久化原文和不可执行草案，再由后端解析为 `intent, target_refs, role_hint, desired_outcome, side_effect, scope_change, budget_change`。前端三态卡：可直接排队（纯解释/只读）、需要确认（明确范围内有副作用/预算变更）、拒绝（超授权/越权/无身份/不可验证）。点击确认时后端重读当前 scope、fencing、预算、角色与能力；过期草案必须重审，不能仅信前端按钮。人工想法先为 hypothesis；Agent 之间的 Challenge/Supply/Counterexample 也不授予目标工具，只有 Coordinator 签发的 assignment/capability 才能调用。相同 revision 只允许有界质询；Reviewer 独立于执行者，不能自证。

### 2.5 停机与继续操作

一个停机卡至少列：稳定 code、类别（安全硬边界/软预算/能力缺口/自然终态）、触发阶段、已完成合同、未完成义务、已用/剩余请求与 Token、是否有不确定的目标调用、下一步和是否允许同任务继续。WAF、连续 429、硬授权边界与不可确定的目标调用不能靠模型一句“继续”绕过；缺模型/账号/Analyzer 是部分完成而不是“安全”。继续只签发未完成合同并复用已验证 checkpoint；对无法证明未执行的目标调用 fail closed，要求人工核对或新 attempt。不要通过提高软限制掩盖无证据空转。

## 3. 数据与 API 增量合同（先迁移/测试，再接 UI）

现有 `agent_*`、scan/attempt、evidence、knowledge 表优先扩展，不平行复制一套事实。以下是逻辑结构，实施时遵守仓库 SQLite 迁移/命名惯例，先查已有列和索引。

| 对象 | 最小字段/约束 | 写入方与读法 |
| --- | --- | --- |
| `scan_ui_state` | `project_id,scan_id,archived_at,archived_by,version`，任务唯一 | 用户可逆更新；不参与运行终态 |
| `collaboration_thread` | `scan_id,attempt,kind,subject_id,created_sequence` 唯一键 | Coordinator 生成，历史消息只追加 |
| `collaboration_event` | `event_id,thread_id,sequence,actor,kind,summary,refs,created_at`，全局单调 sequence | DB 事务提交后通知；按游标补拉、脱敏投影 |
| `human_directive` | 原文密封引用、草案、风险判定、确认 revision、决议/拒绝原因 | 用户提交；Coordinator 核验后才派单 |
| `stop_snapshot` | `scan,attempt,code,class,phase,obligations,budget,uncertain_calls,checkpoint_rev` | reducer/调度器写入；只读诊断可作为迁移前过渡 |
| `asset_profile` | `project_id,normalized_origin,authorization_binding,last_seen,version`，同项目同授权去重 | 只引用已授权实际观察；跨项目不混合 |
| `asset_assertion` | `kind,value,confidence,support_refs,contradict_refs,model_version,status` | 分类与技术栈先 hypothesis，证据冲突保留 |
| `endpoint_pattern` | 路径模板/方法/来源/最后验证/私有级别/支持任务 | 假设路径不自动请求；只有新目标线索＋scope 才可提议 |
| `task_learning` | `scan,attempt,tool_summary,hypotheses,review_corrections,gaps,refs` | 终态幂等生成一次 L0，失败任务亦保留教训 |
| `pattern_card` | `canonical_key,version,support_scan_ids,counterexamples,quality,privacy,state` | L1 聚类成 L2，不以同任务多模型重复计数 |
| `approved_skill_version` | `skill_id,version,scope,approval,active,rollback_to,retired_at` | L3 发布/回滚/退役；旧任务保留所用版本引用 |

所有跨任务聚合先按租户/项目/授权归属及隐私级别过滤。存量 JSON 通过只读 adapter 产生 canonical 投影，保留原哈希与导入 provenance；导入错误记录在隔离队列，不把不完整历史结果当新证据或触发模型。数据库索引需覆盖 `(project_id,scan_id,attempt,sequence)`、项目＋更新时间稳定游标、`(asset_id,kind,status)` 与知识 canonical key；大任务使用分页/窗口摘要，避免打开协作页全表解码。

新读接口建议：任务服务端筛选＋游标、某轮详情快照、某线程 `after_sequence`、证据精确定位、资产画像与断言来源、知识检索/审核历史。写接口建议：归档/取消归档、提交/确认/驳回人工草案、批准/退役 Skill；每个写入需版本/CAS 与后端身份、项目、权限检查。协议字段稳定后再修改 Vue 类型与页面；不要让前端直接写底层表或拿任意文本当 Coordinator 合同。

## 4. 资产汇总：AI 参与，但事实与推断分层

资产页先用实际证据绘出“目标—系统类型假设—框架/版本断言—已验证端点—候选问题—Reviewer 结果”。类型允许多标签和 `unknown`；登录页、静态资源、响应头、JS 框架指纹只能提升置信度，不能凭一个标题定性“政务后台”。AI 产出短解释与支持/反例引用，确定性聚合器统计任务数、身份覆盖和证据数量。

图表首选可点击矩阵（类型 × 技术栈 × 已验证目标/任务/发现），加时间趋势和覆盖缺口表；词云仅辅助导航，不编码风险。历史“这个框架发现过问题”必须同时给分子、分母、版本范围、确认状态、时间与反例。相似内部接口只作项目隔离的路径模板候选，来源明确、去掉账号/订单号/token；新目标必须有自己的页面/JS/网络证据和授权路径才允许在预算内提议，不能批量猜路径。

## 5. 多 Agent 学习与 Skill 防膨胀

L0 是不可变的实际任务、工具、证据、Reviewer 纠错和停机记录；L1 是可审核的单任务候选，不进入全局系统提示；L2 是多个**独立任务**支持且保留反例的规范化 PatternCard；L3 才是人工/策略批准的有限数量活跃 Skill。每次任务结尾产生“有效方法、无效假设、工具成本、缺口、可复用线索”的短报告，绝不保存模型逐 token 思维链。学习的目标是更好选择方法与停止条件，不是扩大目标授权。

检索流程固定：先授权/隐私/项目过滤，再任务模式和阶段匹配，再质量/新鲜度/反例排序，然后选小量卡片和固定文本预算。可配置全局与分类活跃上限、重复相似度与退役阈值；超过上限只能合并、替换或经审核新增，不能每扫描自动加一条永久 Skill。记录每次注入的 skill/version、模型使用和事后收益/误导，低收益自动提出退役建议但不擦掉历史引用。批准与回滚在 UI 可审计；导出默认去标识化，账号/私有 URL/请求正文不进入通用 Skill。

## 5.1 轻量沙箱与工具供应：不默认依赖 Kali

具体工具矩阵、能力包 manifest、隔离适配器、UI 与验收关口见 `NEST_LIGHTWEIGHT_SANDBOX_TOOL_SUPPLY_2026-09-25.md`。

本节是**待实现的运行合同**，不是当前产品已具备的隔离证明。概念上分开「模型能请求什么工具」（Broker 授权）与「工具进程在哪执行」（Sandbox）：任何容器/虚拟机都不能取代 Broker 对每一条目标请求的 scope、身份、速率、预算和 fencing 检查。反过来，只有 Broker 而无进程隔离，也不能允许不可信代码/插件直接拿宿主机权限。Container rootless/seccomp 只是纵深防护，不应声称绝对隔离。

| 工作类型 | 默认执行地点 | 能力与网络 | 缺失时 |
| --- | --- | --- | --- |
| Coordinator、Reviewer、只读分析 | 宿主 Rust 的纯数据 API，或无网络只读进程 | 仅冻结证据、共享图、邮件；无目标套接字、shell、安装权限 | `unsupported_capability`，不能借执行专家的令牌 |
| HTTP/站点侦察 | 内建 Rust HTTP Broker；需要额外解析时才用受限 worker | 只连冻结授权目标，经 Broker 逐请求核验；临时目录和输出配额 | 记录精确缺口和未完成合同，安全暂停 |
| 浏览器前端 | 按版本固定的浏览器 worker/缓存镜像 | 浏览器进程和下载目录隔离，路由交给 Broker；不得默认访问整个内网 | 浏览器能力不可用时保留 HTTP 证据和 coverage gap |
| 源码/依赖静态分析 | 只读源快照 + 有界 worker | 默认无网络，写入仅临时结果目录；不得执行项目构建脚本，除非另行审批 | 静态分析子项部分完成，不改写源码 |
| 需要特别系统工具的专家 | 经管理员审核的可选能力包 | 最小镜像/可执行文件，能力、摘要、许可证、版本与 OS/架构固定 | 明示安装请求；拒绝或离线时不自动拉取 |
| Kali 等完整发行版 | 仅用户显式选择的独立兼容模式 | 自身工具仍逐项经过 Broker；不得赋予特权容器、宿主网络或挂载工作区根目录 | 不影响 Native 默认流程 |

供应链流程：维护一个机器可读的 `ToolCapabilityManifest`（`id/version/platform/source/digest/license/requiredPermissions/outputSchema/timeout/resourceCaps`）。安装器和 Agent 分离；发布时预置最小包，扩展包由管理员在联网预备阶段从允许源拉取、校验摘要/签名、缓存为只读版本。扫描进行中**禁止** `curl | sh`、动态 `pip/npm/apt install`、滚动 `latest` 镜像或 Agent 自行提权；离线机器只使用本地已验证缓存。镜像锁 digest，升级是新版本审批和回归，不原地替换活动 attempt 的工具版本。记录每个 attempt 实际 tool digest、安装清单和调用结果以便重放；清理临时目录不能删除证据引用的原件。

运行边界：按 lane 配置非 root 用户、只读文件系统、最小挂载、去 capabilities、默认 seccomp、CPU/内存/进程/输出/时长配额；在宿主机或 Docker 不支持某项隔离时做 `unsupported_sandbox` 可见失败，不偷偷降级为宿主 shell。网络需要允许清单、DNS 解析结果约束和重绑定防护；任何目标触达都要经 Broker 归属授权，不能靠容器网段泛放行。宿主 OS 适配器分别验证 macOS、Linux、Windows，不把 Linux rootless 选项当作跨平台既定事实。UI 配置页显示能力包可用/版本/来源/缺失/审批，任务停止卡给出精确缺口和可恢复下一步。

验收：断网启动、包源摘要不符、工具缺失、容器退出、超时、OOM、跨项目路径、DNS 重绑定、崩溃恢复、工具版本升级与权限降级均有回归；失败时不得发目标请求、不重放不确定调用、不把能力缺失写成安全完成。参考 Docker 官方的 rootless、seccomp、资源限制和 digest 固定实践，以及 Strix 官方的按任务限量注入 Skills 与根 Agent 不直接触达目标；只借鉴约束，不复制其运行时或宣称容器天然安全。

## 6. Qoder 按阶段交付，不得合并“看起来完成”

1. **基线和回归**：冻结当前状态；旧 JSON/SQLite/日志的 Golden 导入、Native-only 静态检查、scan/attempt/证据深链回归先红。记录已有功能、缺口和用户改动，不 reset。
2. **任务与详情**：服务端全库检索、可逆归档、历史 attempt 快照及证据 revision 深链；UI 三栏/窄屏/空态/加载态。确认旧结果页没有丢入口。
3. **停机与恢复**：持久 stop snapshot、硬/软/能力/终态四类、准确下一步；崩溃注入和不确定目标调用不重放。
4. **协作闭环**：持久频道/线程和人工消息；EvidenceGap 的提案→独立有界补证合同→新事实 revision→独立 Reviewer 重评。每个动作可从 DB 重放，不制造聊天气泡。
5. **资产画像**：仅从已验证证据聚合，AI 假设有支持/反例，跨项目隔离；未证实相似接口永不自主触达。
6. **L0–L3 治理**：按任务学习、聚类去重、审批/版本/退役/回滚、检索配额和收益反馈；旧知识接口兼容。
7. **发布验收**：完整质量门禁、安装/升级矩阵和用户授权目标实测。缺少授权材料时只标“待实测”，不模拟完成。

每阶段提交：变更文件清单、DB 迁移与回滚策略、先红后绿测试、界面截图或可复核操作路径、边界反例、已知缺口和本地门禁输出。不得用编译通过代替端到端验收，不得把 Stage 10 角色枚举当真实专家，也不得把 Stage 1–6 的局部功能称作整套发布完成。

## 7. 两个外部入口的联调停止线

用户给出的 `http://222.92.38.52:8099/login` 和 `http://222.92.38.52:10080/dashboard/analysis` 不能仅凭两个 URL 推断可扫描整个 IP 或邻近路径。正式测试前录入授权书有效期、主体、两个端口与路径/方法白名单、生产/测试标记、请求速率/总量、时窗、允许身份及账号矩阵、写操作和清理条款、WAF/429 停止规则。凭据走身份通道，不贴入聊天或文档。先匿名只读，再按批准的身份与控制组验证；记录每条请求/stop code/覆盖缺口。浏览器若拒绝某端口，记为环境限制而非绕过；HTTP 200 不等于业务可用或扫描成功。未经上述登记不得部署任务去猜测接口，也不得声称已完成真实目标验收。
