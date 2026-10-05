# 源码 CI 单任务发布策略审计

状态：已实现单任务发布阈值的持久冻结和执行接入；最终顺序流水线 session 12044 退出 0，Rust 主库 1027 项／历史导入器 30 项、严格 Clippy 和 fmt 全部通过。实际 SFC 146 项、前端构建和 localhost 浏览器回环也已通过。此处是本轮修复的质量门禁结果，不是整个 Master Plan 完成声明。

## 1. 实际缺陷与本次范围

工作台已有 `maxCritical`、`maxHigh`、`blockRelease` 控件，启动和重试也会保存这些字段。但源码流水线完成时调用 `ci_gate_policy` 读取可变全局 `ciGate`，其语义是合计 blocking 数量，而不是独立危急／高危阈值。用户设置因此未进入实际 gate；运行中更改全局设置还能改变判定。

本次修复这条链路，不把“CI 发布门禁”与执行授权、目标保护、资源预算、沙箱或主机边界混为一谈。关闭超阈值阻断只影响已确认风险的发布策略，不绕过执行安全控制，也不把基础设施失败或覆盖不足改成通过。

## 2. 实现合同

1. `source_ci_policies` 按 `scan_id + attempt_number` 保存危急／高危阈值和发布开关。字段类型、0～10000 范围、当前 CI attempt、唯一性和外键由数据库约束；禁止更新、替换以及在父 attempt 存在时单独删除。正常任务／attempt 删除仍可级联清理。
2. 工作台发布事务在 attempt 建立后保存策略，在事务末尾再次核验。写入报错、被 `IGNORE`、值不符或缺失都回滚发布，不留下可启动的半任务。
3. CI worker 在快照／分析前加载本 attempt 策略；每个 analyzer 前后和最终 gate 投影事务内复核。来源是发布时记录，不是完成时的 settings、可变 context、聊天或历史 JSON。
4. 未发布策略的历史 CI attempt 明确拒绝直接执行，要求按正常启动流程创建新 attempt。没有为旧任务推断或补造许可；历史查看／导入不受此表授权。
5. `critical_count > maxCritical` **或** `high_count > maxHigh` 才超限，两类阈值不相加；恰等于阈值不阻断。只有已确认的 Reviewer 决定参与计数，Rejected／Insufficient 不因严重度标签阻断。
6. 开关开启且超限：`blocked`。未超限但有确认的 Critical／High：保留 `warning`。关闭阻断且有这些风险：同样保留 `warning`，不是抹掉风险。覆盖不足仍为 `coverage_incomplete`，基础设施故障仍为 `infra_failed`。
7. gate 的 JSON 携带 `policySource=workbench_task` 和完整 `releaseLimits`。旧 standalone GatePolicy API 保留其原有合计阈值语义；生产工作台 CI 不再读取它作为默认回退。
8. 新 attempt 独立冻结自己的控制值，旧 attempt 的值不被重写；不重新开放旧执行权。

主要实现：`commands/native_source_ci_policy.rs`、`commands/native_source_scan.rs`、`commands/workbench_startup.rs`、`native_pipeline/ci.rs`、`db_schema.rs`。没有新增主机执行能力，没有部署或访问外部测试 URL。

## 3. 测试与失败记录

新增 8 项测试：6 项实际工作台发布／源码入口／SQLite 回归，2 项基于数据库不可变 ReviewDecision 的 gate 判定回归。覆盖原始控制值、运行中 context／global 变化、独立阈值、等于阈值、关闭阻断、非 confirmed 排除、覆盖与基础设施失败、事务回滚、非法值、不可变性、独立重试、级联删除、缺失／损坏策略以及分析途中策略失效。

开发证据：

- `/tmp/oviraptor-source-ci-policy-red.log`：首次 2 项测试因 CI 夹具误带灰盒认证而失败；不是有效的缺陷红测，后续修正夹具。
- `/tmp/oviraptor-source-ci-policy-red-v2.log`：修正后 2 项按预期失败，分别为策略表缺失与执行报告未使用任务控制值，实际复现缺陷。
- `/tmp/oviraptor-source-ci-policy-targeted.log`、`...-targeted-v2.log`：新增 8 项全部通过。
- `/tmp/oviraptor-source-ci-policy-source.log`：48 通过／1 失败；旧 CLI 隔离测试的 CI 夹具没有新要求的 attempt／策略。补齐前置记录，不删除执行或隔离断言。
- `/tmp/oviraptor-source-ci-policy-source-v2.log`：源码相关主库 49 项／导入器相关 2 项通过。
- `/tmp/oviraptor-source-ci-policy-workbench.log`：工作台 40 项通过。
- `/tmp/oviraptor-source-ci-policy-retirement.log`：3 通过／1 失败，指出新增 CI 测试改变了已有审核文件的上下文摘要。
- `/tmp/oviraptor-source-ci-policy-retirement-v2.log`：4 项通过。
- `/tmp/oviraptor-source-ci-policy-clippy.log`、`...-fmt.log`：严格 Clippy 与格式检查通过；顺序 session 64060 退出 0。

REM-012 没有扩大文件范围或新增兼容入口。只更新三个已审核 fixture 的证据摘要，并分别验证“删除本次新增内容后，完整文件 hash 恢复为先前值”：

- `agent_tests_backend_residual.rs`：仅增加 5 行 CI attempt／策略准备；旧 `5f42439c…` → 新 `55571451…`，17 次字面量不变，PATH 正对照、真实四分支执行及完成回执断言均保留。
- `tests_native_ci.rs`：仅增加 2 个策略测试；旧 `944bafa4…` → 新 `0b1943bc…`，唯一历史导入字面量和原断言不变。
- `tests_native_routing.rs`：首轮完整回归后补齐 CI 路由夹具的 attempt／策略准备；旧 `3076b497…` → 新 `ae72a9c7…`，4 次历史字面量不变，原 `inconclusive`／退出码 3 和拒绝旧后端的断言不变。

不是自动重生成 allowlist 或用改白名单代替源代码审核。

## 4. 完整验证

首轮完整 Rust：session 16020 退出 101，`/tmp/oviraptor-source-ci-policy-full.log` 主库 **1026 通过／1 失败**，449.47 秒。唯一失败为 `stage_4_ci_scans_carry_the_gate_and_the_freeze_into_the_scan_record` 的旧夹具缺少冻结策略；后续导入器／Clippy／fmt 未执行。确认进程终止后补齐上述夹具，不放宽生产缺失策略拒绝规则。

最终重跑：session 12044 **退出 0**，顺序 Stage 4 → 残留守卫 → 完整 Rust → 严格 Clippy → fmt；启动至退出期间冻结 Rust，不并发编译／编辑可执行输入。实际结果：

- `/tmp/oviraptor-source-ci-policy-routing.log`：Stage 4 路由 **3 项通过**。
- `/tmp/oviraptor-source-ci-policy-retirement-final.log`：残留守卫 **4 项通过**。
- `/tmp/oviraptor-source-ci-policy-full-v2.log`：主库 **1027 项通过、0 失败**，429.60 秒；历史导入器 **30 项通过、0 失败**，2.00 秒。
- `/tmp/oviraptor-source-ci-policy-final-clippy.log`：全目标／全特性 `cargo clippy -- -D warnings` 成功结束。
- `/tmp/oviraptor-source-ci-policy-final-fmt.log`：全 workspace `cargo fmt --all --check` 通过，无输出。
- 终态后执行 `git diff --check`，退出 0。

实际 SFC → 前端 build → localhost 浏览器回环：session 80872 **退出 0**，日志 `/tmp/oviraptor-source-ci-policy-ui.log`、`...-build.log`、`...-loopback.log`。实际 SFC **146 项通过**，vue-tsc／Vite 通过；主 JS 828.82 kB 的原有拆包警告仍在。本地回环 `passed:true`，匿名与对照采集均 complete，observedRequests=8，isolatedIdentity=true。分析器测试使用本地 fixture SARIF，不是已部署真实目标和生产 analyzer 镜像的证明。

以上为本次修改后的实际终态，不沿用上轮 1019／30 或首轮 1026／1 替代最终结果。首轮失败日志保留，夹具修复没有删除或放宽原断言。

## 5. 尚未完成，不能据此声称交付的部分

- 源码 `scopeMode=full/diff/auto` 仍未与实际分析输入集合绑定；CI freeze 仍使用旧的 `CiScope::Diff` 调用。必须保留整仓来源完整性核验，同时实现实际分析 manifest／材料化范围，不能仅改 JSON 标签；CodeQL 的项目上下文需求也不能变成显式 diff 的隐式扩权。
- scanMode／人工限制、工具／规则配置仍未形成完整的源码冻结派发合同；本次只冻结发布阈值，不冻结所有执行设置。
- CI 目前沿用源码入口把 scan ID 交给 review 查询的旧逻辑；独立 source Reviewer 的真实 root／attempt 绑定、当前候选 revision 的资格与多决定去重需单独修复和验收。本次 gate 单元测试证明判定器对已提供决定的阈值语义，不是“源码发现→真实独立 Reviewer→正确 CI 结论”的端到端证明。
- 新策略表不是模型可写记忆，也不赋予旧 JSON、聊天或 skills 执行权限；完整工作台派发绑定、在途撤权、通用恢复、其余专家与真实环境验收仍在主计划范围内。
- 本次没有新增 UI 控件；接通现有控件至后端策略／报告，不代表任务详情、资产学习及全部聊天 UI 重组已经完成。
