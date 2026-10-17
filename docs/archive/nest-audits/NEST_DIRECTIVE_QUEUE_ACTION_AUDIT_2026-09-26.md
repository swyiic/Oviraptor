# 人工指令的持久化队列动作审核（2026-09-26）

## 续核：完成回执、恢复与历史投影一致性

本节为当前增量；后文数字及未完成项为初版历史快照，整体进度以 `NEST_IMPLEMENTATION_PROGRESS_2026-09-23.md` 最新记录为准。尤其角色提案、终态收口、源码引导与 Stage 9 补证再审已有后续实现，不能仅按本文件初版清单判断不存在。

发现并通过红测复现两个漏洞：已完成优先级规则恢复只读动作类别，未复核原始确认；SQLite trigger 使用 `RAISE(IGNORE)` 丢弃回执/完成事件时，原调用仍可返回完成。损坏回执也可能继续显示成功。

本次修复：

- `queue_actions::project_receipt` 共用于事务提交后置检查、规则恢复及聊天读取。校验确认草案摘要、冻结 payload、任务/attempt/目标/线程/原领取 fence、动作类别、回执字段和 applied→completed 事件顺序。
- 新规则的回执必须与本次实际计算值相等，且持久化状态/事件齐备后才提交并返回新队列；静默漏写同样回滚。
- 恢复从指令侧查找完成记录，不再因回执丢失而静默漏掉规则。各历史规则均检查，排序仍最多七类，不按消息数量重复排序。
- 历史核验使用原领取 fence，不要求当前租约有效；正常新租约恢复、终态和过期租约下只读历史均保留。读取不修复/改写历史，不重新校验当前图版本来否定过去的合法动作。
- 无法核验时投影清空 `queueAction`，标为 `receipt_unverified` / `queue_action_receipt_unverified`。页面显示明确警告，即使收到旧的成功回执也不同时展示“已落实”。保存的历史状态保持原样，不能把它当作当前已核验完成。

这是持久化记录之间的一致性校验，不是针对任意全库恶意重写的密码学真实性保证。队列摘要不保存敏感路径，也不能单凭摘要重新构建当时完整队列。异常不会新增目标请求、模型调用、权限或自动修复动作。

新增 3 项 Rust 测试（包含多种故障子场景）：回执缺失、草案/冻结 payload/claim fence/类别损坏、完成事件丢失、回执字段矛盾、草案删除、插入回执或 applied/completed 事件被静默忽略、终态历史只读。原有真实 child、新 fence 恢复、目标隔离、优先级顺序和实际模型输入回归继续运行。模板测试增加无法核验警告及压制陈旧成功提示。

验证记录：红测 2 项实际失败（`/tmp/oviraptor-queue-integrity-red.log`）；首轮动作专项 16 项通过（`/tmp/oviraptor-queue-integrity-green.log`，不包含随后新增的终态测试及扩展故障子场景）；聊天 99、完整 UI 248 通过；前端构建通过，主 JS 866.76 kB 告警保留。完整 Rust session `91631` 已收取 exit 0，主库 **1348**、历史导入器 **30** 通过，包含扩展后的全部 **17** 个动作专项；日志 `/tmp/oviraptor-queue-integrity-full.log`。本次编译不含后来增加的提案交付校验；严格 Clippy 尚待执行，不将前序结果冒充当前代码全门禁。

本轮没有新增主机模块、扩展自然语言句式或完成 Master Plan。全历史一致性核验仍有随历史条数增长的读取成本，至多七次排序不等于端到端常量时间；大规模数据库/真实桌面 IPC 验收仍需另证。

## 结论与边界

本轮把明确、已确认的类别优先级指令接到 Native 的实际 pending queue，不再只是把文字送给模型。完成态仅表示 **Coordinator 的队列优先级规则已提交**，不表示模型已执行对应检查，更不表示漏洞已确认或 Reviewer 已完成。

这是 Master Plan §7.5.3 的 priority change 分支，不是整个自然语言指令闭环。复杂指令、禁用/暂停分支、角色提案到 Assignment、实际补证再审仍需继续实现。没有改变 `web_only`，没有增加主机执行器，没有部署或访问外部目标。

## 操作者现在能使用什么

支持完整且无歧义的句式：

- `请优先检查权限控制`、`优先检查权限控制`。
- `prioritize authorization`、`优先检查 authorization`。
- 可在前面加 `@coordinator `、`@协调 ` 或 `@总控 `。
- 其他类别为信息泄露、错误处理、认证会话、输入反射、隐藏接口、业务流程，也可以使用对应的 canonical coverage family 名称。

确认卡先展示 `prioritize_family:authorization` 等冻结动作及其影响。用户确认后才能领取、执行；这项纯本地调度动作的估算 token 和请求数均为零，但后续扫描仍按原计划记账，不是免费扫描。

否定、多子句、多个类别、URL、主机操作、提高预算及 `@Reviewer` 等其他角色请求不能凭其中几个关键词变成队列动作。它们继续走原有拒绝、提案或待落实模型上下文流程，不能显示本动作的完成回执。当前明确句式覆盖有限；不得把这一实现宣传成已能任意理解并执行整段自然语言。

旧 JSON 的 `priorityChanges: string[]` 形状不变。既有 `evaluate_requested_priority_change` 等值仍按原有待评估请求处理，不能在恢复时重新解释并自动执行。确认 hash 继续绑定冻结的动作内容。

## 队列语义

1. 只匹配 `family:<family>` 和 `contract:<相同 category>|<path>`，不根据 API 路径猜测类别。
2. 稳定地把匹配项提前；不删除、生成或修改待办，重复项也不丢弃。类别以外的项保持相对顺序。
3. 没有匹配待办时，指令进入 `deferred`，原因码为 `priority_no_matching_pending_work`，不安装规则、不产生完成回执；后续出现工作也不会暗中激活它，需要新的已确认指令。
4. 已处在前面的匹配项可以保存优先级，回执为 `changedOrder=false`，界面不声称发生了顺序变化。
5. 规则只在相同 scan / attempt / target / root 中重放，较晚确认并提交的规则优先。领取使用持久化插入顺序，避免秒级时间戳和随机 UUID 导致批次颠倒。
6. 同一类别多次确认的历史回执全部留存，恢复时每类只重放最近一条；最多七个类别的排序，不随历史消息数重复排序。
7. 每轮在产生模型请求前重放规则，并替换真实 state block。新发现的匹配待办在下一轮排序；未匹配的工作不被跳过。

模型仍在工具权限、证据合同和任务预算内选择下一步。这是待办优先级，不是强制某个漏洞一定被验证，不允许为了服从排序绕过前置证据或安全门禁。

## 事务、恢复与多智能体连接

新增 `agent_directive_queue_actions`：一条指令至多一条动作记录，包含持久化顺序、类别、只读 JSON 回执和时间；更新由数据库 trigger 拒绝。新旧数据库均通过现有 `ensure_schema` 安装，不重写旧指令或历史导入。

执行路径在 IMMEDIATE 写事务内：

1. 重查租约 fencing token、活动 attempt 和源草案的确认/hash/文本/线程/证据绑定。
2. 重放本作用域已完成的规则。
3. 应用新的冻结动作，写不可变回执。
4. `accepted → applied → completed` 与协作事件在同一事务提交。
5. 提交成功才返回新内存队列；失败时调用者不能发布新顺序。

回执包含匹配数、是否改变顺序、队列前后摘要、租约 epoch、零额外 model/target requests、`coverageVerified=false`。不保存原始敏感路径列表。时间线读取动作表并经本文件续核所述共享校验后展示，不从模型原文编造完成状态。

崩溃发生在提交前：事务回滚，下次可重试。崩溃发生在提交后、checkpoint 前：规则已是权威状态，下次从原队列重放；不新增回执、不重新计费。新租约可以重放已完成的规则，但未执行的旧租约请求仍须重新确认，不能借恢复自动取得执行权。

生产 Native 在 WebExecutor child 中运行时，inbox 仍通过真实 run 记录解析所属 Coordinator root。增加真实 `multi_agent_prepare` 创建子运行的测试，防止只验证单 root 的快捷路径。

动作完成的指令从本轮模型上下文移除，不再伪装成尚未落实的自然语言请求。其他未执行的已接受约束保持原来的送达/恢复机制；不得因本动作完成而删除其他人工要求。

## 代码与回归

- `src-tauri/src/agent_runtime/multi_agent/directive/queue_actions.rs`：明确句式、稳定排序、原子回执、作用域重放。
- `src-tauri/src/agent_runtime/multi_agent/directive.rs`：确认前冻结动作、共同事务内的上下文核验、确认顺序领取。
- `src-tauri/src/collaboration_events.rs`：可升级动作表及不可变约束。
- `src-tauri/src/commands/agent_native.rs`：在实际模型请求前调整队列及 state block。
- `src-tauri/src/commands/native_scan_branches.rs`、`src/types.ts`、`AgentDialog.vue`：从权威回执投影及展示动作边界。
- `agent_tests_directive_actions.rs`：14 项动作回归，覆盖确认前草案、否定/复合表达、原子回滚、恢复幂等、无匹配、优先级覆盖、新待办、完整性/暂停、升级、未确认、批次顺序、真实写锁竞争、跨目标/预算隔离、实际模型请求及真实 child 连接。
- `tools/test_agent_dialog.cjs`：新增实际 SFC 模板输出检查，共 30 项；验证确认卡、完成说明、已在前的情况以及只有模型送达而无动作回执的情况。使用 Vue setup、模板编译和服务器渲染，不是浏览器视觉或 Tauri IPC 验收。

先失败的解析测试证据：`/tmp/oviraptor-20260926-action-red.log`。早期 13 项动作加已有指令回归共 32 项通过，见 `/tmp/oviraptor-20260926-actions-contracts.log`。

最终门禁：

| 验证范围 | 结果 | 日志 |
| --- | --- | --- |
| Rust 离线、全 targets、全 features、单测试线程 | 主库 **683** 项、历史导入器 **30** 项通过，包含最终真实 child 测试及退役字面量门禁 | `/tmp/oviraptor-20260926-actions-final-rust.log` |
| Clippy 全 targets / 全 features | `-D warnings` 通过 | `/tmp/oviraptor-20260926-actions-final-clippy.log` |
| Vue setup + 实际模板输出回归 | **30** 项通过，含无匹配待办的明确提示；不是浏览器 DOM、视觉或 IPC 测试 | `/tmp/oviraptor-20260926-actions-dialog.log` |
| 前端 build | 通过；主 JS chunk **771.97 kB**，体积告警仍存在 | `/tmp/oviraptor-20260926-actions-build.log` |
| 本地 Native 浏览器回环 | **8** 次请求，匿名与隔离身份采集通过 | `/tmp/oviraptor-20260926-actions-native.log` |
| fmt / `git diff --check` | 通过 | 命令直接返回 0 |

Rust 全量通过后仅补充了前端无匹配待办的提示与相应模板断言，已重新执行上述 30 项前端测试及 build；Rust 生产代码和测试没有再改。日志在本机临时目录，不能替代持久化安装包或跨平台验收产物。

## 仍未完成的原目标

1. 复杂人类指令分解、部分接受、暂停/禁用合同的真实状态机。
2. `directive → proposal/Assignment → child-run → mailbox → result` 的全部意图连接及逐动作完成判定。
3. accepted 但未落实指令在任务结束时的明确收口，以及长期上下文生命周期。
4. 旧租约重新确认的产品化入口、线程/未读/草稿跨刷新持久化。
5. Stage 9 实际补证和再审、Stage 10 剩余专家、沙箱隔离与工具供应的真实环境验收。
6. REM 全量验收矩阵、安装包、Linux 实机与用户授权目标测试；前端主 chunk 体积告警。

以上保留原 Master Plan 范围，不因为优先级分支通过而缩减目标或宣布整体完成。
