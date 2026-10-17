# Source Reviewer 真实执行与聊天交付审计（2026-09-27）

## 当前结论

2026-09-27 后续：正式裁决现已通过重新核验的只读 API 接入任务中心证据页，支持历史轮次和确认发现分页，详情见 `NEST_SOURCE_FINDINGS_READ_AUDIT.md`。它不补发执行权限，不从历史 JSON 构造确认资格，不替代总体覆盖审查或其他 Findings/导出验收。下文保留此前执行链阶段的历史状态。

2026-09-27 最新恢复实现与验证记录见 `NEST_SOURCE_REVIEW_REENTRY_AUDIT.md`：已新增四阶段完成且原租约有效的 Reviewer 顶层恢复，覆盖五种持久边界和未知结果拒绝；不代表完整 crash recovery。主库全量 1187 通过 / 2 项登记失败，恢复测试与 claim 写后权限回归通过；两项登记修正后分别补测通过，完整导入器 30、严格 Clippy/fmt 通过。为控制测试负载未再次运行整轮全量，不声称单轮全绿。以下消费/执行结果保留为此前增量历史证据。

源码 Coordinator 已接入独立 EvidenceReviewer 的真实 assignment、child run、模型调用回执、费用结算、mailbox ACK 和追加式聊天事件。v3 正式裁决、共用四阶段证明与历史只读审计之后，最新增量已接入类型化裁决消费者、确认 Finding 后端投影和同 attempt 的原子 CI 收口。源码回归 **主库 192/历史导入器相关 2**、CI 分类与策略 **26** 项通过；精确代码/测试范围及后续验证见 `NEST_SOURCE_DECISION_CONSUMER_AUDIT.md`。本轮未重跑整个工程全量、UI 或浏览器，不能套用下方旧全量结果。

**整体 Master Plan 未完成。** 候选审查完成不等于覆盖完成，也不等于 CI 通过。analyzer 阶段仍保留 `evaluate_unreviewed` 的历史 gate；v3 根收口后才由审计过的正式裁决生成 reviewed 投影。统一 Findings UI/读 API/源码导出、总体覆盖审查和完整顶层恢复仍未完成；Reviewer 有效原租约边界的局部恢复见上述最新审计，不改写下文历史测试结果。

## 生产合同和边界

- 原 v1 根保持 `schemaVersion=1`、8 次模型请求上限，不重新注册为 v2，不补写新权限。
- 原 v2 根保持 `sourceToolsPhaseVersion=1`、`sourceReviewPhaseVersion=1`、9 次模型请求上限，不补写正式裁决。
- 新 v3 根另冻结 `sourceDecisionPhaseVersion=1`，模型请求上限仍为 9；不是再增加一次模型调用。注册已有 v1/v2/v3 根均保留原合同，旧版本携带新决策标志或新版本缺标志均拒绝。
- RepoMapper 与 SourceAnalyst 各有初评和源码工具阶段；独立 Reviewer 是第五个 assignment/child，使用 `review` lane。
- Reviewer 仅有 `evidence.read`、`review.write`。模型请求没有 tools；不授予 HTTP、浏览器、shell、主机或目标请求权限。
- 审查合同绑定本 attempt 的冻结分析视图、分析结果、计划、实际 analyzer 修订/来源、图候选/作者/修订与工具回执。
- 模型返回必须逐候选精确覆盖，使用真实候选身份、材料摘要和允许的证据引用。缺失/外来/重复候选、伪造引用、自审和不符格式的结果不发布。
- 模型返回 `confirmed` 不直接授予 Finding 资格；必须经完整正式裁决发布与消费审计才产生确认 Finding 投影，整体覆盖资格仍是独立合同。

生产调用链：`run_native_source_assessments` → 初评/工具阶段证明 → `source_reviewer::prepare` → 真实 specialist transport → `deliver_source_candidate_review` → 根收口复核。

## 交付与故障语义

1. 使用独立 `source_review_result` 消息类型，不把源码身份强行转换为 Web 图的数字候选身份。
2. 同一事务核验模型回执、冻结材料、授权、预算、实际发送者、收件人、唯一消息、交付和 ACK。
3. 终结 child 后再次证明用量、权限撤销和 lane 释放。
4. 追加式协作事件必须同时存在最新 completed assignment、terminal child 和已 ACK mailbox。触发器静默忽略聊天事件也导致事务失败，不允许“数据库完成但聊天消失”。
5. 根终态写入后再验证全部材料和交付证明；根写入触发器污染 Reviewer 结果时，整次根收口回滚。
6. 已收到模型响应但本地交付失败时，先暂停/撤权并保留实际回执和预算预留，然后仅重试一次本地交付。恢复不重新调用模型、不恢复工具权限、不虚构退款。
7. 持续失败保留已收到响应与未结算占用，不能标成成功；未知模型响应也不自动重发。

## v3 正式裁决增量

- 新表 `agent_source_review_decisions` 与 Web `agent_review_decisions` 隔离，保留 `analyzer_revision` / `graph_candidate` 类型身份。
- 确定性 ID 与记录摘要使用 SHA-256；记录绑定 scan/attempt/root、材料摘要、完整候选决定（含候选摘要）、真实 Reviewer run/assignment、已 ACK 消息及模型请求/响应哈希。
- 发布器自行重建真实交付证明，不接收调用方提供的“已审核”结构。逐候选写入、消息 ACK、用量结算、child 终结在同一个交付事务内完成；写后再次重建证明和核对完整行集。
- UPDATE / REPLACE / 单独 DELETE 由数据库触发器拒绝；任务删除可沿 root 外键级联。完整既有行集允许只读幂等重放，部分/冲突/多余/损坏行集拒绝修补；终态根不允许缺失行集重新发布。
- 公开交付审计要求 v3 行集与真实回执逐项完全相等。终态根 manifest 记录 `sourceDecisionIds`；旧 v2 不补写行集。
- 本节记录最初 v3 发布器边界；随后消费者及初评共用证明已接入，见最新消费审计。消费者走完整交付审计，不直接按表中 `verdict` 计数；总体覆盖审查和顶层恢复仍需完成。

上述第 6 项最初仅是**当前进程内的本地交付恢复**。后续已加入有效原租约下的 Reviewer 顶层重入，详见最新恢复审计；仍不是进程强杀/重启后的完整恢复，不能从任意未完成阶段恢复，不能据此宣称 crash recovery 完成。

## 聊天 UI

`AgentDialog.vue` 根据真实 `fromRole=evidence_reviewer` 渲染 Reviewer 消息，而不是创建假的对话。实际完整/增量聊天 API 回归核对正序列号、`fromRunId`、`fromRole`、消息类型、ACK 与摘要。

源码审查气泡明确提示：这是源码候选审查回执，不代表总体覆盖完成、漏洞已确认或 CI 通过。状态使用 `source_candidates_reviewed_coverage_open`，同时保留 `independentReviewCompleted=false`；仅 `independentCandidateReviewCompleted=true`。

## 同期修复

- 重新注册损坏/不受支持的根计划返回绑定或状态冲突，不能修补并覆盖旧根。
- 通用 scheduler 的 Reviewer 准入、Reviewer 交付审计与命令层收口现共用 `source_phases::audit`，完整核验四个角色/阶段的冻结 slice、回执、ACK、精确用量、绑定及撤权。任务结束或切换 attempt 后允许只读核验旧证据，但不能重启、续权或把旧决定用作新任务资格。详见 `NEST_SOURCE_PHASE_PROOF_AUDIT.md`。
- Source 心跳按角色续期两项确切权限：初评为 `evidence.read`/`mailbox.write`，Reviewer 为 `evidence.read`/`review.write`。
- 心跳在全部写入之后重新检查执行状态、实际调用、lane 和全部未撤销权限数量，并要求两个 run 的续期写入都发生；不只数预期权限。
- 故障注入先证明旧实现会接受写后新增的 `target.http` 权限，修复后额外权限、丢 lane、取消及被忽略的 run 更新均被拒绝并回滚。

## 验证证据

全部模型与浏览器测试均使用 localhost，没有访问用户授权 URL，没有部署或主机层操作。CPU 事故后的 Rust 验证使用 `nice -n 15`、`-j 1`、`--test-threads=1`，一次只执行一个 Cargo 命令；这不是 CPU 硬上限。

| 验证 | 实际结果 | 日志与范围 |
| --- | --- | --- |
| 共用阶段证明与历史审计（最新） | 主库 189/189、历史导入器相关 2/2 通过 | session 41212，`/tmp/oviraptor-shared-source-proof-regression.log`；`source_` 筛选，208.92 s / 0.22 s，不是全工程 |
| 共用证明严格 Clippy | exit 0 | session 26961，`/tmp/oviraptor-shared-source-proof-clippy.log`；新增两项真实红测及 CPU 采样记录见阶段证明审计 |
| v3 源码回归首轮 | 186 通过、1 失败 | `/tmp/oviraptor-source-decision-v3-regression.log`；失败为损坏测试同时改写全部身份，先撞唯一约束，未进入审计断言；不是通过结果 |
| v3 Reviewer 修正复测 | 12/12 通过 | session 64611，`/tmp/oviraptor-source-decision-v3-reviewer.log`；生产约束未放宽，改为单候选损坏并增加跨根/自审/回执/裁决污染及多余行测试 |
| v3 源码完整筛选回归 | 主库 187/187、历史导入器相关 2/2 通过 | session 75321，`/tmp/oviraptor-source-decision-v3-green.log`；197.13 s / 0.25 s。是 `source_` 筛选，不是整个工程全量 |
| v3 严格 Clippy | exit 0 | session 38588，`/tmp/oviraptor-source-decision-v3-clippy.log`；`--features import-tools --all-targets -j 1 -- -D warnings` |
| v3 fmt / 空白检查 | 通过 | 本轮测试退出后再次检查；没有残留测试/Cargo/rustc 进程 |
| source_ 回归 | 主库 182、导入器 2 通过 | `/tmp/oviraptor-source-reviewer-v2-regression.log`；早于聊天事件写后证明与心跳补强 |
| 最新 Reviewer 定向 | 7 项通过 | `/tmp/oviraptor-source-reviewer-timeline-v2.log`；真实模型、六类交付故障、根写后污染、本地恢复、完整/增量聊天 |
| 心跳回归首轮 | 1 失败，复现写后额外权限 | `/tmp/oviraptor-heartbeat-postcondition-red.log`；真实红测，不算通过 |
| 心跳修复定向 | 1 项通过，包含 10 类状态/故障分支 | `/tmp/oviraptor-heartbeat-postcondition-green.log` |
| 实际 UI | 155/155 通过 | `/tmp/oviraptor-reviewer-current-ui.log` |
| 前端构建 | 通过 | `/tmp/oviraptor-reviewer-current-build.log`；主 JS 831.95 kB 警告保留 |
| localhost 浏览器回环 | exit 0 | `/tmp/oviraptor-reviewer-current-browser.log` |
| 最新严格 Clippy | exit 0 | `/tmp/oviraptor-heartbeat-final-clippy.log` |
| fmt / 空白检查 | 通过 | 运行于最新 Rust 修改之后；文档结束前再次检查 |
| 前序完整 Rust（v2 基线） | exit 0；主库 1172/1172、导入器 30/30 | session 51972，`/tmp/oviraptor-reviewer-heartbeat-full.log`；分别 526.88 s、2.22 s，包含 v2 Reviewer 和心跳补强，不含本轮 v3 |

前序 v2 全量退出后再次通过 fmt/空白检查，未发现残留 `oviraptor_lib`、Cargo test/clippy 或 rustc。运行中两次线程采样为 4，后续 CPU 抽查见 66.7%、67.6%、99.3% 和 100%；这些离散采样不代表完整峰值监测，不承诺 CPU 硬上限。

本轮 v3 编译、测试与 Clippy 顺序执行。最终源码筛选回归运行中一次线程采样为 3；末次进程 CPU 抽查 45.5%，不是峰值。session 75321 已退出 0，退出后无残留测试/Cargo/rustc，未启动第二个并行编译或测试。

真实失败历史还包括：根冲突错误码不一致、测试直接修改不可变模型回执被生产触发器拒绝。后者已调整为先证明生产保护，再在回滚事务中移除触发器模拟存储损坏；没有为了测试放宽生产不可变规则。

## 继续执行顺序与验收要求

1. **正式源码决策存储/发布。** v3 已实现并通过上述定向回归、数据库重开及删除级联验证。整个工程全量仍待后续集成验收；不能借用旧 v2 的全量结果宣称新代码整体全绿。
2. **统一源码收口证明（已实现并通过上述筛选回归）。** 四阶段证明已提取到授权无副作用的模块，覆盖 mailbox ACK、精确用量、绑定、fence、冻结 slice 和已撤销权限；历史只读材料与执行恢复分开。30 种阶段损坏在准入/交付两个入口被拒绝，历史完成/旧 attempt/原仓库移动均可核验，冻结文件丢失或跨 attempt 则拒绝；公共 scheduler 和 Reviewer prepare 不能据此重启。总体覆盖证明和消费者资格不能仅由此推断，下一步仍是第 3 项。
3. **Finding 与 CI 消费。** 只投影来自上述正式记录且仍匹配本次冻结输入的决定。测试 confirmed/rejected/insufficient_evidence、修订变更、缺候选、孤立记录、自审、旧 attempt、历史导入影响。覆盖缺口独立保留，不能只删 `source_review_not_completed` 来换取绿色。

   当前真实时序须保留：`native_source_scan.rs::run_native_source_scan_using` 先完成 analyzer/导入、冻结计划并持久化未审查 CI 结果，外层 `run_native_source_scan` 随后才执行多智能体并将结果放入 `sourceMultiAgent`。因此正式 CI 消费必须新增审查完成后的、绑定同一 attempt/冻结输入的事务投影，不能只替换 analyzer 阶段的函数名，也不能改写已经冻结的旧计划来消除 gap。`ci::evaluate` 目前仅供测试且使用 Web `ReviewDecision`，不能解除 `cfg(test)` 就当作源码资格验证。
4. **顶层恢复。** 分类持久状态：未派发、调用在途/结果未知、已收到响应未交付、已交付未收口。已收回执仅本地交付；未知结果不自动重发。新增进程边界回归，证明不会重复调用/退款/重复记账；终态根不能被恢复覆盖。
5. **大候选集与 Reviewer 运行控制。** 当前模型输出上限 2048，尚无分批审查合同；必须以新冻结合同约束候选批次、费用与时限，不能无限追加调用。补 Reviewer 专属在途取消、deadline、运行配置漂移测试。
6. **全 Master Plan 核验。** Strix 精确残留白名单、人工指令完整闭环、所有计划角色、历史分页、部署/跨平台、沙箱/离线工具供应等仍按原范围验收，不能用本页的源码 Reviewer 进展替代全项目完成。

## Strix 静态复核提示

历史复核曾发现 `commands/agent_tests_backend_residual.rs` 中 `Command::new("strix")` 是隔离测试子进程的 fake CLI 阳性对照。后续已改为中性 `oviraptor-path-control`，旧名陷阱只允许被发现，连阳性对照也不得启动；补充带引号/多行等启动语法守卫，详见消费审计。本轮完整回归又检出该文件旧全文摘要未更新，重新审阅其 16 处旧名称后仅更新对应 fixture 登记，详见恢复审计。不能把此单文件维护当成所有历史引用的最终分类验收。

其余已检查位置包括历史 JSON 适配、旧目录只读发现、数据库迁移、旧 backend 拒绝、历史来源标签与旧知识包 schema 兼容。部分代码路径不等于主计划列举的精确白名单路径，**本次检查不足以宣布 Strix 全项最终验收通过**。不得为清空搜索结果而破坏旧 JSON 兼容或删除真实历史来源。
