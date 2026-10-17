# Native HTTP 停止原因、人工核对与复核边界

状态：本增量实现及验证记录，不是 Master Plan 或人工对账功能已全部完成的声明。衔接 `NEST_NATIVE_HTTP_JOURNAL_AUDIT_2026-09-27.md`。所有网络测试均为 localhost；未部署、未访问外部目标，未新增目标主机执行能力。

后续增量：`NEST_REQUEST_OPERATOR_REVIEW_AUDIT.md` 已交付逐请求人工核对声明、追加更正和 UI；本文件“仅有说明/无人工入口”是此增量当时的状态。新接口不结算退款、不解除停止，也不构成完整任务结案/恢复功能。

## 1. 本轮修复的问题

1. journal 领取失败原来容易统一变为本地存储失败，无法区分预算、撤权、完整性错误与结果待核对。
2. 已登记请求发生网络错误或响应体不完整后，后续工具、同一模型轮次的第二个调用、隐式子请求可能继续执行。
3. 一个工具包含多个请求时，内部失败可能被外层正常返回掩盖，且部分证据引用可能因包装结果而失去顶层链接。
4. `finish_tool` 将任何带错误码的结果都标成 refused，间接把原本已批准、已领取请求的 invocation 改成 deny，导致 journal 校验原始授权时失败。
5. Native 返回新的停止码后，共享 reducer 可能把它归为普通未完成甚至完成；空 checkpoint 原因还可能让运行行被标记为可自动继续。
6. Reviewer 的证据不足结论可能覆盖原来的人工核对/授权失效原因，并触发自动补证。
7. 初次全量回归发现：缺少第二个身份会话被误当成执行器整体撤权，普通证据缺口因此过早停止。

## 2. 停止分类合同

| 事实 | 处理 | 不得发生的行为 |
| --- | --- | --- |
| 请求结果未知、journal/checkpoint 需要人工核对 | `request_reconciliation_required`；停止当前执行器 | 退款、换 URL/调用编号绕过、自动重放、用覆盖率宣称完成 |
| 有效授权、活动 attempt、能力租约或身份绑定无法核验 | `execution_authorization_denied`；停止当前执行器 | Agent 自行续期、补授权或扩大范围 |
| 请求硬预算耗尽 | 保留 `hard_request_budget` | 伪装存储故障、重置内存计数后继续 |
| 本地证据/回执持久化失败 | `persistence_failure` | 退回已占用预算、重新发请求“补证” |
| journal 绑定/计数完整性错误 | `evidence_integrity` | 静默修数、自动把历史事实改写成允许 |
| 取消 | 保留取消语义 | 后续新工具继续派发 |
| 参数错误、候选 URL 越界、某个对照会话缺失 | 拒绝该候选请求，保留可解释的缺口 | 降级为匿名身份执行、把普通证据缺口一律当成整支撤权 |

`identity_session_unavailable` 不再是执行器级停止：缺失身份的请求仍在发出前拒绝，模型可以记录证据不足。`tool_identity_binding_denied` 仍是硬停止。二者不能混用。

当前两个新增原因使用 `TerminalState::Incomplete`，目标投影显示 paused；其运行行和 checkpoint 保留明确停止码并终结本次运行，不因报告的 resumable 标记自动开放恢复。paused 不代表用户可安全点击继续。实际 Native 恢复还会在模型调用前检查未知请求与权威计数；本轮没有交付人工清除该阻断的接口。

## 3. 执行和持久化

- HTTP wrapper 在首次致命错误后保存停止状态；修改 URL、invocation、内存计数都不能清除它。
- 工具 dispatcher 将嵌套停止提升到顶层 `code`，原始结果保留于 `partialResult`。已有 `rawArtifactId`/`responseDifferenceArtifactId` 同时保留给 invocation 和界面索引，不把部分证据称为整个工具成功。
- Native 在持久化当前工具结果后检查停止，不开始同一轮次剩余工具或额外模型调用。
- 已领取请求的失败 invocation 标为 failed，原 allow 保留；派发前真正被拒绝的调用仍为 refused/deny。即使内存报告的请求增量为 0，持久化 claim 也能证明原授权已使用。
- 注册执行器的网络发送失败、响应体读取失败使用静态脱敏错误，不回显请求 URL/query 中的敏感值。
- 新的 TerminalSignals 字段带 serde 默认值，旧 JSON 缺少字段仍可读取；未改动历史导入格式或新增可执行旧后端。
- backend → report → reducer → run row → checkpoint → target 的投影保留停止原因；已有证据、硬预算和已闭合覆盖账本不能将这两个停止改写成完成。
- Reviewer 可审查已存在证据并保存自己的决定，但普通复核结论不能清除人工核对/授权停止；这类停止不自动触发 Gap Investigator。复核发生真正失败或用户取消时仍保留相应故障/取消语义，不谎称复核成功。

重要限制：收到响应头不等于响应体完整、业务效果已知或证据足以支持漏洞。响应体中断时可能已有 header receipt，因此 unresolved 计数可以为 0，但本次工具仍须停止核对；不能只看计数决定是否完成。

## 4. 界面

请求消耗组件增加“如何处理未决请求”的解释，仅在未决占用大于 0 时出现：

- 在途请求也可能暂时未决，不把数量直接当作任务已停止。
- 要核对目标应用日志与本地工具记录，不自动退款或重放。
- header receipt 不代表全部响应和副作用已经确认。
- 明示当前没有人工结算/忽略未决后继续的按钮；新建任务也不能抹掉旧任务的未知结果。

不创建假审批、假重试或无后端保障的“继续”按钮。

## 5. 回归证据

新增/扩展的测试覆盖：真实 localhost 发送失败和半截响应、同模型轮次双工具只执行首个、隐式子请求失败、已领取请求授权历史守恒、跨 URL/invocation 停止保持、恢复前零额外模型/目标调用、部分证据链接、两类停止的实际目标/run/checkpoint 投影、历史 JSON 默认兼容，以及首次 Reviewer/重复复核不自动补证。

- 最终定向日志 `/tmp/oviraptor-http-stop-projection-verified.log`：execution_stop 4 项及 HTTP journal 15 项通过；集合有重叠，不应相加作为独立测试数。
- 首次全量 `/tmp/oviraptor-20260927-http-stop-full.log`：913 通过、2 失败。真实回归是缺失会话误停止；另一项旧测试试图复用已经硬预算停止的 runtime 来断言撤权原因，现分别验证停止保持和独立 runtime 撤权。未放宽实际授权检查。
- 新增投影测试曾因夹具表名错误、没有建立目标行而失败；修正为真实表并显式建立目标，未修改生产代码去迁就错误夹具。
- 实际 Vue SFC `/tmp/oviraptor-20260927-http-stop-ui.log`：108 项通过。前端 build 通过，主 JS 803.85 kB，现有拆包警告保留。
- 本地浏览器 `/tmp/oviraptor-20260927-http-stop-browser.log`：8 请求，匿名/身份对照采集完成，身份隔离 true。以上 UI/build/browser 进程均已收齐 exit 0；随后修改仅涉及 Rust 和审计文件。
- 最终严格全 targets/features Clippy（`-D warnings`）和 fmt check 通过，进程 exit 0；日志 `/tmp/oviraptor-http-stop-authoritative-clippy.log`、`/tmp/oviraptor-http-stop-authoritative-fmt.log`。
- 最终全 targets/features 串行完整回归 `/tmp/oviraptor-http-stop-authoritative-full.log` 已收齐 exit 0：主库 **919 通过、0 失败**（349.14 秒），历史导入器 **30 通过、0 失败**，主二进制 0 项。启动此最终回归后没有再修改 Rust/前端源码，仅更新审计文档。
- `git diff --check` 通过。本结果验证本增量，不替代安装包、部署、真实授权环境或完整 Master Plan 验收。

Strix 精确 allowlist 只更新已再次审查的 contract、runtime_adapter 与 test fixture 内容哈希；字符串仍仅为只读历史解析/封口说明/测试，没有新增旧后端执行路径。

## 6. 后续仍需实现，不能省略

1. 真实未知请求列表与人工核对 API/UI：不可变决定、操作者、证据引用、决定版本；不退款、不自动重放、不扩大授权。当前引导文字不等于这套工作流。
2. 真实进程强杀/断电恢复矩阵；现有数据库故障注入和 localhost 网络失败不能代替完整验收。
3. 分别核算前置侦察、浏览器子请求及各执行通道；本 journal 不是完整网络包计数。
4. Stage 10 剩余专家、完整知识/skills 治理、资产分析、任务 UI 重组、实际沙箱工具供应与性能、安装包及外部授权环境验收。
5. Web→目标 Linux 主机的逐动作语义边界与结构化人工批准，参照 `NEST_HOST_BOUNDARY_DECISION_2026-09-26.md`。当前 Web-only，不新增 Host Agent；Linux 工具沙箱不是目标主机授权。

整体目标保持未完成。不得因本轮回归通过就宣称 Master Plan 完成，也不得移除授权、隔离或目标保护门禁来换取任务持续运行。
