# 来源关联补证任务：新事实与独立审核收据验收

## 结论与边界

后续状态说明：补证提交的请求键与冻结输入现已由本地 SQLite journal 持久化，可跨组件重开只读对账，并保留已删除任务的永久创建 ID。详见 `NEST_GAP_SUBMISSION_RECOVERY_AUDIT_2026-09-26.md`。这替代下文“该补证提交跨卸载/重开对账未实现”的历史状态；通用启动事务、桌面进程强杀、模型终态/跨 fencing 恢复仍未完成。

本增量接续 `NEST_GAP_FOLLOWUP_HANDOFF_AUDIT_2026-09-26.md` 的草稿交接：来源关联任务现在把原缺证项交给实际执行上下文，再由新任务自己的独立 Reviewer 审核新事实，并持久化原缺口的单独评估收据。仅保存关系、任务结束或模型自称完成均不能使 `gapResolved` 变为 true。

这替代上一份审计中“关联投影一律 false／运行时尚未消费来源关系／缺口解决收据未实现”的当时状态。它不是整体 Stage 9、Stage 10 或 Master Plan 完成声明，也不是外部目标验收。

原任务、原证据和原候选 revision 不被重写；新任务的 revision 从自身证据链开始。跨 root supersedes 仍被禁止，旧 root 的执行权、租约及授权不会复活。原任务页面显示的是关联补证结果，不会因为一个子任务解决一个来源缺口就声称全部缺口已解决。

## 实际执行链

1. `multi_agent_prepare` 在创建新 specialist 或目标 capability 前重验来源。原项目、精确 URL、source hash、已确认 Investigator assessment、独立 Reviewer 和封存 artifacts 不一致时拒绝继续。
2. 执行上下文接收 `followupObjective`，明确原缺证项、原假设和 `targetRequestsGranted:0`。这是任务说明，不是执行授权；新任务仍使用自己的授权与控制组。
3. 新任务获取自己的事实。当前补证证明只接受现有 verifier 校验通过的 Broker HTTP 事实，检查 root、manifest、record/body hashes 和实际文件归属。
4. 独立 Reviewer 接收冻结候选及单独的 `gapFollowup`。即使没有新 finding，关联任务也会执行真实 Reviewer，不沿用普通 coverage-only 任务的跳过路径。
5. Reviewer 分别给出本次 finding 的 `verdict` 和原假设的 `hypothesisVerdict`，逐项说明原缺证项是否补足。所有原项目都必须按顺序保留，`addressed` 必须引用已展示的新任务事实；旧事实、外部 root 和模型编造的引用不被接受。
6. 审核决策、mailbox 确认、child 终态、候选发布、四类真实事件和补证收据在同一业务事务中交付。费用和 received 模型回执单独保留，业务回滚不抹去已发生的成本。
7. 任务详情从数据库读取审核收据，重新验证模型输入/输出、已确认消息、manifest 与 artifacts，再展示解决／仍缺证／无法验证。原任务不需要恢复为运行中。

## 语义与数据约束

- `agent_gap_review_receipts` 以 Reviewer request 为主键，保存来源 hash、assessment 和 resolved；更新被不可变触发器禁止。
- `received_for_audit` 只读取已 completed、预算已结算、lane 已释放、能力已撤销的独立 Reviewer 回执。它不签发租约，也不调用模型。
- 模型请求必须是 schema 1、无工具、system/user 两条消息，user 内容匹配冻结候选；回执中的实际回答必须与落库决策和 assessment 一致。
- 所有原缺证项 addressed 且原假设明确 confirmed/rejected，才可标记对应来源 gap 已解决。缺口补足不等于漏洞被确认；否定原假设同样可能解决该缺口。
- 原假设与新 finding 可有不同结论。没有新 finding 时，两种 verdict 必须一致，不能凭补证审核捏造新的漏洞记录。
- Reviewer 最多看 64 个已验证新事实，并显示省略数量；未展示的引用不得用于解决缺口。不发送原始 body 文件，只发送经过脱敏的 Broker 记录与预览。
- 历史投影选取当前 attempt 的最新审核候选。新 attempt、缺失收据、消息未确认或证据损坏不能继承旧的“已解决”徽标。
- 连续补证的历史假设投影移除 `evidence.followupObjective`，不递归携带先前任务说明。coverage-only 链保留原假设，有新 finding 时选择新的假设；原始封存候选本身不变。

## UI 行为

- 关联子任务展示原缺口是否补足、原假设结论、逐项原因和新 fact refs。
- 来源任务逐个展示补充任务的独立审核状态，和任务运行状态分开；completed 不替代审核结果。
- 已关联子任务事件可触发终态来源页面刷新，但不混入另一任务的聊天内容，也不比较不同任务的独立事件 cursor。
- 已知关联任务仍在 scanning/pausing 时，终态来源页继续刷新；组件卸载和任务切换仍保留迟到响应防护。

## 回归证据

新增 9 个 Rust 测试：

1. 真实 loopback 独立 Reviewer 收据解决来源缺口；不伪造 finding，不改旧 root，重放零模型调用，终态可历史读取。
2. 旧/外部事实、缺失 assessment、错误 source hash、无新事实的 addressed 声明拒绝。
3. 收据 ABORT、IGNORE、AFTER 删除故障完整回滚；解除故障后从原 received 回执恢复，无模型重试。
4. 新旧证据字节、消息确认、模型回执、attempt、收据变更不再显示 resolved。
5. 项目顺序/文本/字段/重复引用/原因/结论严格校验；原假设与新 finding 结论分离。
6. 双线程恢复只能提交一份收据。
7. 来源失效在新 specialist/capability 写入前被拒绝。
8. 没有新事实时持久化 insufficient 结果，执行一次 Reviewer 和一次受限 Investigator，重入不重复调用。
9. 连续 32 层 coverage-only 假设投影不递归膨胀；新 finding 选择正确、历史对象不修改、旧嵌套指导被移除。

新增 3 个实际 Vue SFC 测试：来源缺口与任务状态分离及安全文本显示；多个关联任务状态分别展示；关联事件刷新终态来源且独立 cursor/无关事件不干扰。前端总计 60 项。

## 质量门禁

最终当前代码全量回归已正常退出，主库 800 项、历史导入器 30 项全部通过。早先 `full.log` 的 797 项通过记录不是最终结果；最终以 `current-full.log` 为准。日志前缀 `/tmp/oviraptor-20260926-gap-review-`。

| 门禁 | 当前记录 | 日志后缀 |
| --- | --- | --- |
| Rust 补证定向测试 | 9 通过，0 失败 | `current-targeted.log` |
| Rust all-targets/all-features | 主库 800 通过，285.22 秒；历史导入器 30 通过，3.49 秒；均 0 失败 | `current-full.log` |
| 严格 Clippy | `--all-targets --all-features -- -D warnings` 通过 | `current-clippy.log` |
| fmt | `--all -- --check` 通过，日志为空 | `current-fmt.log` |
| Vue SFC | 60 通过，0 失败 | `current-ui.log` |
| 前端 build | 通过；主 JS 786.50 kB，保留 >500 kB 拆包警告 | `current-build.log` |
| Native 浏览器本地回环 | 通过，8 个请求，双身份隔离 true | `current-native.log` |
| git diff --check | 通过 | 终端直接检查 |

保留的非最终失败记录：`tests.log` 的回执变异夹具先被生产不可变触发器正确阻止，现先断言正常修改被拒绝，再在隔离测试库中移除该触发器模拟磁盘损坏；`clippy.log` 的复杂 tuple/临时 owned 比较已改为命名结构体与解析后的 JSON 比较。不放宽生产触发器或 lint 门禁。

## 尚未完成的产品要求

1. 未确认创建请求跨组件卸载/重启的持久化对账，以及通用确认启动的事务、竞争和清理。
2. 终态/跨 fencing 的通用恢复；当前活跃相同 fencing 的业务补交和终态只读审核不得混为一谈。
3. Stage 9 剩余专家/授权流程、Stage 10 的独立专家，以及更完整的 UI 与资产/知识/技能生命周期。
4. 浏览器、文件等非 Broker HTTP 证据必须各自实现来源校验，不能仅凭字符串引用参与补证解决。
5. 工具沙箱供应、性能、发布安装包和桌面完整流程验收。未部署、未访问外部授权 URL，也未执行目标主机操作。
6. Linux 主机验证仍不在当前可执行范围。默认 Web-only，边界候选仅记录；未来主机审批和执行必须另案实现。

所有本轮网络测试仅使用本地 loopback fixtures；不能据此报告真实授权目标扫描已通过。
