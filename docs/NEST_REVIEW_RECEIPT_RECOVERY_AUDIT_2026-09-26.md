# Reviewer 已存回执的运行期补交审计（2026-09-26）

同日后续：Investigator 的已存回执本地补交、并发交付和时间线后置检查另见 `NEST_INVESTIGATOR_RECEIPT_RECOVERY_AUDIT_2026-09-26.md`。本篇的 Reviewer 验收记录保留，不据此扩大为终态/跨 fencing 通用恢复。

## 范围

本轮接通独立 Reviewer 已保存模型回执的本地业务补交，补强审核重放与真实聊天事件的一致性。继续保持 `web_only`，未部署、未访问外部目标、未新增主机执行器或主机审批权限。

这不是 Master Plan 完成声明，也不是任意任务断点续跑。前提是原 Coordinator 仍可执行、原 attempt 仍活跃、原 fencing 仍有效且未换代。根任务已终止、暂停、授权过期、未知模型结果、历史半交付记录均不由本入口恢复。

## 原缺口与本轮修改

上一轮已保证 Reviewer 决策、mailbox、child 完成与候选发布原子提交，但生产交付失败后仍只能留下 failed/paused child；测试手工再次调用交付函数不等于生产已支持补交。本轮增加 `multi_agent_review_recovery.rs`，并将补交接入：

1. 生产 Reviewer 费用结算或业务交付失败后的清理分支：清理成功后最多本地补交一次；失败保留原错误与恢复错误，不重试模型。
2. 相同冻结候选/revision 的重入：尝试补交原 failed 请求，然后使用正常审核重放路径读取和验证结果。没有创建新 assignment 或新模型调用。
3. 并发读取兼容：其他读取者若已完成同一回执，当前读取者继续验证已完成记录，不把局部补交竞争当成再次调模型的理由。

### 可接受的原 child 状态

- **failed 且已结算**：核对保存用量等于原结算；要求预留已释放、没有 lane、没有活跃 capability；直接补齐业务结果，不再扣费。
- **paused 且待对账**：要求原预留、原 lane 与任务身份匹配，且能力已撤销；用保存的真实用量在原预留下结算，直接结束为 completed。

两者都不会临时变回 running，不刷新 Coordinator 租约，不恢复 capability，也不重新访问目标。成功的 child 使用 `review_receipt_reconciled` 终态码，保留原 terminal reason 并追加补交说明。

### 回执与候选绑定

补交在一个 IMMEDIATE 事务内验证：

- 当前 scan/attempt/root/target/角色/lane/assignment/fencing 绑定。
- specialist 回执必须为 received；验证冻结 request/hash、response/hash、事件及 checkpoint。
- 拒绝模型拒绝响应和不符合严格 schema 的 decision。
- 请求必须为 failed、无 decision、无先前该 assignment 消息、无更新候选 revision。
- assignment 的 candidateId/candidateRevision 必须匹配冻结请求。
- 保存的模型请求必须为无工具的两条消息输入，user JSON 必须等于脱敏后的冻结候选。
- 证据 manifest 和待发布候选必须仍与原快照一致。

hash 是一致性校验，不是对数据库完全控制者的密码学签名；不据此宣称抗数据库全权篡改。

## 重放与聊天时间线

原已完成重放只核对候选 id/revision/reviewer/status。本轮改为在同一数据库读快照中验证 stage、kind、recordKey、title、severity、record 及 scan/target；confirmed 还必须有内容一致的实际 `sentinel_findings` 发布记录。状态仍为 published 但内容被修改或删除时，不再返回原成功结果，也不自动重发模型。

故障注入证实：若 `agent_collaboration_events` 的插入被 IGNORE，原业务事务可能成功而没有对应聊天事件。修复后，正常交付及本地补交均检查本事务新增的四类真实事件：assignment 完成、child 完成、review gate 决策、匹配实际 delivered/ack 时间的 review mailbox。任一事件缺失，整次业务交付回滚。

读事务在进入 Investigator 之前释放，避免持有旧读快照同时启动另一连接的写入。

## 回归覆盖

新增测试位于 `src-tauri/src/commands/agent_tests_review_recovery.rs`：

1. 原子补交矩阵：failed/paused 两种状态，ABORT/IGNORE 故障覆盖预算、child、assignment、lane、decision、request、mailbox、发现发布与四类聊天事件。共 60 个故障组合及 4 个成功对照；每次移除故障后再次本地补交，模型请求始终为 1、用量为 20 tokens/1 request。
2. 绑定与损坏拒绝：暂停/换 attempt/终止 root/过期租约/换 owner、未知或损坏回执、错误 task slice/lane、缺失 lane、活跃 capability、变更候选等 16 组。普通 SQL 修改回执先被 immutable trigger 拒绝；测试明确移除该保护后再模拟坏盘数据，验证读取层仍拒绝恢复。
3. 冻结输入与严格 decision：真实收到的模型输入与候选不一致，以及真实收到无效 decision，各自拒绝补交且不修改业务状态。
4. 真实生产链：一次性原交付故障自动补交；持续故障保留失败，移除故障后同候选重入补交。完成重放只读；候选/发布内容被修改、发布行删除时不得冒充成功。

共享快照比较已从 14 张表扩展至 15 张表，纳入 `agent_collaboration_events`。新增禁止重新 running、签发/恢复 capability、续租的测试触发器，证明补交不是重新授予执行权。

## 门禁记录

最终版本门禁结果：

- `cargo test --offline --all-targets --all-features`：主库 **772** 项、历史导入器 **30** 项通过，零失败。包含隔离后的发布记录删除断言；主库耗时 156.61 秒。
- `cargo clippy --offline --all-targets --all-features -- -D warnings`：通过。
- `cargo fmt --all -- --check`、`git diff --check`：通过；两个新增 Rust 文件另经定向 rustfmt 格式化。
- `npm run test:agent-dialog`：**38** 项通过。
- `npm run build`：通过；主 JS **777.36 kB** 的拆包警告仍存在。
- `node tools/test_native_runtime.cjs`：通过，本地 **8** 次请求，匿名/身份对照完成且身份隔离通过。
- 未修改 backend retirement allowlist，未放宽授权、历史 JSON 导入或退役验收要求。

日志前缀：`/tmp/oviraptor-20260926-review-recovery-`。`final-full.log` 为最后全量门禁；`full.log` 为首轮全量通过记录；另有 `clippy.log`、`fmt.log`、`ui.log`、`build.log`、`native.log`。

## 明确保留的未完成项

- Investigator 已有原子交付，但失败回执的业务恢复仍未接通。
- 已终止/过期/换代任务的独立只读对账 API/UI；进程崩溃后的统一恢复与历史半完成数据处理。
- 同候选恢复入口不替代完整外层任务重启、观察 UI、历史 pending/tally 收敛。
- Stage 9 的补证合同签发、真实采证、新 revision 再审；Stage 10 的其他专家。
- 完整沙箱供应、整体 UI 重排、资产画像和技能质量治理，以及部署与授权 URL 验收。

继续沿 Master Plan 推进，不以本轮有限恢复能力替代以上要求，不通过放宽授权/预算/证据门禁或重调模型来伪造恢复成功。
