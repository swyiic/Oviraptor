# 预算账本只读差额盘点（2026-09-30）

## 范围

这是 Master Plan §5.4 的先行诊断，不迁移、释放、消耗或退款。当前 `agent_budget_ledger` 仍是每 root 一行的 token/request 可变汇总；不能把它解释成十维追加账本。

## 本轮落地

- 接入此前未编译的 `multi_agent/budget_gaps.rs` 草稿并修复元组索引错误。
- 在同一个 deferred 读事务内取 root 作用域、账本、所有 `budget_settled_at=''` 的 assignment 预留、Coordinator 租约、root/child 工具调用。将预留账本与 assignment 总量分别对账；缺账本返回未知差额 `None`，负差额表示少预留。
- 未结束的工具调用只计数量，不猜测花费；空 `contract_key` 只计不可归属数量。此报告无法识别崩溃窗口中没有持久 invocation 的网络请求或模型调用。
- 合并 token 总量不能拆分 input/cached/output；目标请求在独立计数链中，不是此汇总账本的维度。故十维向量仅 `model_requests` 有汇总级表示，九维仍缺完整账本。

## 验证及边界

`cargo test --offline -j 1 --lib budget_gaps::tests -- --test-threads=1`：3/3。`cargo fmt --all -- --check`、`cargo clippy --offline --all-targets --all-features -j 1 -- -D warnings`、`git diff --check` 通过。未跑全量 Rust、UI、安装态或真实 URL。

## 同日续做：按需诊断接线

- 新增独立的 `get_native_budget_diagnostics(scanId, attemptNumber)`：在一个只读快照中校验任务未删除且轮次为当前轮次，最多读取 50 个当前 Native Coordinator root，显式返回总数/截断标识；不将多次逐 root 查询塞进高频状态投影。
- 任务详情页的折叠面板只在展开时读取；展开期间以已提交的协同事件唤醒，15 秒兜底对账。收起、切换轮次或卸载后丢弃旧回包。诊断不会产生预算写入或改变调度准入。
- 新增生产入口的定向测试，覆盖当前轮次、其他任务、历史轮次、child 排除、删除隔离及只读性。原来的 staging `allow(dead_code)` 已移除；测试专用的独立快照包装仅在测试编译。
- 验证：新增生产入口定向 1/1，原差额测试 3/3；`npm run build`、`cargo fmt --all -- --check`、严格全目标全特性 Clippy、`git diff --check` 通过；低优先级单线程完整 Rust 库测试 **1517/1517** 通过（1576.73 秒）。仍未做安装态、真实 URL、端到端 UI 交互或 CPU 长稳测试。

仍**没有**持久 append-only entry、消费/预留事务接线、未知费用对账；不能恢复或继续未对账的未知请求。下一批需设计 10 维 schema 与幂等事件，并在同一 fenced 事务里接调度、模型/目标 Broker 和收口。
