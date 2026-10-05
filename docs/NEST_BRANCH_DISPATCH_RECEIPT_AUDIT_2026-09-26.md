# Native 分支持久派发凭据审计（2026-09-26）

后续增量：`NEST_WEB_DISPATCH_BINDING_AUDIT_2026-09-26.md` 增加普通 Web 启动配置绑定与派发事务前后复核；显式同 attempt 恢复仍未实现。后续全量运行发现子进程测试只断言非零退出不够严格，现保留 stdin writer 直到 wait 结束，并在 Unix 断言 SIGKILL，避免 EOF panic 被误计为强杀。下方 836／30／76 保留为当时快照，当前门禁以该后续审计为准。

## 1. 本轮交付与边界

本增量在普通 Web 启动原子性修复之上，补齐 **分支取得执行权之前的持久凭据**。新建 Web、源码分支共用入口；不恢复旧执行后端，不改历史 JSON，不增加主机能力，不触达外部测试 URL。

它解决一个具体缺陷：此前 `NativeBranchGuard` 只用 OS 文件锁和 pending 分支判断能否开始执行。进程退出释放文件锁后，数据库无法区分“从未取得执行权”与“已经开始、结果不明”。重复调用 launcher 可能再次启动同一分支。

**这不是完整的 startup 恢复器。** 本轮没有恢复按钮、启动时自动扫描重派、运行配置重建或外部未知结果对账，也不把新 attempt 的创建等同于同 attempt 恢复。

## 2. 实际数据及执行顺序

### 2.1 表与历史兼容

新增 `native_branch_dispatches`，主键 `(scan_id, attempt_number, branch)`，外键关联对应分支。只保存：

- 分支归属；
- `claim_id`：初始为空，成功领取时生成 UUID；
- `claimed_at`：初始为空，领取时写 UTC 时间。

不写模型密钥、Cookie、代理密码、身份文档、完整环境变量或启动命令。

不将旧 pending 分支回填成空凭据。旧数据不存在派发记录时，其执行历史是未知的，不能通过升级数据库获得重放许可。

### 2.2 登记

`register_native_branches_in` 在原调用者事务中同步登记分支和空派发凭据。普通 Web 的 task/target/attempt/branch/dispatch 在同一 startup 事务提交；源码/组合路径仍使用其已有分支登记事务，**不是完整源码入口已原子化**。

每次 INSERT 检查影响行数，最终检查完整分支集合及派发集合。ABORT、IGNORE、删除、提前填入 claim、改写分支状态都会拒绝交付并回滚。

### 2.3 领取

`NativeBranchGuard::claim` 的顺序是：

1. 取得原有 scan/attempt/branch OS 活调用锁。
2. 打开独立连接，将此连接 SQLite synchronous 设置为 FULL。
3. 开始 IMMEDIATE 事务。
4. 检查当前 attempt、scanning 状态、pending 分支、删除墓碑、关联项目活动状态、环境准备租约。
5. 仅对存在且两个 claim 字段均为空的记录，CAS 写入新 claim。
6. 核对数据库实际 claim 内容，并再次检查准入条件。
7. 成功提交后才构造有清理责任的 `NativeBranchGuard`，返回给执行器。

因此已有 Web 的模型资源策略调用、前端采集／浏览器和目标执行，及源码 worker 的运行，仍在公共领取入口之后。凭据提交失败不会构造 failure guard，不会把另一调用者的分支写成 failed。

FULL 是代码层的提交持久性要求；本轮没有进行硬盘断电或存储设备故障实验，不宣称覆盖这些硬件故障。

### 2.4 不删除或重置 claim

guard 释放、正常 Drop、取消／暂停、应用重启均不把 claim 清空。OS 锁释放只意味着活调用锁可用，不意味着已经发出的请求可以再次发送。

已领取分支再次 claim、缺少凭据的历史分支 claim 均拒绝，并保持任务／分支状态不变。不同分支仍独立领取。

正常完成状态依旧由分支完成回执表达；派发凭据不是完成证明，也不是实时心跳。不要将它解释成更高级别的精确一次外部执行保证。

## 3. UI 和 API 的真实含义

`get_native_scan_status` 在已有只读快照中 LEFT JOIN 派发表，返回每个分支的 `dispatch`：

| state | 含义 | 可否因此自动重派 |
| --- | --- | --- |
| `never_claimed` | 新入口登记过，尚未领取执行权 | 不可；仍缺少恢复时配置／授权校验 |
| `claimed` | 已持久领取，可能正在运行，也可能退出后结果未知 | 不可 |
| `legacy_unknown` | 缺少派发凭据，不能判断旧进程是否执行过 | 不可 |
| `invalid_receipt` | 字段不符合预期的防御性读模型 | 不可 |

返回 `claimedAt`，不返回内部 `claim_id`；每项 `automaticReplayAllowed` 都为 false。

`NativeRunStatus.vue` 在分支卡展示上述说明。旧 API 没有 dispatch 时显示“历史执行缺少派发凭据”，不会显示“从未执行”。刷新、挂载和轮询只调用状态读取 API，不会启动任务。原有终态／停机诊断不被 claim 状态替换。

## 4. 验证覆盖

新增 Rust `tests_native_branch_dispatch.rs` 共 8 项，其中一项是只供父测试调用的子进程入口；普通全量运行时该入口直接返回。有效断言覆盖：

- 释放活锁后 receipt 仍在；重复领取拒绝且不触发失败清理；另一分支仍可领取。
- 状态读取不写数据库，不暴露内部 claim UUID。
- 老库重新初始化不回填不存在的记录；历史分支不获重放许可。
- 分支登记 INSERT 的 ABORT/IGNORE/AFTER 删除、claim 篡改、branch 篡改完整回滚。
- 普通 Web startup 中新增登记失败时，task/targets/attempt/current surface 回滚，本次已知文件按原清理合同处理；成功时 receipt 和账本绑定同一 attempt。
- 领取 UPDATE 的 ABORT/IGNORE/AFTER 删除与篡改、批次变化、分支终止、准备租约变化被拒绝。
- 延迟外键导致 COMMIT 失败时，不返回 guard、不启动效果、不误报分支失败；后续一次新的合法领取仍可成功。
- 暂停、替换 attempt、项目归档、Web 项目丢失／删除、准备租约、墓碑阻止领取。
- **真实子进程强杀**：父进程等待具体子进程握手后 kill/wait。分别覆盖领取前、领取提交后、领取后写入一次临时本地副作用标记。无 Drop 的强杀后，前者保留未领取；后两者保留 claim 且重复入口拒绝。本地标记不重复。

强杀测试调用的是生产 `NativeBranchGuard`，不是 Tauri 完整应用、真实 browser worker 或真实目标。它证明领取凭据与锁释放的分界，不证明完整扫描已自动恢复。

新增 `tools/test_native_run_status.cjs` 编译实际 Vue SFC setup/template，6 项覆盖四种凭据状态、旧 API、转轮次后的迟到响应；所有读取均约束为状态 API，模板验证危险文本被转义。

## 5. 质量门禁

本轮日志前缀：`/tmp/oviraptor-20260926-dispatch-`。

- 初始定向 Rust：7 项通过；此后增加了 Web startup 集成回滚测试，最终以全量为准。
- 实际 Vue SFC：76 项全部通过（原 70 + 本轮 6）。
- 前端构建通过；主 JS 792.37 kB，拆包警告仍未解决。
- Native 本地浏览器回环通过：8 次请求，身份隔离 true。
- 最终严格全 targets／features Clippy 通过（5.50 秒）；`final-clippy.log`。
- 最终 fmt 检查通过；`final-fmt.log` 为空。
- 最终全 targets／features Rust：主库 **836** 项全部通过（223.68 秒），历史导入器 **30** 项全部通过（1.77 秒）；`final-full.log`。
- 最终 `git diff --check` 通过；`diff.log`。新增未跟踪源码、测试与本审计文件另查行尾空白，未发现问题。

执行命令：

```sh
cargo test --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1
cargo clippy --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
node --test tools/test_agent_dialog.cjs tools/test_agent_workbench.cjs tools/test_native_run_status.cjs
npm run build
node tools/test_native_runtime.cjs
git diff --check
```

初次 `full.log` 也是 836／30 全绿，但早于“Web 项目丢失／删除时禁止领取”的补充检查；当前代码验收以 `final-full.log` 为准。没有调整已有门禁、退休后端 fixture 白名单或降低断言。

## 6. 仍必须继续完成

1. **未领取 attempt 的显式同轮次恢复**：启动时冻结无明文密钥的运行配置引用、目标与策略、artifact 指纹；恢复时核对文件、模型配置、凭据句柄、项目、授权、租约和预算，配置变化不能被默默接受。
2. **已领取但无最终回执的恢复**：区分仍有活 owner 与 owner 已退出，逐项核对已发出的 browser/model/target 调用。不能把文件锁可用当作“没有外部效果”。
3. 新旧 attempt 间的通用未结算义务／终态对账；本轮防重仅限同一 branch/attempt 的入口，不是所有新 attempt 的跨轮防重已交付。
4. 源码、CI、组合任务整个启动过程原子化；当前共用派发表不能替代这项工作。
5. 进程强杀后的孤立私有文件识别与清理 journal；不增加任意 task_path 的递归删除。
6. Tauri 桌面进程强杀／IPC 丢失及真实完整 worker 的受控本地验收。
7. Master Plan 其他未完成项仍按实施进度记录继续，不能由绿色单轮门禁推定整体完成。

配置恢复的实际约束：当前 `ModelRuntimeEnv` 还包含 `api_base` 与 `api_key`，`AgentWebPipelineRuntime` 从当前设置重新计算代理池、noProxy、预算、packet budget 和技能。现有 task JSON 没有保存足以重建全部执行配置的合同。后续不能直接把这些结构序列化进派发表，也不能无提示地用更改后的全局设置替代原执行配置；须设计凭据句柄及绑定验证，显式拒绝不匹配，并单独验收后再提供恢复动作。
