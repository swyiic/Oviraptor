# Investigator 已存回执的运行期补交审计（2026-09-26）

## 范围与结论边界

本增量补齐 Deep Investigator 在 Reviewer 判定证据不足后，已收到模型结果但业务交付失败的本地恢复。恢复原提案及 Coordinator 的确定性评估，不执行提案中的采证建议，不签发目标请求权限，不新增主机能力。继续保持 `web_only`，仅本地回环验证，未部署或访问外部目标。

它不是任意任务续跑，也不是 Stage 9 的“提案 → 批准新合同 → 采证 → 新 revision → Reviewer 再审”闭环。根任务已结束/暂停、租约过期或已换代、模型结果未知、历史消息半交付，以及清理本身未完成的 running child，均不在本入口的自动恢复范围内。

## 原问题

1. 双向 mailbox 已原子交付，但相同缺口重入只会返回“需要恢复”，未接入已有真实 received 回执。
2. 正常交付和恢复没有证明同一事务的四个实体都产生可见聊天事件；SQLite IGNORE 可能静默漏记时间线。
3. 已结算 child 的失败清理只依赖 SQL 返回成功，没有检查 run 终态、能力撤销和 lane 释放是否实际完成。
4. 缺口完成重放分多次读取，可能跨数据库快照；并发恢复需要避免重复业务提交。

## 实现

### 生产接入与状态转换

`multi_agent_investigate_review_gap` 在费用结算/交付失败后先清理，清理成功后至多进行一次本地补交；保留原错误和补交错误，不重调模型。相同 candidate/revision 的 failed/paused assignment 重入也使用这一入口。

`complete_gap_delivery` 将正常交付与恢复合并到一个 IMMEDIATE 事务中，重读封存 Reviewer 结果与 manifest，再校验冻结输入。恢复专用逻辑位于 `multi_agent_gap_recovery.rs`：

- failed + 已结算：必须无预留、无 lane、无活跃 capability，核对已保存的实际费用，不再次扣费。
- paused + 待对账：必须有原任务/attempt/target/lane 匹配的占位，且能力已撤销；在原预留下结算真实费用。
- 两种状态都直接转 completed，终态码为 `gap_receipt_reconciled`。不先变 running，不续租，不重新赋予能力。

### 原回执绑定

恢复要求相同活跃 Coordinator/attempt/fencing；复用 specialist 回执校验，检查 received 状态、request/response hash、事件及 checkpoint。hash 是存储一致性校验，不是抵抗数据库全权修改者的签名。

额外检查：角色/lane/trigger/revision、精确 task slice、无该 assignment 先前消息；保存的请求必须为无工具的两条 system/user 消息，user JSON 必须等于脱敏冻结输入。严格提案解析在同一事务中执行，无效提案即使已改变 child 状态，也会整体回滚。来源事实必须仍然可信。

交付内容仍为两条真实消息：Investigator `gap_proposed` 与 Coordinator `proposal_assessed`，分别送达并确认。评估明确 `targetRequestsGranted=0`；提出新合同只是申请建议，不是执行授权。

### 原子性、重放与并发

业务提交前，检查本事务新增的四个实体事件：assignment 完成、child 完成、proposal mailbox、assessment mailbox。两条 mailbox 事件必须与实际 kind/deliveredAt/acknowledgedAt 一致；缺任一项则回滚。

普通已完成重放在一个读事务中检查封存 Reviewer、assignment、child 和两条消息；历史 v2 兼容校验继续保留。进入恢复或调度前释放读快照。

并发恢复在取得写锁后重新检查：若另一调用者已经完整补交原 child，则校验已完成消息和资源收口，直接返回，不重复结算、发送消息或增加完成事件。

### 失败清理修复

共享 `stop_failed_child_preserving_usage_in_transaction` 的已结算分支新增提交前检查。assignment/run 未实际失败、能力未撤销或 lane 未释放，均返回错误并回滚清理事务。

如果撤权/释放 lane 故障导致清理本身失败，本轮不把仍运行的 child 当成已安全暂停，不自动接管其他可能仍活跃的工作者。测试在移除故障后显式完成清理，再验证原 received 回执可以补交；**这项测试不证明生产已经具备自动重试清理的恢复管理器。**

## 回归证据

新增 `agent_tests_gap_recovery.rs` 的 6 个测试函数：

1. failed/paused 两种原状态，52 个 ABORT/IGNORE 写入故障组合及 4 个成功对照，覆盖结算、状态、lane、双向 mailbox 和四实体时间线。每组检查 15 张表快照回滚、总账增量及重复补交不变。
2. 16 组授权/回执/任务/证据损坏。正常修改 specialist 回执先被不可变触发器拒绝；显式移除保护后模拟存储损坏，读取层仍拒绝。
3. 真实生产交付分支只阻断 running child 的普通投递，证明失败清理后直接使用原回执完成，无第二次 HTTP。
4. 两线程并发补交同一暂停 child，均得到完成结果，但只有一个完成事件、两条已确认消息和一次模型调用。
5. 4 组精确冻结输入、task slice 和严格提案拒绝，均无副作用。
6. 8 组已结算失败清理故障，验证静默跳过与显式错误都不会留下半清理状态。

既有 Investigator 原子交付测试同步更新：普通投递故障移除后可从原回执重入；清理故障必须先完成明确清理。原有“不退费、不二次派发、无半消息”的断言保留并加强。

## 质量门禁

最终验证全部退出 0：

| 检查 | 结果 |
| --- | --- |
| `cargo test --offline --all-targets --all-features` | 主库 778 通过，0 失败，223.50 秒；历史导入器 30 通过 |
| `cargo clippy --offline --all-targets --all-features -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `npm run test:agent-dialog` | 38 通过，0 失败 |
| `npm run build` | 通过，主 JS 777.36 kB 拆包警告保留 |
| `node tools/test_native_runtime.cjs` | 通过，8 次本地请求，身份隔离成立 |
| `git diff --check` | 通过 |

日志前缀：`/tmp/oviraptor-20260926-gap-recovery-`。最终 Rust、Clippy、格式日志分别为 `final-full.log`、`final-clippy.log`、`final-fmt.log`；其余为 `ui.log`、`build.log`、`native.log`。第一次全量测试的 `full.log` 留作诊断记录：777 项通过，旧错误字符串断言失败 1 项；修正为具体的缺少回执错误并增加全表快照不变断言后，再次完整运行才得到上述最终结果。

## 后续仍需完成

- 终态任务、跨 fencing 的通用人工对账 UI/API，以及未知费用/未知结果处理。
- 清理失败、进程崩溃和历史 pending/tally 的恢复管理，不能靠本地补交函数冒充整场任务恢复。
- Stage 9 的明确审批、新合同、新证据 revision 与再审闭环；其余 specialist 的真实执行链。
- Master Plan 剩余 UI、沙箱、资产/知识沉淀、部署及真实授权环境验收。

未改动 Strix 退役允许清单；本次门禁通过不等于总体目标或 Master Plan 完成。
