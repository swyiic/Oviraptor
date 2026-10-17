# Source 共用阶段证明与历史只读审计（2026-09-27）

## 范围与结论

本增量修复两个实际复现的问题：Reviewer 前置审计漏查部分已完成阶段的 ACK、用量和任务绑定；已结束扫描的回执审计错误依赖活动 attempt。不是新增执行能力，也不是 Finding/CI 消费或整个 Master Plan 已完成。

## 共用阶段证明

`src-tauri/src/agent_runtime/multi_agent/source_phases.rs::audit` 是命令层根收口、Reviewer 准入及 Reviewer 交付审计共用的四阶段证明。必须在调用者事务内运行，不修改数据库，不续租，不补写损坏记录，不调用模型。

证明从指定 attempt 的冻结材料重建预期 slice，不能把 assignment 内的现存 JSON 自己当成“正确答案”。核验内容包括：

- 非 Reviewer assignment 恰为四份，每个 RepoMapper/SourceAnalyst 分别有初评和工具阶段；角色、阶段及 revision 不可重复或替换。
- 全量 task slice、冻结材料摘要、工具集合、零目标请求权限。
- assignment/root/child/scan/attempt/target/lane、lease epoch/fence、终态、无取消、已结算且无保留预算。
- 无残留 lane lease 或未撤销 capability。
- 初评真实模型回执；工具阶段完整 journal、transcript、工具输出和用量证明。
- 结果消息的发送者/接收者、root、角色、kind、revision、correlation、dedup、唯一性、已交付及 ACK、精确 payload。
- child 用量与真实回执精确一致；费用和工具结果数使用溢出检查累计。

根收口额外保留原有 Reviewer、总预算 ledger、child/call/消息计数及终态写后证明。Reviewer 不因复用阶段证明而获得 Web、shell、网络或主机能力，模型请求上限与旧合同保持不变。

## 历史读取与执行授权分离

`source::historical_materials` 只从指定 scan/attempt 的 scope 合同、快照元数据、选中文件视图、分析器结果回执及冻结计划恢复证据；仍检查摘要和实际冻结文件。它不以当前 scan 的路径或最新 attempt 代替旧材料，也不要求原始工作仓库仍存在。

运行路径 `source::restore_materials` / `source::task_slice` 继续检查活动 attempt、当前扫描配置、原始目录与可执行 Coordinator。通用 scheduler、Reviewer prepare、实际 transport、首次裁决发布及交付提交的运行授权检查未被移除。

只读历史审计成功，不等于允许恢复任务、重发调用、重新创建缺失裁决或把旧 attempt 的决定投影到新 attempt。冻结视图/回执丢失或损坏仍拒绝审计；没有从可变历史或原仓库自动重建。

## 回归证据

所有模型请求均为 localhost 夹具，无外部扫描、部署或主机操作。

- `/tmp/oviraptor-phase-proof-red.log`：新损坏测试真实失败。初评/工具 ACK、交付次数、消息来源、费用、attempt/fence、残留权限等损坏被原 Reviewer 两个入口接受。
- `/tmp/oviraptor-phase-proof-green-2.log`：共用证明后定向测试 1/1 通过。
- `/tmp/oviraptor-phase-history-red-2.log`：新历史测试真实失败，结束扫描后报 `agent_attempt_not_active`。
- `/tmp/oviraptor-phase-history-green.log`：历史只读路径接入后 Reviewer 14/14 通过，59.51 s。
- 随后扩充：四个角色/阶段组合分别损坏用量、parent、lane 和取消状态；合计 30 种损坏，两个审计入口均须拒绝且不写入。历史测试增加原仓库移动、冻结文件缺失、跨 attempt 身份，并调用真实公共 scheduler / prepare 验证不能重启。
- 最新扩充版 `source_` 筛选回归：session 41212，exit 0，主库 **189/189**（208.92 s）、历史导入器相关 **2/2**（0.22 s）；日志 `/tmp/oviraptor-shared-source-proof-regression.log`。不是整个工程全量，包含上述两个新增/扩充测试。
- 严格 Clippy：session 26961，exit 0，`--features import-tools --all-targets -j 1 -- -D warnings`；日志 `/tmp/oviraptor-shared-source-proof-clippy.log`。fmt 和空白检查通过。

保留的非验收失败：第一次共用模块提取的补丁顺序错误导致编译失败（`/tmp/oviraptor-phase-proof-green.log`）；历史测试首版调用了不可见的内部 scheduler API，编译失败（`/tmp/oviraptor-phase-history-red.log`），已改为调用真实公共入口。没有扩大 API 可见性或放宽生产约束使测试通过。

## CPU 控制与观测

每次 Cargo 均 `nice -n 15`、`-j 1`，Rust 测试 `--test-threads=1`，编译/测试不叠加。`/tmp/oviraptor-shared-source-proof-cpu.log` 对本次源码回归每两秒采样主测试进程 CPU、nice 和线程数。采样不是 CPU 硬限制，不能推断两次采样之间或子进程的峰值；不能以本次结果倒推先前 360% 的唯一原因。

监控 session 49059 已 exit 0，随 Cargo 退出结束。共 98 次测试进程采样，UTC 08:55:59–08:59:13（上海 16:55:59–16:59:13），CPU 采样最大值 **99.9%**、线程数 **2–4**；监控开始前的短时间没有采样。本轮未重新运行全工程 Rust、UI 或浏览器，不能引用前序 v2 全量作为本增量证明。

全部测试与 Clippy 退出后再次检查，未发现残留 `oviraptor_lib`、Cargo 或 rustc 进程；最终文档更新后的 fmt/空白检查通过。

## 下一步，不得跳过

2026-09-27 后续状态：下列第 1–2 项的后端类型化消费者、报告投影和 CI 原子收口已实现，源码 192/2、CI 26 项回归通过；统一 Findings UI/读 API/源码导出尚未完成。当前边界与精确证据见 `NEST_SOURCE_DECISION_CONSUMER_AUDIT.md`。下列列表保留为本审计发布时的后续要求，第 3–4 项仍待完成。

1. Finding/CI 增加真实审查后的同 attempt 事务投影，消费完整、已审计的类型化源码决定；不能复用 Web 数字 revision 来冒充源码身份。
2. 保留 analyzer 阶段冻结的原计划及未审查 gate 历史。候选审查与总体覆盖资格分离，不能只删 gap 换绿色。
3. 分类顶层恢复状态，已收回执只做本地交付；在途/未知调用不能自动重发或退款。
4. 完整工程、UI/浏览器及所有 Master Plan 条款仍须最终验收；前序 v2 全量不证明当前 v3 完成。
