# 源码裁决消费、Finding 投影与 CI 原子收口审计

更新：2026-09-27。本文记录当前实现，不替代整个 Master Plan 的最终验收。

## 当前结论

v3 源码正式裁决已接入类型化只读消费者、已确认 Finding 后端投影和当前 attempt 的 CI 收口。不是读取 `verdict` 字段后直接计数，也没有把源码哈希改造成 Web 的数字 revision。

**总体覆盖审查、统一 Findings UI/读取接口与源码导出、顶层崩溃恢复仍未完成。** `source_coverage_review` 继续存在；候选全部审查完毕不等于扫描充分，不得仅为绿色结果删除该缺口。

## 消费边界

实现：`src-tauri/src/agent_runtime/multi_agent/source_decisions.rs`。

- `SourceDecisionSet` 字段私有，不提供 Deserialize 或任意 JSON 构造入口；只有 `read_audited` 能生成可供 CI 消费的类型化结果。
- 必须在调用者事务中读取。消费者重新核验四个前置阶段、冻结材料、独立 Reviewer、真实模型请求/响应、消息交付与 ACK，以及正式裁决完整行集。
- 必须是 v3 合同。历史 v1/v2 不升级、不补写裁决，也不能从历史 JSON 获得执行权限。
- 决定绑定 scan/attempt/root、类型化候选身份和精确材料摘要。`analyzer_revision` 与 `graph_candidate` 保持不同身份合同。
- 仅 `confirmed` 产生 Finding 投影；`rejected` 和 `insufficient_evidence` 不产生确认漏洞。展示位置来自已经审核绑定的冻结候选，Reviewer 严重性与理由来自正式决定。
- 查询旧 attempt 允许只读审计，但缺失冻结文件仍失败；读取不修复数据、不授权重启、不使旧结果成为新 attempt 的 CI 资格。

## CI 与根收口

实现：`native_pipeline/source_ci.rs`、`commands/native_source_completion.rs`、`commands/native_source_coordinator.rs`。

1. 源码 CI 接受类型化结果后再次审计，拒绝已经失效的缓存集合。
2. freeze 由该 attempt 的封存分析结果、快照及实际分析视图重建，含 analyzer 版本、rule-pack 摘要、full/diff 范围和文件数，不信任可变报告。
3. 实际写入入口通过 `load_workbench_ci_policy` 读取当前活动 attempt 的不可变策略。历史审计不绕过这层执行准入。
4. 与原 Web CI 仅共享风险计数后的状态判定逻辑，不共享候选身份或裁决表。
5. 高危/危急阈值和阻断开关保持原语义。只移除已完成的 `source_review_not_completed`；保留 analyzer 缺口，增加 `source_coverage_review`，证据不足时增加 `source_candidate_evidence_incomplete`。
6. CI 状态、原因、根终态及 `TerminalReduced` 事件在同一事务内写入。写后重新核验完整证明、策略、材料、活动 attempt、模型配置以及数据库中的 gate 内容；静默忽略写入、触发器污染或撤权导致整体回滚。
7. `independentCandidateReviewCompleted=true` 与 `independentReviewCompleted=false` 同时保留。当前生产不能宣称总体覆盖已获独立审查。

## 报告与历史兼容

`merge_native_source_assessments` 只合并已提交的生产结果，不创建权限或裁决。

- 暴露 `sourceDecisionProjection`、`sourceFindings` 和完整 `sourceMultiAgent`。
- 原 analyzer 的 gate/gaps 分别保留为 `analyzerGate` / `analyzerGaps`；最终展示使用已提交的 reviewed gate。
- 原始 `NativeSourcePlan` 不改写。
- `sourceClaims` 仍是灰盒线索，不被替换为确认漏洞。
- 根事件保留精确投影与 gate，供历史追溯；这不代表统一 Findings 页、读 API、SARIF 往返或所有导出消费者已经接入。

## 已验证证据及范围

所有模型调用均使用 localhost 夹具，没有外部扫描、授权 URL 探测、部署或主机操作。

| 验证 | 实际结果 | 日志 |
| --- | --- | --- |
| 初版类型化消费者 Reviewer 回归 | 15 通过；早于 CI 集成 | `/tmp/oviraptor-source-decision-consumer-reviewer.log` |
| 初版 reviewed CI Reviewer 回归 | 17 通过；早于图候选/历史/报告扩展 | `/tmp/oviraptor-source-reviewed-ci-reviewer.log` |
| 扩展后的 `source_` 回归 | 主库 192、历史导入器相关 2 通过；主库 247.57 秒 | `/tmp/oviraptor-source-reviewed-ci-regression.log` |
| `ci_` 分类与策略回归 | 26 通过，30.03 秒 | `/tmp/oviraptor-reviewed-source-ci-regression.log` |
| 扩充后的裁决损坏/缓存失效回归 | 1 通过，7.76 秒；包括逐项篡改及发布后额外行 | `/tmp/oviraptor-source-consumer-stale-set-regression.log` |
| 残留模式、语法夹具和空基线 | 3 通过，0.78 秒 | `/tmp/oviraptor-residual-syntax-regression.log` |
| 旧 CLI PATH 陷阱 | 1 通过，1.20 秒；实际 Web/Code/Greybox/CI 分支 | `/tmp/oviraptor-retired-cli-trap-regression.log` |
| Native 与历史导入网络哨兵 | 1 通过，1.03 秒；正向对照仅连接本地代理 | `/tmp/oviraptor-retirement-egress-regression.log` |
| 当前严格 Clippy | `--features import-tools --all-targets -j 1 -- -D warnings`，exit 0，25.19 秒 | `/tmp/oviraptor-source-consumer-residual-clippy.log` |

上述源码回归包括：analyzer 与图候选确认/拒绝/证据不足、类型化身份、精确 attempt、真实终态事件、原计划/freeze 不变、报告合并、旧 v2 不补资格、历史读取与执行分离、30 种前置证明破坏，以及 5 类 gate 事务写入故障。失败路径不重放模型，不虚构退款。

缓存集合失效回归先读取合法 opaque set，再在同一事务中破坏正式记录，要求新的读取和持有旧集合的 CI 评估均拒绝且不修复；额外行测试不依赖解除 UPDATE/DELETE 保护。扩充发生在 192/2 回归之后，已单独运行该修改测试，不能把 192/2 描述为包含这个扩充。fmt 检查和 `git diff --check` 通过。全工程 Rust、UI/浏览器和安装包本轮尚未重新验收。

## CPU 与测试调度

Rust 命令串行，以 `nice -n 15`、Cargo `-j 1`、测试 `--test-threads=1` 执行；这不是 CPU 硬限制。上述 192/2 回归的主测试进程每两秒采样，共 123 个样本，观测最大值 100%；日志 `/tmp/oviraptor-source-reviewed-ci-cpu.log`。未覆盖编译、后代进程和采样间峰值，不能据此解释或否认用户先前观察到的 360%。该测试、Cargo 与采样器均已正常退出。

## 继续实施顺序

1. 保持本消费者为统一 Findings 读取/历史/API/导出的资格来源；禁止消费者从任意 JSON、角色标签或原始 analyzer 输出授予确认漏洞资格。
2. 设计独立总体覆盖审查合同并接入真实执行、审查、消息和收口，不复用候选审查完成来冒充覆盖完成。
3. 分类顶层恢复状态：未派发、在途且结果未知、已收到未交付、已交付未收口。只有已保存响应可做本地交付恢复；不得自动重发未知模型请求、退款或恢复已撤销权限。
4. 大候选集合、Reviewer 在途取消/截止期/配置漂移、全 UI/导出及所有 Master Plan 条款仍需分别验收。

## 同期 Strix 残留守卫修复

静态守卫此前漏检带引号的进程启动表达式，测试 PATH 正向对照也真的启动过名为旧 CLI 的假程序。现改用 `oviraptor-path-control` 做正向启动对照；旧名称只保留可发现的哨兵程序，任何启动均令测试失败，四个真实分支继续检查哨兵未被触发。

检测器增加整文件正则，覆盖带引号、大小写、命名空间、换行、raw string 与 `.exe` 写法；独立 JSON 语法夹具只作为待扫描文本，不执行其中代码。所有残留 baseline 继续为空，未扩张豁免。语法与空基线 3 项回归通过：`/tmp/oviraptor-residual-syntax-regression.log`。这仍是静态模式守卫加行为陷阱，不是能够证明任意动态程序名安全的完整 Rust 语义分析器；最终仍需依照 Master Plan 逐条解释历史兼容引用。

Master Plan 第 14 节启动/路由模式命令在当前活动目录中的匹配只剩 `tests_native_code.rs` 与 `tests_native_greybox.rs` 两处检查禁用词的测试文本，已人工确认不是启动调用。没有通过拼接字符串隐藏旧 CLI 调用。全量大小写无关的历史名称搜索仍有其他兼容/测试引用，此处不把两条搜索结果或空活动基线冒充全部历史引用的最终分类验收。
