# 已确认人工指令 → 独立只读 Agent 评估审核

日期：2026-09-26。本文是增量交付记录，不是完整 Master Plan 验收报告。

## 续核：完成交付的历史投影与模型上下文完整性

本轮门禁续核（替代本节下方“尚待执行”的旧进度描述）：最终修正后离线全特性 `directive_` 专项 **97 通过／0 失败**，日志 `/tmp/oviraptor-proposal-integrity-directive-green.log`。严格全目标、全特性 Clippy `-D warnings`、`cargo fmt --all -- --check` 与 `git diff --check` 已通过；Clippy 日志 `/tmp/oviraptor-proposal-integrity-clippy.log`。最新代码完整 Rust `--all-targets --all-features` 单作业、单线程回归已正常退出，主库 **1353／0**、导入器 **30／0**、二进制 **0／0**，日志 `/tmp/oviraptor-proposal-integrity-full-current.log`。完整 UI **248／0**、生产构建及本地 Native 浏览器回环重新通过，日志分别为 `/tmp/oviraptor-proposal-integrity-{ui-current,build-current,native-runtime-current}.log`；聊天 **99** 项为本次先前的专项结果。主 JS **867.35 kB** 分包告警仍在。此前 **1348／30** 是本次提案修复之前的队列快照。完整目标、真实桌面／平台与安装包验收仍未完成。

发现：原上下文查询直接 inner join 提案表并读取 `response_json`，聊天直接读取提案摘要；完成后删除结果 mailbox、清空 ACK 或损坏结算/原确认时，没有共同的历史证明检查。完成 UPDATE 与协作事件被 `RAISE(IGNORE)` 静默跳过时，原收尾路径也缺少完整提交后置校验。

本增量接入统一只读校验：原确认草案及冻结 payload、原领取 fence、root/assignment/child 绑定、请求与结果 mailbox 的完整路由/内容/消费、模型事件、实际费用结算、能力释放及状态协作事件顺序。正常完成、完成重放、终止后的本地补回执、聊天和后续模型上下文共用它。历史验证不要求当前可执行租约、不重查后来变化的图版本；执行入口的当前授权检查保持原样。

上下文选择与证明在同一个读事务内完成；先核验全部终态候选，再选择至多五条成功建议，避免损坏/删除的提案被 inner join 或 LIMIT 静默隐藏。已损坏记录拒绝作为模型输入，不调用模型来补造回执。聊天隐藏成功摘要及本地成功标记，关联结果消息也显示未核验，而不是仍显示原摘要。未派发就取消的子任务保留原失败状态，不强制制造模型交付证据。有效交付仍只代表收到建议，不代表建议执行或独立 Reviewer 通过。

这是跨记录一致性检查，不是对任意整库协同篡改的密码学证明；完整历史读取仍有随记录数量增长的成本。未增加角色、工具、权限、目标请求或主机执行能力。

验证进度：聊天模板先红后绿，99 项通过；完整 UI 248 项及前端构建通过，主 JS 867.35 kB 体积告警保留。Rust 新回归覆盖成功/无效响应下的记录损坏、静默漏写原子回滚和终态/新 fence 的只读历史。**新 Rust 回归、严格 Clippy 与新代码全量尚待执行**；正在运行的 `91631` 是先前队列增量编译，不包含本提案修复。没有执行 Rust 修复前红测，不将静态分析称作红测。前端日志位于 `/tmp/oviraptor-proposal-integrity-{ui-red,ui-green,all-ui,build}.log`；临时日志不是永久发布凭据。

下文为各次历史快照，不替代本节最新状态；整体目标保持未完成。

## 已实现的路径

确认后的单角色 `@mapper` / `@investigator` 提案现在创建真实 Assignment、child run、预算预留和请求 mailbox，并由独立模型请求评估冻结的 Web 证据。请求被消费后才允许启动；启动使用事务 CAS，只允许一个调用者取得执行权。结果持久化后才结算、结束 child、消费结果 mailbox 和完成人工指令。

模型调用不提供工具，不访问目标，不增加主机权限。完成只代表 Coordinator 收到只读评估，绝不代表建议已执行、漏洞已证实或 Reviewer 批准。聊天时间线显示真实提案状态，并转义模型摘要。

新增持久化表 `agent_directive_proposals`，状态为 prepared / executing / uncertain / received / completed / failed。不可变关联、响应写入条件和状态转换由数据库触发器约束。恢复时重新验证原文、确认草案、证据引用、revision、scan/attempt/target/root 和 fencing；不采用新租约下的旧执行权。

## 预算、上下文与恢复边界

- 每条提案确认预留 4000 tokens、1 次模型请求。旧估算草案要求重新确认，不自动扩大先前许可。
- 初始预算充足时从 WebExecutor 分配中留出一条评估的空间，不增加根预算；Reviewer 的 8000 tokens / 1 request 底线保持。小预算或后续余额不足会明确 deferred，不宣称无限持续群聊。
- 冻结输入最多 2600 字节。过长证据使用明确标记的 UTF-8 摘录，记录源摘要和字节数；不静默截断人工指令。模型输出最多 512 tokens，并验证 JSON 字段与长度。
- 每轮只带回最近 5 条同范围已完成评估，明确标作不可信建议，而非工具权限或事实证据。
- 通用模型客户端原有的云端自动重试可能让一次预留产生两次 HTTP；本路径改用 `complete_once`，503/401 均不会隐式重试。其他调用路径不受此改动影响。
- 响应先落盘，结算、结束 child、结果 mailbox、指令回执的后续故障可恢复，不重复发起模型 HTTP。无效 JSON 会结算已发生的调用，但指令为 failed。
- 模型调用报错时结果未知，保留预留并标 uncertain，不按零成本处理、不自动重试。若进程在 executing 提交之后、响应落盘之前崩溃，仍可能留下 executing；自动对账和人工收口尚未实现。

## 已观察到的验证

新增定向测试 12 项通过：原子调度/回滚、真实 Mapper 与 Investigator HTTP、无效输出、401/503 不重试、角色及容量延期、源内容篡改、并发启动去重、四类落盘故障后的恢复、证据摘录、真实 Native 模型上下文、真实 WebExecutor child 读取根 inbox 与 Reviewer 预算保护。

这里的 HTTP 是本地测试服务，不是外部模型厂商或授权目标。测试没有借用已有 specialist 的测试替身。

完整验证结果：

| 验证 | 结果 |
| --- | --- |
| Rust 全目标、全特性、离线串行测试 | 主库 695、历史导入工具 30，通过 |
| 严格 Clippy（`-D warnings`） | 通过 |
| 聊天组件测试 | 31 项通过，含真实 SFC SSR 文案与转义；非浏览器视觉/Tauri IPC 验收 |
| 前端类型检查和生产构建 | 通过，主 JS chunk 773.68 kB 告警仍在 |
| Native 浏览器本地回环 | 通过，观察到 8 次请求，身份隔离通过 |

全量 Rust 日志：`/tmp/oviraptor-20260926-proposals-full-rust.log`。定向日志：`/tmp/oviraptor-20260926-proposals-tests.log`。503 隐式重试的修复前失败记录：`/tmp/oviraptor-20260926-proposals-retry-red.log`。临时目录日志不作为永久发布证据。

## 不得据此宣称完成的部分

1. 多角色指令拆分、其他专家执行、Reviewer 的新证据/候选版本再审尚未由此路径交付；不支持的提案明确延期。
2. uncertain、旧 fencing 指令的对账/重确认 UI 仍需完成。后续已接入目标结束事务与结构化收口聊天回执，详见 `NEST_DIRECTIVE_CLOSURE_AUDIT_2026-09-26.md`；这不等于终止后本地补结算或人工对账已完成。
3. 首次审核发现请求 mailbox 的“查 acknowledgement → deliver → acknowledge”存在并发竞争窗口。后续已复现并修复，见下文；本条不再列为该提案路径的未修复问题，不代表其他角色的 mailbox 已完成同等并发验收。
4. 真实沙箱/工具镜像、Linux 部署、安装包、外部授权 URL 验收未进行。
5. Web→主机开关仍是 Master Plan §6.4 的设计，不是已实现主机审批后端。保持 `web_only`，无可执行 Host Agent；Linux 工具运行环境不授予目标主机权限。

整体目标保持未完成；后续工作须保留以上缺口，不以本轮测试通过替代产品验收。

## 同日后续：并发消费与原子收尾

先红回归实际复现四项问题：四个工作线程同时运行完整 apply 时，某一轮因消息已被另一线程确认而误报失败；ack 故障后仍留下 delivery 时间与计数；已 ack 重放绕过当前租约/任务活动状态；已完成提案接受不匹配的 result message ID。失败日志为 `/tmp/oviraptor-20260926-proposal-concurrency-red.log`，不是仅靠静态推测。

修复后的请求消费在同一 IMMEDIATE 事务中验证 lease、活动 attempt、Assignment 关联、角色、消息路由、证据 revision 和完整 payload，并原子写入 delivered/ack。已 ack 的合法重放不重复增加计数，但仍执行全部验证。

响应落盘后的预算结算、child 结束、结果 mailbox 插入/消费和最终指令回执现在共用一个事务。此事务不包含任何模型或目标 I/O。任意一步失败时整体回滚；并发恢复持有同一写锁，不再用锁外读取的 child 状态决定重复 finish。完成态重放仍验证结果消息 ID、payload、已结算预算、child 与 directive 状态，不能以“已经完成”为由接受不匹配回执。原有普通 child 的结算/结束入口保留独立事务包装，复用新增事务内函数。

新增 6 项测试，提案定向测试共 **18 项通过**。除以上四项外，覆盖预先落盘的有效/无效响应在四个完整 apply 入口中的并发恢复，以及已 ack 消息路由被改动时拒绝重放。原有四类收尾故障注入测试增强为：失败后 child 仍 running、结算时间为空、账本 spent_requests 为 0、结果消息不存在；恢复后只有一笔结算。定向日志为 `/tmp/oviraptor-20260926-proposal-concurrency-green.log`。

这些测试使用本地真实 SQLite 与回环 HTTP，不证明外部模型服务、安装包或其他角色所有路径的可靠性。未知模型结果、任务结束时未落实指令、旧租约人工对账、多角色拆分与 Reviewer 补证再审仍未交付完整闭环。

后续修复的完整门禁：Rust 主库 **701 项**、历史导入 **30 项**通过（全量日志 `/tmp/oviraptor-20260926-proposal-concurrency-full.log`）；严格 Clippy、fmt、聊天组件 **31 项**、生产构建、Native 本地回环 **8 次请求**与 `git diff --check` 通过。前端没有新增改动，主 JS chunk 仍为 **773.68 kB** 并保留体积告警。未部署、未访问外部目标、未引入主机能力。
