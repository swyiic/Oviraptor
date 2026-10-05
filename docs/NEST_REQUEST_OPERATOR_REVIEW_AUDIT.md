# 目标请求人工核对：实现、边界与验收

状态：已实现逐请求查看、人工声明和追加更正；最终验证结果见第 6 节。不是未知请求自动结算、原任务安全恢复或 Master Plan 全部完成的声明。衔接 `NEST_NATIVE_HTTP_STOP_RECONCILIATION_AUDIT.md`。

回执一致性后续：历史/幂等读取、团队全量/增量时间线现统一核验真实事件，拒绝缺失/重复/矛盾事件；实际 UI 核对完整返回回执，坏回执会使聊天缓存失效并要求完整快照恢复。新增红测、修复边界和最新门禁见 `NEST_REQUEST_REVIEW_RECEIPT_INTEGRITY_AUDIT.md`。下文第 6 节是最初增量的历史验证记录；本后续不解除未知请求或赋予执行权限。

## 1. 交付范围

新增 `AgentRequestReview.vue`，接入目标详情的 Agent 请求统计旁。真实桌面命令为 `get_agent_request_reviews` 与 `record_agent_request_review`；不是模拟聊天，也不是模型工具。浏览器呈现验收单独使用 localhost 模拟 IPC，不将该模拟视为真实桌面后端验收。

支持三类真实持久化来源：

| 来源 | 显示内容 | 不能推断的结论 |
| --- | --- | --- |
| Native HTTP journal | 子运行、调用编号、请求序号、工具状态、header receipt | 有响应头不等于完整响应体或已确认目标效果 |
| External Surface capture | assignment/child 与 claimed/received 状态 | claimed 不证明未发送；received 不证明漏洞已验证 |
| Authorization probe | assignment/child、合同及 owner/cross/tester 侧 | 空引用不证明未发送；有引用不保证文件未丢失或未篡改 |

不只列 `response_status=0` 的请求，避免遗漏“响应头已收到、响应体中途失败”的停止。旧 baseline 计数显示为“没有逐请求账本”，不合成请求行。当前范围不含前置确定性侦察或尚未接入这些账本的其他网络通道。

## 2. 人工声明的精确含义

操作者可提交三种判断：

- `still_unknown`：仍无法确认。
- `effect_observed`：人工观察到目标效果。
- `not_sent_attested`：人工判定未发送。

全部都是 **operator attestation**，不是机器证明。第三种也不退款、不改 claim。必须填写核对依据并显式确认；界面提示没有找到日志不能证明未发送。后端限制 4000 字节、拒绝异常控制字符，以及现有脱敏器可识别出的敏感格式。此检测不能保证识别任意凭据，禁止粘贴原始敏感日志的提示仍保留。

当前桌面没有独立的操作者认证体系，因此 actor 固定记录 `local_operator`，界面明示“本地操作者（非认证身份）”，不虚构用户认证。

此操作不修改：

- 请求预算、原始 claim/header receipt、工具结果或证据文件；
- 任务、attempt、子运行的执行/停止状态；
- 租约、冻结计划、授权范围及原始 Reviewer 结论。

它不执行 HTTP、浏览器、远程命令或模型调用，不授权自动重放，不为新任务授予权限。用户获得的是可持久化、可回看、可更正的人工处置记录，而不是“忽略后继续”按钮。

## 3. 数据一致性与失效处理

`agent_request_reviews` 保存操作 UUID、请求身份摘要、来源快照及摘要、上一核对 ID、判断、依据、actor、时间和绑定内容摘要。UPDATE 被拒绝，更正必须追加；正常删除父扫描时依既有数据删除语义级联清除，不能把它描述成抗管理员删除的审计系统。

读取使用 SQLite 一致性事务，先校验预算继承链和三类来源的任务绑定。提交使用 `IMMEDIATE` 事务重新读取并比较：

1. scan/attempt/target/request 必须匹配；删除墓碑拒绝。
2. 快照变化（包括迟到 header 或工具状态变化）拒绝新提交，要求刷新。
3. 上一核对 ID 变化拒绝并发覆盖。
4. 同一操作 UUID、相同输入的重试返回原回执；相同 UUID 的不同内容拒绝。
5. 写入后再次验证回执链、请求来源、预算和真实协作事件，少写/静默丢写/源事实被触发器修改均回滚。

已经提交的声明不会因后来收到响应失去历史意义。UI 会显示“机器记录在上次核对后发生变化”，操作者可以追加更正；重试已提交操作仍读取原回执，不重新解释历史声明。

人工记录与来源请求绑定，resume 继承请求时保留此前声明；fresh attempt 不继承旧请求清单。旧 attempt 有权威 journal 时，新 checkpoint 不再遮挡其历史账本；没有旧 journal 时仍拒绝猜测历史计数。

每次新记录在同一事务产生一条 `request_review` 协作事件。团队时间线显示“请求核对 · 人工声明”，不会伪装成 Reviewer 通过或 Agent 收到授权，也不会自动将自由文本核对依据喂给模型。

## 4. UI 与异步竞态

- 默认显式点击加载，不自行提交判断。
- 保存前必须勾选确认；保存中禁止重复点击。
- IPC 回应丢失后保留相同操作 UUID 和原始输入，可重试同一提交，也可刷新服务器记录确认是否已写入。
- 修改 scan/attempt/target 或卸载组件后，旧请求的成功/错误不能污染新上下文；包含 A→B→A 情况。
- SentinelBoard 获取执行详情增加请求代次与目标绑定，切换 URL 时重新加载，不让旧目标核对入口留在新目标下面。
- 人工依据按文本渲染，不渲染 HTML。

## 5. 本次未交付，后续必须继续

后续前置修复见 `NEST_PAUSE_QUIESCENCE_AUDIT.md`：暂停标签必须等待实际调用所有权退出，不能作为请求已撤销或可重放的凭据；完整结案/新任务交接仍未交付。

1. **停止任务的完整人工结案/新任务衔接。** 本次声明不修改停止状态；尚不能把“已记录人工判断”等同于“未知效果已安全解决”。之后需要独立设计结案回执、并发/进程所有权、停止中在途请求和新授权合同，不能只把本次表中的判断作为解除门禁条件。
2. 全通道逐请求 Broker、浏览器和代码执行的统一审计、完整进程 kill/restart 恢复矩阵。
3. Stage 9 新合同采证/新 revision 再审、Stage 10 其余专家，及资产/知识/skills 生命周期、沙箱供应链和性能验收。
4. Web→主机完整语义分类与审批后端仍未完成；本次没有新增 Host Agent。
5. 大量历史请求的分页、检索、证据引用选择器与本地操作者身份管理尚未加入。
6. 未部署、未访问用户提供的外部 URL，未执行目标主机验证。对外部授权目标的验收不能由 localhost 结果代替。

## 6. 验证记录

新增后端用例覆盖：真实 Native claim 不退款/不重放、迟到回执与修订链、公开面 claimed/received、授权对照三侧、输入拒绝与跨目标、并发唯一胜者、故障注入回滚、内容篡改、resume/fresh 分离、老库升级重启、较新 checkpoint 下历史账本查看，以及真实团队时间线投影。

前端测试编译执行实际 Vue SFC，覆盖确认、重复点击、丢响应重试、上下文切换、错误作用域、追加更正、HTML 转义及刷新找回回执。

当前已收集：115 项 Vue 测试通过；前端构建通过，主 JS 811.42 kB，既有拆包警告仍在；本地浏览器工具回环 8 条观察请求、匿名/对照采集完整、身份隔离通过。实际浏览器还用本地模拟 IPC 完成核对组件的查看、填写、确认、保存、展开历史，并观察到保存成功但未解除执行停止的提示。

最终全量 Rust 已收集进程退出码 0：主库 929 通过、历史导入器 30 通过、0 失败。严格全 targets/features Clippy（`-D warnings`）及 `cargo fmt --all -- --check` 均通过，串联进程退出码 0；前端测试/构建/浏览器回环进程亦已收集退出码 0。`git diff --check` 通过。这些结果仅证明本次本地回归，不代表真实目标验收或整个 Master Plan 完成。

首轮全量 Rust 为 927 通过、2 失败：新增历史账本测试只改变 context attempt，未同步数据库当前 attempt，被既有 checkpoint 写入保护正确拒绝；现补齐夹具，不放宽生产保护。另两份包含历史兼容字面量的文件因本次改动触发 REM-012 摘要复核，已检查数据库中历史路径修复/旧配置删除/旧假完成迁移及 UI 历史只读标签，确认未恢复可执行旧后端后更新这两项理由与摘要，不扩大 allowlist 范围。早期构建中的 Array.at 兼容性问题及 Clippy 的测试 needless borrow 已修正。

日志：

- `/tmp/oviraptor-request-review-full.log`
- `/tmp/oviraptor-request-review-verified-full.log`
- `/tmp/oviraptor-request-review-verified-clippy.log`
- `/tmp/oviraptor-request-review-verified-fmt.log`
- `/tmp/oviraptor-request-review-clippy.log`
- `/tmp/oviraptor-request-review-fmt.log`
- `/tmp/oviraptor-request-review-all-ui.log`
- `/tmp/oviraptor-request-review-build.log`
- `/tmp/oviraptor-request-review-browser.log`
