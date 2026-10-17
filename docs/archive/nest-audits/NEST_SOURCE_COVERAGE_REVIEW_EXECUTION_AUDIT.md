# 源码总体覆盖 Reviewer 与消费链审计

状态：v4 实现与专项验收增量；本地新任务默认注册已切到 v4，新增默认入口与授权恢复验收进行中。本文不宣布 Master Plan、修改后完整质量门禁或安装包上线完成。

## 1. 实际执行链与版本边界

v4 使用独立的总体覆盖审查 subject，不把候选审查冒充总体覆盖，也不把覆盖准备索引冒充模型交付：

| 阶段 | subject / phase | assignment 触发条件 | 发布内容 |
| --- | --- | --- | --- |
| 候选复核 | `source_candidates` / `source_review` | `source_candidates_ready` | 逐候选不可变裁决 |
| 总体覆盖复核 | `source_coverage` / `source_coverage_review` | `source_coverage_ready` | 当前冻结范围的不可变覆盖裁决 |

二者可使用同一种 `EvidenceReviewer` 角色，但 assignment、child run、请求/响应回执、mailbox、ACK 和发布记录必须独立。覆盖 Reviewer 不能是初始评估者、工具执行者或候选 Reviewer。角色名相同不等于同一执行身份；模型相同也不证明模型能力相互独立。

零候选仍执行总体覆盖复核，但不创建假的候选 Reviewer、不发布空候选裁决集。v4 请求预算显式冻结，v1–v3 继续沿用原合同；恢复旧任务不增发阶段或预算。普通生产注册入口 `commands/native_source_coordinator.rs` 现传入版本 4，但已有根仍优先恢复其冻结版本，而不是以新默认覆盖历史。

涉及模块：

- `agent_runtime/multi_agent/source_plan_contract.rs`：阶段与预算合同。
- `source_review_subject.rs`：两个审查 subject 的区别；不能通过 SQL 过滤隐藏未计划的 assignment。
- `source_coverage_contract.rs`：证据引用、独立身份、响应 schema 与不可抹除的确定性缺口。
- `source_coverage_reviewer.rs`、`commands/native_source_coverage_reviewer.rs`：真实派发、保存响应交付与恢复。
- `source_coverage_decisions.rs`、`db_schema.rs`：从已交付回执生成不可变发布；不接受外部报告作为裁决权威。

## 2. 恢复、未知效果与发布

已保存响应的恢复只继续本地交付，不重新请求模型。未知派发结果保留为需要核对的状态，不退款后自动重发、不伪造 ACK、不因“已有候选审查”而结案。

覆盖裁决发布必须与交付/ACK/费用结算共享事务。发布失败不抹掉已发生的模型费用。更新、替换、单独删除不可变裁决均拒绝；明确删除所属扫描时允许外键级联清理，不能留下无法归属的记录。

历史报告读取允许验证旧尝试，但不领取新 lease、不续期、不修复损坏数据。新尝试不能替换旧尝试的证据；旧 JSON/SARIF 回导只保留历史材料，不恢复 Native assignment、授权或裁决表。

## 3. 统一消费投影

新增 `source_review_projection.rs::SourceReviewProjection`，只允许在保留的读取事务中从真实账本构建；不提供从外部 JSON 反序列化成审查权威的入口。

- v4 同时核对真实覆盖裁决和适用的候选裁决。覆盖仍待交付、ACK 缺失、摘要损坏或发布缺失时，整个正式投影不可用。
- v4 零候选不构造 `SourceDecisionSet`；投影明确记录 `not_applicable_no_candidates` 与 `source_coverage` 材料主体。
- v1–v3 出现非计划内覆盖发布，或零候选任务出现非计划内候选发布，均拒绝，不静默忽略。
- `CoveragePreparation` 的历史字节和 `prepared_not_reviewed` 含义不变。已完成审查通过新的消费摘要表达，不能改写准备材料来获得完成标志。

接入点：

| 消费者 | 权威输入与行为 |
| --- | --- |
| CI | `native_pipeline/source_ci.rs` 重新审计整个投影，拒绝过时输入；风险门禁仍按冻结策略判定 |
| 结案 | `native_source_completion.rs` 核对候选和覆盖两部分；零候选 v4 也必须形成真实 CI gate |
| Findings / 概览 | `native_source_findings.rs` 共用投影；不能核验时撤销确认数，不把未知当零 |
| JSON / SARIF / bundle | 同一读取快照生成报告及覆盖来源；回导仍为历史 |
| 非 CI 报告合并 | 保留分析器阶段原缺口，使用真实覆盖裁决更新最终缺口，不重新加回阶段占位项 |
| UI | `SourceReviewEvidence.vue` 核对材料绑定与字段一致性，显示充分性、理由、证据与来源；损坏刷新后撤销确认展示和导出入口 |
| 聊天 | `AgentDialog.vue` 区分实际覆盖回执与候选回执，不把消息本身当作审查通过 |

必须分别展示：候选审查是否完成、总体审查是否完成、冻结范围覆盖是否充分、CI 是否通过。任何两者不能互相替代。没有发现不等于没有缺口，更不等于整个系统安全。

## 4. 本轮验收证据与快照区分

以下结果来自已收取终态的本机命令；测试 provider 是本机夹具，不是外部模型质量验收，也不是授权网站实扫。

| 验证 | 已收取结果 | 日志 |
| --- | --- | --- |
| 消费链接入后的 `--lib source_coverage_` | 21 passed / 0 failed，248.46 秒 | `/tmp/oviraptor-source-coverage-consumers-tests.log` |
| 新增待交付、非计划发布、重新初始化/级联边界后的 `--lib source_coverage_consumers_` | 5 passed / 0 failed，80.81 秒 | `/tmp/oviraptor-source-coverage-consumers-boundaries.log` |
| 最新消费边界，含充分覆盖与非 CI 合并断言 | 6 passed / 0 failed，84.33 秒，exit 0 | `/tmp/oviraptor-source-coverage-consumers-boundaries-v3.log` |
| 实际 SFC 全套 `npm run test:ui` | 201 passed / 0 failed | `/tmp/oviraptor-source-coverage-consumers-full-ui.log` |
| `npm run build` | exit 0；仍有大于 500 kB 的分包告警 | `/tmp/oviraptor-source-coverage-consumers-build.log` |
| `cargo clippy --all-targets --all-features -j 1 -- -D warnings` | exit 0，29.40 秒 | `/tmp/oviraptor-source-coverage-consumers-clippy.log` |
| `node tools/test_native_runtime.cjs` | exit 0；本机浏览器匿名采集、跨源阻断、身份隔离通过 | `/tmp/oviraptor-source-coverage-consumers-native-runtime.log` |

最新消费边界增加真实零缺口材料的充分覆盖/CI 通过路径，以及共享夹具内的非 CI 合并断言。充分覆盖用例使用实际的外置 Git 元数据仓库，先冻结真实材料再执行，不删除数据库中的 gap、不改生产排除规则；检查报告、CI、Findings 和导出一致且没有伪造候选裁决。首轮 5 passed / 1 failed：测试把内部快照误按外层报告读取 `review.coverage`；修正为真实快照的 `coverage` 并补查 `ciGate` 后，上表最新 6 项通过。失败日志保留在 `/tmp/oviraptor-source-coverage-consumers-boundaries-v2.log`，不是生产门禁被放宽。

消费链接入快照的全量命令 `nice -n 15 cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -j 1 -- --test-threads=1` 已收取 session `84418` 的 exit 0：主库 **1293 passed / 0 failed（1056.89 秒）**，历史导入器 **30 passed / 0 failed（2.27 秒）**，另一 target 为零测试。日志 `/tmp/oviraptor-source-coverage-v4-all-targets-all-features.log`。随后独立运行 `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`，exit 0。此前 255 项源码整组、1268 项主库属于更早快照。

上述全量编译早于新增 v4 授权矩阵、暂停恢复权限断言及默认入口验收修改；它证明消费链快照，不证明接下来的默认 v4 改动。新增验收覆盖覆盖 child 的取消/旧 attempt/过期/旧 fence/权限撤销/丢失 lane，拒绝路径要求所有数据库表不变且实际模型调用数不增加；暂停且已保存响应的正向恢复禁止任何新建或恢复 dispatch grant。默认入口先以新增断言复现 v3，再修改并单独验收，结果另记。

边界回归检查的不只是返回值，还包括读取前后完整数据库状态一致，以及实际本机模型请求次数不增加。恶意记录和损坏回执用事务内注入，随后回滚；不修改用户历史数据库。

### 4.1 默认 v4 启用增量（与上述消费链全量分开）

先在实际默认注册测试增加 schema 4、覆盖阶段和 10 次请求预算断言：旧入口实际返回 3，红测 **0 passed / 1 failed，exit 101**，不是编译失败（`/tmp/oviraptor-source-v4-default-red.log`）。随后只将生产新根默认注册切到 4，已有根仍从自身 plan 恢复；原 v3 权限测试显式固定 v3，不改其历史含义。

| 修改后验收 | 已收取结果 | 日志 |
| --- | --- | --- |
| 默认注册与历史版本兼容 `--all-features --lib source_coordinator_` | 10 passed，exit 0，14.56 秒 | `/tmp/oviraptor-source-v4-default-registration.log` |
| 实际生产入口、结案故障和派发 `--all-features --lib source_dispatch_` | 9 passed，exit 0，40.69 秒 | `/tmp/oviraptor-source-v4-default-dispatch.log` |
| 授权拒绝与暂停恢复 `--all-features --lib source_coverage_reviewer_reentry_` | 2 passed，exit 0，153.82 秒；含 16 种拒绝条件和 8 个恢复场景 | `/tmp/oviraptor-source-v4-authority-reentry.log` |
| claim 后权限漂移 `--all-features --lib source_coverage_reviewer_dispatch_rechecks_permissions_after_claim_without_http` | 1 passed，exit 0，7.33 秒；4 种事务内权限漂移 | `/tmp/oviraptor-source-v4-claim-authority.log` |
| 默认 v4 修改后的独立 fmt | exit 0 | `/tmp/oviraptor-source-v4-default-fmt.log` |
| 默认 v4 修改后的严格 all-targets/all-features Clippy | exit 0，13.27 秒 | `/tmp/oviraptor-source-v4-default-clippy.log` |

生产入口测试不提前注册测试专用根，直接调用 `run_native_source_scan`：新 plan v4、零候选、5 个真实 assignment/ACK、7 次模型请求/140 fixture token、覆盖裁决已发布，但缺工具的覆盖仍不充分且 CI 不通过。真实 trace 校验覆盖 Reviewer 身份、费用只计一次、终态后不重发和发送者损坏降级；结案故障逐项检查根事务回滚，而已交付覆盖裁决/费用保留。成功审查不被重新解释为成功扫描。

授权矩阵及暂停恢复 session `25462`、claim 写入后权限漂移 session `85461` 均已收取 exit 0。撤销 `review.write`、撤销 `evidence.read`、替换 fence 或注入额外 HTTP grant 均在真实 HTTP 派发前失败并回滚全部表。修改后独立 fmt 与严格 Clippy 已通过。默认 v4 快照完整 all-targets/all-features 回归 session `62699` 已收取 **exit 0：主库 1295、历史导入器 30**，主库 1182.61 秒，日志 `/tmp/oviraptor-source-v4-default-all-targets-all-features.log`。该进程编译后继续新增了聊天阅读状态的 Rust/数据库/前端代码；因此本次全量只证明默认 v4 快照，不能当成后续持久化增量的全量证据。后续验证见 `NEST_CHAT_ASYNC_AUDIT_2026-09-26.md`。

## 5. 仍须完成的验收与产品工作

1. 默认 v4 边界测试、严格 all-targets/all-features Clippy、fmt 与完整 Rust 全量已收取；后续聊天持久化修改仍须按 Master Plan §15 完成新快照的完整通用门禁，不能复用旧二进制结果。
2. 核对 v4 的授权撤销、过期、旧 fence、取消、不同恢复边界与消费者的一致性；未覆盖项不能用测试文件存在代替。
3. 本地默认入口已切 v4；继续验证完整注册、实际执行、聊天/费用/终态与 UI/导出的生产合同及旧任务版本兼容。单行切换不代表发布完成，当前安装 App 仍须独立核验。
4. 当前快照会把 `.git` 等被排除目录记录为缺口，既有测试也要求保留。不得在 Reviewer/CI 里偷偷删除该缺口来制造通过。若产品要区分“合同明确排除项”和“未完成覆盖”，需另做版本化范围合同与历史兼容设计；本增量不改变其含义。
5. 源码工具/辅助/深度三档、真实 WebView/IPC/安装包、规模化性能、用户指令与全模块恢复、Strix 退役及历史兼容的全套回归仍按总计划逐项验收。主机能力继续后置，不借本次源码增量启用执行器。

保持一个 Cargo 进程、`nice -n 15`、单编译 job 与单测试线程；这些措施不是硬 CPU 上限。未执行用户目标 URL、部署、主机采集或外部模型调用。
