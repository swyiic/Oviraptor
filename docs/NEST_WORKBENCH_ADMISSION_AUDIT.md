# 工作台异步准入失败与目标归属审计

状态：源码／灰盒／CI 可靠性增量。不是 Host Agent 实现，也不是完整派发绑定或重启恢复合同。

## 1. 实际问题与修复

线程创建成功不等于分支取得执行权。原源码与非冻结 Web worker 在 `NativeBranchGuard::claim` 失败后只写日志；对确实没有领取执行权的工作台分支，也可能留下 `pending`／`scanning`。

生产源码 launcher 和非冻结 Web 准入路径现在共用 `claim_workbench_pipeline_branch`：

1. 正常准入仍走原有 OS 调用者锁、数据库校验和持久化 dispatch claim。
2. 准入失败时，只尝试收口当前 `code/greybox/cicd` 工作台任务。普通 Web 的冻结 binding／人工恢复合同不受此路径改写。
3. 重新持有相同分支的 OS 锁，并在 IMMEDIATE 事务中检查当前 attempt、任务／attempt 状态、删除标记、分支状态及准确的空 claim 回执。
4. 只在这些条件满足时，通过同一个生产分支 reducer 记录 `failed`、`failurePhase=branch_admission`、`executionStarted=false`；不创建虚假 claim。
5. 分支、任务、attempt、该分支目标状态在同一事务收口；独立 sibling 分支、目标和 dispatch receipt 保持不变。另一分支仍 pending 时任务保持 scanning，全部失败才 failed，混合成功与失败为 partial。
6. 读取检查分支报告、目标状态、attempt 状态／checkpoint 和 sibling 未变。静默忽略或已覆盖的后写篡改使事务回滚；提交未知不自动重放。

模型／工具／目标调用都发生在成功领取执行权之后。准入失败记录使用固定错误代码，不把任意数据库错误中的输入数据写入报告。

## 2. 一并确认的问题

- 工作台发布的目标未显式设置当前 `last_attempt_number`。新建与重试现在在发布事务中写入，并纳入精确目标后置检查；不靠实际请求发生后再补归属。
- 分支结束但仍有 sibling pending 时，原 reducer 重写 `status=scanning` 会触发环境准备的激活门禁。现在这种情况只更新进度摘要，不重新写状态。结束为 failed／partial／completed 等时仍写真实终态；没有禁用或放宽环境准备门禁。
- 通用 `sync_sentinel_attempt` 的现有接口会吞掉写入失败；本增量在自己的事务内读取状态和 checkpoint 检查，防止此路径提交不一致结果。没有声称所有旧调用者都已补齐相同保障。

## 3. 不作“未执行”判断的情况

- 另一个线程持有相同分支的执行锁，包括重复启动。
- claim 已存在，即使原进程已经退出。
- claim 回执缺失、不完整，或 claim 提交返回结果未知。
- 任务暂停、停止、删除、attempt 已更换、已有结案回执，或普通 Web 任务。
- 数据库不可用或无法确认收口写入。

这些情况不能靠重写成失败来解除恢复门禁。原有证据和未知效果保留；需要各自的人工核对／恢复合同。

## 4. 验证记录

新增 `tests_workbench_admission.rs`，覆盖真实生产 helper 和 source launcher：

- 环境准备期间真实准入失败，只有未领取分支收口；对应目标与任务投影一致。
- 实际创建源码线程后拒绝准入，不产生源码快照或分析结果。
- 活跃重复调用及已释放的历史 claim 不被改写。
- 缺失／故意损坏的半条回执、提交未知、暂停／更换／删除及普通 Web 排除。
- 四张状态表的 ABORT／IGNORE 注入，各状态一同回滚，移除故障后正常收口。
- 后写伪造 claim、修改 sibling／目标、改写报告及更换 attempt 时回滚。
- sibling 仍 pending 时，attempt checkpoint 写入被忽略也能检测。
- 失败 source 不取消独立 Web；最终 partial 而非 completed；任意错误内容不进入报告。
- 真实两线程竞争 claim／收口，只有一个获胜；不靠时间睡眠确定赢家。

开发过程日志保留：首轮编译因 statement 尾表达式借用生命周期错误失败（`/tmp/oviraptor-workbench-admission-targeted.log`）；修正后 4 通过／3 失败，暴露目标 attempt 未写入、重复激活门禁及损坏回执 fixture 违反数据库 CHECK；接着 5 通过／2 失败定位到目标状态。损坏回执 fixture 显式关闭自身连接的 CHECK 以模拟损坏，不更改生产约束。修复生产目标归属／进度更新并加强后置检查后，工作台定向 19 项通过，见 `/tmp/oviraptor-workbench-admission-targeted-v4.log`。这些不是修改前红测证据。

最终验证已取得终态：

- 工作台定向 20 项通过，见 `/tmp/oviraptor-workbench-admission-targeted-final.log`。
- `cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1`：主库 999、历史导入 30 通过，零失败；见 `/tmp/oviraptor-workbench-admission-full.log`。同一顺序流水线 session 9647 已退出 0。
- 严格全目标／全功能 Clippy 与 fmt 检查通过，见 `/tmp/oviraptor-workbench-admission-final-clippy.log`、`/tmp/oviraptor-workbench-admission-final-fmt.log`。
- 任务详情根据真实 `never_claimed` 回执与严格结构化报告显示“未执行·准入失败”或“未执行·线程未启动”。已领取、回执损坏／缺失或报告不匹配时不显示未执行，不增加自动重试。新增 3 项实际 SFC 测试；七组 UI 测试共 146 项通过，见 `/tmp/oviraptor-workbench-admission-ui-final.log`。
- `npm run build` 通过，见 `/tmp/oviraptor-workbench-admission-build-final.log`；主 JS 828.82 kB，超过 500 kB 的拆包警告保留，不作为已优化。
- `node tools/test_native_runtime.cjs` 已退出 0，仅使用 127.0.0.1 的真实浏览器回环验证匿名捕获、只读请求、跨源阻止和身份隔离，见 `/tmp/oviraptor-workbench-admission-loopback.log`。

上述结果证明本增量的回归门禁通过，不证明下节剩余合同或整体 Master Plan 已完成。

## 5. 尚未完成

- 工作台准备到派发的完整配置／认证版本／工具链冻结与灰盒跨重启恢复，不能套用普通 Web 的绑定结论。
- 缺失回执、提交未知、数据库持续不可写、进程在收口前退出等情况没有被本增量自动解决。
- 其他旧状态写入路径的全量事务与后置检查、所有工具通道预算、后代进程停止及未知效果闭环仍需继续。
- 多角色剩余执行合同、知识与 skills 生命周期、资产分析、桌面端到端与授权环境验收仍按 Master Plan 推进。

本轮未部署、未访问外部测试 URL、未执行主机操作；没有以多个 Agent 同意代替人工授权。
