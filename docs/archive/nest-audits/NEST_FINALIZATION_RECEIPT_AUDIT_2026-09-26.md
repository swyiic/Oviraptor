# 编排终态、指令回执与失败结算审核（2026-09-26）

## 范围与实际缺陷

本次接续 `NEST_DIRECTIVE_CLOSURE_AUDIT_2026-09-26.md`，修复的是实际 Native 编排，不是新增主机权限或外部目标测试。

新增回归先在原实现上证明：

1. 真实 `run_agent_target` 启动时预算初始化失败，同时 Coordinator 终态写入失败，只返回启动原因，吞掉持久化失败。
2. `applied` 指令没有动作回执，coverage-only Reviewer 仍将其批量写成 `completed`。
3. Executor 费用结算失败后，旧清理调用把 assignment 标为 failed、child 标为 terminal，并把尚未结算的 48,000 token / 18 request 预留清零。此数值仅为本地测试夹具，不代表真实用量。

前两项失败日志：`/tmp/oviraptor-20260926-finalize-red.log`；第三项：`/tmp/oviraptor-20260926-settlement-red.log`。

## 已实施

### 统一结束路径并保留原始失败

`commands/agent_backend.rs` 的启动清理、Executor 结算失败、执行失败/取消、不兼容恢复、Authorization 失败和 Reviewer 返回，统一经 `finalize_agent_target` 收口。

- 数据库打开或 `finish_coordinator_run` 失败，返回明确的 `persistence_failure`，不靠错误文案推断类型。
- 结果同时保留原始 terminal code、原始原因和收口错误；Executor 结算失败时还保留此前执行结果。
- 错误经脱敏后记录到 runner log。数据库事务失败时不声称 root 已经终止；root 与指令收口依然原子回滚。
- 启动清理无法取得 run/数据库/lease 时同样显式返回错误，不再通过 `if let` 静默忽略。
- 本次未改变 lease 接管协议；启动失败后的 lease 恢复仍使用现有 Coordinator 机制，不能据此宣称所有崩溃恢复/过期租约场景已经验收。

### 不再用整体任务结果代替人工动作回执

删除 `finish_applied_directives` 函数及全部生产调用。Reviewer 首次执行、已冻结判定重放和 coverage-only 分支均不再批量结束人工指令。

- 队列动作继续由自己的原子动作回执完成。
- 只读提案继续由 assignment、模型响应、费用结算、结果 mailbox 与确认回执完成。
- 其他未解决指令统一交给 root 结束事务；无动作回执的 `applied` 变为 `deferred / directive_task_ended_execution_receipt_missing`，不冒充成功，也不假定动作已经执行失败。
- 既有并发、暂停、旧 fencing 测试改为检查真正的 Coordinator 终态入口，不保留只为旧批量函数服务的测试路径。

### 结算失败保留预留，已结算清理仍可释放 lane

新增 `stop_failed_child_preserving_usage`，用于 Executor 结算失败和 Authorization 失败清理。它在 IMMEDIATE 事务中复验当前 lease、assignment/child/root/scan/attempt/target 绑定。

- 尚未结算：assignment 与 child 置为 paused，标记 `child_usage_reconciliation_required`，撤销该 child 的 capability；保留原预留与 lane，不创建零费用回执，不自动重派发。
- 已经结算：沿用正常 child 失败清理，后续 mailbox 失败不再永久占用 lane。
- 撤销能力或其他写入失败：整个暂停事务回滚，调用方返回清理错误以及原始错误，不隐藏半完成状态。
- 旧 fencing：拒绝修改新持有者状态。
- 暂停只表示等待核对，不表示已完成费用恢复。后续本地对账入口仍需单独交付。

## 验证覆盖

本次新增 6 项测试，累计终态收口测试 18 项：

- 启动与 root 写入双重故障，真实编排同时返回两种原因。
- 模型认证失败与指令收口失败，root/指令事务回滚，原模型原因保留。
- 模型失败、Executor 结算失败、指令收口失败叠加，三种原因均保留，预留不释放。
- Reviewer 整体成功而指令无动作回执，不产生虚假完成。
- Executor 结算失败保持预留与 lane，撤销权限，重复暂停幂等，root 结束不退还未结算预留。
- 清理写入故障整体回滚，旧 fencing 拒绝。

模型服务与网站均为本地回环测试夹具；没有访问外部授权 URL，没有部署或执行主机操作。

最终验证（均针对本次最终代码）：

- Rust 全量：**719 项主库 + 30 项导入测试通过**，0 failed；日志 `/tmp/oviraptor-20260926-finalize-final.log`。
- 严格 Clippy（`--all-targets --all-features -- -D warnings`）通过；日志 `/tmp/oviraptor-20260926-finalize-clippy.log`。
- `cargo fmt --all -- --check` 与 `git diff --check` 通过。
- 前端真实 SFC SSR：**32 项通过**；日志 `/tmp/oviraptor-20260926-finalize-ui.log`。
- 生产构建通过；主 JS chunk **775.26 kB**，原体积告警仍在；日志 `/tmp/oviraptor-20260926-finalize-build.log`。
- Native 浏览器本地回环：**8 次请求**，匿名/对照采集完成、身份隔离通过；日志 `/tmp/oviraptor-20260926-finalize-native.log`。

本次未修改旧后端残留白名单；全量的残留扫描/历史兼容测试通过。临时日志不是安装包验收或真实环境验收的替代品。中间 717 项全量与 68 项定向结果不作为最终代码的测试数量。

## 仍未完成

1. 已持久化模型响应的终止后本地回执/结算补齐；未知响应与旧 fencing 的人工对账。
2. 本次暂停的未结算 child 的对账 API/UI，以及数据库写入失败时可操作的修复入口。
3. 已确认/未确认、scan 级未绑定指令的完整生命周期。
4. Reviewer 新证据合同签发、补证与重评闭环，真正独立专家扩展。
5. 沙箱与工具供应、真实授权 URL、批准镜像及跨平台安装包验收。

全局目标仍未完成，不能将本次修复称为整个 Master Plan 交付。
