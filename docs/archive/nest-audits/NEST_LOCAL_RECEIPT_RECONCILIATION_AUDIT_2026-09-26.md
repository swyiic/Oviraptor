# 终止任务的已存响应本地回执补齐（2026-09-26）

## 交付边界

本增量接续 `NEST_FINALIZATION_RECEIPT_AUDIT_2026-09-26.md`。处理的是：独立只读评估的模型响应与用量已经提交到数据库，但 Coordinator 在完成消息、预算结算和指令回执提交前终止。旧路径只能留下 `receipt_pending`，没有让操作员安全补齐的入口。

这不是重新运行扫描，不调用模型或目标，不执行评估建议，不授予主机权限，不绕过现有 live lease/Broker/调度器校验，也不代表 Stage 9/10 或整体 Master Plan 已完成。

## 接口与界面

- Tauri 命令：`reconcile_scan_directive_receipt(scan_id, attempt_number, directive_id)`。
- 实现：`agent_runtime/multi_agent/directive/reconciliation.rs::reconcile_received`。
- 前端：`AgentDialog.vue` 的“补齐本地回执（不重试模型）”按钮。
- 返回值：持久化的 `localReconciliation` 回执；API 使用对应 TypeScript 类型，不把返回值伪装成 `void`。
- 时间线从数据库投影此回执，成功/无效评估失败分别显示；保留“未执行建议、不代表漏洞验证或 Reviewer 通过”的说明。
- 前端只对当前任务视图中的 `receipt_pending + received` 卡片开放操作；后端是最终裁决者。前端卡片存在不能绕过任务、租约或原始证据校验。
- 用户切换任务、项目、attempt、A→B→A 或卸载组件后，旧请求不能写入当前错误状态，也不能解除新操作的发送锁。

## 允许补齐的精确条件

所有读取与写入均在同一个 SQLite `IMMEDIATE` 事务中，取得写锁后重新校验：

1. scan 存在、未软删除，仍为该 attempt；root 为相同 scan/attempt/target 的已终止 Coordinator。
2. directive claim 与当前保留的 coordinator ownership 记录的 root、epoch、fencing token 完全相同。
3. 允许 lease 的有效期已过，但只能用作历史归属证明；不续租、不取得 live capability，不绕过其他执行入口的过期拒绝。
4. 原始 `taskClosure` 为 `receipt_pending`、来源状态为 `assigned`、禁止自动重试，且根终态码一致。
5. 重新加载已确认的源草案，校验 hash/revision/text/thread/fact 引用及 assignment/child 的角色、目标、attempt 和 fencing 关联。
6. 首次补齐必须为 `deferred` 指令、`received` 提案、paused 且未结算的 assignment/child；不接管 prepared/executing/uncertain 或其他状态。
7. 原请求 mailbox 的完整路由、关联、revision、payload 和已投递/确认状态必须匹配冻结输入。
8. 保存的响应及用量通过校验；实际用量不得超过该 assignment 预留，cached 用量不得超过 total。

不接受已替换 ownership 的旧租约、其他 scan、旧 attempt、活动 root、删除任务、损坏草案/绑定或未知模型结果。它们需要不同的人工对账/修复工作流，不能由此接口猜测结果或费用。

## 原子写入与幂等

同一事务内：

1. 将原保存用量计入预算账本，释放本 assignment 预留的余额。
2. paused child 直接转为 terminal，绝不暂时改回 running；assignment 结算，权限保持撤销，清理对应 lane。
3. 写入有确定 dedup key 的 `human_assessment_result`，验证完整消息路由与内容，再确认投递。
4. 再次读取并验证已确认回执，防止 SQL 成功但触发器跳过实际更新。
5. 提案及指令写入 completed/failed；原 `taskClosure` 和原指令 `finished_at` 保持不变。
6. 追加不可修改的 `payload_json.localReconciliation`，含响应、用量、原收口记录的 hash，结果消息/assignment/child ID 及补齐时间。
7. 提交前验证 child、assignment、结果关联、费用及权限撤销状态。

`localReconciliation.modelRequests=0` 和 `targetRequests=0` 表示**本次补齐没有新增请求**，不表示原模型调用免费；夹具原调用使用 20 tokens / 1 request，补齐后账本记录此实际成本。

相同请求重放时，核对不可变回执、响应/用量 hash、消息确认、child/assignment 结算、预算归属和权限状态；不新增事件、不再次收费。并发请求只提交一份回执。无效但已保存的模型评估记为 failed 并结算原调用，而不是退款或假称完成。

## 测试与已发现缺陷

新增 `agent_tests_directive_reconciliation.rs`，覆盖：

- 有效/无效保存响应，过期但未换代的 ownership，原始收口保持不变。
- 精确结算、重放无写入、不可变回执与 Native 时间线投影。
- 预算、child、assignment、lane、消息插入/确认、proposal、directive 各阶段注入失败，比较全表快照证明事务回滚。
- 并发请求单回执、单次 delivery/费用。
- prepared/executing/uncertain/活动 root、旧 attempt/旧 fencing、软删除、错 scan、损坏 source/input/revision/target/ack 的拒绝。
- 真实 SQLite 写锁等待后替换 fencing，旧请求必须拒绝。
- 重放时发现损坏结果 payload、确认、用量、结算标识或重新开放 capability，必须拒绝且不写入。

故障注入确实暴露并修复一个缺陷：`UPDATE` 被 `RAISE(IGNORE)` 静默跳过时，旧代码仍返回成功。新增提交前 `verify_result(..., true)` 后，必须取得已落盘的有效确认。修复前失败证据：`/tmp/oviraptor-20260926-reconciliation-red.log`。

聊天增加 6 项实际 SFC setup/template 测试：按钮条件、结果文案、HTML 转义、当前卡片身份/参数、双击防重与 task/project/round-trip/attempt 四类竞争。它们不是浏览器视觉验收或真实 Tauri IPC 自动化。

## 验证结果

本次实际执行通过：

- `cargo test --offline --all-targets --all-features -- --test-threads=1`：主库 **728 项**、历史导入工具 **30 项**；新增补齐回归 **9 项**包含于主库。
- `cargo clippy --offline --all-targets --all-features -- -D warnings`。
- `cargo fmt --all -- --check` 及 `git diff --check`。
- `npm run test:agent-dialog`：**38 项**。
- `npm run build`：通过，主 JS **777.36 kB**，大 chunk 警告仍在，不能宣称性能优化完成。
- `node tools/test_native_runtime.cjs`：本地浏览器回环 **8 次请求**、匿名及身份对照完成，identity isolation 通过。

日志：`/tmp/oviraptor-20260926-reconciliation-{full,clippy,fmt,ui,build,native}.log`。此处仅证明本地回归，不等价于发布包/真实 Tauri IPC/浏览器视觉/用户授权目标验收。本轮未改变 Strix retirement allowlist，也未削弱 live lease/工具执行门禁。

## 仍需继续

- 未收到/无法确认的 provider 响应，以及旧 fencing 的成本/结果人工对账。
- Executor/Authorization 通用未结算暂停 child 的恢复方案。
- scan 级未绑定指令、异常历史绑定和显式修复入口。
- Reviewer 新证据获取、冻结新 revision 与再审闭环；其余专家真实执行链。
- 沙箱、工具供应、部署与授权目标验收；前端分包和视觉验收。

此次没有部署，没有请求用户提供的公网目标，没有实现或启用 Linux/Windows 主机执行能力。
