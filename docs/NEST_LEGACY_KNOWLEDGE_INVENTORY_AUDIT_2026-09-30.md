# NEST 旧知识三表只读盘点审计（Loop3，2026-09-30）

**整体仍未完成；本批仅新增只读盘点能力，不复制、不推进水位、不删除，为 §11.4 拆除迁移依赖提供“精确清理预览”的前置条件。**

## 1. 红（失败证据）

- 新增 `legacy_knowledge_inventory()` 初版返回 `Ok(())`，与签名 `Result<Vec<LegacyKnowledgeEntry>, String>` 不符，`cargo test --no-run` 编译失败：
  - `error[E0308]: mismatched types ... expected Vec<LegacyKnowledgeEntry>, found ()`（`src/db_neutral.rs:358`）。
- 修复为 `Ok(out)` 后，新测试一次通过（下节）。中间编译失败保留为本批的失败证据，不隐藏。

## 2. 最小实现（生产约 55 行，测试约 75 行，均远低于 400 行上限）

- `src-tauri/src/db_neutral.rs`：新增 `LegacyKnowledgeEntry`（legacy/neutral 名、旧表存在性、旧行数、旧最大 id、中性行数、迁移水位）与 `legacy_knowledge_inventory()`，全程只 SELECT：
  - 旧表缺失 → 0 行/0 id，不报错（新库场景）；
  - 旧表存在 → COUNT(*) + MAX(id)，中性表 COUNT(*)，水位取 `neutral_knowledge_migration` 现值，原样返回，不写回。
- `src-tauri/src/db_neutral_tests.rs`：新增 `legacy_inventory_reports_exact_counts_without_copying_or_cleaning`：
  - 新库：三表均 `legacy_exists=false`，0 行；
  - 升级库（`seed_legacy_history`：skills 2 行/max 8、knowledge 1 行/max 11、candidates 1 行/max 13）：盘点值精确相等，水位 == 旧最大 id，中性行数 ≥ 旧行数；
  - 只读性：连续两次盘点结果相等，`neutral_knowledge_copy_is_faithful` 仍过，三旧表仍保留（清理批准前不得删）。

## 3. 验证

| 范围 | 结果 | 本地证据 |
| --- | --- | --- |
| 新测试 + 同模块迁移回归（copy/faithful/backend_default/rewrite/neutral×7，集合重叠） | 15/15 通过（8+7） | `/tmp/oviraptor-inventory-loop3-rust.log` |
| `cargo fmt --all -- --check`、`git diff --check` | 通过 | 同上轮门禁日志习惯 |
| 旧表保留复核 | 三旧表迁移后仍存在（未删） | 新测试内断言 |

未运行：完整 Rust 全量、完整 UI、生产构建、安装包、真机延迟。后端只读新增无前端影响，故未跑 UI。

## 4. 剩余工作

1. 拆除 `NEUTRAL_COPIES` 自动 copy 仍需“当前表引用核实 + 拒绝旧库恢复路径 + 本盘点确认 + 备份”四件套，本批只交付盘点，第 2–4 件未做，不得删迁移。
2. `result_ingestion_runs.rs` 旧 stage 清理分支、`strix-current-attempt/signature` 标记清理仍为过渡保留。
3. 安装日志持久回放、外部脚本逐路通知、安装包/真机验收仍未完成（见 progress.md A9/A10）。
