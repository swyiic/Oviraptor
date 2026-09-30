# 目标驱动进度（Goal: NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md）

## Goal（要做什么）
1. Strix-free Native Runtime：删进程/CLI/安装/升级/镜像/backend选择/fallback，旧 reader 全部退役。
2. 可审计联合多智能体：Coordinator 唯一租约+fencing、Assignment/attempt 分离、合同 owner、多维预算账本、不可变 evidence revision、typed mailbox+ack、独立 Mapper/执行专家/Reviewer run、Reviewer 强制门禁。
3. 四类任务全 Native：Web / Code / Greybox / CI，缺能力只报 gap，不断回退。
4. 历史报告无损只读导入（canonical artifact import），与执行面隔离。
5. 实时展示逐路验收 + 安装包/真机发布验收。

## Scope（不做什么）
- 不保留 Strix 历史兼容、字段别名、旧目录回退、旧配置自动提升。
- 无“精确对象+数量+备份+用户确认”不碰真实用户 DB / CAS / 不可变 revision / 源报告。
- 不借退役删授权、目标范围、审批、证据核验、租约隔离、预算/Reviewer/清理门禁。
- 缺密码/支付/真机短信/删库授权 → 记 Blocked 停下，不瞎试。
- 单步循环，一次只做一件事；同一验收连续 3 次不过就换方案。

## Acceptance（可验证）
| ID | 验收 | 验证方式 | 状态 |
|----|------|----------|------|
| A1 | 构建门禁全过 | `cargo fmt --check` + `cargo test` + `clippy -D warnings` + `npm run build` + `git diff --check` | 🟡 定向过，全量未收 |
| A2 | 静态残留零活动路径 | §14 rg（strix/backend kind/旧命令/旧配置键） | ❌ 52 文件残留登记；Loop1/2 清掉 2 处展示+1 处分支 |
| A3 | 历史导入合同 | IMP/COR/IDM 定向回归（旧格式拒绝，现格式往返） | 🟡 定向过，全量未收 |
| A4 | 中性运行时+DB迁移 | 新库无 `strix_*` 活表、`agent-jobs`、backend 默认 native、旧 run 封口 | 🟡 部分（迁移依赖未拆） |
| A5 | 四类任务 Native | CODE-/GRY-/CI-专项 | 🟡 Web/Code 主链通，Greybox 同图与 CI 全门禁未完 |
| A6 | 多智能体基础语义 | lease/fencing/mailbox/预算守恒/revision 并发+故障注入 | ❌ 缺 `assignment_attempts`/`contract_owners`/`receipts` 表、10 维账本降级为单行汇总 |
| A7 | 调度+独立角色+Reviewer 门禁 | 独立 run、lane 容量 1、无 Review confirmed=0 | 🟡 主链 4 角色通，其余专家缺 |
| A8 | 协同台 | 草案四态、真实事件、无伪造 typing、无 Secret 泄漏 | 🟡 部分 |
| A9 | 实时化 | 提交唤醒；安装日志持久回放；外部脚本逐路 | ❌ 安装日志仍瞬时 300 条；部分写入靠 15s 兜底 |
| A10 | 发布验收 | 安装包 + 真机延迟 + 长稳 CPU | ❌ 未做（Blocked：需安装包签名环境与真机） |

**完成度：40%**（2/10 全过；6 部分；4 未过。数字=验收项加权，不是代码行数。）

## 2026-09-30 Loop1 ✅ A2-子项：历史 UI 展示标签中性化
- 改：`execution/presentation.ts`、`sentinel/presentation.ts` 历史后端展示 → `历史封存（只读）`；`strix` 条件保留作旧数据兼容。
- 验：`test_execution_presentation/details/fuse*` 52/52 过；展示品牌 0 命中；`git diff --check` 过。
- 记：`docs/NEST_HISTORICAL_LABEL_NEUTRALIZATION_AUDIT_2026-09-30.md`

## 2026-09-30 Loop2 ✅ A2-子项：旧 stage 特权分支退役
- 改：`appsec_validation.rs` 空 source_types 回退统一 `scanner`，删 `stage=="strix"→ai_validation`。
- 验：新测试红→绿 + 关联 2 项，3/3；fmt 过；`clippy --lib -D warnings` 过。
- 记：`docs/NEST_HISTORICAL_SOURCE_BRANCH_RETIREMENT_AUDIT_2026-09-30.md`

## 2026-09-30 Loop3（进行中）A4-子项：skills/knowledge 旧别名只读盘点
- 做：`db_neutral.rs` 新增只读 `legacy_knowledge_inventory()`，只 SELECT，不复制、不推进水位、不删除。
- 验（待）：`legacy_inventory_reports_exact_counts_without_copying_or_cleaning` 新测试红→绿 + `db_neutral` 全模块回归 + fmt。
## 2026-09-30 Loop3 ✅ A4-子项：旧知识三表只读盘点
- 加：`db_neutral.rs` 只读 `legacy_knowledge_inventory()`（COUNT/MAX/水位三元组，不复制不删除）；测试覆盖新库 0 行与升级库精确值（2/8、1/11、1/13）及双读一致性。
- 红：初版 `Ok(())` 类型错，E0308 编译失败，修复为 `Ok(out)` 后绿。
- 验：新测试+同模块回归 15/15 过；fmt、`git diff --check` 过。
- 记：`docs/NEST_LEGACY_KNOWLEDGE_INVENTORY_AUDIT_2026-09-30.md`
- 下一步 Loop4：`result_ingestion_runs.rs` 旧 stage/`strix-*` 标记清理分支的中性化评估（只读投影确认优先，不直接删）。

**完成度：42%**（A2 展示+分支子项过，A4 盘点子项过；A6/A9/A10 仍未过。）
