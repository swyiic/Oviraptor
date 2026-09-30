# NEST 历史来源专用分支持续退役审计（Loop2，2026-09-30）

**整体仍未完成；本批仅删除 `appsec_validation` 中的旧 stage 特权分支，不改动执行、授权、预算、Reviewer 或真实存量。**

## 1. 红测复现

- 新增 `src-tauri/src/commands/tests_historical_source_branch.rs`（约 45 行），在同一 scan 下分别插入 `stage='strix'` 与 `stage='nightly'` 的无代码、无运行时证据行，调用 `appsec_candidates`。
- 删除前失败证据：`strix` 行返回 `["ai_validation"]`，期望 `["scanner"]`：
  - `left: ["ai_validation"] right: ["scanner"]`，`source_key: strix:vulnerability:old-1`。
- 该分支把旧来源 findings 送入本应只属于人工 Repeater 验证（`save_investigation_validation`，`source_type='ai_validation'`）的类型空间，并受 `DELETE ... WHERE source_type <> 'ai_validation'` 的保留语义影响，属于 REM-009 禁止的旧来源专用分支。

## 2. 最小实现（生产代码 5 行净删除）

- `src-tauri/src/commands/appsec_validation.rs:197-203`：空 `source_types` 回退统一为 `"scanner"`，删除 `if stage == "strix"` 特权分支并注明 `ai_validation` 仅属于人工 Repeater 验证。
- `source_key` 仍保留原始 `stage` 用于可追溯性，不掩盖来源；不删除旧行，不改动 `sentinel_findings` 投影。
- 新增测试文件 1 个（<60 行），`tests.rs` 增加 1 行 include；无新依赖、无 DB 迁移、无 API 变更。

## 3. 验证

| 范围 | 结果 | 本地证据 |
| --- | --- | --- |
| 本批新测试 + 同文件关联回归（sast/dast 相关、CI 门禁） | 3/3 通过 | `/tmp/oviraptor-historical-branch-loop2-rust.log` |
| `cargo fmt --all -- --check` | 通过 | `/tmp/oviraptor-historical-branch-loop2-fmt.log` |
| `cargo clippy --lib -- -D warnings` | 通过（16.89s 编译） | 本审计 §4 终端输出 |
| `git diff --check` | 通过 | `/tmp/oviraptor-historical-branch-loop2-diffcheck.log` |
| 旧分支残留复核 | `stage == "strix"` 在该文件中 0 命中 | `BRANCH_GONE` |

未运行：完整 Rust 全量（1504 项）、完整 UI、生产构建、安装包、真机延迟。下一步涉及后端语义时补跑对应模块全集。

## 4. 剩余工作

1. `result_ingestion_runs.rs:74,99` 的旧 `stage IN ('strix',...)` / `strix_run:%` 清理分支仍作为过渡清理保留，需在历史只读投影确认旧行不再进入现行结果面后拆除，不可直接删除导致旧行复活可见。
2. `strix-current-attempt / strix-result-signature` 标记清理（同文件 7,107,119 行）同上，需待旧库升级路径确认。
3. skills/knowledge 三表自动 copy（`db_neutral.rs:62-113`）仍是产品级迁移依赖，按 §11.4 需先做只读盘点/预览/备份，再拆除；本批未动。
4. `ReleaseNotesDialog.vue` 归档文案、recon 旧目录回退、migration 5 文件、安装日志持久回放、外部脚本通知、真实桌面验收仍未完成。
