# 源码初评生产派发验收（2026-09-27）

## 结论与边界

源码初评已由生产 `run_native_source_scan` 调用，不再只是注册入口和测试夹具：分析器与 canonical import 完成后，创建/恢复真实 Coordinator，顺序派发 RepoMapper、SourceAnalyst，保存真实模型回执、结算 token/request 预留并交付 mailbox，最后收口根运行。

**这是初评，不是完整源码多智能体审查。** 独立 Source Reviewer、SourceBroker 工具执行、真实 CI 审查资格、美元费用账本与任务聊天界面的完整端到端仍未完成。`source_review_not_completed` 和 CI 覆盖缺口继续保留。整个 Master Plan / 当前目标未完成。

本轮没有部署、没有访问用户提供的外部 URL、没有启用 Host Agent。测试只调用临时 localhost 模型服务。Linux 工具沙箱不等于目标主机权限；旧 JSON 不因本改动获得执行权限。

## 生产链路

1. 生产入口在分析器运行前验证发布的源码模型合同；分析器/导入事务与原来的无模型测试 seam 不变。
2. 模型阶段取得独立 OS invocation 锁。它与可续期的 Coordinator 能力租约分离；第二个调用者不能靠复用同一个 fence 抢占运行。
3. 再验证实际模型/任务/私有密钥绑定。沿用按 scan/attempt 唯一、insert-only 的 root 注册规则，不覆盖已有使用量，不从新材料 hash 生成第二份预算。
4. 先生成本轮冻结输入并预估预留，再通过 scheduler 创建真实 assignment/child-run。只授予 `evidence.read` / `mailbox.write`；无工具、Web 请求或主机执行权限。
5. 在写入 `agent_specialist_calls` 的同一个 IMMEDIATE 事务中复核 runtime、模型凭据、endpoint、代理选择、root token/request 上限和预留；INSERT 后再次检查，触发器导致的撤权会连同 dispatch claim 一起回滚。
6. 使用 `complete_once` 发送单次模型请求。provider 错误/取消或不确定 IO 不被自动重发；已有 received 回执用于本地重放，不重新消费额度。
7. 真实回执驱动结果结算和 mailbox 交付/ack。失败清理保留未知用量与预留，不把未知调用当成未调用退款。
8. 两个初评都完成、预留清零、能力撤销后，根运行才记为 `completed_with_gaps`；失败路径记为终止的未完成状态（既有词汇中为 `status=terminal, terminal_state=paused`），未决 child/预算保留。分支报告新增 `sourceMultiAgent`，不足时保留初评缺口。

初评角色当前顺序执行，因为同一源码目标的只读分析 lane 是独占的。这不是两个模型伪装成多个聊天气泡，也不是已实现相互质询/独立 Reviewer。

## 预算、时限与长调用

- 源码请求显式发送输出上限：默认至多 2,048 tokens，本地模型已有更小上限时取更小值。不会以“预留 8,000”为由向云模型发送无限输出请求。
- 输入准入采用序列化消息字节数 + 512 的保守估算，再加输出额度；本地窗口不够直接报告缺口，不无声截掉源码证据。**这不是精确 tokenizer 计数或费用证明**；provider 超出预留时仍不能虚构结算。
- 输出上限、输入、endpoint/proxy hash 等进入冻结请求；公共持久化不存实际密钥。
- 运行期限来自已发布合同。恢复时扣除 root `started_at` 已用时间，不重置总时限；in-flight 取消探针同时检查总期限、assignment/root/fence/能力与当前模型合同。
- 活跃调用的 Coordinator/两项能力租约临近到期时，可在同一 fence 下续期。它不产生新 capability、不重领预算，不续活已过期、撤销、丢失 lane 或非 executing 调用。已用时限不随 heartbeat 延长。
- **美元账本仍缺失。** 明确配置 `maxBudgetUsd` 时，在创建 root 前返回 `source_model_cost_accounting_unavailable`；没有绕过用户费用上限，也不假设本地模型免费。无显式美元上限时仍执行 token/request/time 约束。这只是当前受限路径，不能视为费用功能已完成。
- 代理为空则直连已发布模型端点；配置代理池时仅选择已授权的 `ALL` 路由，无此路由则明确失败，不擅自改走其他路由。该选择与整个代理配置均受发布绑定约束。

## 实现文件

- `src-tauri/src/commands/native_source_scan.rs`：生产入口调用及真实分支报告。
- `src-tauri/src/commands/native_source_coordinator.rs`：模型阶段所有权、准入、两个初评角色、heartbeat、时限恢复与收口。
- `src-tauri/src/commands/multi_agent_runtime.rs`：源码请求输出上限、派发时合同验证和运行中取消/续期。
- `src-tauri/src/agent_runtime/multi_agent/specialist.rs`：事务内 `start_authorized`，派发后复核；无授权回调的 `start` 仅保留测试用途。
- `src-tauri/src/commands/tests_source_dispatch.rs`：8 项新增回归；原真实 transport 夹具也改为在发布之前配置 localhost 模型，不再用未发布的模型替换已登记合同。

## 验证与失败记录

新增 8 项：

1. 真正生产入口调用两个 localhost 专家；断言 wire 输出上限/无工具、两个完成 assignment、两个 ack mailbox、实际 40 tokens/2 requests、0 预留，以及独立审查/CI 缺口仍在。
2. 显式美元上限阻止未计费派发，不创建 root。
3. 凭据、endpoint、代理、预留、截止时间、root 预算、INSERT 撤权变化均不得到达模型；撤权触发器与 claim 整体回滚。
4. provider 500 只调用一次，unknown usage/paused child/预留不丢失，恢复不得重发。
5. 独立 invocation 锁拒绝第二个执行者且不改写 root。
6. cloud/local 输出上限与输入窗口准入。
7. 活跃续期保持 fence/预留/调用数；过期 root/capability、撤销、丢失 lane、忽略写入不能续活。
8. 恢复扣除持久已用时间；过期任务不创建 assignment，不重置 started_at。

首轮记录（不冒充最终全绿）：

- session 76261：源码专家 8 通过/2 失败。旧 fixture 固定预留 8,000，不足新的完整输入保守估算。改为按真实请求计算预留，没有放宽生产门禁。
- session 25044：源码相关 138 通过/1 失败。新测试误把既有未完成状态的持久字符串断言为 `incomplete`，实际 `TerminalState::Incomplete` 映射 `paused`；修正测试词汇，没有改动终态映射。
- session 53240：8 项新增回归全部通过（7.59 秒）。
- session 31174：严格 Clippy 编译发现生产误用 `#[cfg(test)] AgentCompletion::bounded`，因此全量测试未启动。已改为真实账本构造，并加入完成/结算/撤权条件。

最终验证：session 25504 已正常退出 0。严格全 targets/features Clippy（`-D warnings`）通过，6.05 秒；全 targets/features 串行 Rust 主库 **1126/1126**（552.27 秒）、历史导入器 **30/30**（2.23 秒）通过，main target 0 项。此结果只对应本初评增量；后续源码工具授权改动必须单独验证，不能借用此结果。

已收集 session 19248 exit 0：七组实际界面组件 **147/147**；localhost 浏览器 `passed=true`、匿名/对照 complete、身份隔离 true、observedRequests=8；前端 TypeScript/Vite 构建通过。主 JS 830.15 kB 拆包警告仍在。

本轮日志：`/tmp/oviraptor-source-dispatch-{focused,source,integration,clippy,clippy-final,full,ui,browser,build}.log`。

## 下一步的真实缺口

1. 配置化 provider/model 定价、费用预留/结算和未知费用对账，包括 cached input、usage 缺失与显式零费用的人工配置；不得从猜测的市场价或“local”标签推导免费。
2. 让独立 Source Reviewer 以本轮 current revision、持久事实、真实源码 root/assignment 身份进行审查；改掉 SourceBroker 的 Web-only 授权依赖。两个初评的 summary 不是审查裁决。
3. 接入 SourceBroker scoped 工具/分析/复核与收口，再由真实审查资格驱动 CI，不能直接删除 `source_review_not_completed`。
4. 尚需补全异常持久化/租约收口失败时的自动对账和用户可见恢复入口；当前保存未决事实不等于自动恢复系统已完成。
5. 源码人工指令的消费/回应、角色间质询、真正聊天 UI 端到端、沙箱/工具供应、知识/skills 治理和资产汇总继续按 Master Plan 完成。当前 UI 147 项通过不证明这些新增完整体验已验收。
6. 真实 pinned analyzer 容器 + 模型 + Reviewer + 安装包部署验收仍待进行。本轮生产入口测试覆盖的是“分析器缺失如实报告 + 模型初评”，不是声称在本机执行了真实 Semgrep/CodeQL 容器。
