# 源码工具轨迹与发送者展示审计（2026-09-27）

本增量修复实际轨迹读取及源码角色标签，不新增权限、不执行外部扫描，也不代表完整聊天或独立 Reviewer 已交付。

## 发现与修复

1. `native_agent_trace.rs` 原先只识别 Web 事件的 `tool` / `invocationId`，而真实源码回执使用 `call.name` 和 assignment/round/index，导致源码工具名为空、调用标识为空。
2. 源码工具同步执行和完成回执在同一事务提交，没有独立开始事件。原轨迹只统计 started，造成真实成功入口显示 0 calls / 4 results。现在依据源码完成回执统计实际调用，不补造 started 事件。以 assignment、round、call index 生成稳定调用标识；相同模型 call ID 不会跨轮或跨角色合并。若同坐标已有 started，不重复计算调用。
3. 四次工具调用包括两次 `repo.inventory` 和两次 `assignment.finish`。轨迹忠实展示全部四次；报告中的成功数据工具仍为 2，finish 不因此变成漏洞证据。
4. 结果邮箱的 `run_id` 是根协调者，而不是发送者。现通过 `from_run_id` 核对 backend、scan、attempt、target、root 和 role 后展示实际发送者及 child session；绑定不匹配则显示 `unknown`，不借用其他任务身份。旧记录没有 from_run_id 时只保留其已记录的 from_agent 标签，不编造 child identity。该显示字段不签发执行或审查资格。
5. `AgentDialog.vue` 和 `NativeRunStatus.vue` 添加 RepoMapper / SourceAnalyst 中文与英文标签；真实角色不再以原始枚举字符串展示。源码结果仍显式保留独立复核未完成。

原协作时间线的 mailbox 查询本来就读取 from_agent/from_run_id；此次修复的是另一条 Native Trace 查询，不能宣称所有聊天通路此前都把发送者记作 Coordinator。

## 回归与证据

- 上一增量 session 58086 已 exit 0：主库 1147、历史导入器 30、fmt/严格 Clippy；session 10887：UI 147、构建和本地浏览器回环通过。均为本增量之前的基线。
- session 67826 exit 0：源码派发 9 项及源码调用身份 1 项通过。真实生产用例保留 6 次 localhost 模型请求、4 个 assignment/ACK、120 tokens；新增断言验证 trace 4 calls / 4 results、两个工具各 2/2、4 个唯一调用标识、实际专家发送者，以及错误 sender/root/role/attempt/target 的降级。此结果之后补充了 Web 配对事件与旧邮箱兼容断言，最终须由下述全量覆盖。
- 首轮 UI session 22725 exit 1：新增测试替换了错误的 mock API，缺少实际模板需要的 stopDiagnostic。修正测试使用生产读取方法 getNativeScanStatus 和完整诊断字段，没有放宽生产模板。
- session 99923 exit 0：真实 SFC 的两组测试 92 项通过，包含源码专家聊天和运行卡片展示；不是 IPC/安装包验收。
- 最终 session 75619 已收取 exit 0：fmt 与严格 Clippy（8.81 秒）通过；全目标全特性 Rust 主库 **1148**、历史导入器 **30** 全部通过。此结果是后续 execution history 增量之前的基线，不覆盖后续修改。
- 最终 session 50568 已收取 exit 0：七组 UI **149** 项、前端构建、本地浏览器回环（8 请求）与 git diff --check 通过。主 JS 830.35 kB 拆包警告保留；此后文档更新又单独执行 git diff --check 通过。

日志位于 `/tmp/oviraptor-source-trace-{dispatch,identity,ui,ui-final,ui-all,fmt-final,clippy-final,full-final,build,browser,diff}.log`。

## 剩余工作，不能视为完成

- 独立 Source Reviewer 的 assignment、真实模型回执、不可变裁决、当前 candidate revision/作者独立性核验、mailbox/ACK 和 CI 资格尚未生产接入。
- 当时 `native_attempt_tool_history` 只查询 `tool_invocations`，不覆盖源码独立回执。后续已新增版本化 execution history API 并切换任务详情入口，旧接口保留兼容；实际验证与边界见 `NEST_SOURCE_EXECUTION_HISTORY_AUDIT.md`，不能用本文件的旧基线替代后续验收。
- Native Trace 仍有 800 条预览上限；完整历史应使用明确的分页合同，不声称当前预览等于完整聊天记录。
- 源码顶层恢复、未知调用核对、美元费用账本、跨 analyzer/model/review 统一持久期限及 Master Plan 其余验收继续保留。
- 未部署，未访问用户外部 URL，未新增主机 Agent 或扩大 web_only 权限。
