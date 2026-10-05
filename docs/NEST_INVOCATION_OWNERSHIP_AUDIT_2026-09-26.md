# 流水线／目标调用者所有权审计（2026-09-26）

## 本轮解决的问题

接续 `NEST_READONLY_RECOVERY_AUDIT_2026-09-26.md`：只读角色内部已能识别没有派发权的观察者，但实际外层入口 `run_agent_target` 仍可能把观察结果当作失败，进而结束另一个调用者正在使用的 Coordinator。更外层的重复流水线还会提前构造 `NativeBranchGuard`；即使立即退出，Drop 也可能把原分支写成失败。

本轮把**活调用者所有权**与可幂等复用的 Coordinator capability lease 分开。不是增加固定执行次数／时长门禁，也不是按“某个锁文件存在”推断进程还活着。

## 实现边界

### 1. 流水线入口先取得所有权

`NativeBranchGuard::claim` 在 Web／Source 工作线程入口执行，先于前端探测、模型本地资源策略、源码分析和失败清理守卫。

- 锁键是数据库规范路径下的 `(scan, attempt, branch kind, branch)`；Web 和 Source 各自独立。
- 取得 OS 文件锁后，还必须检查当前 attempt 活跃、分支已经登记且仍为 pending。
- 没有取得所有权就只记录 runner 日志、退出，不构造失败守卫，不发布分支终态。
- 源码尚在运行时，已经结束的 Web 分支也不能重新进入。
- 旧 attempt 的守卫退出仍受原有数据库 attempt fencing 约束，不覆盖新 attempt。

### 2. 单目标入口也有独立所有权

`run_agent_target` 在后端选择／计划冻结、目标 scanning 投影、SRC 接收器、root 复用与 `multi_agent_prepare` 前，取得 `(scan, attempt, target kind, target URL)` 的 OS 锁。

- 不活跃／被替换 attempt 拒绝进入。
- 同一 attempt 已有 terminal／legacy_backend_removed Coordinator 时拒绝重新执行。
- 未入场返回 `Err`，与 `AgentTargetOutcome` 分离；观察者没有一个可传给整体失败收口的终态值。
- `OwnedAgentTargetOutcome` 同时持有真实结果和锁；生产调用者完成目标投影后才释放，避免“执行函数刚返回、投影还没写”的竞争窗口。
- 去掉流水线中提前进行的重复后端选择；这项计划写入现在留在取得目标执行权之后。
- 若持有分支权的调用者遇到目标 admission 拒绝，记录明确诊断并 disarm 分支失败守卫，不把它误写成 `branch_worker_exited`。

### 3. 文件锁的限制

实现位于 `commands/native_invocation.rs`。使用现有 Rust 文件锁 API，无新依赖、无数据库 schema 变更；数据库符号链接规范化后使用同一锁目录，键使用带结构边界的 JSON 哈希，文件名不保存目标 URL。

锁目录／文件必须为常规类型；Unix 文件使用 `0600`、`O_NOFOLLOW`，拒绝文件／目录符号链接。锁文件不删除，避免已锁 inode 被替换后形成双持有者。正常 Drop 解锁；OS 会在进程退出时释放文件句柄。

**OS 锁空闲不证明上一轮目标请求／子进程结果已知，也不授权重发。** 恢复仍依赖持久化 dispatch receipt、现有 attempt/fencing、能力与请求账本。本轮没有实现进程崩溃后的完整恢复。

## 新增验证

`commands/tests_native_invocation.rs` 与 `commands/agent_tests_invocation.rs` 共 9 项：

1. 重复 Web 分支不持有失败守卫；Source 可并行完成，Web 终态不能重开。
2. 8 个线程同时 claim，只有一个持有者；失败者不改变 pending。
3. 旧 attempt 守卫退出不影响新 attempt。
4. 真实第二进程的竞争探针（主测试显式启动）。
5. 第二进程无法取得已持有的目标锁；正常 Drop 后可取得。
6. 数据库符号链接别名不能绕过锁；不同 scan／attempt／kind／target 独立。
7. 符号链接锁目录拒绝，目标目录无新文件。
8. 真实 `run_agent_target` + loopback 模型请求挂起时，重复调用在入场前被拒绝；比较 13 张运行／证据／预算／消息／任务表前后完全相同，模型仅收到一次请求。原调用的真实 provider 错误自行完成收口；锁跨越返回与投影仍有效，终态重入不再执行。
9. 暂停及替换 attempt 的目标入场无数据库写入。

这些证明对应活调用竞争和入口接线；没有把 mock 返回当成外部目标验收，也没有声称验证了断电／进程强杀后所有子进程的收敛。

## 退役后端门禁复核

第一次全量：765 项通过，1 项 REM-012 文件哈希检查失败。原因是测试调用适配了新的 admission 返回值，不是新增运行时后端。

对 `agent_tests_backend_residual.rs` 的 1 处、`agent_tests_e2e.rs` 的 3 处 `.unwrap().outcome`，在内存中移除仅这项改动后，SHA-256 分别严格等于此前登记的完整文件哈希。因此旧 CLI PATH 陷阱、零旧后端 run、历史兼容及 native 断言的其余字节没有改变。

仅更新这两条 fixture 的已审查哈希及说明；literal 数量仍为 17／9，扫描范围、规则、测试断言、类别和零活符号要求均未放宽。

## 最终质量验证

- `cargo test --offline --all-features --lib native_invocation -- --test-threads=1`：新增 9 项通过。
- `cargo test --offline --all-features --all-targets -- --test-threads=1`：最终重跑主库 **766**、历史导入器 **30** 项通过。
- `cargo clippy --offline --all-targets --all-features -- -D warnings`：通过。
- `cargo fmt --all -- --check`、`git diff --check`：通过。
- `npm run test:agent-dialog`：**38** 项通过。
- `npm run build`：通过；主 JS **777.36 kB** 分包告警仍在，未声称性能优化已完成。
- `node tools/test_native_runtime.cjs`：通过；本地浏览器 **8** 次请求、匿名／身份对照均完成、身份隔离通过。

以上不是发布包、真实 Tauri IPC 全链路／视觉验收或用户授权目标实测证明。

日志前缀：`/tmp/oviraptor-20260926-invocation-`，包含 `new`、`full`、`clippy`、`fmt`、`ui`、`build`、`native`。

## 尚未完成，不能外推的部分

1. 重复调用的 UI 观察／订阅、已完成 target 的只读结果重建及流水线 tally 收敛。现在异常的非拥有目标调用会留下明确 runner 诊断并停止本调用，不接管另一调用，也不伪造整个分支已经结束；对只剩历史 pending 的情况仍需完整恢复产品流程。
2. 进程崩溃、锁已释放但目标工具／子进程结果未知的通用恢复；新锁不替代未知回执、未结算成本和清理义务。
3. specialist 终止／过期后的通用本地对账 API/UI、Reviewer/Investigator 业务发布与恢复。
4. Stage 9 的 gap → 新合同 → 实际采集 → 新证据 revision → 独立再审闭环。
5. 其余角色、沙箱／工具供应运行时、前端拆包与视觉验收、部署包及授权 URL 实测。

Master Plan 仍未完成。本轮没有部署、访问用户目标或加入主机执行能力；`web_only` 边界保持不变。
