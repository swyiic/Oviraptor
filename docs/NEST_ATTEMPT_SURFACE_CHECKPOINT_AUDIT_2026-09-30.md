# NEST 轮次准备 checkpoint 审计（2026-09-30）

**整体 Master Plan 未完成。** 本文描述当前可执行语义；原 Loop4 首版仅锁定“resume 时连同学习结果一起清除”的行为，后续审阅写端后已更正，不再是待人工裁决项。

## 当前合同

- 学习写端 `knowledge_learning_candidates.rs` 按扫描 ID 独立写入／覆盖 `learning_outcome`。它代表扫描级最后一次学习结果，不是旧 attempt 的临时 checkpoint。
- `prepare_current_attempt_surface` 在 Web `resume` 准备时清除退役 attempt checkpoint（`strix_run`、`strix_events`、`strix_coverage` 的精确与前缀阶段），保留 `learning_outcome` 与当前 `frontend_recon`。重复准备也不能清掉学习结果；下一次学习写入可以覆盖它。
- Web `fresh` 是完整结果面重建，仍清空所有 checkpoint。旧 findings／marker／signature 的隔离清理未在本批移除，不能因退役诉求直接让旧行重新进入当前结果页。
- 未清理任何真实用户数据库或历史源报告；保留扫描级学习结果也不赋予旧数据执行权限。

## 代码与回归

- `result_ingestion_runs.rs` 只从退役 checkpoint DELETE 条件移除 `learning_outcome`，其余清理与 marker 逻辑保持原样。
- `tests_result_attempt_surface.rs` 的 `resume_prepare_keeps_learning_outcome_and_current_recon` 预置两条旧 checkpoint、学习结果和当前 recon；断言旧行消失、当前行保留，并再次调用准备函数核对幂等性。另两条原有 marker／findings 回归保持通过。
- 退役字面量登记只对这两个审阅过的文件更新 SHA-256、实际出现次数与理由，没有批量重算或放松守卫。

## 验证与剩余

关联轮次测试 3/3、退役字面量守卫 1/1、完整 Rust 库测试 1513/1513（离线、低优先级、单线程，约 1688 秒）、全目标／全特性严格 Clippy、Rust fmt、`git diff --check` 通过。完整 UI、其他 Cargo 目标、安装包、真实桌面和授权 URL 未在本批运行。

这不表示 Strix 字段／存量数据已清零，也不表示多智能体、实时链路和发布验收完成。后续数据清理必须先精确盘点、备份和确认，不得以本回归通过为由删除真实数据。
