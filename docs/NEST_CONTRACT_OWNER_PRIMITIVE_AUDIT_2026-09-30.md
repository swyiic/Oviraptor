# NEST 合同唯一 owner 原语审计（Loop5，2026-09-30，A6）

**整体仍未完成；本批只交付 §5.3 的表 + 原语 + 回归，不接调度器，不碰预算。**

## 1. 背景（看代码结论）

- 现状单 owner 靠 `agent_lane_leases` 容量 1（`scheduler/assignment.rs:254-278`）+ 确定性 `assignment_id`（同文件 186-190）近似保证，没有 Master §5.3 要求的 `agent_contract_owners(root,contract_key)` 独立表与 fencing 语义。
- 预算账本仍是单行汇总（`db_schema.rs:1078-1093`），改 append-only 属于动现钱迁移，风险最高，另起 Loop，不在本批。
- 最小切口：新增表 + acquire/release 原语（纯加法，无存量迁移），调度接线放 Loop6。

## 2. 红→绿（两次失败证据）

1. `E0583/E0433`：先加 `pub mod contract_owner` 声明与测试引用、后写实现，编译失败证明测试先行。
2. 首跑回归 1/2 失败：释放后重取被 `contract_not_held` 拒绝。裁决为“释放即告一段落、可重新获取并保留 released_at 审计”，修原语后 2/2 通过。
3. 门禁 clippy（`--all-targets --all-features -D warnings`）7 项 dead-code：新原语暂只被测试调用。按仓库既有惯例（`tool_supply.rs`、`assignment.rs` 的带理由 allow）加 staging-only allow，Loop6 接线时摘除；Loop3 的两个 item 同理补 allow。之后门禁全绿。

## 3. 最小实现

- `db_schema.rs`：`agent_contract_owners(root_run_id,contract_key PK,assignment_id,lease_epoch,fencing_token,state held/released,result_node_id,acquired/released_at)`。`assignment_id` 故意不用 FK（同事务先占 owner 后插 assignment）。`initialize` 每次全量跑 `CREATE TABLE IF NOT EXISTS`，新旧库同覆盖，无需额外迁移。
- `multi_agent/contract_owner.rs`（141 行）：`contract_key`（`\u{1f}` 连接防碰撞）、`acquire_contract_owner`（同 owner 幂等、异 owner `contract_owned_by_other`、旧 epoch `stale_contract_fencing`、released 可重取并审计）、`release_contract_owner`（仅 owner + 同 fencing 可释放，行保留）。
- `stage1a_tests/contract_owner.rs`（48 行）+ 1 行 include：单 owner/抢占拒绝/旧 fencing fail-closed/非 owner 释放拒绝/释放后重取/root 隔离。

## 4. 验证

| 范围 | 结果 | 本地证据 |
| --- | --- | --- |
| 新测试 2 项 | 2/2 通过 | `/tmp/oviraptor-contract-owner-loop5-rust.log` |
| `clippy --all-targets --all-features -D warnings` | 通过 | 本轮终端（31.30s 编译） |
| `cargo fmt --all -- --check`、`git diff --check` | 通过 | 同上 |

未运行：完整 Rust 全量、完整 UI、构建、安装包（生产行为零变化：调度器未调用新原语）。

## 5. 剩余工作

1. Loop6：调度器 `schedule_child_in_transaction` 内同事务 acquire（key 组装规则 + 冲突错误映射 + 并发回归），届时摘除本批 allow。
2. A6 另两缺：`assignment_attempts` 表、append-only 多维预算账本（含 indeterminate/幂等键），后者需动现钱，先出对账只读方案再动手。
3. Blocked 沿用：A10 安装包/真机、Loop4 `learning_outcome` 耐久裁决。
