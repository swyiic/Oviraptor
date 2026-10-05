# External Surface 匿名入口合同增量审计（2026-09-26）

## 1. 范围与结论

本增量实现 Stage 10 第一个 External Surface 合同：对冻结授权入口进行一次匿名 GET，由独立模型上下文分析实际响应，通过持久化 assignment、child run、mailbox 和证据图交付给 WebExecutor。不是所有公开面能力、所有专家或整个 Master Plan 完成。

没有新增 Host Agent，没有部署，没有访问用户提供的外部 URL。网络测试仅使用 localhost 受控目标和模型响应器。

## 2. 生产执行合同

- HTTP(S) 精确入口，禁止 userinfo 和被已知凭据脱敏规则识别的目标；去掉 fragment。没有 Cookie、Authorization 或身份复用，不跟随重定向，不枚举路径，不执行浏览器和主机动作。
- 不能把 URL 过滤当成任意查询参数均不含敏感值的证明；最终匿名目标仍需依赖授权输入。
- ExternalSurface 持有独立 TargetTouching lane 和限定 `public_surface_get` 能力；通用 replay/compare/discovery/browser/finish 工具不能经此角色调用。
- 每次采集前核验 scan/attempt/target、冻结 Web 计划、Coordinator fencing、根任务状态、assignment/run/lane、租约有效期/撤权、预算及熔断。
- 在 SQLite FULL synchronous 与 IMMEDIATE 事务内先写唯一 claimed 记录，检查真实写入结果，提交后才允许网络派发。checkpoint 缺失视为新状态，损坏/版本不支持/负消耗拒绝，不能借解析失败重置预算。
- 已领取和结果未知的记录保守占用共享目标请求额度。同一 attempt 不自动重复采集；恢复需要另行授权的新尝试，不删除旧 claim。
- 接收后保存私有响应文件、JSON 与摘要、真实 observed 证据，再将 claim 转为 received。写入忽略、触发器破坏、证据落库失败不能被当成成功。
- 重定向不跟随；401/403 保存为实际观察，不自动认定漏洞；429/WAF 保存回执后按相应保护停止码返回 Limited，不进入模型分析，不当成普通初始化错误。
- 响应体有上限，传输有连接/总超时；取消在派发前及落库前重查。当前在途阻塞 HTTP 不保证瞬间取消，已发生效果不能撤销。

## 3. 真实协作与证据真实性

- Coordinator→ExternalSurface 的执行指派及 ExternalSurface→Coordinator 的结果摘要都真实写入并定向确认；不是 UI 模拟聊天。
- 子角色独立模型请求、事件、snapshot、预算结算；模型不获得工具，输出是限定 JSON，必须保留覆盖缺口，禁止 confirmedFindings=true、未知字段和空白条目。
- 页面摘录明确标为不可信输入。模型总结不是 observed 证据，也不能自己确认漏洞。
- 结果交付前重新核验 received 回执、完整 request/response、摘要与私有文件，拒绝模型调用期间发生的文件替换或损坏。
- Reviewer 只能引用经过验证的采集证据，要求原 child 已完成、assignment 已结算、回执与证据绑定一致。删除回执后的孤立图节点不能继续用于审核。
- WebExecutor 的 evidence digest 和 assignment 接收实际公开面观察；相同 attempt 重入不会发第二次请求。
- 模型预算不足时返回 not_scheduled，不伪造完成、不创建空任务。目标请求成本与模型调用成本分别记录。

## 4. 新增 12 项回归

1. 真实 localhost 目标与模型：一次匿名请求、独立模型、两条确认消息、已结算 child、真实事实及 Reviewer 资格。
2. 真实 bootstrap 到 WebExecutor 的证据交接、重入不重发。
3. 302/401/403/429 行为与保护停止码。
4. claim 的 IGNORE/ABORT/后续篡改：零网络派发、事务回滚。
5. 撤权/过期/陈旧 fencing/计划变化/暂停/目标变化：零派发。
6. 回执更新或事实插入失败保留 claim，不能重发。
7. 非 JSON、擅自确认漏洞、空白覆盖缺口不能完成。
8. 不足模型预算不调度、不采集。
9. 模型调用期间修改原始文件：结果交付拒绝，费用仍保留。
10. 响应期间暂停：保留 claim，不调用模型、不重放。
11. 通用目标工具拒绝与损坏 checkpoint 不重置预算。
12. 结果确认静默忽略：保留未结算费用/lane、撤销能力、不重复目标或模型请求。

现有两个生产入口测试同时强化为：四个真实 run、三个 child assignment/snapshot/模型角色、五条已确认消息、精确 received 回执和一次匿名入口 GET。未通过关闭生产 ExternalSurface 来维持旧计数。

## 5. 回归过程中发现的问题

- 首轮定向测试 7 通过/2 失败：测试使用了错误 mailbox 字段，并把新的取消检查器误当成共享 token。修正为真实字段与数据库暂停；后续 9 项、扩展后 12 项定向通过。
- 首轮完整主库回归 891 通过/3 失败：预算测试使用不存在的数据库引发下溢；两个旧编排测试仍期待三个 run。改为初始化真实数据库并显式核验可用预算，更新计数并增加真实事实断言。
- 新增强断言的第一次定向复测 1 通过/1 失败：测试误用 assignment_id/status，修正为实际 id/state 后两个生产编排测试通过。
- 手工复核 `agent_tests_e2e.rs` 的九处历史/负控制 Strix 字面量，均未变化且不是生产执行入口；仅更新该测试文件的退役审查摘要与理由，没有增加豁免文件或关闭门禁。

## 6. 最终验证

最终完整回归与检查已收集，相关进程退出码均为 0：

- Rust 全 targets/features：主库 **894 通过、0 失败**（373.51 秒）；历史导入器 **30 通过、0 失败**。
- 严格 Clippy（全 targets/features，`-D warnings`）通过。
- `cargo fmt --check` 通过。
- 实际 Vue SFC/状态测试 **101 通过、0 失败**。
- Native localhost 浏览器回环通过：8 次请求、匿名采集 complete、对比采集 complete、身份隔离 true。
- 前端构建通过；主 JS **800.39 kB** 的拆包警告保留。
- 退役允许清单 61 个文件摘要核对无过期项，完整测试中的退役检查通过。
- 文档收口后 `git diff --check` 通过。

日志前缀：`/tmp/oviraptor-20260926-public-surface-`。最终 Rust 使用 `verified-full.log`，Clippy 使用 `verified-clippy.log`，fmt 使用 `verified-fmt.log`；前端使用 `ui.log`、`native.log`、`build.log`。

## 7. 尚未交付及不得夸大的边界

后续更新：跨角色显示／预算记账已由 `NEST_TARGET_REQUEST_ACCOUNTING_AUDIT_2026-09-26.md` 对应增量处理。以下旧统计缺口保留为本次匿名入口交付时的历史记录；后续投影也不包含前置侦察或尚未持久化的崩溃窗口流量。

- 只实现匿名入口 GET，不实现本角色的 browser/discovery、更广泛采集、其余 Stage 10 专家或自动主机操作。
- captured 请求已在专门回执与共享目标预算中记录；旧 native checkpoint/部分运行统计仍以 WebExecutor 为计数来源，跨角色目标请求总数的统一显示尚需补齐。不能将旧界面数字当成所有角色的完整流量统计。
- claimed/未知效果跨 fencing 对账和通用恢复尚未实现；不靠重试解决。
- 在途目标请求撤销不保证立即生效；仍受总超时约束。
- 原始私有文件使用目录隔离/0600，不是任务密钥加密。
- 完整 Web→主机语义隔离、主机授权 UI/API 未完成；见 `NEST_HOST_BOUNDARY_DECISION_2026-09-26.md`。
- 真实部署、用户授权 URL 验收、沙箱完整供给与性能、资产/知识闭环、其余 UI 重组及整体 Master Plan 仍需继续。
