# Reviewer / Investigator 原子交付审计（2026-09-26）

## 范围与结论

本轮修复独立 Reviewer、只读 Investigator 已收到模型结果后的业务交付一致性，不新增角色、不扩大目标权限、不增加固定执行次数门禁。没有部署、没有访问外部授权 URL，继续保持 `web_only`。

这是 Master Plan 的执行可靠性增量，不是完整计划交付证明。尤其不能把下面的原子写入或测试中的本地重新提交，宣传为已实现跨进程、终止后、过期租约下的通用业务恢复。

同日后续 `NEST_REVIEW_RECEIPT_RECOVERY_AUDIT_2026-09-26.md` 接通了活跃根任务、相同有效 fencing 下 Reviewer failed/paused received 回执的生产本地补交，并增加交付聊天事件核验与已完成发布内容重放检查。下文保留本原子性交付增量当时的测试结果；Investigator 恢复、终止后通用恢复和历史半完成处理仍未完成。

## 复现的缺口

原 Reviewer 依次独立提交决策、消息发送、消息确认、child 完成、发现发布。向 child 完成写入注入错误后，生产编排留下：

- review request：`confirmed`；
- assignment / child：`failed`；
- finding candidate：`pending`；
- review decision mailbox：已确认。

这不是成功完成的一次独立审查交付。回归测试先要求失败时不得留下已确认请求与消息，修复前确实失败，修复后通过。

Investigator 的 `gap_proposed`、Coordinator 的 `proposal_assessed`、两条消息的确认和 child 完成同样曾分开提交，后一步失败会遗留前面的半轮对话。

## 实现

### Reviewer

`complete_review_delivery` 用一个 SQLite IMMEDIATE 事务完成：

1. 复核活跃 Coordinator、当前 attempt 与有效 fencing。
2. 核验冻结候选及证据 manifest，保存决策、更新 review request。
3. 发出并消费确切绑定 assignment / revision 的 `review_decision`；比较脱敏后完整 payload，不只比较消息类型。
4. 完成 child 与 assignment，回收 capability、释放 lane。
5. 绑定候选 revision，按 verdict 发布或标记候选。
6. 核验候选写入数、实际发布内容、消息确认、已结算预算、child 终态及能力/lane 回收，再提交。

任一步失败，整段业务交付回滚。编排随后走原有失败收口，保留原始错误；若失败收口本身失败，也把 cleanup 错误返回，不吞掉。

候选处理不再把 SQLite `RAISE(IGNORE)` 造成的零行/少行更新当作成功。confirmed 发布同时覆盖插入与已有结果的 upsert；提交前比对 key、title、severity、record_json、状态和发布时间。

### Investigator

冻结 gap 读取提供事务内版本，避免嵌套事务。缺口重新核验、严格解析 proposal、两条消息发送及确认、child 完成与回收合并为一个事务。提交前用 `completed_gap_round_valid` 核验完整两条消息、角色、方向、candidate/revision、schema 和评估一致性，再核验预算结算、能力与 lane。

它仍是只读缺口分析，`targetRequestsGranted=0`。当前不签发新目标合同，也没有完成 Stage 9 的实际补证采集与新 revision 再审。

### 模型费用和回执不随业务回滚

真实模型调用已经发生，所以 received 回执、usage/event/checkpoint 和已完成的费用结算继续保留。业务交付失败不能退款为零、清除 received 响应或自动重调模型。

费用结算失败仍走既有的暂停、撤权、保留预留与 lane 的待对账路径；本轮没有改变这套预算守恒逻辑。

## 验证设计

测试文件：`src-tauri/src/commands/agent_tests_review_atomic.rs`。

- Reviewer：12 个写入位置 × ABORT/IGNORE，共 24 组；使用真实本地 HTTP 模型传输和 durable received 回执。包括决策、request、消息、ack、assignment、child、撤权、lane、候选绑定、插入/upsert、候选结束。
- 每组包含 3 个候选和 1 条已有发布记录，验证部分失败不会泄漏前面已写的候选，也不会损坏已有结果。
- 比较 14 张表的完整行快照，证明事务失败后无业务、预算或回执残留变化。
- 去除故障后以相同已收到结果本地提交，验证 3 个候选、1 条确认消息、1 次模型请求、一次费用结算。这个测试仅证明交付函数可再次提交，不代表生产已提供终止后恢复 API。
- Investigator：8 个写入位置 × ABORT/IGNORE，共 16 组故障，另有 2 组成功及重入对照。验证无半轮消息、无 completed 子任务、received 与已用费用不丢失。
- Investigator 失败后的生产重入仍返回需恢复，不再调模型；成功重入只读验证原结果。
- 既有生产 Reviewer 完成失败回归加强为 request=`failed`、candidate=`pending`、零成功决策确认，而不是容许 confirmed 请求残留。

## 最终门禁结果

- `cargo test --offline --all-targets --all-features`：主库 **768** 项、历史导入器 **30** 项全部通过。主库含本轮新增的 2 项矩阵测试（24 + 16 组故障，以及 2 组 Investigator 成功对照）及加强的既有回归。
- `cargo clippy --offline --all-targets --all-features -- -D warnings`：通过。
- `cargo fmt --all -- --check`、`git diff --check`：通过。
- `npm run test:agent-dialog`：**38** 项通过。
- `npm run build`：通过；主 JS **777.36 kB** 的大包警告仍存在，本轮没有声称修复前端拆包。
- `node tools/test_native_runtime.cjs`：通过，本地 **8** 次请求、匿名和身份对照均完成，身份隔离通过。
- 未修改 backend retirement allowlist，未减弱历史 JSON 导入或退役门禁。

验证日志：

- `/tmp/oviraptor-20260926-review-atomic-red.log`：修复前的生产回归失败证据。
- `/tmp/oviraptor-20260926-review-atomic-green.log`：该回归修复后通过。
- `/tmp/oviraptor-20260926-review-gap-atomic-full.log`：最终全量测试。
- `/tmp/oviraptor-20260926-review-atomic-{clippy,fmt,ui,build,native}.log`：其他门禁。

## 尚未完成

1. Reviewer / Investigator 的 paused、terminal 或过期归属下的通用本地业务恢复 API/UI；当前进程中断后不能承诺自动补交。
2. 已有历史半完成记录的显式修复与幂等发布校验；本轮原子性只防止新交付产生上述中间状态。
3. 整体任务观察者 UI、历史 pending / tally 收敛、未知调用与进程崩溃恢复。
4. Stage 9 的 gap → 合同 → 实际采集 → 新 revision → 独立 Reviewer 闭环，以及 Stage 10 其余专家。
5. Master Plan 其余沙箱、工具供应、UI、资产/学习与部署/授权 URL 验收要求。

以上继续留在总目标内，不以门禁通过替代产品和真实场景验收。
