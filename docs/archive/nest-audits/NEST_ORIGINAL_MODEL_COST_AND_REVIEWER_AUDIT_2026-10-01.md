# 原模型费用、Reviewer 闭合与聊天日志开发审计（2026-10-01）

持续 Goal 保持 active，Master 未完成。此批为框架开发及负向回归，尚未做最终桌面、安装态或两个授权 URL 的整体验收。全部 SQLite/文件/I/O 用例仅为临时库、临时目录及 localhost；没有真实 DB/CAS/资产/安装包或提交改动。HEAD 保持 `59be3d86d25adda5b1f975759bf256ef3b94f32b`；保留此前所有未提交成果，没有重置或批量覆盖。

## 先证明问题与作用域

- Source/专家原费用：`/tmp/oviraptor-original-source-specialist-red.log` **0/2，exit 101**。真正接管后已收到 provider 费用，两项原生产回执 API 都拒绝整个事务，原 model request 费用仍为零。测试覆盖同 root 新 Coordinator generation 和不同 root 接管，逐表比较，不允许账单成为业务发布或恢复权限。
- 单轮上限：`/tmp/oviraptor-original-source-round-cap-red.log` **0/1，exit 101**。Source 单轮持有 8,000、实际精确分类合计 8,001、logical assignment 还持有后续轮次余额，旧算法会消费后续余额并认作已知。最小修复使 `estimate` 同时成为原单轮的计费判定上限；超出保留该轮 8,000 未决，真实 usage 留在费用事实内，禁止自动结清。
- 未报告用量：`/tmp/oviraptor-original-model-unknown-admission-red.log` **0/1，exit 101**。合法 unlimited/零 token 预留 worker 的原迟到估算只消费 model request，没有 token forfeit，原 admission 漏掉未决。现在直接核验费用事实的 uncertain、未报告或不完整分类，不从余额零推断已知费用。
- Reviewer：`/tmp/oviraptor-reviewer-final-attempt-red.log` **1/3，exit 101**，证明最后 publication/Gap receipt 清空原 worker 完成时间仍被接受。扩大保留 failure、heartbeat、deadline、原 run/assignment 故障及已失败/过期保存回执恢复断言。随后 `/tmp/oviraptor-reviewer-final-coordinator-red.log` **0/1，exit 101**，证明最后 publication 更换同 root Coordinator fence 可逃逸；有效根状态不足以证明本次发布权限。
- 聊天：原生产 composable 六项生命周期测试 **0/6**（`/tmp/oviraptor-chat-status-lifecycle-red-final.log`），另两个实际边界失败记录在 `...-fallback-red.log` 和 `...-stalled-read-red.log`；不是气泡或 UI 模拟生成状态。
- Runner：`/tmp/oviraptor-runner-log-write-red.log` **1/5，exit 101**。直接读盘证明 password/API Key/Bearer 及 stderr 诊断仍含秘密；正常长文本/多行追加护栏为绿。误用 `runner_log_persisted_` 过滤的零项运行不作红测证据。

## 最小修改与持久边界

新增独立 `agent_model_cost_facts`，仅保存 family、原 root/assignment/attempt UUID/child、原 Coordinator epoch/fence、round、request hash、phase 和费用 hash/usage。主键按 family/child/round，终态不可替换/更新/删除，无 task FK 级联；不复制模型文本、请求内容、工具参数或凭据，不回填旧权限。

当原 Coordinator lease 失效时，Source received/uncertain 与专家 received/uncertain/typed unsent 仅走费用通道。事务开头捕获同一 OriginalReceiptOwner 和完整原 dispatch 行，验证原 request hash、role、child、epoch/fence、额度、状态、完整字段；费用最后写入后再次核验同一证明。known 只消费原 worker；unknown 按原轮 estimate 保留未决；unsent 不退款、不重发。业务 journal/event/checkpoint/messages/tool receipts、worker、capability/lane、Root 和新 Coordinator 均不变，received 返回原权限错误而不交付执行响应。普通 live 回执仍保持原事件/检查点/费用整体事务。

测试证明费用/最终 fact trigger 中删除或改变原 call、改变原 child role/lane、改变 worker 审计字段、保存事实后重放/改变 phase 均拒绝或整体回滚。真实 localhost 专家调用在返回前接管，原账单保存、只发一次；旧响应不产生 model completion event，第二次调用在出网前拒绝。原费用 API 不是旧 worker 的执行授权。

Reviewer 在原 child 完整关闭后捕获全部 attempt/assignment/run 持久列、原 UUID/worker/fence、关闭事实及无活 capability/lane，保留到最后 Gap receipt 后比较同一份证明；不在末尾用新读取构造替代 owner。末尾还验证原 Coordinator epoch/fence/有效期及根任务可发布状态。已失败 worker 的本地保存结果可发布，其失败审计不变；普通过期执行继续拒绝。此批仍没有生产 expired 收尾或重派。

聊天订阅单飞重试，成功后从数据库补读；隐藏取消读取/重连 timer，可见合并补读，晚到 listener 在卸载时释放。保留游标/任务围栏和旧断言。Runner 仅对新增持久诊断写入前脱敏，不用 UI 的 1,200 字符上限裁剪完整日志；历史文件没有改写，Native JSON stdout 保持字节/解析合同。

## 验证与当前快照

- 初修 Source/专家 **2/2**；费用/Reviewer专项先 **10/10**，随后 **11/11**。均与扩大集合重叠，不相加。
- `/tmp/oviraptor-original-source-specialist-reviewer-log-affected.log` **270/270，exit 0，569.68 秒**，1,366 项未运行。覆盖预算、attempt、Source/专家、Reviewer/保存回执、消息、HTTP、请求审查及日志；发生在最后 Coordinator fence 增补之前。
- 最后只改 Reviewer delivery 与其专属测试两文件；`/tmp/oviraptor-reviewer-final-authority-affected.log` **31/31，exit 0，37.97 秒**（1,606 项未运行），最新 Reviewer 发布/恢复/Gap/请求审查路径再次通过。其他费用与日志源码逐文件摘要不变；不把前一 270 集合借作最新完整全量。
- 聊天状态/事件 **94/94，exit 0**：`/tmp/oviraptor-chat-status-lifecycle-final.log`。`/tmp/oviraptor-chat-status-lifecycle-typecheck.log` TypeScript **exit 0**。这证明生产 composable 与类型合同，不是实际 Tauri 断线/可见窗口验收。
- 当前严格全目标全特性 Clippy **exit 0，29.57 秒**：`/tmp/oviraptor-original-source-specialist-reviewer-log-clippy.log`。退役字面量登记 **1/1，exit 0**；作用域 rustfmt 与差异空白检查通过。没有重跑最终 all-target/all-feature 全量测试、完整 UI/构建或更新安装包。
- 820 源码/测试/配置路径最终摘要：`/tmp/oviraptor-original-source-specialist-code-snapshot.json`，集合摘要 `c3571512c1be8e387a43fba498575bede4450fb647d9ec9caa89635b5b904e3a`。计数范围与此前 832 文件摘要不同，不能据此推断删除文件。
- 新增及修改责任文件均小于 400 行：model facts 156、dispatch proof 85、receipts 237、Source dispatch 365、specialist 341、Reviewer delivery 284/helper 102/test 342、费用测试 274/141、runner测试 132、聊天 composable 274/测试 199/入口 325。

## 未完成与下一批

安全 reassign/ordinal 2 仍没有生产 API，逻辑 Assignment 仍含 Leased/Running/LeaseExpired。模型/工具/capability 主键仍以 logical assignment 为中心，必须隔离原 worker 历史后才能重派；不得放宽状态、改 current child pointer 或重放原未知调用来伪造恢复。

下一批先让已保存结果在 `worker=expired` 时本地收尾且原 worker 保持 expired 完整审计；再在 IMMEDIATE 内精确核验原 UUID/worker fence/deadline与当前同 root Coordinator，仅撤旧执行权、暂停逻辑 Assignment/run。第一切口保留 lane/contract/slot 及未知预算，不调用普通 finish 的 unsent 退款，不恢复 Root、不重派；必须用双连接 barrier 证明与续租竞争唯一胜者。尚未实施这一 API，也没有真实 Supervisor/强杀恢复。

Source 工具模型的 typed unsent 与完整跨 generation/删除恢复、Root/单智能体全费用、动态 grant/精确人工对账、Supervisor真实事件tick与全角色/模式/Reviewer矩阵、复杂聊天四态和工具开始/完成回执、AST/浏览器/外部进程逐行日志仍待实现。公共进程下一切口应沿真实 `docker start --attach → process::run` 增加有界observer与scan/attempt/process/stream/sequence持久日志，先证明进程结束前已落盘，再验证断线、强杀及JSON字节合同。

纠正旧历史快照：安装日志已有持久回放（installation_logs/journal.rs 和 useInstallLogPanel.ts），但仍缺安装批次身份及真实安装/断线验收，不能继续把“没有持久回放”写作当前事实。精确旧数据盘点/可恢复备份与授权清理流程、最后完整门禁/当前安装态/两个给定匿名只读 URL 也未完成。用户登录身份待后续提供；本批没有访问授权网站。Goal 保持 active，不能标记 Master complete。
