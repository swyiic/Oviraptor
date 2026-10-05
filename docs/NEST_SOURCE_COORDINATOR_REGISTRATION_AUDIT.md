# Source Coordinator 注册与恢复验收（2026-09-27）

## 状态与边界

后续更新：此文是注册增量完成时的历史快照。2026-09-27 后续已接入生产源码初评派发；当前状态、限制与新增验证见 `NEST_SOURCE_INITIAL_ASSESSMENT_DISPATCH_AUDIT.md`。下面“尚未调用”描述的是当时状态，不是最新状态。

本增量实现源码 Coordinator 的事务化注册/恢复入口，并让源码专家测试使用此入口，不再手工构造根运行。**生产启动器尚未调用此入口；源码自动模型派发、独立 Reviewer 和整个 Master Plan 仍未完成。** 暂时保留该入口的 staged/dead-code 标记，不能为了消除标记而在分析器返回后创建无人收口的 prepared 根运行。

未启用 Host Agent、未授予目标主机权限、未部署、未访问外部授权 URL。没有修改旧 JSON 的结构或赋予其执行权限。

## 修改内容

- `agent_runtime/multi_agent/source.rs`：抽出只读材料恢复。注册前可以核对本轮 scope、snapshot、analysis view、分析结果回执、源码计划；不必先造一个 Web 或 Source root 来获取材料。这不是工具授权。
- `commands/native_source_coordinator.rs`：`IMMEDIATE` 事务中核验实际发布合同及私有密钥绑定，再恢复上述材料。根 ID 只取 scan/attempt，材料变化不能换一个 ID 重领预算。
- 首次登记只执行 INSERT，禁止通用 `create_run` 的 conflict UPDATE。恢复按原始数据库字段严格比较 backend/role/root/parent/lane、计划、结果、完整冻结合同、预算、取消与终态。
- 已有用量、预留、heartbeat、child、调用回执均不改写。终态、暂停、取消、旧引擎或绑定改变返回冲突，不修复、不复活。
- 已存在另一源码 root 时拒绝接管/增建；Web sibling 不因本登记入口被改写。并发调用由 SQLite 事务串行化。
- 插入后复核初始状态、源码 root 唯一性、实时模型配置、发布合同和材料。写入被忽略、写入失败、触发器造成撤权/预算变化均回滚。
- token 上限来自已发布合同。单独冻结 8 次模型请求硬上限、4 次软上限；这不是 Web 目标请求额度，更不是美元费用结算。
- 注册本身不领取执行租约、不创建 child、不发送模型请求。后续执行器仍需独立预算准入、有效租约与逐次派发验证。

## 验收

新增 8 项回归：

1. 真实材料与已发布 runtime 绑定；恢复保留整个根行，注册不签发租约。
2. 终态、历史引擎、暂停、取消、角色/计划/证据/权限/预算变化不被覆盖。
3. 私钥缺失、暂停、删除、替代 attempt、结果回执缺失不生成 root。
4. 忽略/失败插入及写入后撤权、预算/用量/模型配置改变整笔回滚。
5. 两个独立数据库连接并发注册得到同一个 root。
6. 已记录但结果未知的模型调用恢复后仍禁止重新派发，预算预留不清零。
7. 自身摘要合法但已变化的结果回执不能生成第二个 root。
8. 不接管相同目标或旧材料目标下、使用旧名称登记的源码 root。

首轮 session 21368：4 通过/2 失败。两项均为测试注入问题：AFTER INSERT 的 RAISE(IGNORE) 不等于跳过 INSERT；模拟缺失结果先被不可变删除触发器拒绝。已分别改为 BEFORE INSERT、仅在测试损坏场景移除删除触发器；未放宽生产不可变约束。

最终验证均已收集终态，不再运行：

- 源码相关 session 62332：exit 0，133 通过/0 失败，86.02 秒；`/tmp/oviraptor-source-coordinator-source.log`。
- 全 targets/features 串行 Rust session 23857：exit 0，主库 **1118 通过/0 失败**（573.16 秒）、历史导入器 **30 通过/0 失败**（2.16 秒）；`/tmp/oviraptor-source-coordinator-full.log`。
- 严格 Clippy session 19242：exit 0，`--all-targets --all-features -- -D warnings`；`/tmp/oviraptor-source-coordinator-clippy.log`。
- 七组实际界面组件 session 63201：exit 0，**147 通过/0 失败**；`/tmp/oviraptor-source-coordinator-ui.log`。
- localhost 实际浏览器 session 15988：exit 0，`passed=true`，匿名/对照采集 complete，身份隔离 true，observedRequests=8；`/tmp/oviraptor-source-coordinator-browser.log`。
- 前端类型检查与构建 session 39468：exit 0；`/tmp/oviraptor-source-coordinator-build.log`。主 JS **830.15 kB** 的拆包警告仍存在，未通过提高阈值隐藏。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --all --check`、`git diff --check` 通过。

完整 Rust 运行期间未修改 Rust/构建输入或重启测试。上一轮 1110/30 是历史基线，本增量以这里的 1118/30 为准；通过回归不等于下面列出的生产功能已经完成。

## 必须继续完成

1. 真正接入生产启动器：注册、派发两类初评角色、持久化回执、mailbox 消费与根/分支收口必须是同一条可恢复生命周期。不要把本入口的测试通过视为生产已运行。
2. 实际模型请求输出限制和预留匹配，明确输入计数/不足处理；不可只在账本预留而给云模型发送无限输出请求。
3. 美元定价、费用预留/结算及未知结果对账。当前冻结 `maxBudgetUsd` 只代表操作者要求，不代表已执行限额。
4. 有效模型合同在真实派发点复核；恢复未知调用不得换 lease/fence 后重发。并发注册幂等不等于并发派发所有权。
5. 时间窗、局部/全局取消、费用与 token 耗尽、结果投递失败、根收口失败的真实生产回归。
6. SourceBroker 原先 Web-only 依赖、独立 Source Reviewer、当前 revision 的 CI 资格、真实聊天/UI 和授权环境验收。`source_review_not_completed` 不得提前删除。
