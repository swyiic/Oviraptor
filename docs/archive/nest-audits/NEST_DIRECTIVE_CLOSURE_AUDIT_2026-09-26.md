# 目标结束与人工指令收口审计（2026-09-26）

## 结论与边界

修复的是 **同一目标 Coordinator 结束时，尚未完成的人工指令及其只读评估子任务如何收口**。不是新增主机执行能力，不是完成全部 Master Plan，也不是人工对账工作台已经交付。

原缺陷通过 5 项红回归复现：未落实指令仍 accepted/pending；扫描仍 scanning 时已结束 root 仍可领取指令；未启动子任务未取消；未知调用未保留明确的待对账状态；结束事务中的指令写入失败不影响 root。日志：`/tmp/oviraptor-20260926-directive-closure-red.log`。

## 实际实现

1. `finish_coordinator_run` 在同一 IMMEDIATE 事务内写 root 终态和指令收口。事务获得写锁后再次验证当前 lease/epoch/fencing。任何收口写入失败都会回滚本事务内的 root、child、assignment、预算与指令更新。
2. 指令选择严格匹配 scan、attempt、target、root。已 completed/failed/rejected/deferred 的历史回执不改动；其他目标/attempt/root 不受影响；同一结束动作重放不新增事件。
3. 新增目标级执行检查：领取、确认绑定草案、模型指令上下文、提案开始/响应落盘、子任务调度/启动、提案 mailbox 消费、Executor 续租均不能因 scan 仍 scanning 而越过已结束 root。通用工具 Broker 和 Authorization 三侧 Broker 也检查对应 root 仍可执行。单智能体历史记录只在 policy=single、无 parent/assignment 且 root 为空的明确自根形式下兼容，不放宽 multi 的父子绑定。
4. `payload_json.taskClosure` 保存原状态、处置、root 终止代码、是否需要对账、关闭时间和 `automaticRetry=false`。一旦形成不能覆盖或删除；将来的对账应新增独立回执，不能改写这份历史事实。
5. Native timeline 暴露结构化回执。AgentDialog 区分未启动、未落实、结果未知、响应已保存但回执待提交、旧租约/执行回执待对账；任务结束后不再显示“其他 Web 工作可继续”或普通进行中的提案文案。

## 状态处理矩阵

| 结束前状态 | 结束时处理 | 预算与权限 | 聊天含义 |
| --- | --- | --- | --- |
| pending / claimed / accepted | deferred；`directive_task_ended_without_action` | 不凭空产生结算 | 无落实动作回执，不等于模型从未看到指令 |
| assigned + prepared 提案 | 证明 assignment leased、未开始、child prepared 后取消；proposal failed | 只释放该未启动 child 的未用预留；撤销能力、释放 lane | 未启动，不是“模型返回了无效答案” |
| assigned + executing / uncertain | executing 转 uncertain；assignment/child paused；directive deferred | 保留预算预留与 lane；撤销该 child 能力；不补造零成本结算 | 结果未知，禁止自动重试 |
| assigned + received | 保留响应和 usage；assignment/child paused；directive deferred | 保留预留等待本地对账；撤销能力 | 已有响应，不应再次调用模型，也不宣称业务动作完成 |
| 旧 fencing 的未完成指令 | deferred，标记 reconciliation_required | 不接管、不退还旧持有者的预算/能力；执行端仍拒绝旧 fencing | 需要核实所有权和执行记录 |
| assigned / applied 但无提案回执 | deferred，标记 execution_receipt_missing | 不推断执行成功或真实费用 | 缺回执，需对账 |

## 数据库升级与历史兼容

新增 proposal 状态 guard v2，只允许有未启动取消证据的 prepared→failed。现有数据库通过 `ensure_schema` 替换旧 guard；重复执行幂等；没有取消证据的直接状态篡改仍被拒绝。

全量回归发现跨表 guard 导致历史 `agent_runs.backend` 默认值迁移的表重建失败。迁移现在在原事务内暂存并删除引用 `agent_runs` 的外部 triggers，表换名后按原 SQL 恢复，失败整体回滚。升级测试验证历史行、外部 guard 的实际拒绝行为和新的取消 guard 均保留。

Stage1A 旧库降级夹具在移除后期表/列前同时移除后期 guard，避免构造现实中不存在的“旧表配新触发器”组合。残留字面量白名单只更新 3 个已审查文件的全文哈希：`stage1a_tests.rs`、`db_neutral.rs`、`db_neutral_tests.rs`。分别仍为 fixture/migration/fixture，字面量次数保持 2/10/26，不新增活跃旧后端路径、不扩大豁免范围。

## 验证

新增 **12 项** Rust 收口测试，使用真实 SQLite、线程写锁竞争和本地回环 HTTP：

- 初始 5 项缺陷回归及事务回滚；
- 精确 target/attempt/root 隔离与既有回执保留；
- 旧 fencing 预算/assignment 不接管；
- 已存响应与未知响应区分、暂停后清理、回执不可变；
- root 结束后现有 Executor 的工具权限和续租拒绝；
- guard 升级和重复初始化；
- 等待写锁期间 Coordinator 被替换，旧结束者拒绝提交；
- 真实 `run_agent_target` 编排中收到迟到人工消息，模型服务返回 401 后指令随 root 收口。

定向日志：`/tmp/oviraptor-20260926-directive-closure-green-2.log`，12 项通过。

前端真实 SFC SSR 共 **32 项通过**，新增测试覆盖五种收口文案、互斥展示和 HTML 转义。这不是桌面视觉或 Tauri IPC 验收。生产构建通过；主 JS chunk **775.26 kB**，体积告警仍存在。Native 浏览器本地回环通过，观察到 **8 次请求**，匿名/对照捕获与身份隔离通过。严格 Clippy、fmt 与 diff whitespace 检查通过。

最终全量 Rust **713 项主库 + 30 项导入**全部通过，包含最后加入的写锁竞争和真实编排回归。完整日志为 `/tmp/oviraptor-20260926-directive-closure-final.log`。此前 711+30 的中间检查不作为最终数量。日志位于临时目录，不替代安装包/真实环境验收。

## 仍必须继续的工作

1. `received` 的终止后本地结算/结果 mailbox 补齐、`uncertain` 的费用/结果核对、旧 fencing 的人工对账与恢复 UI 尚未完成。当前是明确保存待对账事实，不是完成执行闭环。
2. 同日后续已删除批量 `finish_applied_directives`，统一编排收口并保留原始/清理错误，新增真实编排故障注入；结算失败不再释放未入账预留。详见 `NEST_FINALIZATION_RECEIPT_AUDIT_2026-09-26.md`。完整恢复 API/UI 及其他崩溃点仍待验收，不能将故障可见等同可恢复。
3. 未确认草案仍可能展示在历史目标中；后端已拒绝其绑定到已结束 root 的执行确认，但完整的草案过期/取消体验仍待整理。未绑定具体 root 的 scan 级草案/指令也不能由单目标结束随意清除，需要单独的 scan 级生命周期设计。
4. 历史事件/源草案或绑定损坏会阻止自动收口，需要明确可操作的修复/对账入口，不能通过绕过校验掩盖损坏。
5. Reviewer 新证据合同签发、补证/重评闭环、Stage 10 专家扩展、沙箱工具供应/部署以及真实授权 URL 与跨平台安装包验收仍未完成。

未部署、未访问用户外部目标、未执行主机操作。全局目标保持未完成。

## 本轮增量：终态提案的静默漏写校验

任务结束事务原先没有核验 `prepared → failed` 与 `executing → uncertain` 两处提案更新。SQLite `BEFORE UPDATE` 触发器若执行 `RAISE(IGNORE)`，可能提交 root 与指令终态而留下旧提案状态。先新增两种状态的故障注入测试；修复前真实红测记录见 `/tmp/oviraptor-closure-proposal-silent-red.log`。现在逐项检查受影响行数，并在同一事务内回读提案 ID、状态与错误代码；不符则回滚 root、子任务、预算及指令变更。`executing` 结果仍为未知，保留请求预留，既不自动重发也不作零成本结算。

新测试还对未派发子任务的 assignment/run 终态更新注入静默漏写，确认现有结束链拒绝不一致提交；该项未另改实现。随后在执行中提案上注入 capability lease 撤销的 `RAISE(IGNORE)`：修复前真实红测表现为 root 可以错误提交终态；现在结束事务检查预期撤销数与最终活跃租约，任何漏写都会整体回滚。`directive_` 专项 **100／100**、`cargo fmt --all -- --check`、离线全目标全特性严格 Clippy 通过。`git diff --check` 检查已跟踪改动；本轮相关新文件尚未纳入 Git，另逐文件检查尾随空白无匹配。完整 Rust 全量曾以单构建作业／单测试线程启动，运行到耗时的源码覆盖测试时为避免长时间占用主机主动中断（exit 130）；**不把本轮新代码标记为全量通过**。此前快照的 1353／30 不适用于本次改动后的全量结论。

模型请求开始后、响应落盘前崩溃留下 `executing`，以及旧 fencing／未知结果的对账和人工收口，仍是独立的未完成工作；本次只修复正常终态事务中的静默漏写。
