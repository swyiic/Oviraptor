# 普通 Web 未派发尝试人工结案审计（2026-09-26）

## 1. 交付范围

补齐普通 Web 任务的操作闭环：启动后尚未领取派发权，模型／工具／身份／配置等输入已经变化，不能恢复旧冻结计划时，操作者可以明确结束旧尝试，然后另行启动经过当前启动检查的新尝试。

这不是扫描执行失败后的通用恢复器，不支持已派发、效果未知、补证任务、跨对象授权控制、源码／CI／混合任务，也不扩展到主机操作。原目标与 Master Plan 仍未全部完成。

## 2. 实际复现与修复

1. 新增后端正向回归最初 2 项失败：没有安全结案入口。日志 `web-closure-red.log`。
2. 实际 Vue SFC 新增结案交互回归最初失败：没有结案函数。日志 `web-closure-ui-red.log`。
3. 中间实现通过结案后，新增同步兼容回归发现 `sync_sentinel_attempt` 会覆盖已明确结案的 stage／更新时间。该轮 5 通过／1 失败，日志 `web-closure-sync-red.log`。
4. 现在历史进度同步跳过有结案回执的 attempt，通用 scan 进度更新也不能重新激活当前已结案 attempt。公共派发资格检查独立拒绝有结案回执的分支，即使其他路径把状态错误改回 scanning／pending，也不能重新派发。
5. 第一轮全量 Rust 为 880 通过／2 失败：新增 `cancelled` 展示文案后遗漏 `presentation.ts` 的退役门禁摘要复核；新尝试的正向测试夹具缺少必需启动文件。人工审阅历史标签映射后更新摘要，补齐测试中的 `targets.txt` 与 `agent-instruction.md`，未扩大豁免或放宽生产检查。日志 `web-closure-full.log` 与 `web-closure-retirement-review.log` 保留失败记录。
6. 扩展正向测试时还发现启动绑定验证必须处于事务内；测试改用事务调用真实验证入口，没有削弱接口要求。最终专项回归 7 项通过，日志 `web-closure-targeted.log`；该日志为最终重跑结果，不是此前失败的原始记录。

上述日志均在 `/tmp/oviraptor-20260926-` 前缀下。临时日志不是永久验收档案；可由本文件所列命令重新验证当前源码。

## 3. 后端合同

入口：`close_never_dispatched_web_attempt`，核心实现 `src-tauri/src/commands/web_dispatch_closure.rs`。

- 必须明确确认，并绑定精确 scan ID 和 attempt number，旧页面不能关闭新尝试。
- 获取 scan 生命周期锁和该 attempt 的原生 Web 分支锁；不创建会在 Drop 时写失败状态的派发 guard。
- 使用 FULL 同步等级及 IMMEDIATE 事务。首次结案要求普通 Web 当前扫描、活跃项目、只有一个 pending Web 分支、真实空派发回执和既有启动绑定。
- 不读取或修复启动文件，不下载／运行工具，不使用身份，不向目标或模型发请求。配置失配不被放宽成可恢复；结案仅结束已证明未执行的旧尝试。
- 有当前 attempt 的 Agent run、进程登记、未确认容器清理、费用／请求增量、目标执行进度、补证／授权控制等情况时拒绝。项目已归档、软删除或环境准备中也不在本入口支持范围内。
- 同一事务写入唯一 `native_web_attempt_closures` 回执、分支结案报告、attempt 与 scan 终态。每次写入必须恰好影响一行，最后复核全部状态、时间、报告和未执行条件。
- 回执 `closed_at` 使用 UTC RFC3339；另存 `terminal_timestamp` 延续既有 SQLite 本地时间排序格式，避免历史列表混用时间格式。重复核验使用已保存的时间，不依赖核验时的时区。
- scan／attempt 标记 cancelled；分支沿用现有终态枚举 failed，但附精确的 `operator_closed_before_dispatch` 报告。它不是目标测试失败，更不是漏洞验证结果。UI 与诊断使用专门的“未执行·人工结束”文案。
- 以写入前的内容摘要核对必须保留的 scan／attempt 字段、全部目标、原派发凭据和启动绑定；不清空累计费用、历史计划、旧目录或 claim。
- 回执禁止 UPDATE；任务删除引发的关联删除仍受原任务删除流程控制。摘要属于业务一致性校验，不是抵抗数据库管理员任意篡改的安全证明。
- 忽略写入、SQL 中止、后续触发器改坏数据、延迟外键在 commit 时失败均回滚，不返回成功、不自动重放。
- 对相同当前 attempt 的重复结案只核验已有回执与保留字段，返回同一个 ID／时间；不重复修改、不关闭新 attempt。已换新 attempt 或回执无法核验时拒绝。

关闭旧尝试与启动新尝试是两个独立人工动作。现有 Web Retry 启动入口仍负责当前身份、目标、熔断、政策、工具和冻结计划检查。结案不预先承诺下一次启动一定被允许，也不解除任何安全／授权条件。

## 4. UI 行为

`NativeRunStatus.vue` 提供“结束未派发尝试”与二次确认面板。服务端 `manualClosureAvailable` 仅为提示，客户端还核对 scan／attempt、scanning、pending 和 never_claimed；最终授权判断始终由后端完成。

- 打开面板与确认是两步；刷新／轮询不执行结案、恢复或重试。
- 结案与恢复互斥、各自单次在途；切换确认面板会清除另一个面板。
- 切换任务／attempt／状态及卸载组件后，旧异步回执不能修改当前视图或触发父组件事件。
- 成功回执必须匹配 scan／attempt、结案 ID、有效时间、未执行终态及禁用自动重放标记。
- IPC 超时／异常只重新读取回执，不自动重发、不自动启动新任务。
- 终态服务端回执会停止组件轮询，即使父组件尚未更新 props。
- 父组件结案事件只刷新数据库视图，不调用 rescan、恢复或身份自动续扫。继续扫描需要用户另行选择“重试未完成阶段”。
- 没有新建虚假的 Agent 对话或模拟执行事件；这里记录的是操作者结案和真实分支报告。

手工审阅 `SentinelBoard.vue` 的历史后端标签与新事件处理，以及 `presentation.ts` 的历史后端标签和新增取消状态文案后，更新了两者的退役门禁摘要。首次全量遗漏后者导致门禁失败，复核后专项门禁通过；没有扩大扫描豁免、放宽运行时后端选择或恢复旧执行能力。

## 5. 回归覆盖

新增 Rust 回归 7 项：

1. 配置改变导致旧尝试恢复失败；结案不动旧文件／绑定；另行使用真实 Web startup 与新 runtime 绑定创建 attempt 2；旧 attempt 的迟到操作拒绝。
2. 明确确认、精确 attempt、生命周期锁和分支锁。
3. 历史同步与迟到通用进度不能覆盖结案；回执阻止错误重新激活后的派发。
4. 已领取／缺失回执、历史未知、模型费用、目标进度、已有 Agent、进程／清理、项目／任务失效、复合类型／补证／授权控制等拒绝矩阵。
5. 四类写入的 ABORT／IGNORE、13 类末尾破坏及 commit 延迟外键失败，全部业务记录回滚、锁释放。
6. 结案与原始派发、结案与人工恢复各只能有一个赢家；释放锁不允许重放。
7. 累计请求／Token 原值保留、状态查询无写入、专用诊断／真实回执、回执更新禁止。

新增真实 Vue SFC 回归 13 项：两步确认、4 类不可关闭派发状态、恢复／结案双向互斥、迟到成功／失败、确认期间状态变化、模糊 IPC 结果、组件卸载、6 种非法成功回执。原有组件用例保留。

## 6. 验证状态

最终全量进程已收取退出码 0：Rust 主库 **882 通过／0 失败**（318.08 秒）、历史导入器 **30 通过／0 失败**（3.25 秒），其他目标 0 项／0 失败。结案专项 7 项通过。严格全 targets／features Clippy、fmt 检查均通过，组合进程退出码 0。

实际 Vue SFC **101 通过／0 失败**；Native localhost 浏览器回环通过（8 个请求，匿名与对比采集 complete，isolatedIdentity=true）；前端构建通过，对应组合进程退出码 0。主 JS **800.39 kB**，仍有超过 500 kB 的拆包警告，不算作已优化。文档更新后的 `git diff --check` 通过，退出码 0。

本轮命令：

```sh
cargo test --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features
cargo clippy --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
node --test tools/test_agent_dialog.cjs tools/test_agent_workbench.cjs tools/test_native_run_status.cjs
node tools/test_native_runtime.cjs
npm run build
git diff --check
```

日志前缀 `/tmp/oviraptor-20260926-web-closure-`。最终全量依据是 `verified-full.log`，严格检查是 `verified-clippy.log`／`verified-fmt.log`，另有 `targeted.log`、`ui.log`、`native.log`、`build.log`、`diff.log`。`full.log` 是第一轮 880 通过／2 失败的历史记录，不应引用为最终通过证据。

## 7. 仍未完成

- 已派发／效果未知任务、跨 fencing／终态的人工对账和恢复。
- 源码／CI／combined 完整启动事务、运行中撤权与跨层收口。
- Stage 10 其余真实专家执行链、协同 UI 全面整理、资产与知识／skills 生命周期。
- 沙箱、不可变工具供应、性能、桌面强杀／断电、安装包与真实授权环境验收。
- Web→主机结构化动作边界与未来独立主机审批。当前不新增 Host Agent；关键词或 `web_only` 标签不代表完整语义隔离。

所有本轮测试仅在临时数据库／文件和 localhost 上执行；未部署，也未向用户提供的外部 URL 执行扫描。
