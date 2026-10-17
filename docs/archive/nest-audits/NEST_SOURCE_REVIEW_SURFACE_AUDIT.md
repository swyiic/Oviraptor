# Source Reviewer 接线前的执行面校验审计（2026-09-27）

## 1. 结论和范围

修复了一个真实调度漏洞：源码根运行的冻结 v1 计划只含两个只读初评和两个源码工具阶段，但通用 scheduler 曾允许直接在此 root 下建立 `EvidenceReviewer` 或 Web 专家 assignment。不能将这种通用 Reviewer 当作已经具备源码材料、作者独立性和当前修订校验的独立源码审查。

本增量使根运行的执行面/阶段版本约束在实际调度、启动与模型派发入口生效。它是独立 Source Reviewer 接线的前置修复，**不是独立 Reviewer 执行链已经完成，也不是永久取消此角色**。v1 attempt 不原地增加阶段或模型预算；下一版真正的源码审查必须有新的冻结编排合同，并完成材料、请求、裁决、mailbox、费用及 CI 资格的整条证明。

没有增加 Web/主机权限、部署、访问外部测试目标或修改历史 JSON 导入语义。

## 2. 已验证的根因

- `scheduler::schedule_child_in_transaction` 原本只对 `RepoMapper` / `SourceAnalyst` 调用源码专用合同校验。通用 `EvidenceReviewer` 虽受 review lane 和 capability allowlist 限制，却不绑定源码阶段合同。
- `specialist::binding` 同样根据 child 角色而非根执行面决定是否校验源码合同。仅限制新的调度调用，不能阻止以前已经建立的通用 Reviewer 行进入派发。
- 新增回归在修改前真实失败：`source_surface_contract_rejects_unplanned_reviewer_and_web_roles` 返回了一个 `ScheduledChild { role: EvidenceReviewer }`，不是预期错误。日志 `/tmp/oviraptor-source-surface-red.log`，session 90073，exit 101。

另一个疑点已经澄清：生产数据库迁移会建立 `agent_evidence_nodes_revision_insert` 触发器，在真实候选节点写入时登记修订。因此“SourceBroker 中没有显式 INSERT revision”不代表修订一定丢失。本次在真实 SourceBroker 成功路径断言了 revision=1、effective snapshot 可见该候选、作者是实际 child、状态仍为 candidate，且不存在的下一修订仍不可见。未增加重复登记或无根据的数据回填。

## 3. 实现

`agent_runtime/multi_agent/source.rs::validate_surface_role` 从真实 root 读取计划与目标，不信任 caller 的 task slice 或角色名来选择执行面：

- 根计划声明 source、根目标或 lease 目标使用 source identity，任一命中均进入源码校验，不能把其中一个字段改成 Web 来绕过。
- 核对当前 schema/工具阶段版本、零目标/主机授权、8 次模型调用合同及根预算一致性。
- v1 不接受临时添加 `sourceReviewPhaseVersion` 或未定义的新版本。
- v1 仅允许已有的两个源码角色；其初评与工具合同仍由已有专用验证器细分校验。
- 保留 `require_open_coordinator` 已支持的 single Web 根身份，用于原有人工提案；源码仍要求 `root_run_id=id`，不能借用此例外。

调用位置：

1. `prepare_readonly_child`，包括已有 assignment 的恢复入口。
2. `schedule_child_in_transaction` 写前、写后。
3. `mark_child_running_in_transaction` 写前、写后。
4. `specialist::binding`，覆盖模型派发前后及已有调用的绑定检查。
5. 源码材料 `restore`，使工具合同/初评输入也不能沿用变更后的阶段授权。

拒绝是事务性的：不留下一半的 child、lane、capability 或预算预留；模型调用落库后的合同漂移会连同调用 claim 回滚，不产生可被误解为未知外部调用的 journal。已有历史审计接口没有被改成执行授权接口。

## 4. 回归覆盖

新增四项测试：

- 未计划的 Reviewer、Web mapper、调查员、Web executor 均不能从源码 root 获取 assignment；无新增执行资源。
- 用一致的旧 assignment/run/lane 行模拟此前通用 Reviewer：启动与模型派发均拒绝，provider/runtime 授权回调不被调用，调用 journal 保持空。
- 更改执行面、schema、阶段版本、Review 版本、预算、目标/主机授权均拒绝；capability INSERT 触发的计划变更整体回滚，随后合法源码专家仍可正常调度。
- child 启动和模型 claim 的写后触发器变更会回滚；去掉故障后相同合法调用可正常取得唯一 Dispatch。

扩展一项已有真实源码轮次测试：验证候选节点通过生产 revision trigger 登记后可以从有效证据快照读取，而非仅检查 nodes 表行数。

测试编写过程另出现两项夹具问题：公共 scheduler 自带事务，不能在外层 SAVEPOINT 中嵌套；调度在预留前拒绝时预算账本允许尚无行。已分别改为恢复原冻结计划、检查 COALESCE 聚合预留。没有放宽生产断言。

首轮全量另发现真实 Web 回归：`directive_proposal_native_loop_receives_actual_specialist_result` 因新校验仅接受 `root_run_id=id`，误拦已有 single Web root。独立复现产生 `source_surface_root_unavailable`、实际模型请求 0/3。修复使根身份查询沿用已存在的 Web 兼容条件，同时源码分支仍显式要求 rooted；新增断言证明 source root 伪装 single 仍被拒绝。这不是放宽源码阶段授权。

## 5. 验证记录

- 首次红测：session 90073 exit 101，真实复现通用 Reviewer 在 source root 下被调度。
- 修订中定向测试：session 46634 exit 101，以上两项夹具问题，已修复。
- 源码定向测试：session 43310 exit 0，主库 167 / 导入器 2；该批编译时包含前三项新增测试，最后一项由下方完整回归覆盖。
- 严格 Clippy：session 73540 exit 0，`--all-targets --all-features -- -D warnings`。
- 首轮全量 Rust：session 65346 已 exit 101，1152 通过、1 失败；失败为上述已修复的 single Web 回归，不能称为通过。
- 修复后 Web 定向 1 项、四项新测试及最终严格 Clippy/fmt：session 6213 exit 0。
- 修复后完整 Rust 回归：session 46549 因测试 HTTP 夹具线程泄漏和用户报告 CPU 360% 被暂停，随后精确终止旧测试子进程，cargo 已 exit 101（人为中断，不是完整测试结果）。日志 `/tmp/oviraptor-source-surface-full-final.log` 保留；没有恢复或重跑全量。详见 `NEST_TEST_CPU_INCIDENT_2026-09-27.md`。
- UI / build / localhost 浏览器 / fmt / diff：session 85881 exit 0；UI 154/154，构建成功，主 JS 831.51 kB 拆包告警保留，浏览器 localhost 8 请求及身份隔离通过。最终文档编辑后还需重跑空白检查。

日志前缀 `/tmp/oviraptor-source-surface-`。不能用前序 execution-history 的 1149/30 结果代替本次全量结果。

## 6. 下一步：真正独立的 Source Reviewer

仍必须实现，不能以本次拒绝逻辑替代：

1. 同时收集本 attempt 接受的 analyzer 候选修订和真实 SourceAnalyst 提交的图候选；保留多 analyzer 来源。同逻辑键的不同 revision 不混并，importer revision hash 不能冒充图的数字 revision。
2. 将材料摘要、真实作者、source view/plan/results 摘要、root/attempt 和精确 revision 绑定到独立 review assignment。核对候选对应的真实模型/工具回执，不能只相信节点存在。
3. 新版本根计划明确预留 Reviewer 模型调用；旧 v1 不自动升级。源码 Reviewer 使用真正的源码 runtime 发布合同、输出上限、截止时间和续租路径，不能因角色也叫 Reviewer 而走 Web 分支。
4. 真实独立模型调用、不可变裁决、邮箱送达/ACK、根费用守恒、终态写后复核与真实聊天事件全部贯通后，再报告独立审查完成。
5. CI 只接受满足当前 root/attempt/material/revision/独立作者/模型回执条件的裁决；继续保持未审查即 coverage gap，不能先恢复旧的通用 review 查询来制造通过。

整体 Master Plan、Source Reviewer、CI 裁决资格、恢复及其他未完成项仍保持未完成状态。
