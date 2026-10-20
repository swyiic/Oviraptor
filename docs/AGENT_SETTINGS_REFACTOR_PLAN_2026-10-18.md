# Agent 与设置统一审计及重构计划

日期：2026-10-18（Asia/Shanghai）。基线：`feature/agent-first-app-platform-20261016`，`30dd219` 加本地未提交改动。

状态：**Goal 已创建，实施中，未整体验收**。本文合并本次 Agent 审计、设置所有权盘点和实际修复记录。测试通过、界面出现控件、保存成功分别记录，不等同于真实功能已经接通。

最新范围调整：用户要求 **HackerOne 停止并隐藏，保留历史代码和数据**；不再推进其启用或同步。用户随后要求优先快速可用版本：当前源码收敛后先交付能启动的本地修复包，其余完整重构继续由 Goal 跟进。

## 1. 目标与操作边界

- 配置按模型连接、资产 API、采集、探测、内容分类、页面规则、Nest 策略、代码规则库、网络、机器环境和 Worker 节点分别展示和保存。
- 明确当前作用于哪个方案、项目、应用或机器，以及真正使用该配置的任务。聊天模型与扫描模型的认证不互相覆盖。
- 修复 Agent 的会话隔离、授权与取消、工具记录完整性、结果真实性和 UI 接线，再补持久化任务计划、恢复、独立验证与子 Agent 协作。
- 保留既有脏工作树和历史配置；不自动提交/推送，不操作真实用户数据库、CAS、凭据或安装环境。回归只使用临时目录、合成请求和临时库。
- 不把删除控件或禁用危险执行算成功能完成。缺少消费者或执行隔离时如实标明，保留后续实施项。

## 2. 设置所有权与真实消费链

本节记录审计基线及各配置的实际消费者。原 `ConfigDialog.vue` 将以下对象混入一个 `config_profiles.settings_json`，保存时还会写独立的全局代码规则库。本轮已拆分编辑与保存边界，实施状态见第 6 节；持久化 owner 的完整拆分仍未完成，不进行真实用户数据的自动迁移。

| 分类 | 当前存储与作用域 | 实际消费者 | 问题与目标 |
| --- | --- | --- | --- |
| 聊天模型 / Grok OAuth / DeepSeek | agent-host 的 provider 凭据文件；宿主 runtime 设置在 `host.sqlite` | `packages/agent-host/src/provider.mjs`、`service.mjs` | 与旧模型方案独立。按 provider、endpoint、认证类型保留凭据；切模型不等于注销其他连接。OAuth 不继承 DeepSeek 端点。 |
| 扫描及旧版对话模型 | 默认 `config_profiles` 的 `modelProfiles` / `activeModelProfileId`，另有 flat cache | `model_configuration.rs`、`conversations/reply.rs`、扫描模型解析器 | 必须单独命名，不能称全应用唯一默认模型。当前聊天 provider 不能悄悄用于扫描，也不能假定 OAuth 等于云 API key。 |
| FOFA 资产 API | 资产任务选择的方案：`fofaKey`、`configPath` | `jobs/native_config.rs:6-37`，`jobs.rs:1096-1108` | key 优先于文件；与模型凭据分开。`fofaEmail` 当前无生产读取。 |
| HackerOne API（已暂停） | 历史 profile 字段和数据保留 | 历史源码 `commands/hackerone.rs` 已从当前命令模块/IPC 注销 | 主界面、设置、host shell 目录均隐藏；旧偏好与 host 刷新不能重新启用。账号字段不再随 FOFA 分类保存。 |
| 资产采集 | Job 指定的 profile | `jobs.rs:1148-1160` | pageSize/maxPages/timeout/interval/history/CIDR/fofaProfile 有效；collectionMode/maxDerivedDomains/includeWeakFingerprints 暂无生产消费，不能继续作为有效控件宣传。 |
| 资产探测 | Job 指定的 profile | `jobs.rs:1296-1297,1349-1377` | workers 是探测并发；与机器 Worker 节点、Nest homogeneous Worker 无关。runProbe 会强制 refine，UI 需体现依赖。 |
| 内容分类关键词 | Job profile 的三组关键词、阈值、replaceDefaultContentRules | `jobs.rs:1363-1372` | 与页面隔离规则、代码安全规则是不同对象。本轮将 contentThreshold 一并归入内容分类独立保存，说明替换默认词库的行为。 |
| 全局划词规则 / 项目页面规则 | 旧 content_rules 表 / 项目资产页面隔离 owner | `jobs/job_resume.rs:10-46`、`AssetRuleSettings.vue:37-49` | 两种作用域不能揉成一个“规则”开关；保留 scope 标识和各自消费者。 |
| Nest Agent 策略 | 默认扫描 profile | `agent_runtime/runtime_config.rs:37-63,337-430` | 分数、超时、token/request/no-tool、localFullPower、promptAudit 为扫描策略，不控制聊天 host。agentBatchSize 只有 cfg(test) 读取。 |
| 前端采集包策略 | 默认扫描 profile 的 frontendPacketMode/Budget | `frontend_recon_compact.rs:374-390` | 存在真实消费者，当前只有 defaults、没有对应编辑，应补精确控件。 |
| 代码规则库 | **全局** `security_rule_packs` | `rule_packs.rs:39-96`、`result_ingestion_engines.rs:2` | 当前任意方案保存都会无条件写三条规则且 enabled=true；改为明确独立保存，保存 FOFA/探测不触碰全局规则。 |
| Native analyzer 镜像 | 默认扫描 profile 的 nativeAnalyzers digest 配置 | `native_source_scan.rs:47-61` | 与全局仓库注册、宿主 Docker 可用性是三件事；需分开显示保存与安装/验证状态。 |
| 业务网络设置 | profile proxyUrl / noProxy / agentProxyEnabled / proxy pool | FOFA/H1、`agent_backend.rs:174-181`、`frontend_recon_artifacts.rs:379-405` | 不控制 host OAuth/API。必须标明消费者和范围；不能用“全局代理”混称。 |
| host 网络 | 进程环境 / 系统代理 | `packages/agent-host/src/net-proxy.mjs` | 当前不读 profile proxyUrl。后续提供宿主独立网络设置，明确实际生效来源，不能伪装已共享。 |
| 本机运行环境 | 机器环境检测/安装/恢复 | `commands/environment.rs:13-20`；App 环境面板 | check_environment 忽略 profile_id、实际读 PATH。windowsRuntimeDirectory/scriptsDirectory 无生产消费者；不能与环境管理重复。 |
| 本机/远端 Worker 节点 | `app_settings` / `worker_nodes` | `worker.rs:92-109,453-475,528-563` | 机器级服务与节点连接；不等于 Nest 子 Agent 或探测并发。 |
| Apps / 数据权限 / Skills | host 独立安装状态、项目 grant、Skill selection/consent | host AppManager / Skills access | 安装、启用、项目授权、提供工具、任务实际执行必须分开显示。不能以安装成功表示 Nest 扫描已接通。 |

### 2.1 保存不变量

1. 修改采集、探测或 API 参数不得写代码规则库、注销模型认证或变更机器环境。
2. 编辑某方案不应因未触碰的模型配置而要求重新连通性测试。测试结论绑定实际模型参数；历史配置存在不等于本轮测试通过。
3. 分类保存只更新该分类字段，保留其他类别与未知历史字段；并发编辑不同类别不得互相覆盖。涉及同类别旧快照时应拒绝冲突。
4. 默认方案仅表示各当前消费者选用的默认方案，不能暗示聊天 host、远端 Worker 和全局规则同步切换。
5. 保留存储兼容与完成持久化拆分分别验收。阶段一仍使用旧 JSON 的分区更新，不能宣称独立数据库 owner 已迁移完成。

### 2.2 额外发现的设置矛盾

| ID | 具体矛盾与影响 | 当前处理 |
| --- | --- | --- |
| S01 | 一个“默认方案”被误当成全应用唯一默认；聊天 host、Nest、显式 profile 资产任务、机器 Worker 实际各有 owner。 | 导航与说明区分消费者，metadata 单独保存；连接与方案的引用存储仍待阶段二。 |
| S02 | 保存 FOFA/探测时覆盖整份 JSON，并改写全局规则包、重启模型测试。 | 分类白名单 + 同分类快照比较 + SQLite IMMEDIATE 事务；不同分类可依次保存，旧快照拒绝同分类覆盖。全局规则有单独保存动作。 |
| S03 | 已保存的无效字段仍显示为可用能力；环境路径与机器环境面板重复。 | 移除无消费者字段的编辑控件和新建默认值，保留历史字段原值；环境检测留在机器入口。 |
| S04 | 探测依赖 refine，但 UI 可显示“probe 开 / refine 关”；内容阈值混在探测连接参数。 | UI 展示依赖并锁定有效 refine 状态；阈值归内容分类。 |
| S05 | 取消唯一默认会出现 0 个默认，UI 按更新时间回退而 Rust 按 ID 回退，可能使用不同账号/模型。 | 拒绝直接取消当前默认，切换时原子指定替代方案。历史已有无默认的数据尚未自动修复。 |
| S06 | “复制一份”对 Vue Proxy 调用 structuredClone，实际抛 DataCloneError。 | 对 JSON DTO 作独立克隆；真实 Vue reactive fixture 验证副本编辑不改变源方案。 |
| S07 | HackerOne 入口按默认方案判断是否启用，执行却读传入 profile；用户选择有凭据方案仍可能看不到入口。 | 曾完成选择一致性修复；用户随后明确暂停此功能。现不暴露入口/IPC/同步，历史实现归档，不再作为当前可用功能交付。 |
| S08 | “网络”混指业务代理、host 系统代理、远端 Worker；“规则”混指关键词、项目页面和机器代码库；“worker”混指三种并发。 | 各入口标明作用域；host 独立网络设置、规则版本与消费者统一仍待阶段二。 |

## 3. Agent 已复现问题清单

| ID | 优先级 | 失败复现 / 根因 | 验收条件 |
| --- | --- | --- | --- |
| A01 | P1 | 同一 runtime 的全局 `_onText` 被并发会话覆盖 | A/B 交错流只进入各自 run，最终消息和历史正确。 |
| A02 | P1 | UI 切项目/会话无 generation，迟到请求错发；授权提示随当前 selectedId 漂移 | 迟到 A 不能覆盖 B；授权绑定 conversation/run/request，不能批准其他请求。 |
| A03 | P1 | DeepSeek→OAuth 继承旧 endpoint；切 API key 会覆盖 OAuth | OAuth/API 双向切换分别复用所属凭据，不把 token 发到其他端点。 |
| A04 | P1 | 同 run 共用 workspaceAuth，只读授权放行并发写入 | 分权限/请求审批；同意 read 不代表 write/run。 |
| A05 | P1 | cwd 不等于隔离；symlink 和脚本可越出 agent 目录 | 读写真实路径受限；执行提供经过验证的隔离。无隔离时拒绝执行并标记缺口。 |
| A06 | P1 | cancel 不解除授权等待；迟到同意后仍写文件 | cancel 结束所有等待并撤销请求；迟到决定无效；执行前后检查 run/scope。 |
| A07 | P1 | 压缩按单条消息删除，留下孤儿 tool；过滤未知工具也遗漏结果 | 任意压缩后调用与结果成对，未知调用有明确失败结果。 |
| A08 | P2 | MCP isError 当成功；验收仅看调过哪个工具 | 失败不进入 confirmedFacts；验收需要本轮成功、相关的只读结果。 |
| A09 | P2 | Chat 缺终止帧 / Responses failed 被判 end_turn | 区分失败、截断、取消、未知、完成，不把残缺回复标完成。 |
| A10 | P2 | final HTTP 期间已接受的 steer 没有下一轮，队列删除 | 已接受插话被本轮处理或持久化为下一轮；不得静默丢失。 |
| A11 | P2 | desktop assets 镜像只比较数量且不删除 | 同数量修改、删除、来源读取失败均有正确状态；避免查询旧镜像当当前真相。 |
| A12 | P1 | 桌面聊天未接 ACP permission.request/snapshot | 桌面能明确审批/拒绝并恢复待决请求，断线有可见状态。 |
| A13 | P2 | 普通 p 渲染回答；IME Enter 无 composing 检查 | 安全 Markdown/代码块/表格恢复；中文选字不误发。 |
| A14 | P1 | .app 未包含 host，回退编译机器源码路径 | 干净机器从包内启动匹配版本的 host 与依赖，不依赖源码 checkout。 |
| A15 | P2 | 安装 v2 native 包后，系统提示禁止真实 saved_assets_*/scan_list/status | 从真实 tools/list 派生能力提示，验收认识真实只读工具。 |
| A16 | 待验证风险 | retry 调用未固定 operationId，注入 worker 可产生不同逻辑身份 | 明确读写/幂等/未知结果边界并复用逻辑ID。真实 AppProcess 超时会停止，不能把注入 worker 结果当生产重复写证明。 |

复现脚本（临时目录）：`/private/tmp/ovi-agent-loop-audit-20261018/repro.mjs`、`/private/tmp/ovi-tools-audit-20261018/tools-audit.mjs`。脚本是当前机器审计证据；重要负例在修复时转为仓库内回归。

## 4. 实施顺序

### 阶段一：设置边界与 Agent 阻塞

- [x] 设置按消费对象拆分导航与编辑入口；建立分类字段白名单和独立保存语义。
- [x] 代码规则库独立保存，去掉业务方案保存的全局副作用。
- [x] 隐去无生产消费者字段，保留历史存储；补有效但未暴露的扫描策略。
- [x] OAuth/API 凭据隔离、会话流隔离、授权/取消修复（本地协议回归）。
- [x] 工具协议压缩、结果真实性、断流与 steer 修复（合成模型响应回归）。
- [x] 聊天项目切换、审批/重连、Markdown/IME 修复（组件与客户端回归）。
- [x] 镜像更新/删除一致性与资产身份复用修复（临时库）。
- [x] 包内 host 资源与版本回执接入桌面构建，脱离源码目录启动验证。

勾选表示源码实现及指定本地验证通过；实际安装 App、真实账号与跨平台行为不在此处勾选范围。A05 的脚本隔离执行、A14 的干净机器完整安装仍列入阶段二。

### 阶段二：持久化配置所有权与完整产品链

- [ ] 用引用与版本将模型连接、集成账号、业务方案从自由 JSON 中拆分，保持明确的旧配置兼容读法。
- [ ] 统一扫描/旧版对话使用的模型解析入口；聊天 OAuth 不自动授予扫描用途，需真实 provider 兼容及用户明确选择。
- [x] host 网络配置与实际生效来源独立展示；不自动修改进程或系统环境（本机路由/断路负例通过，真实账号未使用）。
- [ ] 明确全局词库、项目页面规则与任务关键词快照；补保存版本/冲突检查。
- [ ] 干净目录验证真实 .app host 打包、Node 运行时、worker、资源与版本回执。
- [ ] 真正隔离执行（平台矩阵、取消进程树、环境白名单、网络策略、资源限额），恢复脚本执行能力。

### 阶段三：持续任务能力

- [ ] 持久化目标、步骤、依赖、验收、产物与待完成工作；重启与模型切换可继续，未知副作用不可重放。
- [ ] 结果验收关联实际工具证据；独立审查与测试有独立状态。
- [ ] 聊天主链路接通子 Agent 创建、等待、取消、回收和结果验收；不借用未接通的扫描能力宣称完成。
- [ ] 任务级固定评测覆盖并发、授权、取消、断线、压缩、恢复、模型切换与部分失败。

## 5. 验证与当前限制

审计基线：`npm run build` 通过；44 项相关定向测试通过，但隔离复现仍发现上述缺陷。`TMPDIR=/private/tmp npm run test:agent-first` 为 680 passed / 32 failed / 19 skipped。32 个失败包含缺少 Assets UI build-manifest、macOS native fixture 平台不匹配和已过期断言，不全是业务故障。

实施后的准确测试命令、结果、仍失败的原因在下节追加。没有运行真实付费模型、真实 OAuth 账户、目标探测、环境安装或用户数据迁移。安装包与真实账号链仍需独立验收。

## 6. 实施记录

- 2026-10-18：创建 Goal 和本文；完成设置消费链盘点。并行修复 provider、授权生命周期/工作区、设置编辑边界；其余事项尚未验收。

### 6.1 已实现的生产路径

- **设置**：独立导航覆盖聊天连接、扫描模型、资产 API、采集、探测、内容分类、Nest 策略、代码规则、业务网络、页面规则、机器环境与节点。`configProfileSections.ts` 与 Rust `workspace_config_profile_sections.rs` 对齐分类白名单；方案名称/默认选择有独立 metadata CAS，未知历史字段保留。代码包启用状态不再被普通方案保存重置。模型连通性仅对已修改的连接要求测试，迟到/失败测试不能批准新的参数。
- **Provider（A01/A03/A09/A10）**：凭据按 endpoint/API 与独立 OAuth 槽保留；切换不注销另一个连接，OAuth 固定官方端点且仅对对应 401 刷新。流回调和配置快照绑定当前 session/run；缺少终止帧、失败响应和残缺工具参数均不报告正常结束。final 请求期间接受的 steer 继续进入处理。
- **权限与聊天（A02/A04/A06/A12/A13）**：授权按 conversation/run/request/kind 排队；取消结束等待，迟到审批失效；桌面接入 ACP 审批、待决恢复与断线重连。项目/会话的异步结果有 generation 校验，中文 IME Enter 不误发送，回答复用安全 Markdown 渲染。取消不能回滚取消前已经准入的 App 副作用。
- **工具与上下文（A07/A08/A15/A16）**：tool call/result 成组压缩，未知工具返回明确失败；原用户要求保留有界摘要及内部 metadata，发给 provider 时剥离内部字段。MCP isError 不算成功，验收只认当前轮成功且相关的读取结果；工作区文件读写成功不能证明资产事实。重试复用 operationId，但没有据此宣称真实外部写入具备 exactly-once。
- **工作区（A05 部分）**：文件工具校验真实路径、符号链接/硬链接和作用域，使用受限读取与原子写入。`workspace_run` 不再发布为可用工具，执行隔离尚未实现，不把限制文件路径当成脚本沙箱。
- **资产镜像（A11）**：一致性快照处理同数量修改、删除和来源读取失败，保留其他来源数据。补充发现 SQLite 复用删除 ID 会让历史 Insight 指向新资产；Assets owner 1.1.0 的已知 v1→v2 事务升级采用 AUTOINCREMENT，并保留历史 ID/索引/Insight，未知结构拒绝升级。host 只接受 schema v2，不替 owner 迁移 v1；真实旧库未升级。
- **桌面分发（A14 部分）**：`build:desktop` 构建并封装 host/UI/SDK/runtime/Skills 与两项内置 App，生成并验证文件清单和摘要。release 不再回退编译机器的 checkout。Node 版本必须至少 24，但 Node 本身仍是外部依赖；没有完成可分发安装包验收。

### 6.2 回归证据与限制

所有数据库回归使用临时库。

| 检查 | 当前结果 | 证据 |
| --- | --- | --- |
| `TMPDIR=/private/tmp npm run test:agent-first`（无 Cargo 并发） | **772 passed / 0 failed / 19 skipped，791 tests** | `/tmp/ovi-goal-agent-suite-serial-final.log`；19 项为本机不适用的 Linux/native/ELF 用例 |
| `npm run build` | vue-tsc 与 Vite 通过 | `/tmp/ovi-goal-build-latest.log` |
| `TMPDIR=/private/tmp npm run test:agent-settings` | 35 passed / 0 failed | `/tmp/ovi-goal-agent-settings-suite.log`；含真实 Vue script/组件编译，非逐页截图验收 |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib configuration_sections -- --test-threads=1` | 4 passed / 0 failed | `/tmp/ovi-goal-sections-rust-accepted.log`；分类隔离/历史字段/冲突/默认替代，临时内存库 |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib agent_host_sidecar::tests -- --test-threads=1` | 7 passed / 0 failed | `/tmp/ovi-goal-sidecar-rust.log`；库编译仍有既有 unused/dead-code warnings |
| `npm run test:desktop-agent-host` | 5 passed / 0 failed | `/private/tmp/ovi-agent-loop-audit-20261018/desktop-distribution-tests.txt`；隔离临时启动，不是安装 App |
| `node tools/prepare_desktop_agent_host.mjs --verify` | 93 文件、摘要/依赖/版本回执通过 | `/tmp/ovi-goal-desktop-final-verify.log` |

完整 suite 的一次负载运行记录：791 tests，770 passed / 2 failed / 19 skipped（`/tmp/ovi-goal-agent-suite-accepted.log`）。两次失败均在 ACP 测试 fixture 的 1000ms initialize 阶段超时，与 Rust 编译同时运行；未修改超时阈值或生产代码。随后单独重跑 ACP 为 24/24（`/tmp/ovi-goal-acp-recheck.log`），无 Cargo 并发的最终整体复跑为上表 772/0/19。不抹除该负例，保留负载下测试启动时限不稳定的风险。

以下证据范围不会因测试全绿而扩大：

- Assets UI 资源已生成；HTTP native close 的 macOS fixture 修复为从真实 FAT Mach-O 提取当前 CPU thin slice，26/26 通过。协议执行仍为注入的 Node peer，不是实际 native 业务二进制验收。
- `npm run build:desktop-agent-host` 和 stage `--verify` 通过，93 个资源文件；将产物移离源码、空 HOME/cwd、临时库启动的 5 项分发测试通过。未启动用户已安装 App。
- Linux 专属 native action/owner/upgrade 与 ELF 用例在 macOS 跳过；不覆盖这些生产平台行为。
- 未使用真实付费模型、OAuth 账号、采集/探测目标、用户库/CAS或环境安装；真实屏幕上的全部 UI 尚未逐页验收。

### 6.3 下一阶段与仍未完成

1. 设计并实现模型连接、平台账号、方案引用的独立 owner/版本；只对临时或新建存储验证兼容读，真实凭据/历史库迁移另行处理。
2. 接通规则版本与消费者，避免“保存成功但消费者未使用”；HackerOne 已按用户要求暂停，保持隐藏。默认方案解析已统一到稳定 ID 顺序，不自动改写历史默认标记。
3. 补临时数据目录下的真实桌面 App 启动/视觉检查与运行时依赖验证；再做平台执行隔离，保持取消与授权约束。
4. 实施持久任务计划、恢复、独立验收与聊天子 Agent 主链路。当前没有达到长期 Agent 的完整能力，Goal 保持 active。

### 6.4 阶段二架构目标（实际进度见第 7、8 节）

每类配置有独立保存接口与 revision。任务明确引用 `id + revision`，后端校验并冻结有效参数；默认值只用于首次选择，不修改已有任务。

| 实体 | 字段 / 引用 | 消费者与边界 |
| --- | --- | --- |
| ModelConnection | owner、transport、model、endpoint、accountRef、deployment；区分 HostChatConnection / RustScanConnection | host 聊天与 Rust 扫描各自 resolver 校验适配器能力。只统一描述格式，不假定 OAuth 与扫描 API 协议兼容。 |
| ServiceAccount | 类型、owner、provider、secretRef、credentialGeneration；业务 API 的代理设置 | FOFA、HackerOne、模型 API、本地 Key、host OAuth 分类型；secretRef 只由所属进程/owner 解析，不把 token 自动填入另一类 API Key。 |
| CollectionPolicy | fofaAccountRef、分页/上限/间隔/超时、历史/CIDR/fofaProfile、可选 probePolicyRef | 资产采集消费者；HackerOne 账号独立用于同步，不混入 FOFA 采集策略。 |
| ProbePolicy | 并发、速率、请求/总超时、重试、分类范围、contentRuleSetRef、明确 refine 依赖 | 资产探测/复测；不包含 Nest 子 Agent 数量或机器节点配置。 |
| ContentRuleSet | 三组关键词、contentThreshold、replaceDefaultContentRules | 资产分类；任务保留采用的规则快照。项目页面隔离和历史全局划词维持各自 owner。 |
| NestPolicy | scanModelConnectionRef、分层预算、无进展限制、本地资源/审计、前端上下文预算、扫描代理池/noProxy、代码规则快照引用 | Rust Nest；不读取聊天质量档位或 host 登录状态。机器环境、Worker 节点、全局代码包仍独立管理。 |

连接关系：聊天会话 → HostChatConnection → host 账号；Nest 任务 → NestPolicy → RustScanConnection → 扫描账号；资产任务 → CollectionPolicy → FOFA 账号及 ProbePolicy → ContentRuleSet；HackerOne 同步 → 指定账号。聊天、Nest、资产、HackerOne 各有明确默认引用。

兼容策略：旧 JSON 提供只读适配，不在读取时迁移或补写凭据；拆分操作先生成字段归属与引用预览，再只对选中对象事务性创建新实体。记录原方案 ID 和来源摘要，保留未知字段。一旦切换到新 owner，只写新实体，避免长期双写。账号快照仅记录引用和凭据代次，任务/日志不复制明文秘密。真实历史库与凭据迁移不在本轮执行。

## 7. 第二阶段实际进展与快速版本

- **四类独立策略已接入**：采集、探测、内容分类、Nest 参数支持明确预览后拆分；策略存储有独立 binding、revision、不可变版本和摘要校验。拆分与后续保存不改写旧 JSON，不迁移模型或账号。分类保存携带预期 revision；损坏引用/内容拒绝使用，不回退旧策略。
- **真实消费接线**：方案列表、共享模型/扫描解析与资产任务启动读取有效策略。资产任务保留有效参数和引用快照；loopback 实验验证旧任务恢复保留 rev1 重试行为（200、2 次），新任务使用 rev2（503、1 次），原 JSON 字节未变。Nest 重启恢复与所有预算快照边界尚未整体验证；不把四类策略存储算成完整六实体架构完成。
- **Host 独立网络**：本机私有 network owner 提供继承启动环境 / 直连 / 手动代理，不修改系统代理。UI 显示去凭据路线及最近实际请求。修复 HTTPS CONNECT 建立隧道后意外另开直连；手动代理失败不降级，取消关闭 socket，Node 全局环境代理不能覆盖显式直连。
- **暂停 HackerOne**：保留源码/历史数据，移除当前命令注册与 host shell 广告；侧栏和设置不显示，旧路由安全拒绝。当前账号分类只编辑 FOFA，保留既有 H1 字段原值。

验证记录：

| 检查 | 结果 / 范围 |
| --- | --- |
| `TMPDIR=/private/tmp cargo test --manifest-path src-tauri/Cargo.toml --lib configuration_ -- --skip mapper_loop_history_actual_two_round_handoff_scope_and_configuration_drift_are_readonly --test-threads=1` | 27/27；配置读写、兼容与 native 服务路径，`/tmp/ovi-phase2-rust-final.log` |
| `service_reprobe_consumes_adopted_probe_policy_and_resume_keeps_frozen_revision` | 真实资产任务启动/取消/恢复 + loopback，1/1（1.37 秒）；临时库、无外部探测 |
| `TMPDIR=/private/tmp npm run test:agent-settings` | 38/38，`/tmp/ovi-quick-settings-final.log`；含拆分预览/确认/修订号与模型、聊天边界 |
| Host 网络 + 既有 Provider/OAuth | 52/52；`/private/tmp/ovi-agent-loop-audit-20261018/network-owner-tests.txt` |
| 网络设置 UI/客户端 | 10/10 + vue-tsc；`/private/tmp/ovi-agent-loop-audit-20261018/network-ui-tests.txt` |
| 活跃 host shell 目录 | 2/2，H1 不在目录；`/tmp/ovi-quick-shell-final.log` |

保留测试环境负例：一次 Rust 回归未设置规范 TMPDIR，6 个 fixture 在路径准入前被拒绝（`asset_service_path_invalid` 等），不是业务执行失败；同一测试二进制在 `/private/tmp` 下通过，随后规范 TMPDIR 的完整定向 27 项通过。该次宽过滤额外跑到已有 mapper 历史测试（约 80 秒且通过），快速版本不重复扩大该项。

快速包收尾范围：构建匹配的 host stage/前端/桌面 release；用 `OVIRAPTOR_ACCEPTANCE_ROOT` 和私密 WebView 验证同一包的启动与关键页面；输出可直接打开的 App。结果见下节。验证期间不读写真实用户数据；不覆盖已安装应用。构建包不内置临时数据目录，正常启动保留原持久化行为。完整模型/账号 owner、任务恢复、子 Agent 和隔离执行继续作为 Goal 未完成项。

### 7.1 已生成本地修复包

- App：`/Users/swyiic/Desktop/Oviraptor-2026-10-18-LocalFix-20261018T011202/Oviraptor.app`。
- ZIP：同目录 `Oviraptor-1.1.66-LocalFix-macOS-arm64.zip`，28,074,788 bytes，SHA-256 `d8cb9f81103213edd2f6ca0258a2c0920198f9f55f88a439841b62c65b0c7e70`。
- 正常产品 identity/version：`com.oviraptor.desktop` / `1.1.66`，本地源码修复构建；没有改写正式版本号或覆盖安装目录。
- `codesign --verify --deep --strict` 通过；ZIP 完整性通过，解压后签名与主程序摘要匹配。包内 host 94 项资源的摘要与构建回执一致。
- 源码摘要：`e677f0005520817794434a8471cac47a9aa232cd9cfb736886aa21d9b1441ffe`；主程序 SHA-256：`fc297668f088f0f892dee428c1502cbd9b814094ef5a7e85380ace4452d09ca6`。同目录 `local-build-receipt.json` / `zip-verification.json` 记录证据。
- 同一交付副本已真实启动：临时数据目录、私密 WebView、包内 host 和两个内置 worker 正常；无真实库/CAS文件句柄、无 provider 配置文件。已检查聊天空态、设置分类、H1 无入口、聊天认证与网络分区和 1180 宽度布局。截图位于 `/private/tmp/oviraptor-ui-prep-20261018-IAESqz/ui-*.png`。
- 独立策略编辑器和 JSON 预览已确认可用；完整创建/revision1 的 UI 流程未验证（期间遇到 ScreenCaptureKit 捕获失败），未据此宣称 UI 创建通过。其 Rust/真实资产消费者和组件回归见上文。
- 最终 UI 回执为同目录 `ui-acceptance-receipt.json`。本次测试 App/host/worker 已全部退出，用户原 App 保持运行。测试使用强制 TERM 后 host 子进程未自行退出，按已核对的 owned 进程组清理；正常 Quit 的回收行为仍需后续专门验证。
- H1 暂停定向：103 passed / 0 failed / 8 archived skipped；旧偏好、host refresh/reset 无法恢复入口，旧/未知路由不会崩溃或调用 H1 IPC。

限制：这是本机 macOS Apple Silicon 修复包，Node 仍使用本机 24+ 运行时；没有做真实 OAuth/付费模型调用、真实数据升级或全产品逐页验收。正常启动继续采用原持久化位置，使用前应退出旧版。Goal 仍 active，尚未以该快速包代替完整重构验收。

## 8. 快速包后的增量（未收入 7.1；已有 LocalFix2 快照，见 8.1）

优先打通日常操作链，保持已交付包不变。以下改动只有完成对应动态回归后才记录为验证通过，不能用语法检查代替生产消费验证。

- **扫描模型与 FOFA 账号 owner**：新增 typed service account、model connection、不可变 descriptor revision 和 profile binding；模型描述只存 account/connection 引用，凭据在独立账户表。预览隐藏密钥，显式拆分与保存都由 IMMEDIATE 事务比较分类值和预期 revision。旧 JSON 字节保持不变；读取不建表、不迁移。改名称复用原 connection/account，改地址或密钥创建新引用，历史版本不被重新指向。扫描 owner 不读取 Host OAuth，独立云模型不隐式读取 `OPENAI_*`。本地 key 与云 key 分开。
- **真实消费与兼容边界**：有效配置统一覆盖方案列表、默认模型解析、资产启动/恢复；native `configure_model` 对已有方案只写模型分类，已拆分时写新 owner。旧全量写入不能覆盖新 owner，也不能将有效凭据回灌 legacy JSON。资产任务快照不再复制无关扫描模型密钥。FOFA 显式 Key 与配置文件来源二选一，拆分不读取文件；运行时文件内容及允许的恢复凭据修复仍可变化，不能把账号引用误称为文件内容冻结。私有 SQLite 账户与保留的 legacy 原文尚未迁入系统密钥库或清理。
- **Nest 任务策略冻结**：新 Web/Source attempt 在现有发布事务冻结 agent 字段与 policy reference；同 attempt 恢复/派生 continuation 使用原版，显式 fresh attempt 读取新版。预算、packet 与私有恢复 HMAC 使用冻结参数。历史缺可验证快照不补表、不补授权；模型连接和原 sealed/claim 门禁继续独立验证。
- **Host 持久任务**：任务目标、步骤依赖、状态、实际工具证据引用和产物写入 conversation task；重启将进行中任务标为 interrupted，人工继续绑定原 conversation/task，不自动重放未知副作用。步骤和任务 revision 防止旧响应覆盖；普通回答结束 run，与显式任务完成分开。该证据结构不等于独立验证器或完整聊天子 Agent 实现。
- **桌面退出**：host 采用独立 owned PGID，正常退出先给 host 2 秒关闭会话与数据库，再仅清理其 owned 进程组；SIGTERM/SIGINT 通过 watcher 请求正常 Tauri 退出，窗口 hide 不触发清理，外部 endpoint 不拥有可杀进程。临时进程树生产 helper 5/5 通过；真实 App Quit 待验证。SIGKILL/崩溃不能依赖退出回调，Windows 整组回收仍未实现。

验证进度：设置组件回归 41/41（`/private/tmp/ovi-connections-ui-final-20261018.log`），独立复审的分类 UI 14/14（`/private/tmp/ovi-connections-ui-review-20261018.log`），策略历史错误/实际 Board 失败入口 8/8（`/private/tmp/ovi-nest-policy-ui-final-20261018.log`）；Rust owner、Nest 消费与整合编译正在统一执行。首轮并行编译发现 fresh protection scope 被误当作有 scan_id 的旧任务，已撤回该处错误继承，保持 fresh scope 使用当前策略；该轮日志保留于 `/private/tmp/ovi-connections-config-tests-20261018.log`，不作为通过证据。

后续动态记录：

- 配置 owner / native 配置消费 `configuration_` **35/35**（`/private/tmp/ovi-phase3-rust-final-20261018.log`）。没有外部模型请求或真实凭据。
- Web owner 解耦的真实新正反例已通过：只改 FOFA/采集/探测/内容规则与模型显示名可恢复；实际模型、凭据、地址、代理、worker 或授权目标变更仍拒绝。该轮 **13/16** 通过（`/private/tmp/ovi-web-owner-binding-final-20261018.log`），3 个旧 fixture 已修正：权限/attempt 漂移继续作为拒绝负例；raw 配置比较改为 normalize 后比较，并保留字节级事务回滚断言。更新后的整合复编尚待执行，不能称此轮全绿。
- Host 任务最终关键链 **15/15**、UI **14/14**、类型检查通过（`/private/tmp/ovi-agent-loop-audit-20261018/task-final-critical.txt`、`task-ui-tests.txt`、`task-ui-typecheck.txt`）。实证修复旧轮次 token 跨轮写入、旧证据验收新步骤、在途写操作提前完成和活跃任务删除。取消/结束撤销 token，每轮 ACP `session/load` 重配 bridge；不丢原会话，未知写入不自动重放。Grok 1.0.62 的隔离 initialize 只确认能力声明，不是实际账号验收。
- Host 整套首轮 **811 tests / 787 passed / 5 failed / 19 skipped**（`/private/tmp/ovi-phase3-host-full-20261018.log`）。5 个失败来自旧 core schema fixture：精确白名单未包含新的 conversation task metadata，泛匹配又把它误作业务表。保留精确 12 表清单，并新增 task 列/外键/metadata payload 和不复制 App 业务目标的断言，定向 **17/17** 通过（`/private/tmp/ovi-agent-loop-audit-20261018/task-core-contract-tests.txt`）。最终整套 **811 tests / 792 passed / 0 failed / 19 skipped**，129.67 秒，日志 `/private/tmp/ovi-phase3-host-full-final-20261018.log`。
- 更新后的 Web 故障回滚 fixture 复编与动态 **1/1** 通过（`/private/tmp/ovi-web-owner-binding-recheck-20261018.log`）；其余真实 preflight/Nest/Source/sidecar 定向检查正使用同一最新测试二进制统一执行。

本轮最终定向回归已完成：

| 检查 | 结果与证据 |
| --- | --- |
| Web binding 全部用例 + 摘要兼容/读取失败 | **17/17**，34.82 秒；`/private/tmp/ovi-web-native-hash-final-20261018.log` |
| Nest 原版恢复 / 新版 fresh attempt / 历史不补授权 | **4/4**；`/private/tmp/ovi-phase3-nest-freeze-final-20261018.log` |
| Source runtime 预算/模型/技能/授权与回滚 | **10/10**；`/private/tmp/ovi-phase3-source-runtime-final-20261018.log` |
| Workbench effective runtime | **1/1**；`/private/tmp/ovi-phase3-workbench-runtime-final-20261018.log` |
| Sidecar/lifecycle | **12 passed / 1 ignored child**；父测试实际启动该隔离 child 并验证信号路径，`/private/tmp/ovi-phase3-sidecar-final-20261018.log` |

Web 调试耗时定位为每次 preflight 以未优化 SHA-256 重读约 244 MB 测试二进制；改用项目已使用的 native SHA-256，保留每次读取全部字节和同样的十六进制格式，不用 metadata 缓存或跳过检查。17 项最终检查从前轮 16 项约 540 秒降到约 35 秒；这是本机调试回归耗时，不是全产品性能基准。已增加标准摘要、跨 chunk 兼容和中途读取失败拒绝的回归。旧串行检查在更换测试二进制前停止，不把其中被中断的 Nest 检查算通过；最终表使用重新编译后的同一二进制。

完整 Goal 仍未完成：新建与分类默认归属、网络/分析器独立消费、独立验收/聊天子 Agent 主链及有隔离保证的 workspace 执行仍需完成；`workspace_run` 继续保持未开放。当前独立 owner 是明确预览后切换，未切换或兼容 API 新建的方案仍走 legacy；不能据此称旧方案体系已整体退役。真实账号调用、历史数据升级及全产品逐页验收没有执行。HackerOne 保持停用隐藏。

### 8.1 LocalFix2 构建快照与真实桌面结果

目录 `/Users/swyiic/Desktop/Oviraptor-2026-10-18-LocalFix2-20261018T035532-9feefa6f`，ZIP 为 `Oviraptor-1.1.66-LocalFix2-macOS-arm64.zip`，28,164,290 bytes，SHA-256 `ea1a15edaea6e8a1f2e58157ff1a9209e862bbca10da690f63738714096c81fa`。源码快照 `743cb0a62bf1d85fb3936894ea64fc8f8f685b9cfe3fd4a4f6495669bc613ab3`；正常 identity/version，签名、ZIP CRC、解压副本签名/主程序与 95 项 Host 资源摘要核对通过。此包保留不变，不包含第 9 节之后的源码修改。

同一 App 在两个独立 `/private/tmp/oviraptor-acceptance-*` 下实际启动，包内 Host 与两个 worker 正常，无真实用户库/CAS句柄或模型凭据。真实 UI 验证消息和持久任务保存、缺模型拒绝继续、H1 隐藏、分类导航与探测单分类预览。**探测拆分未通过**：单窗口默认数据的 `20.0` 经 JavaScript 传回 `20`，触发 Rust `Value` 数字表示差异导致错误 CAS 冲突，已交第 9 节修复；不把组件测试代替该失败。

正常 Cmd+Q 退出码 0，无残留 owned App/Host/worker，无关进程仍在。另一个私有 App 收到仅针对其 PID 的 SIGTERM 后，App 与全部 owned 子进程也正常退出，无需事后清理；实际退出码 0，原验收脚本预期 143 因此保留失败。当前 Tauri runtime 的 `RequestExit(code)` 设置 `ControlFlow::Exit`，macOS tao 返回 0；退出清理通过不代表 143 断言通过。证据在交付目录 `ui-exit-acceptance-receipt.json`，完整原始回执在 `/private/tmp/oviraptor-exit-prep-20261018-lj31ccli`。测试未停止 `/Applications/Oviraptor.app` 中现有用户进程。

## 9. 用户追加要求：取消配置方案浮窗，所有设置直接分区编辑

本节优先于前面保留总浮窗的过渡方案。用户明确要求把配置方案里的东西全部拆开，取消该浮窗；不是仅调整浮窗导航。

- [x] 移除配置中心总方案浮窗和“编辑此分类”二次跳转；模型、资产 API、采集、探测、内容规则、Nest 策略、业务网络、分析器设置直接在各自页面编辑保存。
- [x] 八类显式 owner、版本与实际消费接线；首次保存只 adopt 当前分类，已有 owner 按预期引用保存新版本，正常页面不要求先选总方案。
- [ ] 分类完全独立的全局默认/catalog/rebind；当前仍通过内部兼容 profile binding 选择，不能宣称底层 profile_id 已整体退役。历史来源只在折叠项暴露。
- [ ] 新建使用原子分类 owner 创建入口；旧数据仅显式转存，不自动迁移真实库或凭据。
- [x] 修复真实 App 复现的浮点/整数 JSON 往返假 CAS 冲突，同时保持真实值和 revision 漂移拒绝；正式默认值经 Node JSON 往返的失败与修复日志保留。
- [x] 聊天 Grok OAuth、扫描云 API、本地模型凭据保持隔离；业务代理与 Host 网络各有明确消费方。新增显式 saved OAuth 选择动作，修复同官方地址 API Key 无法切回 OAuth；普通模型编辑仍保留认证选择。
- [x] 补真实分区保存、重新进入、消费者读值和签名包临时 UI 回归；已生成 LocalFix3，新目录交付，未覆盖既有包。实测范围与边界见 9.2。

前端（含配置中心 App 接线）、Rust owner/消费链与独立审计并行推进，HackerOne 继续停止并隐藏。

### 9.1 源码与定向证据

- `ProfileSectionEditor` 为固定分类的内联表单；`ConfigDomainSettingsSection` 显示分类版本、折叠历史来源和真实作用域。总方案与 metadata 浮窗均不再由 App 渲染，历史名称/默认标记管理也使用内联界面。保存后先刷新真实有效值，再重建表单；外部变化保留草稿，旧页面迟到响应不能覆盖新页草稿。
- QueryPanel 新任务自动使用稳定的当前兼容 ID，不再要求选择总方案；显式历史来源与断点恢复仍保留。集合缓存、响应体上限、私网开关、TLS 校验、协议回退和单主机间隔补入各自分类；旧不可变版本未包含新增字段时保持原行为，下一次显式保存才冻结这些值。
- `runtime_owners` 管理 network/securityRules；代理凭据在独立 typed 表中，版本 descriptor 只保留引用。新资产任务与恢复任务落库前移除 URL userinfo，运行时按明确引用在内存 materialize；恢复仍明确使用当前 transport，不虚称原 transport 冻结。分析器读取当前有效 owner，未新增每 attempt 镜像冻结契约。
- 模型连接测试也要求明确 API 地址和云端 Key，不再从 `OPENAI_*` 借值；本地连接保持自己的凭据。Host 的 saved OAuth 选择是独立 session-authenticated API，运行中/切换中/登录待决时拒绝，更新真实 runtime 配置，不暴露 token。
- 前端定向 **103 passed / 0 failed / 1 H1 paused skip**、类型检查通过；包含八个真实 SSR 页面、八类精确请求、草稿竞态与隐私预览。资产真实 SFC **77/77**；恢复/工作空间/项目回归 **67 passed / 1 paused skip**。OAuth 选择、原 OAuth 与设置客户端定向 **27/27**（`/private/tmp/ovi-saved-oauth-selection-final-20261018.log`）。
- 后端独立 rustc harness **24/24**（`/private/tmp/ovi-runtime-owner-20261018-e7y562j6/after.log`），独立 owner 复审 **8/8**（`/private/tmp/ovi-independent-runtime-owner-review-20261018.log`）。统一 Cargo 首轮发现真实 consumer 测试的模块归属错误，已仅用 `cfg(test)` 私有转发修正，失败日志保留；整合重编与真实包验证仍在进行。

原子新建八类总 scope 的临时原型未合入，生产没有调用不存在的 create/receipt IPC；分类另存新 owner、全局分类默认/catalog 属后续工作。历史原文与凭据不自动清理或迁移，真实账号/付费调用、真实数据升级与全产品逐页验收未执行。

### 9.2 LocalFix3 已构建与实际桌面验收

交付目录：`/Users/swyiic/Desktop/Oviraptor-2026-10-18-LocalFix3-20261018T042455-aab8a6bb`，包含 App、ZIP、构建/ZIP/UI 回执和真实页面截图。ZIP 为 `Oviraptor-1.1.66-LocalFix3-macOS-arm64.zip`，28,209,001 bytes，SHA-256 `e347c829fd22b3bd2b39d56c85be94d0646e742565c5675ddd8d958da0ca46dc`；源码摘要 `7c7a225082e487a34e177486feb1ebb4d071fd78ae5bdb9298b5c66e31698314`。正常产品 identity/version，ad-hoc 本地签名，无 notarization；Node 24+ 仍使用本机运行时。ZIP CRC、解压副本签名、主程序与 95 项 Host 资源摘要通过；UI 验收后原签名和 ZIP/主程序摘要再次一致。

| 验证 | 结果 |
| --- | --- |
| 当前统一 Rust 配置回归 | **49/49**，`/private/tmp/ovi-inline-settings-rust-final-20261018.log`；包含真实默认值 JavaScript 数字往返、八类 owner、旧版本兼容、模型隔离和实际扫描消费者 |
| 真实临时任务启动 / 恢复 | **2/2**，`/private/tmp/ovi-inline-runtime-consumers-final-20261018.json`；断言新 snapshot 不落代理凭据且恢复使用应有引用 |
| Host/SDK/Apps 整套 | **815 total / 796 passed / 0 failed / 19 platform skips**，`/private/tmp/ovi-inline-host-full-final-20261018.log` |
| 前端 | 类型检查与正式构建通过；103 passed / 1 H1 paused skip；真实八页 SSR + 分类请求/竞态证据见 9.1 |
| 同一 App 的真实临时 UI | 八类全部通过直接保存；探测 96→4 保存第 1 版，采集页大小 500→123 保存第 1 版，返回探测仍为 4，再改 5 保存第 2 版 |
| 模型/聊天隔离 | 通过临时 `127.0.0.1` 有限回环服务测试本地扫描模型并保存，实际只发 1 次测试请求；聊天仍未配置、Grok 未登录，Host provider 文件不存在 |

关闭临时 App 后只读其已 checkpoint、无 WAL 的测试库：八类 binding 全部存在，仅 probe 为 revision 2，其余为 revision 1；新 owner 的 workers=5、pageSize=123，legacy 原字段仍为 96/500。没有读取/修改真实用户数据库、CAS、模型账号或已安装 App。首次整套的两个资产 author-build stale 失败已通过重建正式 Assets 页面产物解决；旧 ConfigDialog 静态断言改为实际 inline editor / App 无浮窗契约，最终整套全绿，原失败日志保留。

桌面边界保留：八类验收完成、运行环境页可见后，追加 Worker 导航时 CUA 返回 `noWindowsAvailable`，重新绑定也超时；原生 App PID 仍在且 stderr 为空，不能据此宣称 Worker 页面通过或 App 崩溃。随后只向已核对 executable/PID/PGID 的该测试 App 发 SIGTERM，退出码 0，owned App/Host/worker 全部退出，无关见证进程和原用户 App 不受影响。此包不声称 Cmd+Q 已重新验证；LocalFix2 的真实正常 Quit 证据独立保留。完整记录在交付目录 `ui-acceptance-receipt.json` 与 `/private/tmp/oviraptor-split-prep-20261018-78io6p4_`。

快速版本已取消目标浮窗并打通八类直接保存；底层兼容绑定、独立分类默认/catalog、完整 Agent 后续能力仍在 active Goal 中，不以本包代替全部目标验收。

## 10. 用户反馈修正：账户、统一模型目录与真正独立的分类默认值

本节接续 LocalFix3，不将旧包的验证结果套用于本轮新源码。用户明确要求恢复 FOFA 账户和密钥显示按钮、聊天与扫描模型同页管理，并修复截图中最近变化事件名与公司文字重叠。本轮交付目标是可直接使用的 LocalFix4。用户后续明确要求“做完当前的停下”：此包和必要验收结束后暂停 Goal；其他未完成项保留记录，不继续扩展。

### 10.1 当前实现及实际消费边界

- FOFA 账户 / Email 与 Key 同页；Key 可显示 / 隐藏。可选账户标识保存在独立 assetApis 版本的 account descriptor，测试和采集真实传入 email，服务端返回不同账户时拒绝。历史无邮箱、仅 Key 配置继续兼容；没有推断缺失账户。断点恢复将邮箱、当前 Key 与其引用一起刷新，避免上一账户的身份配下一账户凭据。错误不回显 Key 或代理凭据。
- 模型仅一个正常导航入口、一个连接目录和编辑器；每个 API 连接明确选择聊天、扫描用途。Rust models 目录保存连接，Host 保存当前聊天选择与精确 policy/revision/modelProfileId 引用；API 参数显式同步，无 Host 密钥导出、OAuth token 复制或 localStorage 秘密。OAuth 在同一目录中展示，其目前实际能力为聊天，Rust 扫描尚未支持 OAuth。参考 [OpenCode providers](https://opencode.ai/docs/providers/) 与 [models](https://opencode.ai/docs/models/) 将凭据连接和模型选择归到统一配置路径，但不套用别家未实现的能力。
- 保存分步骤确认 Rust 连接、扫描默认和 Host 聊天状态。Rust 已确认而 Host 失败时仅重试未完成步骤，保留同一请求编号；界面离开、重新加载后旧请求不得再开始下一步写入。重试前重新核对当前 Host 选择，不覆盖已经观察到的其他选择。Host 与 Rust 之间仍无跨进程原子事务，Host 两次 IPC 之间的瞬时并发仍有边界，不宣称原子绑定。
- Host 显式无鉴权连接不继承旧 Key、不发送 Bearer；相同地址下切换不同目录连接不能暗借原连接凭据。主动选择 API 连接或停用聊天会取消旧 OAuth 登录，迟到的 device-code / token 返回不能覆盖新选择。OAuth 仍只用于官方端点。
- 七个其他分类拥有各自目录、不可变版本、另存、默认值和显式历史导入。默认选择有 generation CAS，避免切走再切回的 ABA；操作编号只记录确认状态，秘密不写浏览器存储。旧总方案不再决定已独立分类的默认值。尚未选择独立默认的分类只读回退实际历史值，未自动迁移真实数据。
- 普通资产新任务明确发送 `configurationSource=current-defaults`、`profileId=0`，数据库 `profile_id=NULL` 并写入可验证来源标记；只读取 assetApis/collection/probe/rules/network 五类。预览读取相同五类且不返回秘密。旧 SDK 缺省来源仍按显式历史 profile 处理；含糊 NULL 历史任务拒绝自动重映射。恢复保留采集、探测和内容规则快照，只刷新运行时账户与网络及对应引用。
- 默认模型解析只读取 models；Nest fresh attempt 只冻结 agent 分类。损坏的无关类别不阻断这些消费者。扫描其他历史参数仍保留兼容入口，尚未宣称所有旧参数均完成逐项归属和废弃。
- 最近变化将 `page_rule_isolated/restored` 等事件显示为可读标签，未知类型有短标签；徽标、资产文本、时间各占独立网格单元，长文本有边界，修复截图的重叠。
- Phase 4 补齐持久化只读调查 / 审查子任务、原父运行绑定的等待 / 取消、父任务结束时回收与 token 撤销；报告仅作参考，不转成已确认事实。有限验收条件有独立工具重读路径和版本有效性，不能当成自动执行测试或全功能验收。修复挂起子任务只读调用导致父取消一直等待的问题；底层仍在执行的真实调用保持追踪。

### 10.2 验证记录与交付状态

源代码定向证据：FOFA 正式同步请求 / native collector 模块 105/105；分类目录与 CAS 39/39；消费者函数 harness 34/34；实际子任务消费者 17/17；Host 连接身份 / OAuth / 完整性 27/27。各组有重叠，不相加，不以模块 harness 代替 App。模型新页面包含切页后迟到请求负例，12/12，原失败脚本修后不再启动 provider/runtime 后续写入；vue-tsc 通过。

整合正在执行：规范临时目录下完整 Host + Assets 回归、统一 Cargo 和签名 App 实际桌面验证；最终结果在本节后续记录。首次 Cargo 发现新增测试漏导入 Uuid，已用完整命名空间修正；首次 Host 整套未指定规范临时目录而触发 macOS `/var/folders` 别名拒绝，已停止自有测试并按 `/private/tmp` 重跑，没有放松生产路径校验。

所有测试数据、合成账户和 loopback 服务仅在本轮私有临时目录；不读写真实用户库、CAS 或真实凭据。真实 FOFA / 付费模型 / OAuth 登录、真实数据升级、跨进程原子绑定、真正隔离的 `workspace_run`、完整 Agent 评估与全产品验收仍未完成。HackerOne 持续停用并隐藏，历史源码与数据保留。

### 10.3 Worker / Tailscale 卡顿阻塞（同一修复版本收尾）

用户反馈整个 App 缓慢并唤起 Tailscale 后，定位到 `worker.rs` 的同步状态探测：未启用 Worker 的设置读取也执行 `tailscale ip -4`，没有超时；macOS 候选和本机 CLI 包装器均可转入 Tailscale App，同一程序未显式设置 `TAILSCALE_BE_CLI=1` 可能选择 GUI。[Tailscale 官方说明](https://tailscale.com/docs/reference/tailscale-cli?tab=macos)。同步 Tauri 命令在界面主线程执行，因而无界等待可以阻塞界面。[Tauri 官方说明](https://v2.tauri.app/develop/calling-rust/)。仅假 CLI 已复现原探测阻塞；没有启动真实 Tailscale。此证据定位一条直接匹配现象的阻塞路径，不将其扩大为已经排除全部数据库或其他系统负载。

旧测试编译和旧 release 编译均已主动终止，不交付它们的中间产物。修复仅覆盖这条既有 Worker 阻塞：普通状态 / health 读取不再启动外部进程；显式启动强制 CLI 并有总 deadline；设置读写与启动探测移出主线程，监听地址取已运行服务的真实绑定，后台启动重验配置避免禁用后的迟到发布。最终只进行一次包含此修复的 release 构建，不再并行整 crate 测试编译或增加 Agent 功能。

本轮其他验证最终记录：前端冻结整合 154 项，153 passed / 1 H1 skip；Host + Assets 整合 600 项，581 passed / 17 platform skip / 2 过期测试契约失败（新增任务元数据与精确证据回执）。精确修正这两处测试后，对应两个文件 41/41 通过，没有放松授权、路径或 schema 断言，没有相应生产改动。统一 Cargo `--no-run` 因发现 Worker 阻塞而主动终止，不能称其测试通过；既有 production-function harness 与最终 release 编译、桌面验收分别记录。

### 10.4 LocalFix4 已交付，当前工作收尾

实际包：`/Users/swyiic/Desktop/Oviraptor-2026-10-18-LocalFix4-20261018T053414-157613ec/Oviraptor.app`；同目录 ZIP 28,334,868 bytes，SHA256 `5c1c618d7a323c4a4290d4873cbc1f68d12e0d34e00e1387b51a23003e7f01bc`。本轮唯一完成的 release 源码 digest：`f7cb128b5b028e4ae96fae300c79473de47da8416419529a073219a8a722e06b`。arm64、产品标识及版本保持真实 Oviraptor 1.1.66，97 个 Host 文件；签名、ZIP CRC、解压后签名与二进制 / Host 资源一致性通过。原 LocalFix3 未覆盖。

同一个实际 App 在 `/private/tmp/oviraptor-acceptance-unified-quit-us8t54jk` 验证：

- FOFA 合成账户 / Key 可输入，眼睛可显示与隐藏；保存后离页再进入仍存在并默认隐藏。
- 唯一模型页保存一个本地回环连接，同时显示聊天与扫描默认标识；实际两端精确引用同一 models policy/revision/modelProfileId。仅一次本机模型连接测试，无真实模型服务或 FOFA 请求；Host 明确 none 鉴权、没有 API Key/OAuth 借用。
- 临时库只新增 assetApis、models 两类独立默认，其他分类与 legacy 原凭据保持；FOFA 邮箱与 Key 同一账户，Host 引用与当前 models 默认相符。
- Worker 未启用的状态页实际打开并可切换离开；新 CLI 环境 / timeout / 孙进程清理另由 5/5 假进程回归验证，未启动真实 Tailscale。此项不是全部历史数据规模的性能验收。
- 合成事件在实际仪表盘最近变化和变化对比中可读，页面隔离标签、公司、说明与时间没有重叠。截图与 `ui-acceptance-receipt.json` 已放入交付目录。
- 启动及界面操作后的自有进程无真实用户数据句柄；实际 Cmd+Q 正常退出 code 0，自有 Host / 两个 worker 全部回收，独立 witness 经核对后单独清理。合成本机模型服务也已停止。启动前用户旧 App 已自行退出，不借此声称测试了其数据或性能。

用户要求“做完当前的停下”。本修复包与必要验收到此结束，暂停完整 Goal；不继续 Agent 扩展或其他待办。真实 FOFA/OAuth/付费模型、真实历史数据升级 / 大库性能、跨进程原子绑定、隔离执行沙箱与完整 Agent 产品验收仍未完成。

## 11. 2026-10-20：本轮单独授权的 FOFA 发现修复 / Hunter 定向补查

完整 Goal 保持暂停。本节仅记录用户后续明确要求的资产发现问题，不据此扩展 Agent 重构。

- 已定位证书漏报：`cert.domain` 查询结果被错误套用实际访问主机的域名边界，导致证书命中的外部 IP 丢失；修复为按证书域名 / CN 校验并保留待核验线索，不依赖 ASN，也不直接认领外部 IP。用户给定的两个 IP 均已做实际默认 TLS 证书核查；本地复现由旧逻辑输出 0 到新逻辑保留 2，相似假域名仍排除。
- FOFA 精度优先、相关性分数不累加、F 点不足停止、字段降级保留合法证书 / CNAME 观察；旧任务快照缺少精度开关时保留原查询计划，未改写 7 万条历史资产。
- Hunter 账户与预算独立，提供只读 userInfo 测试。按用户提供的官方接口文档实现 base64url、第 1 页、近 30 天以及域名 / 已观察证书 / 已观察 CNAME 三种定向查询；查询前预检账户与额度，持久化请求编号，结果未知不自动重放。沿用现有资产身份与多来源导入路径，证书 / CNAME 的外部 IP 保留为低分线索。
- 实际网站、携 Key 的 search 与 userInfo 当前都得到站点防护层 HTML 403；没有获得真实 Hunter JSON、账户权益或积分回执。用户确认同机需关闭 VPN 才可访问；FlClash 处于全局 + TUN 模式。仅准备 Hunter 直连规则草稿，路由变更需单独明确确认，未直接关闭用户 VPN。
- 当前定向模块回归 131/131、真实编译 SFC 回归 83/83、前端构建及 Rust tests 类型检查通过；模块 harness 的 jobs importer 是测试桩，不能将其计为真实 Hunter 入库或完整 App 验收。

详细实现、官方摘录依据、TLS 观察和未完成边界见 [HUNTER_FOFA_DISCOVERY_FIX_2026-10-20.md](HUNTER_FOFA_DISCOVERY_FIX_2026-10-20.md)。正式包与实际 App 检查另存交付回执，不以本节替代。

后续用户授权的Hunter独立HTTPS验证已通过：一次定向查询返回两个指定IP，使用2个免费积分、0权益积分；生产解析真实响应回放1/1。FlClash仍为global、附加规则0，未改变VPN。尚未验证App网络路径与全流程入库，详见专项记录文末。
