# 已派发 Web 任务的人工结案审计

状态：人工结案及后续独立草稿交接已实现，最终组合主库 979／导入器 30、实际 SFC 143 项、严格 Clippy/fmt、构建及空白检查通过。本增量补充真实的人工结案入口，不是远端效果已确认、费用已结清或整个 Master Plan 已完成的声明。后续交接合同、首轮失败与最终证据见 `NEST_CLOSURE_HANDOFF_AUDIT.md`。

## 1. 解决的工作流缺口

原有 `close_never_dispatched_web_attempt` 只接受从未派发、无执行进度的 Web 尝试。人工请求核对只保存声明，不能改变原执行状态。对于已派发、已经停下但仍有未知结果的任务，原来没有明确的人工结案路径。

本增量新增 `preview_web_administrative_closure` 与 `close_web_task_administratively`：用户可以明确停止继续旧任务，同时保留原请求、原始执行终态、未决费用、预算预留、lane、证据及核对记录。这里的“结案”是任务管理决定，不是机器验证结果。

## 2. 执行合同

- 当前只接受有原始派发凭据的普通 Web 任务，当前 attempt 必须存在。
- 任务必须处于允许的已停止状态；`scanning`、`pausing`、未派发、源码或混合路径不接受。
- 预览和提交都持有任务生命周期锁，在 IMMEDIATE 事务内取得已登记分支、target、frontend producer/recon 等调用的退出证明。遗留进程记录、未确认容器清理不能被忽略。提交采用 FULL 同步。
- 用户必须先取得本任务/attempt 的预览，再明确确认。提交绑定 UUID 与预览摘要；记录、预算或当前状态变化时拒绝过期预览。
- 新增一条不可覆盖、不可更新、不可删除的 `native_web_administrative_closures` 回执，并把任务管理状态标为 `cancelled`，写入明确的未决结案说明。
- 不改旧 attempt、分支结果、child-run、assignment、请求回执、已花费/预留预算、lane、原能力租约或历史证据。历史能力记录的存在不是执行权限：实际工具仍需活跃 attempt；封存后不能重新获得 Coordinator 租约。
- 旧任务不可恢复、重试、增加 attempt、新建 run 或删除。数据库触发器和启动入口共同阻止重开。其他独立任务不受此封存约束影响。
- 同 UUID、同 attempt、同预览摘要的重试只核验原回执，不产生第二次结案或任何执行；冲突操作返回错误，用户可以重新读取已有回执。
- 回执的 `executionSettled=false`、`automaticReplayAllowed=false`、`requiresIndependentTask=true` 是明确合同，不由 Agent 的判断改写。

### 快照与保留检查

预览保存按任务关联的记录数量与摘要，不将数据库中的认证内容、工具输出或消息原文送入确认面板。按 `scan_id`、`owner_scan_id`、run/root/coordinator/child、assignment 与 directive 的归属列枚举；记录排序后摘要稳定，不依赖数据库物理顺序。任务状态、管理检查点和更新时间是预期可变化字段；结案回执及其新增事件单独核验。

提交前重新比对预览；写入后比对保留快照并检查精确回执、唯一真实协作事件。任何忽略写入、额外预算变更、lane 丢失、缺失/矛盾聊天事件都会回滚。

此快照机制不是数据库签名，也不是所有未来表结构自动获得完整归属证明。新增不使用这些归属列的间接业务表时，必须补充其归属与保留测试。之后的合法人工核对可以继续追加记录，不会因为记录摘要变化而使既有结案回执失效。

## 3. 界面与聊天

- Native 状态区提供“人工结案并保留未决结果”。后台尚未满足条件时不提供该动作。
- 确认面板明确显示任务/attempt、不可撤销、未知效果仍需核对、不释放预算、不重放扫描。
- 提交期间防重复点击；切换任务/attempt 或卸载后，迟到响应不能更新当前界面。
- 响应必须与全部 11 个回执字段合同匹配，包含本次 UUID、预览摘要、原状态和明确的未结清标识。失败保留同一操作用于显式核对，不自动重新提交。
- 任务列表和详情不再提供封存任务的重试、删除操作。展示“已人工结案 · 原执行结果未结清”，历史 attempt/分支原状态仍保留。
- 团队聊天来自真实的 `administrative_closure` 协作事件，游标增量去重；状态读取在游标过滤前核验回执和事件。
- 聊天出现结案完整性错误时清空缓存并拒绝迟到旧成功响应，不能继续显示未经核验的“已结案”。

`local_operator` 仅表示本地桌面操作者，不是已经集成企业身份认证的批准人身份。此功能与 Linux 主机审批无关，不授予任何新执行能力。

## 4. 验证

新增 8 项定向 Rust 测试已通过：

1. 保留未知义务、预算/lane、完整旧 attempt，禁止恢复、重试、删除、创建 run、替换结案回执和重获 Coordinator 租约。
2. 明确确认、准确 attempt、过期预览和各类本地调用所有权。
3. 忽略写入、预算篡改、删除 lane、丢失/矛盾事件时整体事务回滚。
4. 真实全量/增量聊天事件、重复初始化数据库、重新打开后的回执，以及游标已超过坏事件时仍拒绝。
5. 活跃/暂停中、未派发、源码与带源码路径的任务拒绝。
6. 遗留进程记录、未确认容器清理拒绝，不尝试杀进程或删除文件。
7. 新增 run 归属记录使预览失效；封存不改变其他独立任务的状态或执行许可。
8. 重复事件、跨任务事件、数字代替布尔值、额外矛盾字段均拒绝，不补造事件。

前一人工结案全量为主库 970 通过/1 失败，唯一失败是退役审核清单中的两个已修改历史显示文件摘要过期，后续导入器/Clippy/fmt 因串行命令停止而没有执行。已复核 `SentinelBoard.vue` 的历史标签与新导航、`presentation.ts` 的只读来源标签及结案展示，再更新对应摘要和理由；没有扩大 Strix 允许范围。前一前端 136 项及构建通过，另行严格 Clippy 通过。含独立交接的最终组合结果见下段，不把先前失败写成全绿。

后续已核实的完整组合：主库 979、历史导入器 30、实际 SFC 143 项全通过，严格 Clippy/fmt、构建及空白检查通过；localhost 浏览器回环完整、身份隔离成立。交接首轮另有 978/1 的派发绑定失败，最终复跑通过但原因尚未证实；准确命令、时间、日志及这个未解决风险统一记录在 `NEST_CLOSURE_HANDOFF_AUDIT.md` 第 5 节。

中间测试问题：扩展后的容器 fixture 曾使用不在数据库枚举中的 `unknown`，导致 6 通过/1 失败；改为真实枚举 `unconfirmed` 后定向 8 项通过。新增聊天渲染 fixture 首次缺少必需的 `stopDiagnostic`，随后补齐真实响应结构。这两项是新增测试数据问题，没有放宽生产约束或删除测试。

日志保存在本机临时目录（不是随代码分发的永久工件）：

- `/tmp/oviraptor-administrative-closure-fixture-failure.log`
- `/tmp/oviraptor-administrative-closure-ui-fixture-failure.log`
- `/tmp/oviraptor-administrative-closure-targeted.log`
- `/tmp/oviraptor-administrative-closure-final-full.log`
- `/tmp/oviraptor-administrative-closure-final-clippy.log`
- `/tmp/oviraptor-administrative-closure-final-fmt.log`
- `/tmp/oviraptor-administrative-closure-final-ui.log`
- `/tmp/oviraptor-administrative-closure-final-build.log`

## 5. 明确未完成的范围与下一步

1. **独立新任务交接已由后续增量实现。** 采用独立于 gap follow-up 的、每个来源结案唯一的规范交接回执；精确关联原结案 ID/来源摘要，重新确认目标、身份与预算，只创建新 draft，不继承旧请求占用、可执行租约、机器成功状态或自动重放指令。范围、真实聊天事件、故障测试与限制见 `NEST_CLOSURE_HANDOFF_AUDIT.md`。
2. **不是费用或远端副作用对账。** 未决费用不退回；模型或目标系统已经发生的效果不会因本地结案撤销。
3. **退出证明限于当前已登记执行路径。** 全渠道网络核算、浏览器后代进程治理、未登记外部执行器，以及完整跨进程异常恢复仍属于后续 Master Plan 工作。不能用此功能宣称全部执行通道已受控。
4. **当前封存回执永久保留。** 可按既有任务归档入口整理列表，但没有物理清除已结案任务的保留策略；以后若要增加，必须独立设计保留期、未决义务和证据引用规则。
5. **性能与 UI 验收边界。** 当前定向检查覆盖功能和一致性；大型数据库快照成本、桌面实机视觉/E2E 及授权真实目标验收尚未由本增量证明。SFC 测试执行真实组件，但 IPC 和宿主节点为测试替身。
6. **未部署、未访问外部测试 URL、未启用 Host Agent。** Stage 9/10 其余能力、资产/知识/skills 生命周期、沙箱供给及完整 Master Plan 继续保持未完成状态。
