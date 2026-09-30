# NEST 历史来源专用 UI 标签中性化审计（Loop1，2026-09-30）

**整体仍未完成；本批仅完成 REM-009 的展示层品牌字面量清除，不涉及执行、授权、预算、Reviewer 或真实数据清理。**

## 1. 范围与合同依据

- Master Plan §2.1-10、§11.5、§13.1 REM-009：当前 UI、Tauri API、Rust/TS 类型和 CSS 无旧命名及专用历史来源标签；不得把残留数据改标为 Native 来掩盖来源。
- 本批仅处理 `historical_label` 残留中的展示字符串，不删除数据兼容条件，不改动后端投影、数据库迁移或真实存量。
- 不自动 commit/push，不删除真实用户数据库、CAS、不可变 revisions 或源报告。

## 2. 红测复现（删除前失败证据）

- 检查脚本对 `src/features/sentinel/execution/presentation.ts`、`src/features/sentinel/presentation.ts` 按行匹配 `/strix/i`，删除前命中 4 行：
  - `execution/presentation.ts:5-6`：`backend === "strix"` 条件 + `return "Strix（历史只读）"`
  - `presentation.ts:373-374`：`engine.includes("strix")` + `? "Strix（历史只读）"`
- 结论：展示层存在品牌专用历史标签，REM-009 未达标。

## 3. 最小实现（3 文件，共约 15 行变更）

1. `src/features/sentinel/execution/presentation.ts`
   - 显示统一改为 `"历史封存（只读）"`，`native -> "原生 Agent"` 不变，未知 -> `"未记录"` 不变。
   - 保留 `backend === "strix"` 条件并注明“旧数据兼容，仅展示中性，不参与后端选择/恢复/执行准入”，待 DB 清理后拆除。
2. `src/features/sentinel/presentation.ts:370-377`
   - `engine.includes("strix")` 分支显示改为 `"历史封存（只读）"`，同上注明旧计划兼容。
   - 不改变 `native` 优先判断，不把未知 engine 冒充 Native。
3. `tools/test_execution_presentation.cjs`
   - 断言 `legacy_backend_removed` 与 `strix` 均返回 `"历史封存（只读）"`。
   - 新增负向断言：展示模块源码不含 `Strix（历史只读）` 及品牌+历史组合字面量。

未改动项：`SentinelVulnerabilitiesPane.vue` 展示文本本已中性（“历史外部·未经原生审核”），其 `stage === 'strix' / source === 'strix'` 条件属于旧数据兼容，本轮不扩校验语义，避免掩盖来源；后端 `historical_external / legacy_backend_removed` 投影不变。

## 4. 验证（集合有重叠，不相加）

| 范围 | 结果 | 本地证据 |
| --- | --- | --- |
| 执行展示+详情+Fuse 两套 | 52/52 通过 | `/tmp/oviraptor-historical-label-loop1-ui.log` |
| 品牌展示字符串检查 | 0 命中（DISPLAY CLEAN） | 上表日志内嵌断言 + `test_execution_presentation.cjs` 负向用例 |
| 残留字面量复核 | 条件兼容 4 行（2 代码+2 注释），展示品牌 0 行 | `/tmp/oviraptor-historical-label-loop1-residual.txt` |
| `git diff --check` | 通过 | `/tmp/oviraptor-historical-label-loop1-diffcheck.log` |

未运行：完整 Rust 全量、完整 UI 765 项、生产构建、安装包、真机延迟。本批为前端展示变更，无后端语义变化，故未跑 Cargo 全量；下一步涉及后端/迁移时必须补定向 Rust 回归 + 严格 Clippy/fmt。

## 5. 剩余工作（不得据此勾选 REM-009 完成）

1. 条件兼容字面量仍有 4 行（旧 `backend === "strix"`、`engine.includes("strix")` 及配套注释），需在真实旧数据清理（精确对象/备份/预览/确认）后拆除。
2. `ReleaseNotesDialog.vue` 内 10+ 处 Strix 历史文案属于发布说明归档，需另起增量决定保留为历史文档还是中性改写，本批未动。
3. `SentinelVulnerabilitiesPane.vue:56,80`、`appsec_validation.rs:198`、`result_ingestion_runs.rs:74,99` 等旧 `stage === 'strix'` 数据分支仍在，需随存量清理逐项退役。
4. 后续 Loop 仍须处理：recon 旧目录回退、skills/knowledge 旧别名、5 个 migration 字段、安装日志持久回放、外部脚本通知、真实桌面验收。
