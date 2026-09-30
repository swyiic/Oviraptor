# NEST 轮次准备 checkpoint 清理锁定审计（Loop4，2026-09-30）

**整体仍未完成；本批只加覆盖、不改语义。`result_ingestion_runs.rs:99` 的清理行为此前零测试覆盖，本轮用回归测试锁定。**

## 1. 动机（无红测，本批是覆盖补齐）

- `prepare_current_attempt_surface`（`result_ingestion_runs.rs:1-122`）在每次 attempt 准备时无条件删除 `sentinel_checkpoints` 中 `strix_run/strix_events/strix_coverage`（精确与前缀）及 `learning_outcome` 行。
- 既有测试（`tests_result_attempt_surface.rs` 前两个用例）只覆盖 findings 与 marker，不覆盖 checkpoints。
- 读路径已是 stage 限定的（`agent_backend.rs:39`、`multi_agent/findings.rs:144` 按 stage 参数查询），DELETE 属于纵深清理而非隔离机制——删不得、留须有据，故先锁定。

## 2. 最小实现（测试约 30 行，生产 0 行）

- `tests_result_attempt_surface.rs` 新增 `resume_prepare_clears_retired_checkpoints_but_keeps_current_ones`：
  - resume 模式预置 `strix_run:old`、`strix_events:old`、`learning_outcome`、`frontend_recon` 四行；
  - 准备后断言仅剩 `frontend_recon`。
- 诚实说明：新测试首跑即绿（刻画型测试），没有红→绿过程；其价值是把未覆盖的退役关键路径变为已覆盖。

## 3. 待人工裁决（Blocked 项，不擅自改）

- `learning_outcome` 写端（`knowledge_learning_candidates.rs:238-244`）注释称其为扫描级耐久结果（“Keep the terminal scan summary intact”），读端准备函数把它与退役 `strix_*` 同等清除，两处意图矛盾。
- 在 resume 场景下，若新轮次学习未完成，旧 `learning_outcome` 将丢失且无恢复路径。
- 需要人确认：A) 把 `learning_outcome` 移出 L99 的 DELETE（耐久语义）；B) 维持现状（轮次内语义），并修正写端注释。确认前不改生产代码。

## 4. 验证

| 范围 | 结果 | 本地证据 |
| --- | --- | --- |
| 本文件 3 用例（2 旧 + 1 新） | 3/3 通过 | `/tmp/oviraptor-checkpoint-loop4-rust.log` |
| `cargo fmt --all -- --check`、`git diff --check` | 通过 | 同上 |

未运行：完整 Rust 全量、完整 UI、构建、安装包。生产代码零改动，故未跑 clippy 全量。
