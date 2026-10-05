# 多智能体子任务完成与结果投递原子性审计（2026-09-26）

## 1. 范围与结论

本增量修复公共 Scheduler 子任务收口，以及 WebExecutor／Authorization 的结果投递。它直接影响真实 assignment、child run、mailbox 与聊天时间线的一致性，不是仅修改 UI 文案。

不新增目标请求、模型重试、工具能力或主机权限。仍为 `web_only`，没有新增 Host Agent 或主机审批后端。测试使用临时数据库与 localhost 夹具，没有部署或访问用户外部目标。Master Plan 及原目标仍未完成。

## 2. 复现证据

1. 原公共 `finish_child_in_transaction` 主要检查 assignment 更新，未完整验证 run 终态、能力撤销与 lane 释放；SQLite `RAISE(IGNORE)`、少写、后续触发器改变状态时，可能错误返回成功。初始故障回归为 1 通过／4 失败，见 `/tmp/oviraptor-20260926-child-completion-red.log`。
2. 原 Executor／Authorization 的 mailbox 发送、确认与任务收口并非一个原子交付单元，收口失败可能留下正常结果消息。
3. 中间 Executor 修订虽然包入事务，却复用了自行开启事务的 `deliver_expected`。新增正常完成回归明确复现 `cannot start a transaction within a transaction`，见 `/tmp/oviraptor-20260926-child-completion-positive-red.log`。仅测试失败路径不能证明修复成功，该中间版本不作为验收结果。

## 3. 公共子任务完成合同

`src-tauri/src/agent_runtime/multi_agent/scheduler.rs` 中的完成入口现在：

- 核对 child handle、assignment、run、父/root、role、lane、scan、attempt、target、lease epoch 与 fencing 的一致性。
- 正常完成要求 running／waiting_review assignment 与 running run；leased／prepared 仅允许失败清理，不允许伪造成功。
- 捕获 root 账本完整预算向量，按当前子任务预留计算期望值，不能释放兄弟任务预算、改变总额或凭空增加／减少已消费额度。
- 核验 run 终态写入恰为一行。
- 在全部资源写入之后，复核 assignment／run 终态、终态码、脱敏原因、完成时间、零预留、能力已撤销及执行槽已释放。
- 要求本次事务新生成真实 assignment 与 run 终态事件；不能用此前的事件顶替，也不能没有时间线却返回完成。
- 不满足条件即返回错误，由所属事务回滚。不会因 SQL 没有抛异常就宣称任务完成。

这仍是数据库业务合同，不是抵抗任意数据库管理员篡改的安全证明。

## 4. 目标执行结果的原子交付

新增 `src-tauri/src/commands/multi_agent_child_delivery.rs`，只接受两组角色／消息：

- WebExecutor → `execution_result`。
- Authorization → `authorization_result`。

执行顺序：

1. 目标工作结束后，先保留真实使用量结算。已经发生的调用不能因后续数据库交付失败被当成没有消耗。
2. 打开单个 IMMEDIATE 事务，重验活跃 attempt、Coordinator 与 fencing。
3. 要求 child 当前 running、预算已结算且预留为零；读取实际 evidence revision，不再硬编码结果消息 revision 为 1。
4. 发送脱敏后的精确结果，使用参与调用方事务的定向确认函数，只确认本次消息。
5. 同一事务内结束 child、撤销能力并释放 lane。
6. 在最后一次写入后，核验消息的 root/run、收发角色、assignment、kind、关联键、revision、payload、投递／确认时间和单次投递计数。
7. 核验本次 assignment、run 和已确认 mailbox 的三个真实时间线事件，复核 Coordinator 仍可执行，最后提交。

其中任一步失败，结果消息与成功收口一起回滚；已在前一事务结算的消耗保留。调用者仍尝试原有失败清理，并向上返回交付错误与必要的清理错误；失败清理可以留下真实失败终态，不能包装为正常成功。

本函数不调用模型、不发送网络请求，不提供重新执行已触达目标操作的能力。重复完成被拒绝，不意味着具备重启后补交或未知结果人工对账能力。

## 5. 新增回归

`src-tauri/src/commands/agent_tests_child_completion.rs` 共 9 项测试，包含内部故障矩阵：

1. 成功／失败收口 × IGNORE／ABORT × 预算、assignment、run、能力和 lane 写入，完整资源快照回滚。
2. 最后一次资源写入之后篡改终态、原因、预留、能力或总账，拒绝并回滚。
3. 伪造 handle role、run role／target／attempt／assignment、assignment target／fencing，拒绝并回滚。
4. 只释放自己的资源，兄弟任务预算／lane／能力不受影响；不能二次完成；未启动任务仍可失败清理。
5. 生产 `multi_agent_prepare` → `multi_agent_finish_execution` 正向路径，确认一个实际已投递／确认的结果与完整资源收口。
6. 生产 Executor 收口失败，不留下 `execution_result`。
7. 缺失本次 assignment／run 终态事件时，不能提交成功。
8. 两个目标角色分别覆盖 17 种故障：消息少写／确认错误、资源少写、事件缺失、消息内容／路由／确认被后改、revision／结算状态被改变、Coordinator 暂停、延迟外键在 commit 时失败；使用量保留。
9. 两个角色分别覆盖成功／失败结果的正常交付、非默认 revision、无关待处理消息不被误确认、重复交付拒绝且不改变状态。

Authorization 夹具通过真实控制组登记、身份绑定和调度约束建立子任务，没有放宽生产 capability 或预算校验来使测试通过。一次中间测试因夹具错误使用 `mailbox.write` 而被真实约束拒绝，随后修正夹具；该失败不作为生产缺陷。

## 6. 验证记录

最终本地门禁已通过。新增定向回归 9 项通过；离线全 targets／features Rust 主库 **875 项**、历史导入器 **30 项**通过，0 失败；全量进程退出码为 0，主库耗时 364.27 秒，导入器 1.78 秒。fmt、严格全 targets／features Clippy、实际 Vue SFC **88 项**、前端构建、Native localhost 浏览器回环（8 请求、身份隔离 true）与文档更新后的 diff 空白检查均通过。此前 866 项是上一增量基线，不代替本次结果。

日志前缀 `/tmp/oviraptor-20260926-child-completion-`：

- `targeted.log`：新增 9 项回归。
- `verified-fmt.log`、`verified-clippy.log`、`verified-full.log`：格式、严格静态检查与全量 Rust。
- `ui.log`、`build.log`、`native.log`：真实 SFC、前端构建、本地浏览器。
- `verified-diff.log`：文档更新后的空白检查。

前端主 JS 795.73 kB 拆包告警保留，本轮不修改 UI。

## 7. 未完成项与边界

- 已派发／效果未知任务、跨 fencing／终态的人工对账；本次原子交付不提供自动重放。
- 源码／CI／combined 完整启动事务，运行期撤权及跨层状态收口。
- Stage 10 其余专家的真实执行链，协同 UI 全面整理、资产与知识／skills 生命周期。
- 不可变工具供应、隔离沙箱、性能、桌面强杀／断电、安装包与真实授权环境验收。
- Web→主机的结构化动作边界与将来独立审批合同，见 Master Plan §6.4；关键词识别与 `web_only` 标签不是完整语义隔离证明。

Strix 退役门禁与历史 JSON 兼容要求不变，不得借本次交付错误回退旧执行后端。
