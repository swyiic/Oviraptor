# Web / Source 执行历史分页审计（2026-09-27）

本增量修复任务详情工具历史缺席源码执行回执的问题。不添加工具执行权限，不迁移或重写历史 JSON，不代表独立 Reviewer 或整个 Master Plan 已完成。

## 实际缺口与交付

- 原任务详情读取 `tool_invocations`，源码独立表 `agent_source_tool_receipts` 没有进入分页结果。新增 `get_native_attempt_execution_history` 并注册桌面 IPC；前端保留方法名称，但调用新命令。旧 Web-only 命令和数字游标不删除。
- v2 游标是 opaque JSON 字符串，绑定 schema、scan、attempt、时间和稳定 key，拒绝错误版本、越轮/越任务、未知字段及超长输入。游标只是读取位置，不是执行授权。
- Web 键使用 `web:` 前缀及零填充 ID；Source 使用 `source:` 前缀及 assignment/round/index。先分页轻量键，再读取安全展示字段；同一次请求在稳定读事务中完成。每页上限 100。
- Source 排序时间来自不可变模型响应落库时间。planned 行在同一事务产生，之后完成不移动分页位置。此时间不是工具开始时间，`startedAt` 保留空值，不制造 started 事件。跨页读取不是整个查询会话的冻结快照；新插入记录需要重新加载最新页。
- completed/refused 要求 assignment、child、root、role、target、模型对应 tool call、完成事件和 receipt hash 相符。未知工具名即使模型/事件/hash 同步被篡改，也显示 unverified。planned 必须无完成事件/hash 且 output 为空对象；损坏回执不显示结束时间。
- 只返回状态、有限标签与回执是否一致，不返回参数、源码输出、模型文本、凭据或原始拒绝信息。`assignment.finish` 单独标为 control，不计作漏洞证据。matched 仅说明本地记录相符，不是独立审查通过，也不是可重放许可。
- 任务详情验证返回 schema/scan/attempt；保留过期响应隔离、分页并发去重、HTML 转义及错误展示。

## 验证范围和真实失败

1. session 17049 已收取 exit 0：混合分页定向 1 项、真实生产入口 1 项、真实拒绝入口 1 项。真实生产用例仍经 localhost 模型运行，验证 4 条源码回执及 4 类损坏降级；不是用假 UI 行代替执行证明。
2. 独立混合分页用例经真实 SourceBroker 执行 read_slice，验证 planned → completed 身份/排序时间不变，混合 Web 行无冲突、游标拒绝、attempt/删除隔离及敏感字段不泄漏。Web 行是明确的投影夹具，不宣称这条用例发出了真实 Web 请求。
3. 首轮严格 Clippy session 34287 exit 101：13 字段匿名元组触发 `type_complexity`。改为具名记录，没有压低门禁或新增 allow。session 45436 已 exit 0：fmt、严格 Clippy、混合分页定向测试通过；新增未知工具 planned/completed 同步损坏断言包含在该测试内。
4. `tools/test_execution_history.cjs` 执行真实 Vue SFC setup/template，mock IPC 和无关组件，验证状态含义、转义、opaque cursor、并发/过期响应、返回范围和 schema。另执行真实前端 API 模块验证新 IPC 命令与参数；不冒充桌面端到端验收。
5. `package.json` 增加 `test:execution-history` 和包含八组测试的 `test:ui`。session 69032 已 exit 0：当时 UI 153 项、构建、空白检查通过；随后新增 IPC 接线测试，最终 UI session 30879 已收取 exit 0，**154/154** 项通过。主 JS **831.51 kB** 拆包告警保留。
6. 最新完整 Rust session 64211 已收取 **exit 0**：全目标全特性串行主库 **1149/1149**（653.93 秒）、历史导入器 **30/30** 通过。包含真实源码生产入口、拒绝回执、混合分页、旧读取接口回归，以及 Strix 字面量白名单、旧 CLI 启动陷阱和禁止旧后端网络访问检查。旧源码轨迹 session 75619 的 1148/30 仅为本增量前基线。本地浏览器 session 94241 已收取 exit 0：8 个回环请求、匿名/对照抓取及身份隔离通过；不是用户授权 URL 或安装包验收。
7. 最终文档同步后再次执行 `git diff --check`，退出 0。Master Plan §15 同步加入 `npm run test:ui`，Rust 总门禁明确为全目标全特性；没有通过删测试或压低门禁取得通过。

日志前缀：`/tmp/oviraptor-execution-history-`，包括 `test/production/denial`、`clippy`（保留红测）、`fmt-final/clippy-final/test-final/full-final`、`ui-all/ui-all-final/build/browser/diff`。

## 下一执行链与不可省略的验收

独立 Source Reviewer 还没有生产接入，不能删除 `source_review_not_completed` 来使 CI 变绿。当前 `native_source_coordinator.rs` 冻结 sourceToolsPhaseVersion=1 和 8 次模型额度，实际只调度两角色初评与工具阶段；`native_source_completion.rs` 核验四份 assignment 并明确标记未复核；`native_source_scan.rs` 使用 `evaluate_unreviewed`。

后续接线必须同时处理：

- 版本化冻结审查阶段及其预算，不从 Web 请求额度借款，不给旧 v1 attempt 静默加权限。
- 为 EvidenceReviewer 创建真实独立 assignment/child，输入绑定当前 scan/attempt/root、分析视图和候选 revision；不能只按 Reviewer 角色名称认定资格。
- `multi_agent_runtime.rs::specialist_round_transport` 目前按 RepoMapper/SourceAnalyst 角色判断是否执行源码输出预算和发布合同预检。EvidenceReviewer 是跨 Web/Source 的共享角色，接入时必须按冻结执行面区分授权，不能直接调用通用入口而绕过源码预检，也不能把所有 Web Reviewer 误判成源码角色。
- 候选有两种真实来源：`AnalysisResults` 接受的导入修订，以及 SourceBroker 产生、带 `created_by_run_id` 的 evidence graph 候选。审查材料必须保留二者的内容、来源与修订对应关系；不能把导入器的 revision hash 随意转成 `ReviewDecision` 的数值 revision，或只审查模型候选而漏掉已接受的分析器结果。
- 真实模型请求及持久结果、作者独立、不可变裁决、重复返回幂等、mailbox/ACK、预算和资源收口必须形成同一可核验链；未知模型结果不得自动重发或退款。
- 审查已完成与候选已确认是两件事。证据不足、拒绝、缺候选、变更 revision、旧 attempt 裁决均需明确处理；审核覆盖不完整不能获得 CI 通过资格。
- 生产 CI 只能消费当前审查交付证明。应包含真实 localhost 生产接线、取消、模型失败、迟到/越轮结果、写入故障及 UI 时间线测试，不能把 test-only evaluate 接回生产冒充闭环。

源码顶层恢复、美元费用账本、整个 attempt 的统一持久时限、完整聊天分页、知识治理、资产汇总、沙箱/工具供应、Strix 全白名单验收、安装包与授权环境验收均仍保留在主目标中。本轮未部署、未访问用户外部 URL、未新增 Host Agent。
