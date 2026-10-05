# 旧 attempt 人工建议回执的本地对账审计（2026-09-28）

## 结论与边界

这是专用于 **已保存的只读人工建议评估** 的历史对账，不是旧任务续跑、跨 attempt 恢复或通用子智能体恢复。旧记录仍不混入当前聊天时间线；状态页仅显示未核验的身份列表与数量。用户需要逐条选择、再次确认，后台才尝试原 attempt 内的本地对账。拒绝或失败不会自动重试模型、访问目标、续租、创建任务或给予旧 worker 新能力。

完成后只能说“该建议的原模型用量及本地回执已结算”，不能据此称漏洞已验证、Reviewer 已通过或扫描覆盖充分。旧 attempt 的 Coordinator root 仍保持终态；当前 attempt 的租约与预算不得被使用。

## 实际入口与约束

- `get_native_scan_status` 在同一读取事务中返回 `historicalPendingReceipts` 和最多 50 条 `historicalReceiptItems`（directive ID、原 attempt、创建时间、固定 `verified:false`）。列表出现只证明有待处理行，不证明响应可信或可结算。超出 50 条仍在数量中显示，不静默标记完成。
- 协同台为每条旧记录提供两步人工确认；只提交精确 `scanId / attemptNumber / directiveId` 至独立 Tauri 命令 `reconcile_historical_scan_directive_receipt`。原 `reconcile_scan_directive_receipt` 继续仅接受当前 attempt。扫描／attempt／项目切换、状态丢失或组件卸载后，旧异步回调不能发布为新任务的成功。
- 历史命令以 `BEGIN IMMEDIATE` 获取写锁后校验扫描未删除、当前 attempt 大于原 attempt、原 Coordinator root 已终止、原 lease epoch/fencing 与 directive claim 完全一致。冻结请求、草案 hash、assignment、目标、模型输出与用量、唯一 `model_round_completed` 事件、回执交付和原预算账本均在同一事务核验。任何一步失败整体回滚。
- 成功写入原 root 的 ledger、旧 child/assignment 终态、结果 mailbox 及 `localReconciliation`；重复或并发请求通过已存回执重验并返回同一结果，不二次记账。旧 root 不重开。

## 验证

- 历史有效回执结算、旧 API 拒绝跨 attempt、篡改原 lease/草案/assignment/事件/ledger 与扫描删除后的零写入、并发幂等、写入失败整体回滚：针对性 Rust 回归通过。
- 当前 attempt 的 `directive_reconciliation` 9 项通过；协同台全套 UI 251 项通过；前端生产构建、严格全特性 Clippy 与格式检查通过。
- `CARGO_BUILD_JOBS=1 cargo test --offline -j 1 --all-targets --all-features --quiet -- --test-threads=1`：主库 **1368/1368**、集成 **30/30**，exit 0。未运行授权 URL、未部署、未进行真实登录或目标扫描。

## 仍未完成

Master Plan 的通用 specialist/Reviewer/Investigator 终态恢复、外部授权环境验收、完整运行级闭环不能由本次人工建议对账替代。若原 lease/事件/冻结记录缺失或被篡改，系统拒绝结算并保留待处理记录；不能把这种拒绝自动改成重新执行旧任务。
