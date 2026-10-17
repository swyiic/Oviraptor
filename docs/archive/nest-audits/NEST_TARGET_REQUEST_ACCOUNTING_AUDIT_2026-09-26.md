# 多智能体目标请求记账修复与验收（2026-09-26）

状态：本轮增量验收记录。不是整个 Master Plan 完成证明，也不增加任何目标操作权限。

## 1. 修复原因

原先任务界面主要显示 Native Executor 的计数。独立 External Surface 与 Authorization 的目标请求没有进入同一展示口径；Authorization 还把三侧目标 GET 记入了模型请求预算，可能挤占后续模型分析与 Reviewer 的预算。

另一个问题是事件恢复把每个 `ToolInvocationCompleted` 都当成一次目标请求：本地证据分析实际可能为零，一次对照工具调用则可能有多个请求。

本轮修复这些真实记账差异，不通过提高预算、取消授权检查或自动重试来绕过错误。

## 2. 统一口径

`runtime.requestAccounting` 是只读投影，范围为本目标当前预算继承链，不是全站网络流量计量器。

- `executorRecordedRequests`：Native Executor 已持久化的累计尝试数，包含已扣除的发送失败；不是成功响应数。
- `externalSurfaceReceivedRequests`：持久化 received 回执中的公开入口响应数。
- `externalSurfaceUnresolvedClaims`：已领取但没有完成回执的公开入口占用。
- `authorizationReceivedRequests`：Authorization 持久化 artifact 引用所记录的响应数；不等于漏洞已被 Reviewer 验证。
- `authorizationUnresolvedClaims`：Authorization 已领取但尚未完成响应记录的占用。
- `recordedRequests`：Executor 已记录数 + 两类独立角色已记录响应数。
- `budgetCommittedRequests`：上述合计 + 两类未决占用。
- `includedAttempts`：从当前尝试向前的明确预算继承链。
- `includesDeterministicRecon=false`：不包含前置确定性侦察。
- `automaticReplayAllowed=false`：读取和展示不赋予重试权限。

未决项可能已经发送，也可能尚未发送。保守占用预算，不冒充已收到响应，不自动重试。

统计无法核验时返回 `available=false`、原因码及 null 合计；UI 显示不可核验，不伪装成 0。终态快照携带同一结构化统计；无法核验时序列化的 `targetRequests` 也为 null。

## 3. 预算与续跑

1. Native 续跑计数已经累计继承，只读取一次，不能把每次尝试的 Native 快照相加。
2. 独立角色 claim 按每个受继承尝试累计，严格匹配 scan、attempt、target、assignment、child、root、role、lane。
3. fresh 新尝试不继承旧占用；resume 按数据库类型化关系找前序尝试。缺失续跑父记录时不能猜测为零。
   父编号必须严格递减，因此不另设任意的 128 次续跑门槛。旧 JSON 即使能被兼容解析，原始 attempt/count 字段缺失、类型不符或为负值时仍不能提供“已核验的零”。
4. 新 Authorization child 不预留或消耗模型请求；每一侧目标请求仍须通过冻结 Web 计划、身份、能力、fencing、登记对照组和共享目标请求上限检查。
5. External Surface 和 Authorization 在领取前的写事务内检查共享占用；Generic Executor 的目标上限扣除两类独立角色占用。
6. 目标上限仍沿用现有策略 `min(max(hardModelRequests,1)*4,400)`，乘法饱和处理。本轮没有新增无限预算，也没有把该派生策略宣称为最终产品预算设计。
7. 历史数据不重新写账、不自动“退费”。旧版本模型账本中已经混记的历史记录不被静默改写。

证据文件丢失或被篡改不能返还已经记录的目标请求。证据是否能进入 Reviewer 仍由独立完整性校验决定。记账校验不是执行授权，也不能取代 finding 验证。

## 4. 事件恢复和角色归属

- Native 在调用工具前后计算真实请求增量，由执行代码写入完成事件的 `targetRequestsDelta`，不是信任模型上报的数字。
- 回放只累计明确的非负增量，并检查加法溢出；本地工具不再制造目标流量。
- 旧事件缺少增量或字段非法时，快照标记无法核验，不猜测一次请求。
- 旧汇总快照之后又回放出新的请求增量时，汇总标记需从来源账本刷新，不能保留旧分项与新总数互相矛盾的快照。
- External Surface 的独立模型快照只拥有自己的 1 次入口响应；根任务快照显示整个预算链的合计。模型调用数仍单独结算。

## 5. 验证范围

新增真实数据库／localhost 回归覆盖：

- Executor 3 次 + 公开入口 1 次 = 合计 4；反复读取不改 Native 计数、不再次发请求。
- 未派发的 claim 只形成未决占用；再次调用不得重放。
- 两个独立尝试的公开入口 claim 与累计 Native checkpoint 正确合并；fresh 归零，查询更旧尝试不能借用未来 checkpoint。
- 损坏 checkpoint、回执 hash、角色绑定、缺失父尝试、计数溢出均不可核验。
- 文件被篡改不退回已记录请求；回执数据库本身损坏则拒绝统计与专用领取。
- 429 保护响应计入请求，但不因此发起分析模型或声称任务成功。
- 独立 child 快照为 1、根任务终态快照为 4。
- 工具事件 0/3/1 正确恢复为 4；旧事件缺少计数不伪造数字。

已有 Authorization 回归增加：全局目标预算用尽时不得领取；三侧实际 localhost 请求合计为 3；断连侧保留未决占用；模型请求预留和结算为 0。

交付原子性故障注入继续验证：mailbox、lane、终态等写入失败不退回目标 claim，也不能触发重放。夹具已改为独立目标 claim，不再拿虚构模型费用代表目标流量。

实际 Vue SFC 验证覆盖真实计数、未决提示、缺失／旧数据、负数／溢出、不一致求和、非法继承链及错误范围标志。前端类型声明包含新接口和 null 语义。

## 6. 验证记录

本增量最终全量结果已收齐：Rust 主库 **901** 项、历史导入器 **30** 项全部通过，进程退出码 0；最终源码验证日志为 `/tmp/oviraptor-20260926-accounting-audited-full.log`，主库耗时 721.63 秒。严格全 targets／features Clippy 与 fmt（`audited-clippy.log`／`audited-fmt.log`，同前缀）退出码 0；实际 Vue SFC **105** 项、前端构建、本地 Native 浏览器回环全部通过。浏览器回环观察 8 次请求、身份隔离 true；主 JS **802.51 kB** 拆包警告仍在。61 项退役例外摘要零过期，`git diff --check` 通过。日志目录为 `/tmp`，不随仓库交付。

上述结果仅对应 9 月 26 日跨角色记账增量，不覆盖后续通用 HTTP journal 修改，不代表整个 Master Plan 已完成。

- 定向记账测试：`oviraptor-20260926-accounting-targeted.log`。
- 交付故障回归：`oviraptor-20260926-accounting-delivery.log`。
- 双尝试预算继承：`oviraptor-20260926-accounting-lineage.log`。
- 全量首轮：`oviraptor-20260926-accounting-full.log`。发现两个仍使用旧 Authorization 模型预算的夹具，修复后另行完整复跑；不把首轮称为通过。
- 前端首轮构建发现遗漏的 `requestAccounting` 类型声明，补齐后构建通过。
- UI、构建、本地浏览器回环：`oviraptor-20260926-accounting-ui.log`、`build.log`、`native.log`（后两者同一 accounting 前缀）。
- 最终严格检查：`oviraptor-20260926-accounting-audited-clippy.log`、`audited-fmt.log`（同一 accounting 前缀）。

## 7. 仍不应宣称完成的事项

- 这是 Agent 账本的真实已记录数和保守占用，不保证覆盖前置侦察、目标侧效果、任意外部流量或进程崩溃前来不及持久化的所有 Generic Executor 请求。
- Generic Executor 尚未统一迁移成每次网络发送前的持久化 claim/receipt；未知效果的通用恢复和人工对账仍需独立验收。
- 历史已混记账本没有自动重分类；新的快照字段保持旧 JSON 可读，但旧数据不能补造不存在的精确计数。
- Authorization 响应计数依据现有持久化 artifact 引用，不是新增的签名回执或文件完整性证明。
- 未实现完整 Host Agent、主机审批后端或 Windows 主机能力；本轮不访问外部授权 URL、不部署服务。
- External Surface 仍只是首个匿名入口合同；其余专家、资产分析、跨任务 skills 治理与整个 Master Plan 的最终验收仍不能据此勾选完成。

下一步必须继续处理真实执行与未知效果恢复，再按主计划逐项验收。不得通过 UI 数字更完整就声称完整自治扫描已经交付。

### 下一轮通用执行器持久化记账的必要入口

已核对 `agent_tools_http.rs::agent_http_exchange`：当前在 `call.send()` 前仅调用内存 `runtime.spend_request()`；`agent_native.rs` 在工具完成审计与后续同步时再持久化。发送后、工具完成／checkpoint 写入前的进程退出仍可能留下费用与效果未知窗口。

后续不能直接把新 claim 数与现有累计 Native 数相加，否则历史和新请求会重复收费。实施时必须先定义版本化来源边界，逐请求绑定 invocation、请求序号、attempt、目标和身份；持久化领取成功后才发送，响应持久化与状态迁移具有唯一性和原子性。相同 invocation 内的身份对照、CORS 辅助请求和发现子请求都须逐项覆盖，不能只包住最外层模型工具调用。

必须验收领取前／领取后／发送中／收到响应后／工具完成前／checkpoint 写入前各故障窗口；恢复只补交已核验的本地结果，不能把缺少响应当成没有发送；取消、预算耗尽、响应丢失和旧 checkpoint 兼容都需保持同一真实占用。这项尚未实施，不由本轮 UI 或测试数字替代。
