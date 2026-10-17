# 工作台源码／灰盒启动发布一致性审计

状态：启动发布增量；不代表整体 Master Plan、桌面端到端验收或完整恢复合同已完成。

## 1. 本轮确认的问题

- `start_workbench_scan_impl` 在 full-power 模式下把用户显式预算改成 `None`，输入检查也未拒绝非有限数。
- 原启动路径按 autocommit 分别写任务、绑定身份、记录 attempt、写上下文/目标/源码清单，再登记分支；后续失败可留下部分数据库状态。
- 后端矩阵在任务发布前通过独立数据库连接写入，发布失败不能一并撤销。
- 重试覆盖 scan 级 task JSON 和根目录认证文件，新尝试准备失败也可能改变旧尝试读取的材料。
- 源码线程使用可能 panic 的便捷 spawn；Web 线程创建错误会从已发布任务直接返回，未登记该分支的真实未启动结果。
- 工作台的 CI policy 未携带统一 Web mode、预算、选定 skills 和人工补充要求，运行时可能退回默认配置。

## 2. 实际改动

### 2.1 同一事务发布

生产入口调用 `publish_workbench_start`。该函数持有 IMMEDIATE 事务，包含：

1. 重新检查活动项目、项目名、删除标记、任务种类及 Native 计划与目标列表。
2. 重试检查项目/类型/源码路径/目标集合不变，旧任务状态可重试，新 attempt 不覆盖已有 ledger。
3. 写任务并原子绑定新任务的草稿身份；身份绑定失败撤销任务。
4. 写后端矩阵投影、attempt、上下文、目标、源码清单和实际运行 policy。
5. 检查熔断目标，不通过重试把熔断状态直接刷成 scanning。
6. 登记所有 source/Web 分支及 dispatch 槽位，复用生产分支登记后置检查。
7. 读取验证任务、attempt、上下文、精确目标集合、身份归属；静默忽略和已覆盖的后写篡改会返回错误并回滚。
8. 提交成功后才能创建执行线程。

这是数据库发布原子性，不是数据库与文件系统的分布式事务，也不是任务执行的 exactly-once 保证。源码清单仍使用原有投影写入；未将所有历史 findings 改造成 attempt 不可变存储。

### 2.2 文件和预算

- 复用经过路径/所有权检查的工作目录创建和独占 attempt 分配。
- 新 JSON 与认证文件分别存放在 `attempt-NNNN/task.json`、`attempt-NNNN/auth-session.json`。
- 新合同重试读取已发布 attempt 的认证文件，缺失时不降级到旧根文件；旧布局继续保留兼容读取。
- 已知提交前失败由目录所有权 guard 清理本次已知文件；不递归删除、不删除历史 attempt 或未知产物。
- 提交错误可能意味着结果未知，此时保留文件等待核对，不自动重放。
- 用户预算要求有限、正数且不超过 10000；full-power 不再将其抹除。统一 Web policy 将 mode/预算/skills/人工要求交给实际 runtime，CI 字段保留。
- CI 初始状态统一为 `not_evaluated`，不假报门禁已通过。
- 纯源码入口不解析 Web worker 路径，不因浏览器 worker 缺失而拒绝源码任务。

### 2.3 线程启动失败

- source 使用可返回错误的命名线程创建接口。
- 同步 spawn 失败记录失败分支，回执为 `failurePhase=thread_spawn`、`executionStarted=false`。
- 不生成虚假的 dispatch claim；其他已登记独立分支保留自己的状态。
- 启动入口读取发布后的当前任务状态返回，避免把已经同步失败的任务仍返回为初始 scanning。

这不包含线程创建成功后所有异步 claim 失败的恢复，也不代表浏览器后代/容器和目标未知效果已经完全收口。

## 3. 验证范围与失败记录

新增 `tests_workbench_startup.rs`，直接调用生产发布函数，未复制另一个测试用事务实现：

- 数字预算边界；真实 runtime 接收 mode/预算和 CI policy。
- source/Web 联合发布；纯源码发布。
- 7 张关键表各自 ABORT/IGNORE 故障，数据库回滚、身份不被消耗，移除故障后可正常发布。
- 项目归档、身份归属变化、目标熔断在提交时拒绝。
- 分支登记阶段发生任务/attempt/上下文/目标/身份篡改时回滚。
- 重试失败保持旧 ledger，拒绝改变任务种类。
- 两个数据库连接并发发布只有一个成功。
- 新 attempt 的失败文件清理不覆盖上一 attempt 的 task/认证材料。
- 分支 spawn 失败不假报执行、不影响另一个 pending 分支。

测试不包含真实目标、模型调用、浏览器网络或桌面 UI；worker 路径在发布测试中是本地占位数据。

首轮定向测试为 3 通过／3 失败：正常创建的 CI 上下文默认 `not_evaluated`，新后置检查最初错误地期待空字符串。已统一创建/重试初始值并保留日志 `/tmp/oviraptor-workbench-start-targeted.log`。随后 9 项通过，见 `/tmp/oviraptor-workbench-start-targeted-v2.log`。没有把该首轮失败称为修改前的红测证据。

最终新增并发和纯源码测试后的定向 **11 项通过**，日志 `/tmp/oviraptor-workbench-start-targeted-final.log`。首轮全量 **989 通过／1 失败**，日志 `/tmp/oviraptor-workbench-start-full.log`；唯一失败为退休兼容文件 allowlist 的 `agent_instruction.rs` 内容摘要变化。逐项核对后，确认该文件仍只有三处原有历史兼容字面量，没有增加执行、安装或 fallback 路径；变化是将只供测试使用的旧目录 helper 标记为 `#[cfg(test)]`。只更新这一条审阅摘要和原因，没有批量重生成基线。

修正审阅摘要后的全量已终态退出 0：主库 **990 通过**、历史导入器 **30 通过**，严格 Clippy/fmt 通过。日志分别为 `/tmp/oviraptor-workbench-start-verified-full.log`、`/tmp/oviraptor-workbench-start-verified-clippy.log`、`/tmp/oviraptor-workbench-start-verified-fmt.log`。前端实际 SFC **143 项通过**，`npm run build` 通过；JS **828.06 kB** 的拆包警告仍在，日志分别为 `/tmp/oviraptor-workbench-start-ui.log`、`/tmp/oviraptor-workbench-start-build.log`。

上述结果是启动发布增量的验证基线，后续工作台异步准入失败及目标 attempt 归属修复另见 `NEST_WORKBENCH_ADMISSION_AUDIT.md`，不能用本节旧结果代替后续修改的验证。

## 4. 仍需继续的工作

- 工作台/灰盒尚未接入普通 Web 的冻结工具链 binding、跨重启派发闭包与人工结案合同；不自动套用普通 Web 的恢复结论。
- 生命周期锁与旧 worker 静默检查仍由入口持有；本轮发布函数的单测不代替完整 Tauri 调用链及真实进程故障验收。
- 认证文档准备与提交之间的内容版本变化、所有准备阶段配置冻结、进程崩溃后的未发布文件回收仍需更完整合同。
- 源码清单读取目前在写事务中，需评估大型仓库的锁占用，并在保留快照一致性的前提下优化。
- 所有执行通道预算、浏览器后代退出、未知效果与重启恢复还不能宣称全部闭环。异步 claim 失败中已证明未领取的工作台分支由后续准入增量处理；已领取／缺失回执／提交未知不据此推定未执行。
- 其余专家角色、知识/资产/skills 生命周期、UI 布局、打包和授权环境验收仍按主计划推进。
- 前一轮出现过的 `web_binding_inputs_changed` 偶发失败根因仍未证明；这次启动修复没有声称解决它。

未部署、未测试外部目标、未启用 Host Agent；原有授权边界不因多 Agent 协商而取消。
