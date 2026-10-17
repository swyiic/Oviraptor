# NEST 合同 owner 调度接线审计（Loop6，2026-09-30，A6）

**整体仍未完成；本批把 Loop5 原语接入调度器同事务，预算账本与 attempt 表仍未动。**

## 1. 接线位置与语义

- `scheduler/assignment.rs`：`active` 检查通过后、lane 占取前，同事务 `acquire_contract_owner`。
  失败（抢占/旧 fencing）直接返回 Err，调用方不 commit 即整体回滚：lane、预算预留、run 行、capability lease 与 owner 行同生共死。
- v1 key = `[attempt, target, role, trigger_code, revision]`，经 `schedule_contract_key` 唯一定义，调度器与测试共用。
  跨 trigger 的逻辑去重（去掉 trigger）有意推迟：`task_slice` 尚无稳定 action 字段，过早模糊 key 会误拦合法派发。记为后续项，不在本批硬上。

## 2. 测试

- `commands/multi_agent/tests/contract_owner.rs`（新，约 90 行）+ 1 行 include：
  - 调度一次 → 恰好 1 行 owner（assignment/epoch/fence 与 lease 一致）；同输入重调度 → Ok 且仍 1 行（幂等）。
  - owner fencing 带外前滚后，原 lease 重调度 → `stale_contract_fencing`，且 lane 仍只有首调度的 1 行（回滚完整）。
- 回归：scheduler/budget/lane/child/specialist 过滤集 99/99 通过（53.52s），既有幂等与并发语义未破。

## 3. 门禁插曲（失败证据）

- `clippy --all-targets --all-features -D warnings` 报 `doc_lazy_continuation`（key 函数文档换行缩进），修文档后通过。
   coincidental 发现：`--lib` 口径窄于门禁，Loop5 起统一按 gate 口径验证。
- Loop5 的模块级 allow 已摘除（acquire/key 已被生产调用）；`release_contract_owner` 仍只有测试调用，保留 item 级 staging allow，待终态回收接线。

## 4. 验证

| 范围 | 结果 | 本地证据 |
| --- | --- | --- |
| 新 2 项 + Loop5 原语 2 项 | 4/4 通过 | `/tmp/oviraptor-contract-wire-loop6-rust.log` |
| 调度/预算/lane/specialist 回归集 | 99/99 通过 | 本轮终端输出 |
| `clippy --all-targets --all-features -D warnings` | 通过 | `/tmp/oviraptor-contract-wire-loop6-clippy.log` |
| `cargo fmt --all -- --check`、`git diff --check` | 通过 | 同上 |

未运行：完整 Rust 全量、完整 UI、构建、安装包。调度行为新增失败模式（旧 fencing 重放），UI 无影响。

## 5. 剩余工作

1. A6 另两缺：`assignment_attempts` 表（lease/逻辑分离 + Expired 语义）、append-only 多维预算账本（维度/kind/幂等键/indeterminate）。后者动现钱，下一步先出只读对账盘点，不直接改写。
2. v1 key 跨 trigger 去重：等 `task_slice` 稳定 action 字段后再收紧，收紧前不得宣称“同一合同只执行一次目标调用”已全证。
3. Blocked 沿用：A10 安装包/真机、Loop4 `learning_outcome` 耐久裁决。
