# Source Reviewer 顶层恢复审计（2026-09-27）

## 结论与边界

后续源码证据页与测试服务生命周期修复，以及包含本恢复代码的新完整组合验证，见 `NEST_SOURCE_FINDINGS_READ_AUDIT.md`。以下全量 1187/2 和补测记录保留为本恢复增量的真实历史，不抹去失败，也不代表后续版本的最终结果。

真实源码入口 `run_native_source_assessments` 已支持**前四个源码阶段完整交付、原 Coordinator 租约仍有效**时的 Reviewer 边界恢复。恢复使用真实持久化 assignment、child、模型回执、mailbox ACK、预算账本和冻结材料；不重放已经完成的初评/工具阶段。

这不是整个 Master Plan 完成，也不是任意位置的完整 crash recovery。更早的部分初评/工具阶段、过期租约下的核对/接管、未知模型结果的人工核对工作流仍需继续。测试经过实际 localhost HTTP 和磁盘 SQLite，并从新数据库连接重入生产顶层；**本轮没有进行操作系统级强杀进程/重新启动应用测试**。

## 恢复行为

| 持久化检查点 | 顶层行为 | 模型请求总数（本测试夹具） |
| --- | --- | --- |
| 四阶段完成，Reviewer 尚未创建 | 审计四阶段与已结算账本，创建唯一 Reviewer | 7 |
| Reviewer 已预留，尚无调用记录 | 核对无调用历史、精确权限及唯一未决预留，派发一次 | 7 |
| 已收到并保存响应，尚未交付 | 审计回执，仅本地交付与收口 | 7 |
| 保存响应后清理为 paused | 保留撤销状态，仅本地交付，不补发能力 | 7 |
| Reviewer 已交付，主任务未收口 | 审计已交付决定，完成唯一主任务收口 | 7 |
| 已派发但接收事务失败、结果未知 | 拒绝自动重发，不退款、不续权、不旋转 fence、不写主任务终态 | 保持 7，账本保留 6 已结算 + 1 未决 |
| 主任务已经终态 | 拒绝恢复，不复活、不重复收口 | 不增加 |

成功分支均检查 5 个 assignment、4 个历史阶段展示项、1 条 ACKed Reviewer 消息、1 个根 terminal-reduced 事件、7 次模型请求与 140 Token、预留归零。上述数值是夹具数据，不是生产任务固定调用次数或固定费用。

## 实现位置

- `src-tauri/src/agent_runtime/multi_agent/source_phases.rs`：从已审计的四阶段回执派生稳定排序的 assessments，恢复展示内容不靠重新请求模型。
- `src-tauri/src/agent_runtime/multi_agent/source_reviewer.rs`：只读 `ReviewProgress` 分类；区分尚未派发、已保存响应、已交付与未知结果；Reviewer 新派发必须同时持有精确绑定、未过期的 `evidence.read` 与 `review.write`，禁止多余活动权限。
- `src-tauri/src/commands/native_source_reviewer.rs`：在 Coordinator acquire 和通用错误收口之前进行只读准入；保存响应直接进入本地交付。无未决 Reviewer 或已经交付时复用完整收口证明；未决 Reviewer 的账本必须等于四阶段实际用量加唯一未决预留。
- `src-tauri/src/commands/native_source_coordinator.rs`：通过准入时沿用原 epoch/fence，不重新跑初评及源码工具阶段。
- `src-tauri/src/commands/tests_source_reviewer.rs`：真实 localhost 检查点、全表持久化快照与实际 HTTP 调用数断言。

每次恢复仍验证当前 scan/attempt、发布运行配置、材料、取消状态、原租约及原任务总时限。恢复不是重新开始计时。恢复准入不签发新权限；后续每个实际写事务仍有原有授权/绑定与写后检查。

## 本轮真实红测与修复

1. **重放已完成阶段**：原顶层重入报 `assignment_dedup_conflict`，试图覆盖已完成的工具阶段；日志 `/tmp/oviraptor-source-reentry-red.log`。修复为读取完整四阶段证明并进入 Reviewer 阶段，保存响应直接交付；初次绿测 `/tmp/oviraptor-source-reentry-green.log`，1 项通过。
2. **撤权后错误终态化**：尚未派发 Reviewer 的能力被撤销时，虽然没有发 HTTP，旧恢复路径仍写入主任务错误终态。日志 `/tmp/oviraptor-source-reentry-matrix.log`，3 通过/1 失败。将权限审计前移到准入，并在调用 claim 写入前后复核双权限；修复后 `/tmp/oviraptor-source-reentry-matrix-green.log`，4 项通过。
3. **损坏账本后错误写入**：`spent_tokens` 与四阶段回执不符时，旧恢复路径在拒绝过程中修改协作事件。日志 `/tmp/oviraptor-source-reentry-accounting-red.log`，1 项失败。增加准入时完整/未决账本审计后，`/tmp/oviraptor-source-reentry-final-matrix.log`，5 项通过。
4. **完整回归发现旧退役登记失效**：`retirement_literal_allowlist_matches_reviewed_sources_and_packaged_inputs` 检测到前序 `agent_tests_backend_residual.rs` 改动未重新登记。重新审阅整个 490 行测试文件及全部 16 处旧名称：仅注释、fixture 路径、拒绝/零旧 run 断言和不执行的 PATH 陷阱；正向可执行对照已改为中性名称，正则扫描覆盖增强，活动 baseline 保持空。仅更新该条既有 fixture 登记的理由、全文摘要（`39c85d5b…` → `b7a01abb…`）和次数（17 → 16），没有新增放行文件或扩展扫描排除。首轮失败日志 `/tmp/oviraptor-source-reentry-full.log` 必须保留，不可宣称首轮全绿。

5. **完整回归发现新增数据漏登记**：`legacy_fixtures_match_their_frozen_manifest` 报冻结清单缺少 `residual_syntax.json`。逐文件核对完整目录后，唯一差异是这份新增的 12 例残留语法检测数据；已有 producer 文件的字节、长度、摘要全部匹配。仅在 `manifest.json` 新增该文件的精确摘要 `f10aadc4b1586e1e4890122e082a73dd5b9611ce21e056dbc4a73456cbd2b5dc` / 970 字节，不改历史文件，不增加扫描排除。该文件是合成检测数据，不冒充历史 producer 产物。

失败日志是保留的修复证据，不能当成通过。本轮采用失败项补测，未再次运行全量；不可把分别通过的结果写成一次完整全绿。

## 故障覆盖

- 真实第七次 HTTP 调用返回后，通过事务故障使 `received` 保存失败；保留 `executing` 日志。恢复不伪造回执、不再次请求、不退款。该测试不是手工插入一条假响应。
- 恢复前破坏阶段 ACK、冻结 slice、子任务取消/attempt、原租约有效期、根启动时间、scan 状态、模型配置、无日志子任务用量，均拒绝。
- 未派发 Reviewer 的全部权限撤销、仅写权限撤销/过期/fence 改变/替换为 HTTP 权限，均拒绝且数据库业务表逐行保持不变。
- 五个可恢复边界分别破坏已用 Token 账本，拒绝且不交付、不增加调用、不改业务表。
- 调用 claim 写入触发撤销写权限、改变写权限 fence 或插入额外 HTTP 权限时，整个 claim 事务回滚，Reviewer 不发 HTTP。
- 已成功收口后再次进入顶层，拒绝复活且所有业务表保持不变。

全表快照在故障注入之后、恢复调用之前取得；使用独立连接的真实顶层重入结果与其比较，不把调用方 connection 的 `total_changes()` 当成全局无写入证明。快照不涵盖日志文件、SQLite 物理页或操作系统副作用。

## 当前验证

- 定向恢复矩阵：`/tmp/oviraptor-source-reentry-final-matrix.log`，5/5 通过，exit 0。该五项及后来增加的派发 claim 写后权限回归均在以下全量运行中通过。
- 全量 Rust session 78274：`/tmp/oviraptor-source-reentry-full.log`，exit 101；主库 **1187 通过 / 2 失败**，661.34 秒。失败仅为以上两项登记，主库失败后 Cargo 未执行完整导入器 target。保留红测结果。
- 修正登记后的退役守卫补测：`/tmp/oviraptor-source-reentry-retirement-recheck.log`，主库 **4/4**，exit 0。
- 修正登记后的历史兼容补测：`/tmp/oviraptor-source-reentry-legacy-recheck.log`，主库 **30/30**、导入器相关 **3/3**，exit 0。两项原始失败均已实际复测通过；之后没有修改 Rust 实现或测试源码。
- 严格 Clippy：`/tmp/oviraptor-source-reentry-clippy.log`，全部 target / feature，`-D warnings`，exit 0。
- 完整历史导入器：`/tmp/oviraptor-source-reentry-importer-green.log`，**30/30**，exit 0。首次命令误用下划线 target，Cargo 拒绝且未运行测试（`/tmp/oviraptor-source-reentry-importer.log`）；修正为声明的 `import-existing-results` 后完成上述验证。
- `cargo fmt --all -- --check` exit 0，日志 `/tmp/oviraptor-source-reentry-fmt.log`；最终空白与残留进程检查记录见 CPU 事故文档。
- 本轮未修改 UI，不用此前 UI/浏览器结果充当当前版本的完整验收。

## 资源与安全范围

Cargo 串行，以 `nice -n 15`、`-j 1` 和 `--test-threads=1` 执行；不是 CPU 硬限制。全量运行每两秒记录 Cargo 及当时可见后代进程，共 340 次采样：测试进程最大读数 100%，可见进程树总和最大 100.1%（包含编译阶段）。这些是采样值，不排除间隔内峰值。18:02 左右线程抽查为 4 个；`/tmp/oviraptor-reentry-cpu-sample.txt` 中两条旧重定向服务线程仍阻塞在 accept，应另行清理生命周期，不能声称所有测试服务均已回收，也不能把它们当成本次高 CPU 的证据。为减少电脑负载，本轮不立刻再跑全量，采用上述定向复测。没有外部模型调用、目标 URL 探测、部署或主机操作；仅使用本机回环模拟模型。

## 仍须完成

1. 部分初评/工具阶段的真实恢复，以及过期租约、未知结果核对的明确状态机和用户入口；不能自动重新派发未知请求。
2. 操作系统级进程中断/重启验收、并发恢复者竞争、恢复审计与实际写入之间的并发状态漂移覆盖。
3. Reviewer 大候选集分批、专属在途取消/配置漂移/总时限矩阵、美元账本；显式美元上限仍然拒绝未计费派发。
4. 总体覆盖独立审查；现有 `source_coverage_review` 缺口必须保留。
5. 统一 Findings UI/读 API/源码导出及 SARIF 往返、全部 Strix 历史引用分类、聊天/人工指令/沙箱/部署和 Master Plan 每项最终验收。
