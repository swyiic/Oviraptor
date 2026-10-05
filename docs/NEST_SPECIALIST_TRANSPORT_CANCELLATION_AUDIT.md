# 专家模型传输解耦与子任务停止审计

状态：共享生产模型传输已不依赖 Web 权限上下文；子任务级在途停止已接入并通过 localhost 回归。**这不是源码 Coordinator 自动编排、独立源码 Reviewer 或 Master Plan 完成交付声明。**

## 1. 实际修复

此前源码专家的真实 transport 测试借用了 Web `AgentRunContext`。生产共享取消探针只检查 scan/attempt，单个 child 或 root 被取消时，整个 scan 仍在运行，模型请求可能继续等待。

本次在 `commands/multi_agent_runtime.rs` 中：

- 抽出 `SpecialistTransportContext` 与 `specialist_round_transport`。只持有数据库路径、scan/attempt/target/run 绑定、模型配置、代理与用量日志目录；没有 Web execution plan、browser、身份或工具授权。
- 原 Web 入口仅映射上述字段，继续复用同一模型网关和 specialist journal，没有新增模型客户端。
- 源码 localhost 测试直接构造这个最小上下文，不再构造虚假的 Web execution plan。
- 在途取消绑定真实 Coordinator fence、assignment、child/root 状态和取消标记、同一 scan/attempt/target/role/lane、有效 `evidence.read` 能力租约及实际占用的执行通道。
- 数据库不可读、全局停止或换 attempt、子任务暂停/取消、根取消/终态、fence 替换、能力撤销/过期、通道丢失都会使取消探针拒绝继续等待。
- 局部子任务、根任务或其租约变化不会停止同一扫描的其他目标；整个 scan 停止或 attempt 更换则使所有旧分支停止。

停止的是本地 HTTP 等待和后续处理，不承诺供应商已停止计算、免收费用，或撤销已经发生的外部效果。

## 2. 回执与未知结果

保持原 at-most-once 模型日志规则：

1. 请求派发前先持久化 executing claim。
2. 取消后不能写假模型成功事件、已完成 mailbox 或零成本成功。
3. 源码合同已被撤销时，旧 fence/角色不能再写 received/uncertain 回执；原 executing claim 保留为待对账。
4. 即便后来恢复取消标记，也不能将未确认的 executing claim 当成尚未执行而自动重发。

本次没有补齐未知模型费用的人工对账 UI，也没有借取消探针证明完整主机授权或所有工具的在途撤权。

## 3. 实际验证与失败历史

- 最终定向 session 68082 退出 0：`source_specialists` **9 passed**（含上一增量 7 项及本次 2 项）；`retirement_` **4 passed**；严格 Clippy 通过。
- 界面 session 24068 退出 0：聊天、工作台、运行状态、请求用量/复核、任务/项目删除共 **147 passed / 0 failed**；日志 `/tmp/oviraptor-source-transport-ui.log`。后续 session 63757 退出 0，前端构建与 localhost 浏览器流水线通过，日志 `build.log`、`browser.log`（同 `/tmp/oviraptor-source-transport-` 前缀）；浏览器回执 `passed=true`、8 个观测请求、匿名/对照均 complete、身份隔离通过。
- 取消探针测试覆盖 12 类变更及数据库不可读；为同一扫描建立另一目标的真实只读 root/assignment，核验局部取消与全局停止的不同效果。
- 真正 localhost HTTP 测试等待供应商收到请求后，取消 child；供应商尚未响应时 transport 已退出。验证 scan 仍活跃、无成功事件/消息、原 claim 未被消除、恢复取消标记后不重发。
- 正常 localhost 测试仍验证两角色各一次真实请求、真实回执和 acknowledged mailbox，输入篡改在网络调用前拒绝，已完成结果重放不再次调用模型。

日志：

- `/tmp/oviraptor-source-transport-focused-v4.log`
- `/tmp/oviraptor-source-transport-retirement.log`
- `/tmp/oviraptor-source-transport-clippy-v4.log`

失败历史保留：初版测试的 scoped thread 捕获 sender 生命周期错误，只调整测试捕获；第二版将两个同目标只读角色同时安排进互斥通道，得到 `agent_lane_occupied`，改为另一个目标的真实 sibling，不放宽生产通道互斥。

上增量完整 session 6329 已退出 101：主库 **1090 passed / 1 failed**。唯一失败是 REM-012 审核摘要与修改后的 `contract.rs`、`tests_native_ci.rs` 不符。已人工复核 Native 角色枚举/解析与新增未复核 CI 用例；三个兼容字面量和单个历史导入断言未扩展为可执行后端。只更新这两个文件的审核理由与全文摘要，没有删除检查或扩充扫描豁免。

本次完整串行 session **41545 已退出 0**：主库 **1093 passed / 0 failed**（526.12 秒），历史导入器 **30 passed / 0 failed**；随后严格 Clippy、fmt 和空白检查均通过。日志 `/tmp/oviraptor-source-transport-full.log`、`clippy-final.log`、`fmt-final.log`、`diff-final.log`（后面三者同 `/tmp/oviraptor-source-transport-` 前缀）。运行期间没有修改 Rust 源码或构建输入；该结果证明的是此传输增量，不包括之后的源码发布合同修改。

## 4. 下一步仍必须完成

1. 在工作台发布阶段冻结源码模型配置、预算与人工指令，并将真实 Source Coordinator 注册/恢复接入生产源码启动器；不得临时借 Web 计划或忽略用户预算来接线。
2. 生产调度 RepoMapper/SourceAnalyst，处理每个阶段的失败、停止、未知结果、费用和最终终态。当前只读初评不是完整深度代码审计。
3. 完成逐工具源码合同与持久 assignment.finish、独立源码 Reviewer、精确本轮候选 revision 的 CI 资格及真实 UI 事件接线。
4. 完成主计划剩余沙箱/工具供应、知识治理/资产、安装包及授权环境验收。

本轮没有部署、没有访问用户公网目标、没有新增 Host Agent 或放宽主机边界。
