# EvidenceGap 来源校验与关联补充草稿审计（2026-09-26）

## 结论与范围

提交恢复后续：本地 SQLite journal 现保存冻结输入、请求键和永久创建 ID，重新打开同一补证来源只读恢复；双窗口、结束竞争和已创建任务删除不会自动生成另一份任务。详见 `NEST_GAP_SUBMISSION_RECOVERY_AUDIT_2026-09-26.md`。下文“组件内键／补证提交跨卸载重开恢复未实现”保留为历史快照，不代表当前状态；桌面强杀及通用任务恢复仍需验收。

后续状态说明：本文件保留草稿交接阶段的验收快照。随后新增来源关联新事实审核及 gap assessment 收据，详见 `NEST_GAP_REVIEW_CLOSURE_AUDIT_2026-09-26.md`；下文关于“gapResolved 一律 false／新证据独立再审收据尚未实现”的描述已被该增量替代，其余未完成边界仍保留。

本增量将 Stage 9 的已完成 Investigator 协商接到真实的补充任务准备入口：聊天中的“准备补充任务”先读取后端验证过的来源，再进入固定原项目、原目标的工作台，保存关联 Web 草稿，登记新任务的授权控制组后由操作员明确启动。

**关联不是解决。** 当前不将新任务完成等同于原缺口关闭，不把旧任务的事实伪装成新事实；状态投影明确返回 `gapResolved:false`。新证据回传、生成新候选 revision、独立 Reviewer 再审及原缺口解决收据仍待实现。整体 Master Plan 未完成。

执行面保持 `web_only`。没有启用主机执行、部署或访问用户提供的外部 URL；测试使用临时 SQLite、本地模型/HTTP 服务和本地浏览器回环。以下是代码/本地验证，不是桌面端真实账号及授权目标验收。

## 实现

### 1. 不可变的证据位置来源

- `agent_evidence_locations` 将 root run、scan、attempt、目标 URL、attempt 目录和目标证据目录绑定；禁止 UPDATE。
- 生产编排在多智能体派发前绑定位置，要求 native Coordinator、活跃 scan/attempt、匹配的 root 身份；work_dir 来自持久化 attempt，不接受前端目录。
- 验证目录必须是实际 attempt 下的 `url-pipeline/target-NNNNN`，拒绝该后缀的符号链接，要求普通文件 `.oviraptor-scan-id` 与当前 scan 完全匹配。
- 历史任务缺少绑定、目录丢失、marker 不符时明确拒绝，不根据当前 URL 列表顺序猜测历史证据位置。

### 2. 后端权威预览

`preview_agent_gap_followup` 要求来源为已经完成、送达并确认的 Deep Investigator `proposal_assessed`，schema v3、需要新 attempt、目标请求授权为零。

预览重新校验独立 Reviewer 的封存候选、缺失证据、完整 gap 协商以及实际文件 manifest/fact；同长度响应正文被替换也必须拒绝。来源摘要绑定原项目、原目标、消息/候选身份和已验证 manifest。只向前端提供脱敏分析内容、标识及摘要，不返回路径、凭据或可执行授权。

### 3. 同事务保存关联与草稿

`create_agent_gap_followup` 的输入不提供目标/项目覆盖选项；后端重新预览并比对摘要，在同一 IMMEDIATE 事务内创建草稿、策略、目标、新身份绑定与 `agent_gap_followups` 来源记录。

- 请求 UUID 和原输入摘要持久化。同键同内容重放返回实际任务及当前状态，不重复创建、不重新占用身份、不启动。
- 同键不同内容明确拒绝；来源变化必须重新预览。
- provenance 写入 ABORT 或静默 IGNORE 均回滚完整草稿和身份归属。
- 保留源目标 URL 的尾斜杠，不将不同资源路径规范化为另一个目标。
- 不继承旧任务身份、capability 或预算许可；创建过程不产生 attempt/run/目标请求。

### 4. 明确的新任务控制组

确认启动关联补充任务时，必须通过新任务的身份及授权控制组校验。控制组对应的计划 attempt 必须与实际分配的 attempt 一致，目录编号跳跃也不能误用旧控制组。

确认前还校验草稿仍在原活跃项目内且只有原目标一项，目标行项目必须一致；仅检查“原 URL 仍存在”会遗漏额外目标，这一扩展范围路径已增加拒绝和回归测试。

这是补充任务的专用前置检查，不代表通用 confirm/start 的全部并发事务和失败清理已经解决。

### 5. 聊天、工作台和持久化关系

- 聊天预览防重复点击；切换任务 A→B→A、卸载后的迟到成功/失败不能导航或污染当前任务。
- 工作台固定原项目/原目标，只保存补充草稿，即使程序调用直接启动入口也不会自动确认。
- 当前组件内的未知提交结果保留冻结输入和幂等键。修改表单后重试仍使用原提交；只有后端明确拒绝才允许重新准备输入。
- 切换补证来源 A→B→A 撤销旧导航意图。未知的 A 提交不能在 B 来源下重放；返回 A 才能使用原键重试。
- 任务详情从数据库显示来源及后续任务，不靠临时聊天拼接关系，不声称原缺口已解决。

限制：前端未确认请求的幂等键目前仅保存在组件内，卸载/重启后的自动恢复尚未实现。离开页面后应先核对任务中心，不能宣称崩溃后自动无重复恢复已完成。

## 测试矩阵

新增后端 8 项测试覆盖：只读预览、无副作用创建、请求重放与冲突；来源消息/审核/权限/项目/绑定变异；来源写入故障回滚；真实 artifact 变更与目录/marker/符号链接拒绝；双 SQLite 连接同键竞争；新身份与明确控制组、过期身份和跳号 attempt；身份归属完整回滚；目标尾斜杠保持。

前端执行真实 Vue SFC：聊天 41 项，工作台 16 项，共 57 项。工作台本轮包含来源切换、未知提交冻结重试及卸载竞态测试；这不是桌面应用端到端测试。

首轮完整测试发现新增证据绑定使旧全编排夹具在预定故障前停止。修复夹具以创建真实 attempt/目录/marker，保留原有角色、独立执行和错误断言，没有生产跳过条件。后续重跑又发现调用者锁测试错误地把 `kind="target"` 改成目录名，已恢复正确锁键并定向通过。

退役清单只更新经审查的 `SentinelBoard.vue`（历史只读标签，新逻辑仅导航到准备/预览）和 `agent_tests_e2e.rs`（九处历史/负向断言未变，只增加两处证据布局准备）的理由与整文件哈希；未增加排除范围、未自动重生成清单、未放宽测试。

## 最终验证

最终代码验证结果：

| 门禁 | 结果 | 日志后缀 |
| --- | --- | --- |
| `cargo test --offline --all-targets --all-features --manifest-path src-tauri/Cargo.toml` | 主库 791 通过、0 失败，248.09 秒；历史导入器 30 通过、0 失败，3.33 秒 | `final-verified-full.log` |
| `cargo clippy --offline --all-targets --all-features --manifest-path src-tauri/Cargo.toml -- -D warnings` | 通过 | `final-clippy.log` |
| `cargo fmt --all --manifest-path src-tauri/Cargo.toml -- --check` | 通过 | `final-fmt.log` |
| `node --test tools/test_agent_dialog.cjs tools/test_agent_workbench.cjs` | 57 通过、0 失败 | `final-ui.log` |
| `npm run build` | 通过；主 JS 784.74 kB，仍有 >500 kB 拆包警告 | `final-build.log` |
| `node tools/test_native_runtime.cjs` | 通过；8 个本地请求、双身份隔离 true | `native.log` |
| `git diff --check` | 通过 | 终端直接校验 |

日志前缀：`/tmp/oviraptor-20260926-gap-followup-`。`full.log`（780 通过/8 失败）与 `final-full.log`（789 通过/2 失败）包含已修复的夹具/清单问题，不能当作最终通过记录。`verified-full.log` 为范围校验最后补充前的全量通过；最终当前代码以 `final-verified-full.log` 为准。`scope.log` 的跨项目故障注入曾因引用不存在的项目先触发外键失败；已保留外键并建立真实第二项目，`final-scope.log` 中 8 项全部通过，随后又经过最终完整重跑。

## 继续实施，不能省略

1. 新任务的真实新事实 → 来源校验 → 新候选 revision → 独立 Reviewer → 持久化原 gap 解决收据；不得复用旧批准或仅凭子任务 completed 标记解决。
2. 未确认创建请求跨卸载/重启的持久化对账；通用确认启动原子性和竞争/清理。
3. 终态与跨 fencing 恢复、Stage 10 独立专家、资产/知识/技能管理、沙箱供应与性能、桌面及授权目标端到端验收。
4. Linux 主机操作另立需求与审批/执行阶段；当前只记录边界，不因沙箱内存在 Linux 或用户聊天说“继续”而授予目标主机权限。

下一步的已核实代码约束：`evidence_graph/store.rs` 将事实节点限定在自身 root/revision 的祖先链，`require_supersedes` 会拒绝跨 root 的覆盖。因此不能把后续任务事实直接写成旧 root 的新发现，不能放宽此约束来接补证链。需要显式的跨任务来源关系和原缺口解决收据，由新任务自己的事实、候选及独立审核支撑；历史封存事实仍保留原身份、时刻和 manifest。旧任务终态和已失效的能力不应为了回写展示状态而重新激活。当前 `multi_agent_runtime.rs` 尚未消费 `agent_gap_followups` 来完成这条业务闭环，不能把关系投影视为运行时完成证明。
