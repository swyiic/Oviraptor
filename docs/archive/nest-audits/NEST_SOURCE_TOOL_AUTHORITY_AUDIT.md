# Native 源码工具授权验收（2026-09-27）

## 结论与范围

源码工具现在具有独立于 Web execution plan 的真实 Native 授权路径：使用发布的 source runtime、冻结分析视图、真实 root/assignment/child-run、逐工具 capability 和当前 evidence revision。原工具派发入口已连接此路径；源码读取不再消费 Web 目标请求预算。

**这不等于源码自动工具编排已经完成。** 生产 `run_native_source_assessments` 仍只调度两个 tool-free 初评，没有自动调度 `source_tools` phase，没有多轮模型工具循环、持久 `assignment.finish` 或独立 Source Reviewer。当前初评收口还是两个完成 assignment；接入新阶段时必须同时调整收口条件，不能提前结案。`source_review_not_completed` 继续保留，CI 不获得审查通过资格。

没有启用 Host Agent、部署程序或访问用户目标 URL。旧 JSON 继续是历史数据，不因导入获得 source assignment 或执行租约。

## 实现合同

1. `multi_agent/source.rs` 提供独立 `source_tools` task slice。角色仅 RepoMapper / SourceAnalyst，工具列表固定，绑定真实 root 当前 revision；不允许把工具 phase 作为 tool-free 初评提交。
2. RepoMapper 不授予候选写入；两个角色均不授予网络、shell、主机动作或 review 权限。未实现的 `callgraph.get_slice` 不进入工具合同。
3. `scheduler.rs` 按 phase 校验完整 source slice 与准确 capability 集合，不仅依据角色名放行。
4. `agent_tools_source.rs` / `agent_tools_dispatch.rs` 验证真实 child/root/assignment、活动 attempt、lane、fence、capability 时效、冻结材料与发布 runtime。实际密钥变化也拒绝，不依赖过时的公开配置字段。
5. runtime 验证复用当前 IMMEDIATE 事务中的 `verify_source_runtime_in`，不调用自行开启事务的 wrapper。工具前后均复核；写入触发器造成撤权、runtime 或 revision 漂移时整体回滚候选与证据修订。
6. 源码工具要求根运行已 `running` 且有合法开始时间。每次执行根据发布的 `timeoutSeconds` 扣除持久已用时间；不能通过续租或恢复重置。空时间、不可解析时间、未来时间和已耗尽期限拒绝读取/写入；工具列表也不会继续广告这些工具。
7. 模型工具列表来自当前仍有效的逐工具 capability；没有 Web 工具混入，撤销后不继续广告。源码工具不消耗 Web request counter，也不因该计数达到上限而错误拒绝合法的本地读取。
8. scoped `assignment.finish` 目前只返回分析摘要，不是持久 assignment 终结。即使调用者提交空 gaps，仍保留 `source_review_not_completed`，不伪造审查完成。

历史 `tests_source_broker_scope.rs` 的 single-Web root fixture 只验证冻结文件行为。它的 fallback 和 revision helper 已限制为 `#[cfg(test)]`；生产没有以假 Web 身份执行 source 工具的回退。下列新增测试使用真实 source 发布与注册，不依赖该旧 fixture。

## 新增 8 项真实 Native 回归

位于 `src-tauri/src/commands/tests_source_tool_authority.rs`：

1. 真 source child 列清单、读取、提交/去重候选，绑定 root/child/revision；故意设置不可用的 Web plan，仍只能执行合法源码工具。
2. RepoMapper 不能提交候选；初评不能获得工具权限，工具 phase 不能冒充初评。
3. 通用 dispatcher 在 Web 请求计数达到上限时仍可读取 source，计数不增加；撤销后工具列表与执行都拒绝。
4. 篡改摘要/root/候选/role/phase/工具/revision/capability 均不能创建 assignment。
5. 暂停、取消、backend、assignment、租约、fence、lane、实际模型凭据变化均在每次读取时重查。
6. candidate INSERT 期间撤权、runtime/revision 漂移或总时限耗尽时，候选与修订整笔回滚。
7. Diff 的真正 Native child 仅能读取选中的 `app.py`；拒绝未选中文件、路径穿越和绝对路径，不能对越界文件提交候选。
8. root 未运行、空/非法/未来 started_at、总时限耗尽时不返回源码，也不广告工具。

## 验证记录

- 前序 session 68294：fixture 将 u8 schema_version 写为 999，编译失败；改为 255。
- 前序 session 13037：5 项中 2 通过、3 失败。发现并修复授权中的嵌套事务；另外修正 root plan 可查询但不是 Web plan 的断言，以及凭据漂移应修改实际 `modelProfiles[0].apiKey` 的 fixture。
- 前序 session 7428：修复后的 5 项通过，不覆盖后续修改。
- session 5740 已收取 exit 0：源码相关 **147/147**，101.60 秒；涵盖当时 6 项授权测试，不覆盖之后的期限/Diff 增量。
- session 78952 已收取 exit 0：当前新增 **8/8**，12.19 秒。日志 `/tmp/oviraptor-source-tool-authority-budget-focused.log`。
- session 60329 已收取 exit 101：严格 Clippy 发现测试先 Default 再设置字段的 `field_reassign_with_default`，完整 Rust 未启动。已改为结构初始化，没有禁用 lint。日志 `/tmp/oviraptor-source-tool-authority-clippy.log`。
- session 70892 已收取 **exit 0**：全 targets/features 严格 Clippy（`-D warnings`）通过，5.42 秒；全 targets/features 串行 Rust 主库 **1134/1134**（588.90 秒）、历史导入器 **30/30**（2.17 秒）通过，main target 0 项。日志 `/tmp/oviraptor-source-tool-authority-clippy-final.log`、`/tmp/oviraptor-source-tool-authority-full-final.log`。这是当前增量的完整验证，不借用前一增量的 1126/30。
- session 8376 已收取 exit 0：七组实际界面组件 **147/147**、TypeScript/Vite 构建通过；主 JS **830.15 kB** 的拆包警告保留。日志 `/tmp/oviraptor-source-tool-authority-ui.log`、`/tmp/oviraptor-source-tool-authority-build.log`。本轮没有变更前端源码；这些组件测试不是完整 Tauri IPC/安装包部署验收。
- session 1496 已收取 exit 0：localhost 浏览器回环 `passed=true`，匿名/对照采集 complete、身份隔离 true、8 个观测请求；断言跨源资源及跨源重定向未到达第二个本地服务。日志 `/tmp/oviraptor-source-tool-authority-browser.log`。
- 当前 `cargo fmt --all -- --check` 与 `git diff --check` 通过。

## 后续必须继续

后续验证收取：session 61232 已退出 0，主库 1146、历史导入器 30、fmt/严格 Clippy 通过；该结果覆盖多轮工具与专用续期，不覆盖随后根收口证明修改。最新收口代码及其独立验证见 `NEST_SOURCE_COMPLETION_PROOF_AUDIT.md`。

后续状态更新（2026-09-27）：第 1 项生产工具阶段与逐轮 journal 已接线，第 2 项持久 finish/mailbox/原子结算已实现但顶层恢复仍未完成；根收口已从两个初评改为四个 assignment。专用工具续期及负面路径 3 项定向测试通过，最终严格 Clippy/fmt 已通过，全量 Rust session 61232 仍待终态。真实回归发现的取消合同误用、单项工具撤权与拒绝部分写入问题及新验证结果见 `NEST_SOURCE_ROUND_EXECUTION_AUDIT.md`。以下条目和 1134/30 记录保留为当时的历史状态，不能据此声称后续增量全绿。

1. 生产自动调度 source tool phase，接入多轮真实模型请求/工具回执，保持单次传输、真实预留/结算、unknown 不自动重放。
2. 持久 assignment finish、mailbox、故障恢复及根任务阶段感知收口；不能把 broker 返回 summary 当作 assignment 已结案。
3. 独立 Source Reviewer 与本轮 root/current revision 的审查资格，真实裁决驱动 CI，不直接删除未审查 gap。
4. 美元预算/配置化定价/未知费用对账，人工指令消费与角色质询、真实聊天端到端、沙箱与工具供应、知识治理和资产汇总，以及安装包授权验收。

### 后续接线时已核实的约束

- `agent_specialist_calls` 目前以 `assignment_id` 为主键、`child_run_id` 唯一；`specialist.rs` 的调用和回执为单次无工具初评设计。不能只给 `specialist_round_transport` 加一个循环或删除工具拒绝判断：必须先设计逐轮不可变 journal、逐轮请求/工具调用 ID、真实 usage 与剩余预留、工具回执恢复及 unknown 不重放语义。不要覆盖已有初评回执或放宽其合同。
- `run_native_agent` 是 Web executor，依赖 Web attempt、Web execution plan、目标请求与 finish_target；不应给 source 填假 Web plan 来复用它。复用模型客户端/证据存储/工具 broker，新增独立源码执行上下文和阶段编排。
- `source_assessment_completion` 当前要求恰好两个已结算初评。加入 source tool phase 和 Reviewer 后，必须按冻结阶段计划验证所有 assignment/回执/预留/能力租约，再收口 root。
- 本轮时间检查是 **source coordinator 模型/工具阶段共用期限**，不是证明分析器和模型共享整个扫描墙钟预算。生产分析器目前仍使用 `ProcessLimits::default()`，且在 source coordinator 创建前执行。跨分析器、模型、复核阶段的整个 attempt 总时限仍需持久化统一起点并向执行器传递剩余额度，不能把本轮 8 项测试解释成该功能已完成。

上述缺口保留原 Master Plan 范围；本增量和整体目标均不得混同为“全部完成”。
